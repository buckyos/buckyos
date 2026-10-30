/**
 * Object URL / DID Object Provider DV (notepads/agent-object-url-todo.md §6 第 2 段, §7):
 * through the real Zone gateway, the Providers at `/kapi/<host>/obj/...` answer Cards,
 * Profiles, properties and IndexTrait queries with the caller's own session, and refuse
 * other users exactly like the underlying kRPC / ACL.
 *
 * Covers: DFS (`obj://dfs/home/<user>`), msg-center sessions / contacts of an owner,
 * HomeStation home / feed / entry (https default URL), AI Workspace workspace / entity /
 * path, the kevent of a msg-center session (`/obj/msg-center/owners/<owner>/...`), and the
 * Agent Provider (`obj://<agent-id>/...`) when the admin has a ready agent.
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors object_url_dv.ts
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER (devtest),
 * BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025), BUCKYOS_TEST_USER (bob),
 * BUCKYOS_TEST_USER_PASSWORD (buckyos2026), BUCKYOS_TEST_AGENT_ID (optional).
 */
import { buckyos } from "buckyos";

type JsonRecord = Record<string, unknown>;
type RpcClient = { call(method: string, params: JsonRecord): Promise<unknown> };

const KRpcClient = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => RpcClient;
const hashPassword = buckyos.hashPassword as unknown as (username: string, password: string, nonce?: number) => string;

const env = (name: string, fallback: string) => Deno.env.get(name)?.trim() || fallback;
const zoneHost = env("BUCKYOS_TEST_ZONE_HOST", "test.buckyos.io");
const BASE = `https://${zoneHost}`;
const adminUser = env("BUCKYOS_TEST_ADMIN_USER", "devtest");
const adminPassword = env("BUCKYOS_TEST_ADMIN_PASSWORD", "bucky2025");
const userName = env("BUCKYOS_TEST_USER", "bob");
const userPassword = env("BUCKYOS_TEST_USER_PASSWORD", "buckyos2026");
const adminDid = `did:bns:${adminUser}`;

let lastNonce = Date.now();
const nextNonce = () => (lastNonce = Math.max(Date.now(), lastNonce + 1));

async function login(user: string, password: string): Promise<string> {
  const nonce = nextNonce();
  const result = await new KRpcClient(`${BASE}/kapi/control-panel`, null, nonce).call("auth.login", {
    username: user,
    password: hashPassword(user, password, nonce),
    appid: "control-panel",
    target: { kind: "system", service_id: "control-panel" },
    login_nonce: nonce,
  }) as JsonRecord;
  return String(result.session_token);
}

async function rpc(service: string, method: string, params: JsonRecord, token: string): Promise<JsonRecord> {
  return await new KRpcClient(`${BASE}/kapi/${service}`, token, nextNonce()).call(method, params) as JsonRecord;
}

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(`assert failed: ${msg}`);
}

async function get(path: string, token: string | null): Promise<[number, JsonRecord]> {
  const headers: Record<string, string> = { accept: "application/json" };
  if (token) headers.authorization = `Bearer ${token}`;
  const r = await fetch(`${BASE}${path}`, { headers });
  const text = await r.text();
  let body: JsonRecord = {};
  try {
    body = JSON.parse(text);
  } catch {
    body = { text };
  }
  return [r.status, body];
}

async function query(path: string, token: string, params: JsonRecord = {}): Promise<JsonRecord> {
  const r = await fetch(`${BASE}${path}/methods/query`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ method: "query", params }),
  });
  const body = await r.json() as JsonRecord;
  assert(r.ok && body.result, `query ${path}: ${r.status} ${JSON.stringify(body)}`);
  return body.result as JsonRecord;
}

function rowsOf(page: JsonRecord): JsonRecord[] {
  return (page.rows as JsonRecord[]) ?? [];
}

