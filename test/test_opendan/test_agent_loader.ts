/**
 * OpenDAN Agent Loader acceptance against a running zone: the Loader's kRPC
 * service, a chat round trip through msg-center (inbox → UI Session → reply
 * in the sender's inbox) and the Session observed through `session.read`.
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors test_agent_loader.ts [text]
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER
 * (devtest), BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025), the agent as
 * agent_target.ts selects it (BUCKYOS_TEST_AGENT_CREATE=<name> |
 * BUCKYOS_TEST_AGENT_ID=<agent_id> | the user's first ready agent; OPENDAN_URL),
 * OPENDAN_REPLY_TIMEOUT_S (180), OPENDAN_FOLLOW_S (0: after the reply, keep
 * printing further messages of the agent until its sessions are idle).
 * The message is sent as the agent's owner: anyone else is not answered.
 */
import { buckyos } from "buckyos";
import { resolveAgent } from "./agent_target.ts";

type JsonRecord = Record<string, unknown>;
type RpcClient = { call(method: string, params: JsonRecord): Promise<unknown> };

const KRpcClient = buckyos.kRPCClient as unknown as new (
  url: string,
  token?: string | null,
  seq?: number,
) => RpcClient;
const hashPassword = buckyos.hashPassword as unknown as (
  username: string,
  password: string,
  nonce?: number,
) => string;

const zoneHost = Deno.env.get("BUCKYOS_TEST_ZONE_HOST")?.trim() || "test.buckyos.io";
const adminUser = Deno.env.get("BUCKYOS_TEST_ADMIN_USER")?.trim() || "devtest";
const adminPassword = Deno.env.get("BUCKYOS_TEST_ADMIN_PASSWORD")?.trim() || "bucky2025";
const replyTimeoutMs = Number(Deno.env.get("OPENDAN_REPLY_TIMEOUT_S") || "180") * 1000;
const followSeconds = Number(Deno.env.get("OPENDAN_FOLLOW_S") || "0");
const text = Deno.args[0] || "你好，请用一句话介绍你自己。";
let lastNonce = Date.now();

