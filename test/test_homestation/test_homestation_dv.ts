/**
 * HomeStation real-zone smoke test against a running zone: the zone index and DID lookup
 * (/home/), a user's home (/home/<user>/...) and kRPC through the zone gateway, the zone feed
 * (/home/~zone/...) and anonymous portal reads (portal.*).
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors test_homestation_dv.ts
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER (devtest),
 * BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025).
 */
import { buckyos } from "buckyos";

type JsonRecord = Record<string, unknown>;
type RpcClient = { call(method: string, params: JsonRecord): Promise<unknown> };

const KRpcClient = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => RpcClient;
const hashPassword = buckyos.hashPassword as unknown as (username: string, password: string, nonce?: number) => string;

const zoneHost = Deno.env.get("BUCKYOS_TEST_ZONE_HOST")?.trim() || "test.buckyos.io";
const adminUser = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
const adminPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
let lastNonce = Date.now();

function nextNonce(): number {
  lastNonce = Math.max(Date.now(), lastNonce + 1);
  return lastNonce;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

function rec(value: unknown, label: string): JsonRecord {
  assert(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
  return value as JsonRecord;
}

async function call(service: string, method: string, params: JsonRecord, token: string | null): Promise<unknown> {
  const rpc = new KRpcClient(`https://${zoneHost}/kapi/${service}`, token, nextNonce());
  return await rpc.call(method, params);
}

async function login(): Promise<string> {
  const nonce = nextNonce();
  const result = rec(
    await call("control-panel", "auth.login", {
      username: adminUser,
      password: hashPassword(adminUser, adminPassword, nonce),
      appid: "control-panel",
      target: { kind: "system", service_id: "control-panel" },
      login_nonce: nonce,
    }, null),
    "auth.login",
  );
  assert(typeof result.session_token === "string" && result.session_token, "login returned no token");
  return result.session_token;
}

async function get(path: string, token?: string): Promise<{ status: number; body: string }> {
  const response = await fetch(`https://${zoneHost}${path}`, { headers: token ? { authorization: `Bearer ${token}` } : {} });
  return { status: response.status, body: await response.text() };
}

const token = await login();
const hs = (method: string, params: JsonRecord = {}) => call("homestation", method, params, token);

console.log("1. zone index, DID lookup and the user's public profile through the gateway");
const index = await get("/home/");
assert(index.status === 200, `GET /home/ → ${index.status}: ${index.body.slice(0, 200)}`);
const indexJson = JSON.parse(index.body) as JsonRecord;
console.log(`  ✓ zone ${indexJson.zone}, default feed ${indexJson.defaultFeed}`);
const home = `/home/${adminUser}`;
const profile = await get(`${home}/profile`);
assert(profile.status === 200, `GET ${home}/profile → ${profile.status}: ${profile.body.slice(0, 200)}`);
const profileJson = JSON.parse(profile.body) as JsonRecord;
assert(String(profileJson.stream) === `cyfs://${zoneHost}${home}/feed`, `stream address: ${profileJson.stream}`);
const located = await get(`/home/?did=${encodeURIComponent(String(profileJson.did))}`);
assert(located.status === 200 && (JSON.parse(located.body) as JsonRecord).user === adminUser, `DID lookup → ${located.status}: ${located.body}`);
assert((await get("/home/?did=did:bns:nobody-here")).status === 404, "unknown DIDs have no home here");
console.log(`  ✓ ${profileJson.did} at ${profileJson.stream}`);

console.log("2. the user's bootstrap through /kapi/homestation");
const boot = rec(await hs("ui.bootstrap"), "ui.bootstrap");
assert(boot.zone === zoneHost, `zone ${boot.zone}`);
assert(boot.user === adminUser, `user ${boot.user}`);
const bootHome = rec(boot.home, "ui.bootstrap.home");
console.log(`  ✓ ${boot.owner} (${boot.user}), ${(boot.friends as unknown[]).length} friend(s), ${(boot.following as unknown[]).length} followed, zone feed writer: ${rec(bootHome.zoneFeed, "zoneFeed").writer}`);

console.log("3. publish a public post (idempotent per key) and read it back");
const key = `dv-${Date.now()}`;
const input = { text: `HomeStation DV smoke ${new Date().toISOString()}`, attachments: [], link: null, audience: { kind: "public" }, zoneFeed: true };
const task = rec(await hs("publish.create", { key, input }), "publish.create");
assert(task.stage === "published", `stage ${task.stage}: ${task.error ?? ""}`);
const again = rec(await hs("publish.create", { key, input }), "publish.create again");
assert(again.objId === task.objId, "same key, same post");
const feed = await get(`${home}/feed?mode=display&limit=5`);
assert(feed.status === 200, `GET ${home}/feed → ${feed.status}`);
const items = (JSON.parse(feed.body) as JsonRecord).items as JsonRecord[];
assert(items.some((i) => i.current === task.objId), "the new post is in the anonymous display read");
console.log(`  ✓ ${task.objId} visible at ${home}/feed`);

console.log("3b. the post is listed in the zone feed and readable by anonymous portal calls");
const zoneFeed = await get("/home/~zone/feed?mode=display&limit=20");
assert(zoneFeed.status === 200, `GET /home/~zone/feed → ${zoneFeed.status}`);
assert(((JSON.parse(zoneFeed.body) as JsonRecord).items as JsonRecord[]).some((i) => i.current === task.objId), "listed in the zone feed");
const portalHome = rec(await call("homestation", "portal.home", {}, null), "portal.home");
assert(portalHome.viewer === null, "anonymous portal viewer");
const portalList = rec(await call("homestation", "portal.list", { feed: "~zone" }, null), "portal.list");
assert((portalList.entries as JsonRecord[]).some((e) => e.objId === task.objId && e.user === adminUser), "zone feed portal entry");
const portalItem = rec(await call("homestation", "portal.item", { feed: adminUser, key: String(task.entry).split("/").pop() }, null), "portal.item");
assert(rec(rec(portalItem.card, "card").item, "item").objId === task.objId, "post link resolves");
console.log(`  ✓ zone feed and portal (default feed ${portalHome.defaultFeed})`);

console.log("4. owner listing, edit, like and comment on the own post");
const published = rec(await hs("published.list", { kind: "all" }), "published.list");
assert((published.entries as JsonRecord[]).some((e) => e.entry === task.entry), "listed in my publications");
const edited = await hs("entry.edit", { entry: task.entry, text: `${input.text} (edited)` });
const card = rec(await hs("item.get", { objId: task.objId }), "item.get");
assert(rec(rec(card.item, "item").entry, "entry").currentObjId === edited, "the old version points at the edit");
const like = rec(await hs("interact.like", { objId: edited, on: true }), "interact.like");
assert(rec(like.like, "like").on === true, "liked");
await hs("interact.comment", { objId: edited, text: "DV comment" });
const comments = rec(await hs("comments.list", { objId: edited, view: "local", type: "text" }), "comments.list");
assert((comments.comments as unknown[]).length >= 1, "comment listed");
console.log("  ✓ edited, liked and commented");

console.log("5. upload a file and fetch its content with the session token");
const upload = await fetch(`https://${zoneHost}/kapi/homestation/upload?name=dv.txt&mime=text/plain`, {
  method: "PUT",
  headers: { authorization: `Bearer ${token}` },
  body: "hello from the DV smoke test",
});
assert(upload.ok, `upload → ${upload.status}: ${await upload.clone().text()}`);
const file = (await upload.json()) as JsonRecord;
const content = await fetch(`https://${zoneHost}${home}/objects/${file.objId}/content?access=${encodeURIComponent(token)}`);
assert(content.ok && (await content.text()) === "hello from the DV smoke test", `content → ${content.status}`);
console.log(`  ✓ ${file.objId} stored in the zone named store and readable`);

console.log("6. evaluation service on an identity and on the uploaded file");
const ident = rec(await hs("eval.evaluate", { request: { target: { kind: "identity", did: boot.owner } } }), "eval identity");
const content_eval = rec(await hs("eval.evaluate", { request: { target: { kind: "content", object_id: file.objId } } }), "eval content");
console.log(`  ✓ identity ${ident.state}, content ${content_eval.state}`);

console.log("7. reading pipeline and sources answer");
const reading = rec(await hs("reading.list", { query: { filter: "all", topicId: null, search: "", showFiltered: false } }), "reading.list");
const sources = rec(await hs("sources.list"), "sources.list");
console.log(`  ✓ ${reading.total} reading item(s), ${(sources.sources as unknown[]).length} source(s)`);
for (const source of sources.sources as JsonRecord[]) {
  const did = String(source.did ?? "");
  const located = did ? await get(`/home/?did=${encodeURIComponent(did)}`) : { status: 0, body: "" };
  const where = located.status === 200 ? `same zone: /home/${(JSON.parse(located.body) as JsonRecord).user}` : "other zone";
  console.log(`    ${source.name} ${did} (${where}) ${source.lastError ? `error: ${JSON.stringify(source.lastError)}` : "ok"}`);
}

console.log("8. withdraw the test post");
await hs("entry.withdraw", { entry: task.entry });
const after = await get(`${home}/feed?mode=display&limit=20`);
assert(!((JSON.parse(after.body) as JsonRecord).items as JsonRecord[]).some((i) => i.entry === task.entry), "withdrawn entries leave the display read");
const zoneAfter = await get("/home/~zone/feed?mode=display&limit=20");
assert(!((JSON.parse(zoneAfter.body) as JsonRecord).items as JsonRecord[]).some((i) => i.entry === task.entry), "withdrawn entries leave the zone feed");
console.log("  ✓ withdrawn");

console.log("HomeStation DV smoke: all checks passed");