/** Card + Profile of a path-type Object URL; returns the Card. */
async function card(path: string, token: string, objUrl: string): Promise<JsonRecord> {
  const [status, body] = await get(path, token);
  assert(status === 200, `Card ${path}: ${status} ${JSON.stringify(body)}`);
  const aka = (body.alsoKnownAs as string[]) ?? [];
  assert(aka.includes(objUrl), `Card ${path} alsoKnownAs ${JSON.stringify(aka)} lacks ${objUrl}`);
  const service = ((body.service as JsonRecord[]) ?? []).find((s) => s.type === "DIDObjectService");
  assert(service, `Card ${path} has a DIDObjectService`);
  const [pstatus, profile] = await get(`${path}?type=profile`, token);
  assert(pstatus === 200 && profile.properties, `Profile ${path}: ${pstatus}`);
  return body;
}

function keventSegment(segment: string): string {
  return /^[A-Za-z0-9_-]+$/.test(segment) && !segment.startsWith("x")
    ? segment
    : "x" + Array.from(new TextEncoder().encode(segment), (b) => b.toString(16).padStart(2, "0")).join("");
}

let passed = 0;
const failures: string[] = [];
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

const admin = await login(adminUser, adminPassword);
const user = await login(userName, userPassword);
console.log(`[auth] admin=${adminUser} user=${userName}`);

await test("dfs: Card / info / listing with the path ACL", async () => {
  const path = `/kapi/dfs/obj/home/${adminUser}`;
  await card(path, admin, `obj://dfs/home/${adminUser}`);
  const [status, info] = await get(`${path}/props/info`, admin);
  assert(status === 200 && info.uri === `cyfs:///home/${adminUser}`, `info ${status} ${JSON.stringify(info)}`);
  const page = await query(path, admin);
  for (const row of rowsOf(page)) {
    assert(String(row.object).startsWith(`obj://dfs/home/${adminUser}/`), `row object ${row.object}`);
  }
  const [denied] = await get(path, user);
  assert(denied === 403, `${userName} reading ${adminUser}'s home: ${denied}`);
  const [profile] = await get("/kapi/dfs/obj?type=profile", null);
  assert(profile === 200, `root profile without a session: ${profile}`);
});

await test("msg-center: an owner's sessions and contacts, refused to another user", async () => {
  const owner = encodeURI(adminDid);
  const sessions = `/kapi/msg-center/obj/owners/${owner}/sessions`;
  await card(sessions, admin, `obj://msg-center/owners/${adminDid}/sessions`);
  const created = await rpc("msg-center", "msg.create_session", { owner: adminDid, peer_did: `did:bns:${userName}`, title: "object url dv" }, admin);
  const sid = String(created.session_id);
  const page = await query(sessions, admin, { page_size: 50 });
  const objects = rowsOf(page).map((r) => String(r.object));
  const expected = `obj://msg-center/owners/${adminDid}/sessions/${sid}`;
  assert(objects.includes(expected), `${expected} in ${JSON.stringify(objects)}`);
  const sessionPath = `${sessions}/${encodeURIComponent(sid)}`;
  await card(sessionPath, admin, expected);
  const [status, state] = await get(`${sessionPath}/props/state`, admin);
  assert(status === 200, `session state: ${status} ${JSON.stringify(state)}`);
  const [denied] = await get(sessions, user);
  assert(denied === 403, `${userName} reading ${adminUser}'s sessions: ${denied}`);
  await card(`/kapi/msg-center/obj/owners/${owner}/contacts`, admin, `obj://msg-center/owners/${adminDid}/contacts`);
  await query(`/kapi/msg-center/obj/owners/${owner}/contacts`, admin);
  const [cdenied] = await get(`/kapi/msg-center/obj/owners/${owner}/contacts`, user);
  assert(cdenied === 403, `${userName} reading ${adminUser}'s contacts: ${cdenied}`);
});