function nextNonce(): number {
  lastNonce = Math.max(Date.now(), lastNonce + 1);
  return lastNonce;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

function asRecord(value: unknown, label: string): JsonRecord {
  assert(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
  return value as JsonRecord;
}

function items(value: unknown): JsonRecord[] {
  return ((value as JsonRecord | null)?.items as JsonRecord[]) ?? [];
}

async function zone(service: string, method: string, params: JsonRecord, token: string | null): Promise<unknown> {
  const rpc = new KRpcClient(`https://${zoneHost}/kapi/${service}`, token, nextNonce());
  return await rpc.call(method, params);
}

async function login(): Promise<string> {
  const nonce = nextNonce();
  const result = asRecord(
    await zone("control-panel", "auth.login", {
      username: adminUser,
      password: hashPassword(adminUser, adminPassword, nonce),
      appid: "control-panel",
      target: { kind: "system", service_id: "control-panel" },
      login_nonce: nonce,
    }, null),
    "auth.login response",
  );
  assert(typeof result.session_token === "string" && result.session_token, "login returned no token");
  return result.session_token;
}

function recordText(item: JsonRecord): string {
  const msg = (item.msg ?? item.object ?? (item.record as JsonRecord | undefined)?.msg) as JsonRecord | undefined;
  const content = msg?.content as JsonRecord | undefined;
  return typeof content?.content === "string" ? content.content : JSON.stringify(item).slice(0, 300);
}

const token = await login();
const { agentDid, ownerDid: selfDid, opendanUrl } = await resolveAgent(
  (method, params) => zone("control-panel", method, params, token),
  adminUser,
  zoneHost,
);
console.log(`agent ${agentDid} at ${opendanUrl}`);
const msg = (method: string, params: JsonRecord) => zone("msg-center", method, params, token);
const dan = (method: string, params: JsonRecord = {}, t: string | null = token) =>
  new KRpcClient(opendanUrl, t, nextNonce()).call(method, params);

console.log("1. loader status");
try {
  await dan("loader.status", {}, null);
  // `debug_jarvis.sh` runs the Loader with --trust-loopback.
  console.log("  ! a call without a token was admitted (host debugging mode)");
} catch (error) {
  console.log(`  ✓ no token rejected: ${error instanceof Error ? error.message : String(error)}`);
}
const status = asRecord(await dan("loader.status"), "loader.status");
assert(status.agent_did === agentDid, `loader hosts ${status.agent_did}, expected ${agentDid}`);
for (const module of (status.modules as JsonRecord[]) ?? []) {
  console.log(`  ${module.name}: enabled=${module.enabled} running=${module.running}${module.note ? ` (${module.note})` : ""}`);
}
const ui = status.ui as JsonRecord | null;
console.log(`  ui: scans=${ui?.scans} last_error=${ui?.last_error ?? "-"}`);
for (const error of (status.errors as JsonRecord[]) ?? []) console.log(`  error[${error.source}]: ${error.message}`);
const info = asRecord(await dan("agent.info"), "agent.info");
assert(info.agent_did === agentDid, "agent.info matches");
const before = (await dan("sessions.query", {})) as JsonRecord[];
console.log(`  ✓ ${status.who} hosts ${agentDid}; ${before.length} session(s) registered`);

console.log(`2. send: ${text}`);
const sentAt = Date.now();
const sent = asRecord(await msg("msg.post_send", {
  msg: {
    from: selfDid,
    to: [agentDid],
    kind: "chat",
    created_at_ms: sentAt,
    nonce: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER),
    content: { format: "text/plain", content: text },
  },
  idempotency_key: `opendan-accept-${sentAt}`,
}), "post_send");
assert(sent.ok === true, `post_send failed: ${JSON.stringify(sent)}`);
console.log(`  ✓ sent ${sent.msg_id}`);

console.log("3. wait for the reply in the sender's timeline");
const seen = new Set<string>();
let sessionId = "";
async function incoming(): Promise<JsonRecord[]> {
  const fresh: JsonRecord[] = [];
  const sessions = items(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", with_object: true }));
  for (const session of sessions) {
    const timeline = items(await msg("msg.list_session", { owner: selfDid, session_id: String(session.session_id), limit: 20, with_object: true }));
    if (!timeline.some((item) => item.msg_id === sent.msg_id)) continue;
    sessionId = String(session.session_id);
    for (const item of timeline.reverse()) {
      if (item.direction !== "in" || Number(item.sort_key) < sentAt || seen.has(String(item.msg_id))) continue;
      seen.add(String(item.msg_id));
      fresh.push(item);
    }
  }
  return fresh;
}
function replyTo(item: JsonRecord): string {
  return String(((item.msg as JsonRecord)?.thread as JsonRecord)?.reply_to ?? "");
}
let reply: JsonRecord | undefined;
const deadline = Date.now() + replyTimeoutMs;
while (!reply && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 2000));
  reply = (await incoming()).find((item) => replyTo(item) === sent.msg_id);
}
if (!reply) {
  console.log(`  ✗ no reply within ${replyTimeoutMs / 1000}s (msg session ${sessionId || "?"})`);
} else {
  console.log(`  ✓ reply after ${((Date.now() - sentAt) / 1000).toFixed(1)}s in ${sessionId} (${reply.box_kind}): ${recordText(reply)}`);
}
// Later messages of the agent (a work session's result relayed by the UI session).
const followUntil = Date.now() + followSeconds * 1000;
let quiet = 0;
while (reply && Date.now() < followUntil) {
  await new Promise((resolve) => setTimeout(resolve, 3000));
  for (const item of await incoming()) {
    console.log(`  + ${((Date.now() - sentAt) / 1000).toFixed(1)}s (${item.box_kind}): ${recordText(item)}`);
  }
  const all = (await dan("sessions.query", {})) as JsonRecord[];
  const busy = all.some((e) => ["created", "running", "ready"].includes(String((e.status as JsonRecord)?.run_state)));
  quiet = busy ? 0 : quiet + 1;
  if (quiet >= 5) break;
}
const sessions = (await dan("sessions.query", {})) as JsonRecord[];

console.log("4. sessions seen through the Loader");
for (const entry of sessions) {
  const read = asRecord(await dan("session.read", { sid: entry.session_id, worklog: 12 }), "session.read");
  const state = (read.state ?? {}) as JsonRecord;
  const outbox = (state.outbox as JsonRecord[]) ?? [];
  console.log(`  ${entry.session_id} kind=${entry.kind} class=${entry.class ?? "-"} route=${entry.route_key ?? "-"} parent=${((entry.origin as JsonRecord)?.parent_session as string | undefined)?.slice(0, 16) ?? "-"}`);
  console.log(`    run_state=${state.run_state} turn=${state.turn_seq ?? "-"} outbox=[${outbox.map((o) => o.status).join(",")}] children=${JSON.stringify(read.children)} hosted=${JSON.stringify(read.hosted)}`);
  if (read.note) console.log(`    note: ${read.note}`);
  for (const line of ((read.worklog as JsonRecord[]) ?? []).slice().reverse()) {
    console.log(`    #${line.seq} ${line.t}: ${JSON.stringify(line).slice(0, 220)}`);
  }
}
const after = asRecord(await dan("loader.status"), "loader.status");
for (const error of (after.errors as JsonRecord[]) ?? []) console.log(`  error[${error.source}]: ${error.message}`);
console.log(`  ui: ${JSON.stringify(after.ui)}`);
if (!reply) Deno.exit(1);
