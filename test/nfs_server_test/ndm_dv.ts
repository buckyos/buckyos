/**
 * NDM gateway DV (share design D18/D19): `/ndm/*` through the real Zone
 * gateway to nfs-server.
 *
 * Covers: routing (tus OPTIONS advertises termination), anonymous rejection,
 * an upload owned by its session (another user's HEAD/PATCH/DELETE → 404),
 * resume after a partial PATCH, FileObject registration bound to the caller's
 * uploads, reads limited to holders and the object-read right, termination,
 * and proxy permission separation (users upload, admins manage).
 *
 * Run: cd test/nfs_server_test && pnpm install && pnpm run ndm
 * Env: BUCKYOS_TEST_ZONE_HOST (default test.buckyos.io), BUCKYOS_TEST_USER
 *      (default bob), BUCKYOS_TEST_USER_PASSWORD (dev config default).
 */
import { buckyos, ndn as sdkNdn } from "buckyos";

function getEnv(name: string, fallback: string): string {
  const v = Deno.env.get(name);
  return v && v.trim() ? v.trim() : fallback;
}

const zoneHost = getEnv("BUCKYOS_TEST_ZONE_HOST", "test.buckyos.io");
const BASE = `https://${zoneHost}`;
const userName = getEnv("BUCKYOS_TEST_USER", "bob");
const userPassword = getEnv("BUCKYOS_TEST_USER_PASSWORD", "buckyos2026");

const { initTestRuntime } = await import("../test_helpers/buckyos_client.ts");
const rt = await initTestRuntime();
const admin = (await rt.buckyos.getAccountInfo())?.session_token as string;