await test("msg-center: a session change is the kevent /obj/msg-center/owners/<owner>/...", async () => {
  const pattern = `/obj/msg-center/owners/${keventSegment(adminDid)}/**`;
  const controller = new AbortController();
  const response = await fetch(`${BASE}/kapi/kevent/stream`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ patterns: [pattern], keepalive_ms: 1000 }),
    signal: controller.signal,
  });
  assert(response.ok && response.body, `kevent stream ${response.status}`);
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const frames: JsonRecord[] = [];
  const pump = (async () => {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let nl;
      while ((nl = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, nl).trim();
        buffer = buffer.slice(nl + 1);
        if (line) frames.push(JSON.parse(line));
      }
    }
  })().catch(() => {});
  try {
    await new Promise((r) => setTimeout(r, 1000));
    const created = await rpc("msg-center", "msg.create_session", { owner: adminDid, peer_did: `did:bns:${userName}`, title: "kevent dv" }, admin);
    const sid = String(created.session_id);
    await rpc("msg-center", "msg.post_send", {
      msg: { from: adminDid, to: [`did:bns:${userName}`], kind: "chat", created_at_ms: Date.now(), to_session: sid, content: { format: "text/plain", content: "object url dv" } },
      idempotency_key: crypto.randomUUID(),
    }, admin).catch((e) => console.log(`  post_send: ${e}`));
    const deadline = Date.now() + 8000;
    while (Date.now() < deadline && !frames.some((f) => String((f.event as JsonRecord | undefined)?.eventid ?? f.eventid ?? "").startsWith("/obj/msg-center/owners/"))) {
      await new Promise((r) => setTimeout(r, 200));
    }
    const ids = frames.map((f) => String((f.event as JsonRecord | undefined)?.eventid ?? f.eventid ?? f.type));
    assert(ids.some((id) => id.startsWith(`/obj/msg-center/owners/${keventSegment(adminDid)}/sessions`)), `events: ${JSON.stringify(ids)}`);
    console.log(`  events: ${ids.filter((i) => i.startsWith("/obj/")).slice(0, 4).join(", ")}`);
  } finally {
    controller.abort();
    await pump;
  }
});

/** Frames of one kevent stream; `close` stops it. */
async function keventStream(patterns: string[]): Promise<{ frames: JsonRecord[]; close: () => Promise<void> }> {
  const controller = new AbortController();
  const response = await fetch(`${BASE}/kapi/kevent/stream`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ patterns, keepalive_ms: 1000 }),
    signal: controller.signal,
  });
  assert(response.ok && response.body, `kevent stream ${response.status}`);
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const frames: JsonRecord[] = [];
  let buffer = "";
  const pump = (async () => {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let nl;
      while ((nl = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, nl).trim();
        buffer = buffer.slice(nl + 1);
        if (line) frames.push(JSON.parse(line));
      }
    }
  })().catch(() => {});
  await new Promise((r) => setTimeout(r, 800));
  return { frames, close: async () => { controller.abort(); await pump; } };
}

const eventIds = (frames: JsonRecord[]) => frames.map((f) => String((f.event as JsonRecord | undefined)?.eventid ?? ""));

await test("taskmgr: a task change is the kevent /obj/taskmgr/<task_id>/changed", async () => {
  const task = await rpc("task-manager", "create_task", {
    name: "object url dv",
    schema_id: "raw/v1",
    input: {},
    executor: { kind: "HumanSet", assignees: [adminUser] },
    idempotency_key: crypto.randomUUID(),
  }, admin);
  const created = (task.task as JsonRecord | undefined) ?? task;
  const taskId = String(created.task_id);
  const revision = Number(created.revision ?? 1);
  const key = `/obj/taskmgr/${keventSegment(taskId)}/changed`;
  const stream = await keventStream([key]);
  try {
    await rpc("task-manager", "update_assignees", { task_id: taskId, add: [userName], remove: [], expected_revision: revision }, admin);
    const deadline = Date.now() + 8000;
    while (Date.now() < deadline && !eventIds(stream.frames).includes(key)) await new Promise((r) => setTimeout(r, 200));
    assert(eventIds(stream.frames).includes(key), `expected ${key}, got ${JSON.stringify(eventIds(stream.frames))}`);
  } finally {
    await stream.close();
  }
});

