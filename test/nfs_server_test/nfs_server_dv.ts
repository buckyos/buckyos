/**
 * nfs-server DV smoke test (service-dv-test skill, Stage 8).
 *
 * Verifies the REAL request chain against a running zone:
 *   this script → gateway (https://<zone>/nfs/v1/*, the NFSP zone-level
 *   protocol path — NOT /kapi/<service>) → nfs-server → $BUCKYOS_ROOT/data.
 * NFSP is a cross-zone protocol rooted at /nfs/v1 (NamedFileSystem_Protocol_v0
 * §4.1); the gateway forwards it verbatim (boot_gateway.yaml).
 *
 * Covers: zone login (environment sanity), hello negotiation, root listing
 * (cyfs:/// = $BUCKYOS_ROOT/data), mkdir → upload → read → list → delete
 * round-trip, and the basic error paths (session gate, NOT_FOUND).
 *
 * Note on auth: nfs-server currently runs WITHOUT per-request authentication
 * (the whole tree is visible by design for this milestone). The zone login
 * here validates the test environment and keeps the script shape ready for
 * the auth milestone; requests to nfs-server itself carry no token yet.
 *
 * Run: cd test/nfs_server_test && pnpm install && pnpm test
 * Env:
 *   BUCKYOS_TEST_ZONE_HOST   zone host (default test.buckyos.io)
 *   NFS_SERVER_URL           override base URL, no path suffix — the client
 *                            appends /nfs/v1/* (skips zone-host derivation)
 *   NFS_SMOKE_SKIP_LOGIN=1   skip zone login (direct smoke against NFS_SERVER_URL)
 *   NFS_SMOKE_ROOT           export root to write into (default: home, else first)
 *
 * For protocol-level coverage run the standalone suite instead:
 *   test/test_nfs_server/run.sh (46 cases, no zone required).
 */
import {
  NfspClient,
  NfspError,
  type WireRef,
} from "../../src/frame/desktop/src/api/nfsp_client.ts";

function getEnv(name: string): string | null {
  const v = Deno.env.get(name);
  return typeof v === "string" && v.trim().length > 0 ? v.trim() : null;
}

const SKIP_LOGIN = getEnv("NFS_SMOKE_SKIP_LOGIN") === "1";

let zoneHost = getEnv("BUCKYOS_TEST_ZONE_HOST") ?? "test.buckyos.io";
if (!SKIP_LOGIN) {
  const { initTestRuntime } = await import("../test_helpers/buckyos_client.ts");
  console.log("[auth] logging in to zone (environment sanity)...");
  const rt = await initTestRuntime();
  zoneHost = rt.zoneHost;
  console.log(`[auth] ok (user=${rt.userId}, zone=${zoneHost})`);
}

const BASE = getEnv("NFS_SERVER_URL") ?? `https://${zoneHost}`;
console.log(`[probe] nfs-server via gateway: ${BASE}/nfs/v1/*`);

let passed = 0;
const failures: string[] = [];

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(`assert failed: ${msg}`);
}
async function test(name: string, fn: () => Promise<void>) {
  try {
    await fn();
    passed++;
    console.log(`PASS ${name}`);
  } catch (e) {
    failures.push(`${name}: ${e instanceof Error ? e.message : e}`);
    console.log(`FAIL ${name}: ${e instanceof Error ? e.stack : e}`);
  }
}

const c = new NfspClient({ baseUrl: BASE });

await test("hello: reachable through gateway, protocol negotiated", async () => {
  const r = await c.hello();
  assert(r.version === "nfsp/0", `protocol version, got ${r.version}`);
  assert(r.session.length > 0, "session issued");
  assert(r.features.includes("watch.sse"), "features advertised");
});

let rootNames: string[] = [];
await test("list /: cyfs:/// exposes $BUCKYOS_ROOT/data directories", async () => {
  const roots = await c.list("/", undefined, ["base", "ident"]);
  rootNames = roots.entries.map((e) => e.name);
  console.log(`  export roots: [${rootNames.join(", ")}]`);
  assert(Array.isArray(roots.entries), "root listing is a list");
  // A provisioned zone always has at least the standard data/ subdirs
  // (home/, srv/, var/, cache/ per bucky_project.yaml data_paths).
  assert(roots.entries.length > 0, "at least one export root visible");
  for (const e of roots.entries) {
    assert(!e.name.startsWith("."), `dot-dir must stay hidden: ${e.name}`);
  }
});

const smokeRoot = getEnv("NFS_SMOKE_ROOT") ??
  (rootNames.includes("home") ? "home" : rootNames[0]);
const smokeDir = `nfs-dv-smoke-${Date.now().toString(36)}`;
const smokePath = `/${smokeRoot}/${smokeDir}`;

await test("write round-trip: mkdir → upload → read → list → delete", async () => {
  assert(smokeRoot, "a writable export root exists");
  await c.mkdir(smokePath);
  const parentRef = (await c.resolve(smokePath)).ref as WireRef;

  const content = new TextEncoder().encode(`dv smoke ${new Date().toISOString()}`);
  const committed = await c.uploadFile(parentRef, "hello.txt", content);
  const nodeId = (committed.ref as { node_id: string }).node_id;

  const back = await c.readFile(nodeId);
  assert(back.status === 200, `read status ${back.status}`);
  const bytes = new Uint8Array(await back.arrayBuffer());
  assert(
    bytes.length === content.length && bytes.every((b, i) => b === content[i]),
    "content survives the round-trip",
  );

  const listing = await c.list(smokePath);
  assert(listing.entries.some((e) => e.name === "hello.txt"), "file listed");

  // Cleanup keeps the test repeatable.
  const rootRef = (await c.resolve(`/${smokeRoot}`)).ref as WireRef;
  await c.delete(rootRef, smokeDir, { recursive: true });
  try {
    await c.resolve(smokePath);
    assert(false, "deleted dir must not resolve");
  } catch (e) {
    assert(e instanceof NfspError && e.code === "NOT_FOUND", `expected NOT_FOUND, got ${e}`);
  }
});

await test("error paths: missing path, session gate", async () => {
  try {
    await c.resolve("/nope-does-not-exist");
    assert(false, "expected NOT_FOUND");
  } catch (e) {
    assert(e instanceof NfspError && e.code === "NOT_FOUND", `expected NOT_FOUND, got ${e}`);
  }
  // A client that never said hello has no session → clean protocol error,
  // not a 500. (Real per-user auth arrives with the auth milestone.)
  try {
    await new NfspClient({ baseUrl: BASE }).list("/");
    assert(false, "expected PERMISSION_DENIED without a session");
  } catch (e) {
    assert(
      e instanceof NfspError && e.code === "PERMISSION_DENIED",
      `expected PERMISSION_DENIED, got ${e}`,
    );
  }
});

await c.bye().catch(() => {});

console.log(`\n${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  for (const f of failures) console.error(`  FAIL ${f}`);
  Deno.exit(1);
}
console.log("nfs-server DV smoke: OK");