const KRpcClient = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => {
  call(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>>;
};
const hashPassword = buckyos.hashPassword as unknown as (u: string, p: string, nonce?: number) => string;
const nonce = Date.now();
const login = await new KRpcClient(`${BASE}/kapi/control-panel`, null, nonce).call("auth.login", {
  username: userName,
  password: hashPassword(userName, userPassword, nonce),
  appid: "control-panel",
  target: { kind: "system", service_id: "control-panel" },
  login_nonce: nonce,
});
const user = login.session_token as string;
console.log(`[auth] admin=${rt.userId} user=${userName}`);

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

function req(method: string, path: string, token: string | null, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("Tus-Resumable", "1.0.0");
  if (token) headers.set("Authorization", `Bearer ${token}`);
  return fetch(`${BASE}${path}`, { ...init, method, headers });
}

async function json(method: string, path: string, token: string | null, body: unknown): Promise<[number, unknown]> {
  const r = await req(method, path, token, { headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const text = await r.text();
  let parsed: unknown = text;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch { /* keep text */ }
  return [r.status, parsed];
}

const b64 = (s: string) => btoa(s);
const stamp = Date.now().toString(36);
const data = new TextEncoder().encode(`ndm dv ${stamp} `.repeat(4000));
const ndn = sdkNdn as unknown as {
  sha256Bytes(d: Uint8Array): Uint8Array;
  ChunkId: { fromMix256Result(len: number, hash: Uint8Array): { toString(): string } };
  FileObject: new (name: string, size: number, content: string) => { genObjId(): [{ toString(): string }, string] };
};
const FileObject = ndn.FileObject;
function chunkIdOf(bytes: Uint8Array): Promise<string> {
  return Promise.resolve(ndn.ChunkId.fromMix256Result(bytes.length, ndn.sha256Bytes(bytes)).toString());
}
const chunkId = await chunkIdOf(data);
const [fileIdObj, fileBody] = new FileObject(`ndm-dv-${stamp}.txt`, data.length, chunkId).genObjId();
const fileId = fileIdObj.toString();

function meta(name: string): string {
  return [
    `app_id ${b64("default")}`,
    `logical_path ${b64(`default/${chunkId}`)}`,
    `file_name ${b64(name)}`,
    `file_hash ${b64(fileId)}`,
    `chunk_index ${b64("0")}`,
    `chunk_hash ${b64(chunkId)}`,
  ].join(",");
}

await test("routing: /ndm reaches nfs-server (tus termination advertised)", async () => {
  const r = await fetch(`${BASE}/ndm/v1/uploads`, { method: "OPTIONS" });
  assert(r.status === 204, `OPTIONS ${r.status}`);
  assert((r.headers.get("tus-extension") ?? "").includes("termination"), `extensions ${r.headers.get("tus-extension")}`);
});

await test("anonymous callers are rejected everywhere", async () => {
  const created = await req("POST", "/ndm/v1/uploads", null, { headers: { "Upload-Length": "10", "Upload-Metadata": meta("x") } });
  assert(created.status === 401, `create ${created.status}`);
  const [s1] = await json("POST", "/ndm/v1/store/get_object", null, { obj_id: fileId });
  assert(s1 === 401, `store ${s1}`);
  const [s2] = await json("POST", "/ndm/proxy/v1/read/object/open", null, { obj_id: fileId });
  assert(s2 === 401, `proxy read ${s2}`);
  const lookup = await fetch(`${BASE}/ndm/v1/objects/lookup?scope=app&quick_hash=${fileId}`);
  assert(lookup.status === 401, `lookup ${lookup.status}`);
});

let location = "";
await test("upload belongs to its session and resumes", async () => {
  const created = await req("POST", "/ndm/v1/uploads", user, {
    headers: { "Upload-Length": String(data.length), "Upload-Metadata": meta(`ndm-dv-${stamp}.txt`) },
  });
  assert(created.status === 201, `create ${created.status} ${await created.text()}`);
  location = created.headers.get("location") ?? "";
  const half = Math.floor(data.length / 2);
  const first = await req("PATCH", location, user, {
    headers: { "Content-Type": "application/offset+octet-stream", "Upload-Offset": "0" },
    body: data.slice(0, half),
  });
  assert(first.status === 204 && first.headers.get("upload-offset") === String(half), `patch1 ${first.status}`);
  for (const method of ["HEAD", "DELETE"]) {
    const other = await req(method, location, admin);
    assert(other.status === 404, `${method} by another session ${other.status}`);
  }
  const foreign = await req("PATCH", location, admin, {
    headers: { "Content-Type": "application/offset+octet-stream", "Upload-Offset": String(half) },
    body: data.slice(half),
  });
  assert(foreign.status === 404, `PATCH by another session ${foreign.status}`);
  const head = await req("HEAD", location, user);
  assert(head.headers.get("upload-offset") === String(half), `resume offset ${head.headers.get("upload-offset")}`);
  const rest = await req("PATCH", location, user, {
    headers: { "Content-Type": "application/offset+octet-stream", "Upload-Offset": String(half) },
    body: data.slice(half),
  });
  assert(rest.status === 204 && rest.headers.get("ndm-chunk-status") === "completed", `patch2 ${rest.status}`);
  assert(rest.headers.get("ndm-chunk-object-id") === chunkId, `chunk ${rest.headers.get("ndm-chunk-object-id")} != ${chunkId}`);
});

await test("registration is bound to the caller's uploads; reads to holders", async () => {
  const put = { obj_id: fileId, obj_data: fileBody };
  const [forged] = await json("POST", "/ndm/v1/store/put_object", user, { obj_id: fileId, obj_data: '{"name":"forged"}' });
  assert(forged === 400, `forged body ${forged}`);
  const [byOwnerOfNothing, why] = await json("POST", "/ndm/proxy/v1/rpc/put_object", admin, put);
  // The admin may manage the store, so registering someone else's chunk is allowed for it.
  assert(byOwnerOfNothing === 204, `admin register ${byOwnerOfNothing} ${JSON.stringify(why)}`);
  const [ok, body] = await json("POST", "/ndm/v1/store/put_object", user, put);
  assert(ok === 204, `register ${ok} ${JSON.stringify(body)}`);
  const [read] = await json("POST", "/ndm/v1/store/get_object", user, { obj_id: fileId });
  assert(read === 200, `holder read ${read}`);
  const opened = await req("POST", "/ndm/proxy/v1/read/object/open", user, {
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ obj_id: fileId }),
  });
  const bytes = new Uint8Array(await opened.arrayBuffer());
  assert(opened.status === 200 && bytes.length === data.length, `holder proxy read ${opened.status} ${bytes.length}`);
});

await test("objects the caller does not hold are not readable by id", async () => {
  // An object only the admin session registered: a user cannot read it by knowing its id.
  const other = new TextEncoder().encode(`admin only ${stamp}`);
  const otherChunk = await chunkIdOf(other);
  const w = await req("PUT", `/ndm/proxy/v1/write/chunk/${otherChunk}`, admin, {
    headers: { "Content-Type": "application/octet-stream", "NDM-Chunk-Size": String(other.length) },
    body: other,
  });
  assert(w.ok, `admin write ${w.status} ${await w.text()}`);
  const [otherIdObj, otherBody] = new FileObject(`admin-${stamp}.txt`, other.length, otherChunk).genObjId();
  const [put] = await json("POST", "/ndm/proxy/v1/rpc/put_object", admin, { obj_id: otherIdObj.toString(), obj_data: otherBody });
  assert(put === 204, `admin register ${put}`);
  const [s1] = await json("POST", "/ndm/v1/store/get_object", user, { obj_id: otherIdObj.toString() });
  assert(s1 === 403, `user store read ${s1}`);
  const [s2] = await json("POST", "/ndm/proxy/v1/read/chunk/data", user, { chunk_id: otherChunk });
  assert(s2 === 403, `user chunk read ${s2}`);
  const [s3] = await json("POST", "/ndm/v1/store/put_object", user, { obj_id: otherIdObj.toString(), obj_data: otherBody });
  assert(s3 === 403, `user registers a chunk it never uploaded ${s3}`);
});

await test("management is separate from uploads", async () => {
  const [u1] = await json("POST", "/ndm/proxy/v1/rpc/outbox_count", user, {});
  assert(u1 === 403, `user outbox_count ${u1}`);
  const [u2] = await json("POST", "/ndm/v1/store/remove_object", user, { obj_id: fileId });
  assert(u2 === 403, `user remove_object ${u2}`);
  const [u3] = await json("POST", "/ndm/proxy/v1/rpc/pin", user, { obj_id: fileId, owner: "share:x", scope: "recursive" });
  assert(u3 === 403, `user pin ${u3}`);
  const [a1] = await json("POST", "/ndm/proxy/v1/rpc/outbox_count", admin, {});
  assert(a1 === 200, `admin outbox_count ${a1}`);
});

await test("termination drops an unfinished upload", async () => {
  const extra = new TextEncoder().encode(`terminate me ${stamp}`.repeat(100));
  const id = await chunkIdOf(extra);
  const created = await req("POST", "/ndm/v1/uploads", user, {
    headers: {
      "Upload-Length": String(extra.length),
      "Upload-Metadata": `logical_path ${b64(`default/${id}`)},chunk_index ${b64("0")},chunk_hash ${b64(id)}`,
    },
  });
  assert(created.status === 201, `create ${created.status}`);
  const loc = created.headers.get("location") ?? "";
  await req("PATCH", loc, user, { headers: { "Content-Type": "application/offset+octet-stream", "Upload-Offset": "0" }, body: extra.slice(0, 50) });
  const gone = await req("DELETE", loc, user);
  assert(gone.status === 204, `delete ${gone.status}`);
  const head = await req("HEAD", loc, user);
  assert(head.status === 404, `head after delete ${head.status}`);
});

console.log(`\n${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  for (const f of failures) console.error(`  FAIL ${f}`);
  Deno.exit(1);
}
console.log("NDM gateway DV: OK");