await test("homestation: https default url, feed rows are https entries", async () => {
  const path = `/kapi/homestation/obj/home/${adminUser}`;
  const c = await card(path, admin, `obj://homestation/home/${adminUser}`);
  const aka = c.alsoKnownAs as string[];
  assert(aka[0] === `https://${zoneHost}/home/${adminUser}`, `default url ${aka[0]}`);
  await rpc("homestation", "publish.create", {
    key: `oudv-${Date.now()}`,
    input: { text: "object url dv", attachments: [], link: null, audience: { kind: "public" } },
  }, admin).catch((e) => console.log(`  publish.create: ${e}`));
  const page = await query(`${path}/feed`, admin);
  const entries = rowsOf(page).map((r) => String(r.object));
  // The stream carries every namespace of the home (feed, reactions, follows), all https entries.
  assert(entries.length > 0 && entries.every((e) => e.startsWith(`https://${zoneHost}/home/${adminUser}/`) && e.includes("/@/")), `entries ${JSON.stringify(entries)}`);
  const post = entries.find((e) => e.startsWith(`https://${zoneHost}/home/${adminUser}/feed/@/`));
  assert(post, `a feed entry among ${JSON.stringify(entries)}`);
  const key = post.split("/").pop();
  const [status, head] = await get(`${path}/feed/@/${key}/props/head`, admin);
  assert(status === 200, `entry head ${status} ${JSON.stringify(head)}`);
  const [inboxStatus, inbox] = await get(`${path}/inbox/props/target`, admin);
  assert(inboxStatus === 200 && inbox.target === `https://${zoneHost}/home/${adminUser}/inbox`, `inbox ${JSON.stringify(inbox)}`);
});

await test("aiworkspace: workspace / entity / path, not visible without a grant", async () => {
  const ws = String((await rpc("aiworkspace", "ws.create", { title: "object url dv" }, admin)).workspace_id);
  const path = `/kapi/aiworkspace/obj/${ws}`;
  await card(path, admin, `obj://aiworkspace/${ws}`);
  const page = await query(path, admin);
  const entities = rowsOf(page).map((r) => String(r.object));
  assert(entities.length > 0 && entities.every((e) => e.startsWith(`obj://aiworkspace/${ws}/entities/`)), `entities ${JSON.stringify(entities)}`);
  const entity = entities[0].split("/").pop()!;
  await card(`${path}/entities/${entity}`, admin, `obj://aiworkspace/${ws}/entities/${entity}`);
  const [status] = await get(`${path}/entities/${entity}/props/content`, admin);
  assert(status === 200, `entity content ${status}`);
  const [denied] = await get(path, user);
  assert(denied === 404, `${userName} without a grant: ${denied}`);
  await rpc("aiworkspace", "ws.delete", { workspace_id: ws }, admin).catch(() => {});
});

const agents = ((await rpc("control-panel", "agent.list", {}, admin)).agents as JsonRecord[]) ?? [];
const agentId = Deno.env.get("BUCKYOS_TEST_AGENT_ID")?.trim() ||
  String(agents.find((a) => a.owner_user_id === adminUser && (a.install as JsonRecord | undefined)?.state === "ready")?.agent_id ?? "");
if (agentId) {
  await test(`agent: obj://${agentId} and its sessions`, async () => {
    const path = `/kapi/${agentId}/obj`;
    const c = await card(path, admin, `obj://${agentId}`);
    assert(String(c.id).includes(agentId), `agent Card id ${c.id}`);
    const page = await query(`${path}/sessions`, admin);
    const sessions = rowsOf(page).map((r) => String(r.object));
    assert(sessions.every((s) => s.startsWith(`obj://${agentId}/sessions/`)), `sessions ${JSON.stringify(sessions)}`);
    if (sessions.length) {
      const sid = sessions[0].split("/").pop()!;
      await card(`${path}/sessions/${sid}`, admin, `obj://${agentId}/sessions/${decodeURIComponent(sid)}`.replace(/ /g, "%20"));
    }
    const [denied] = await get(path, user);
    assert(denied === 403 || denied === 401, `${userName} reading ${adminUser}'s agent: ${denied}`);
  });
} else {
  console.log("SKIP agent: no ready agent of the admin (BUCKYOS_TEST_AGENT_ID / test_opendan/agent_target.ts --create)");
}

console.log(`\n${passed} passed, ${failures.length} failed`);
for (const f of failures) console.log(`  - ${f}`);
if (failures.length) Deno.exit(1);
