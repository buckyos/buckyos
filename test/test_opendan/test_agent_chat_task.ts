/**
 * Agent Chat acceptance against a running zone: a Turn is a TaskMgr task,
 * a slow Turn sends a placeholder and replaces it with one edit, a fast one
 * only sends its reply; the anchor message carries `agent_task`.
 *
 *   deno run --config ../deno.json --allow-net --allow-env \
 *     --unsafely-ignore-certificate-errors test_agent_chat_task.ts [text]
 *
 * Env: as test_agent_loader.ts (the agent as agent_target.ts selects it),
 * plus OPENDAN_FOLLOW_S (0: keep printing
 * the agent's messages and the task tree until its sessions are idle) and
 * AGENT_CHAT_CANCEL=1 (cancel the task in TaskMgr once the placeholder
 * arrived: the session stops, the task ends as Canceled, the placeholder is
 * closed; use a text that keeps the agent busy).
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
// Cancel the Turn's task in TaskMgr once its placeholder arrived.
const cancelTask = Deno.env.get("AGENT_CHAT_CANCEL") === "1";
const text = Deno.args[0] ||
  "请用 shell 执行 `sleep 6; date`，然后告诉我输出的时间。";
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

const token = await login();
const { agentDid, ownerDid: selfDid, opendanUrl } = await resolveAgent(
  (method, params) => zone("control-panel", method, params, token),
  adminUser,
  zoneHost,
);
console.log(`agent ${agentDid} at ${opendanUrl}`);
const msg = (method: string, params: JsonRecord) => zone("msg-center", method, params, token);
const tasks = (method: string, params: JsonRecord) => zone("task-manager", method, params, token);
const dan = (method: string, params: JsonRecord = {}) =>
  new KRpcClient(opendanUrl, token, nextNonce()).call(method, params);

function msgOf(item: JsonRecord): JsonRecord {
  return (item.msg ?? item.object ?? {}) as JsonRecord;
}
function textOf(m: JsonRecord): string {
  return String((m.content as JsonRecord | undefined)?.content ?? "");
}
function taskIdOf(m: JsonRecord): string | undefined {
  const t = m.agent_task as JsonRecord | undefined;
  return typeof t?.task_id === "string" ? t.task_id : undefined;
}
function editTarget(m: JsonRecord): string | undefined {
  const r = m.relates_to as JsonRecord | undefined;
  return r?.rel === "edit" ? String(r.target) : undefined;
}
function taskLine(t: JsonRecord): string {
  return `${t.task_id} "${t.name}" phase=${t.phase}${t.outcome ? `/${t.outcome}` : ""}` +
    `${t.wait_reason ? ` wait=${(t.wait_reason as JsonRecord).kind}` : ""} msg=${JSON.stringify(t.message ?? "")} rev=${t.revision}`;
}

console.log("1. loader");
const status = asRecord(await dan("loader.status"), "loader.status");
for (const module of (status.modules as JsonRecord[]) ?? []) {
  console.log(`  ${module.name}: enabled=${module.enabled} running=${module.running}${module.note ? ` (${module.note})` : ""}`);
}
const taskModule = ((status.modules as JsonRecord[]) ?? []).find((m) => m.name === "task_mgr");
assert(taskModule?.running === true, "the Loader has no task_mgr module running (old opendan build?)");

console.log("2. edit capability of the reply path");
try {
  const cap = asRecord(await msg("msg.get_edit_capability", {
    msg: { from: selfDid, to: [agentDid], kind: "chat", created_at_ms: Date.now(), content: { format: "text/plain", content: "" } },
  }), "edit capability");
  console.log(`  user → agent: ${JSON.stringify(cap)}`);
} catch (error) {
  console.log(`  ! msg.get_edit_capability: ${error instanceof Error ? error.message : String(error)}`);
}

console.log(`3. send: ${text}`);
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
  idempotency_key: `agent-chat-task-${sentAt}`,
}), "post_send");
assert(sent.ok === true, `post_send failed: ${JSON.stringify(sent)}`);
console.log(`  ✓ sent ${sent.msg_id}`);

console.log("4. messages of the agent, and the task of the Turn while it runs");
const seen = new Map<string, JsonRecord>();
let sessionId = "";
async function incoming(): Promise<JsonRecord[]> {
  const fresh: JsonRecord[] = [];
  const sessions = items(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", with_object: true }));
  for (const session of sessions) {
    const timeline = items(await msg("msg.list_session", { owner: selfDid, session_id: String(session.session_id), limit: 30, with_object: true }));
    if (!timeline.some((item) => item.msg_id === sent.msg_id)) continue;
    sessionId = String(session.session_id);
    for (const item of timeline.reverse()) {
      if (item.direction !== "in" || Number(item.sort_key) < sentAt || seen.has(String(item.msg_id))) continue;
      seen.set(String(item.msg_id), item);
      fresh.push(item);
    }
  }
  return fresh;
}

let anchor: JsonRecord | undefined;
let anchorId = "";
let final: JsonRecord | undefined;
let taskId: string | undefined;
let lastTaskLine = "";
let cancelRequested = false;
const phases: string[] = [];
const deadline = Date.now() + replyTimeoutMs;
while (!final && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 1000));
  for (const item of await incoming()) {
    const m = msgOf(item);
    const at = ((Date.now() - sentAt) / 1000).toFixed(1);
    const target = editTarget(m);
    console.log(`  + ${at}s ${item.msg_id} (${item.box_kind}, ${item.state})${target ? ` edit→${target}` : ""}${taskIdOf(m) ? ` agent_task=${taskIdOf(m)}` : ""}: ${textOf(m).slice(0, 160)}`);
    if (!anchor && !target && taskIdOf(m)) {
      anchor = item;
      anchorId = String(item.msg_id);
      taskId = taskIdOf(m);
    }
    if (anchor && target === anchorId) final = item;
  }
  if (taskId) {
    try {
      const t = asRecord(asRecord(await tasks("get_task", { task_id: taskId }), "get_task").task, "task");
      if (cancelTask && !cancelRequested && t.phase !== "Terminal") {
        cancelRequested = true;
        const r = asRecord(await tasks("request_control", { task_id: taskId, action: "Cancel", request_id: `accept-cancel-${sentAt}`, recursive: true }), "request_control");
        console.log(`    cancel requested: ${JSON.stringify(r).slice(0, 200)}`);
      }
      const line = taskLine(t);
      if (line !== lastTaskLine) {
        console.log(`    task: ${line}`);
        lastTaskLine = line;
        phases.push(`${t.phase}${t.outcome ? `/${t.outcome}` : ""}`);
      }
      // A fast Turn: its only message is the anchor itself.
      if (!final && t.phase === "Terminal" && Date.now() - Number(t.completed_at ?? Date.now()) > 8000) final = anchor;
    } catch (error) {
      console.log(`    ! get_task ${taskId}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}
assert(anchor && taskId, "no message of the agent carried agent_task");
assert(final, `the Turn did not end within ${replyTimeoutMs / 1000}s`);
const placeholder = final !== anchor;
console.log(placeholder
  ? `  ✓ placeholder ${anchorId} replaced by one edit ${final.msg_id}`
  : `  ✓ fast Turn: one message ${anchorId}, no placeholder`);
if (placeholder) {
  assert(taskIdOf(msgOf(final)) === undefined, "the edit must not carry agent_task");
  const edits = [...seen.values()].filter((i) => editTarget(msgOf(i)) === anchorId);
  assert(edits.length === 1, `expected one edit of the placeholder, got ${edits.length}`);
}

console.log("5. the task and its tree");
const followUntil = Date.now() + followSeconds * 1000;
let quiet = 0;
do {
  const all = (await dan("sessions.query", {})) as JsonRecord[];
  const busy = all.some((e) => ["created", "running", "ready"].includes(String((e.status as JsonRecord)?.run_state)));
  quiet = busy ? 0 : quiet + 1;
  if (quiet >= 3) break;
  for (const item of await incoming()) {
    const m = msgOf(item);
    console.log(`  + ${((Date.now() - sentAt) / 1000).toFixed(1)}s ${item.msg_id}${editTarget(m) ? ` edit→${editTarget(m)}` : ""}${taskIdOf(m) ? ` agent_task=${taskIdOf(m)}` : ""}: ${textOf(m).slice(0, 160)}`);
  }
  await new Promise((resolve) => setTimeout(resolve, 3000));
} while (Date.now() < followUntil);
let root: JsonRecord | undefined;
for (let i = 0; i < 20; i++) {
  root = asRecord(asRecord(await tasks("get_task", { task_id: taskId }), "get_task").task, "task");
  if (root.phase === "Terminal") break;
  await new Promise((resolve) => setTimeout(resolve, 1000));
}
assert(root, "task read");
console.log(`  root: ${taskLine(root)}`);
console.log(`    schema=${root.schema_id} creator=${JSON.stringify(root.creator)} executor=${JSON.stringify(root.executor)}`);
console.log(`    input=${JSON.stringify(root.input)}`);
console.log(`    result=${JSON.stringify(root.result ?? root.error ?? null).slice(0, 300)}`);
assert(root.schema_id === "opendan.agent_turn/v1", "task schema");
assert(root.phase === "Terminal", `the Turn's task is still ${root.phase}`);
if (cancelTask) assert(root.outcome === "Canceled", `a canceled Turn's task ended as ${root.outcome}`);
const input = asRecord(root.input, "task input");
assert(input.agent_did === agentDid && typeof input.session_id === "string" && typeof input.turn === "number", "task input names the Turn");
try {
  const grants = asRecord(await tasks("list_task_access", { task_id: taskId }), "list_task_access").grants as JsonRecord[];
  for (const g of grants ?? []) console.log(`    grant: ${JSON.stringify(g.subject)} ${JSON.stringify(g.actions)} ${g.scope}/${g.data_scope}`);
  assert((grants ?? []).some((g) => (g.subject as JsonRecord)?.kind === "User" && (g.subject as JsonRecord)?.user_id === adminUser), "the root task is granted to the owner");
} catch (error) {
  console.log(`    ! list_task_access: ${error instanceof Error ? error.message : String(error)}`);
}
const tree = (asRecord(await tasks("get_task_tree", { root_id: root.root_id, limit: 100 }), "get_task_tree").tasks as JsonRecord[]) ?? [];
for (const t of tree) {
  if (t.task_id !== taskId) console.log(`  ${t.parent_id === taskId ? "└─" : "  └─"} ${taskLine(t)} parent=${t.parent_id}`);
}
const events = (asRecord(await tasks("list_task_events", { task_id: taskId, limit: 50 }), "list_task_events").events as JsonRecord[]) ?? [];
console.log(`  events: ${events.map((e) => e.event_type).join(", ")}`);
console.log(`  phases seen: ${phases.join(" → ")}`);

console.log("6. the session's own record");
const read = asRecord(await dan("session.read", { sid: input.session_id }), "session.read");
const state = asRecord(read.state, "state");
const binding = ((state.turn_tasks as JsonRecord[]) ?? []).find((b) => b.task_id === taskId);
console.log(`  turn_tasks: ${JSON.stringify(binding)}`);
for (const o of (state.outbox as JsonRecord[]) ?? []) {
  if (o.turn === input.turn) console.log(`  outbox: ${o.key} purpose=${o.purpose ?? "reply"} status=${o.status} attempts=${o.attempts} msg_id=${o.msg_id}`);
}
assert(binding?.reported === true, "the task's end was recorded by the session");
assert((binding?.placeholder ?? null) === (placeholder ? anchorId : null), "the binding names the placeholder");
console.log("  ✓ done");
