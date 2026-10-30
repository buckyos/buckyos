/**
 * TaskMgr kevent gateway smoke (DV-07): a task mutation is published as the kevent
 * `/obj/taskmgr/<task_id>/changed`, and a child's mutation also as the root's
 * `/obj/taskmgr/<root_id>/tree` (buckyos doc/key url.md §7). Both are subscribed through
 * the Zone gateway `/kapi/kevent/stream`.
 *
 * Env: BUCKYOS_TEST_ZONE_HOST (test.buckyos.io), BUCKYOS_TEST_ADMIN_USER (devtest),
 * BUCKYOS_TEST_ADMIN_PASSWORD (bucky2025), BUCKYOS_GATEWAY_BASE_URL (https://<zone>).
 */
import { buckyos } from "buckyos";

type JsonRecord = Record<string, unknown>;
type RpcClient = { call(method: string, params: JsonRecord): Promise<unknown> };

const KRpcClient = buckyos.kRPCClient as unknown as new (url: string, token?: string | null, seq?: number) => RpcClient;
const hashPassword = buckyos.hashPassword as unknown as (username: string, password: string, nonce?: number) => string;

const env = (name: string, fallback: string) => Deno.env.get(name)?.trim() || fallback;
const zoneHost = env("BUCKYOS_TEST_ZONE_HOST", "test.buckyos.io");
const BASE = env("BUCKYOS_GATEWAY_BASE_URL", `https://${zoneHost}`);
const adminUser = env("BUCKYOS_TEST_ADMIN_USER", "devtest");
const adminPassword = env("BUCKYOS_TEST_ADMIN_PASSWORD", "bucky2025");

let lastNonce = Date.now();
const nextNonce = () => (lastNonce = Math.max(Date.now(), lastNonce + 1));

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(`assert failed: ${msg}`);
}

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

const token = await login(adminUser, adminPassword);

async function taskMgr(method: string, params: JsonRecord): Promise<JsonRecord> {
  const result = await new KRpcClient(`${BASE}/kapi/task-manager`, token, nextNonce()).call(method, params) as JsonRecord;
  return (result.task as JsonRecord | undefined) ?? result;
}

async function createTask(name: string, parentId?: string): Promise<JsonRecord> {
  return await taskMgr("create_task", {
    name,
    schema_id: "raw/v1",
    input: { case: "DV-07" },
    executor: { kind: "HumanSet", assignees: [adminUser] },
    parent_id: parentId,
    idempotency_key: crypto.randomUUID(),
  });
}

/** `obj://taskmgr/<task_id>` kevent key (doc/key url.md §7). */
function taskEventId(taskId: string, event: "changed" | "tree"): string {
  const segment = /^[A-Za-z0-9_-]+$/.test(taskId) && !taskId.startsWith("x")
    ? taskId
    : "x" + Array.from(new TextEncoder().encode(taskId), (b) => b.toString(16).padStart(2, "0")).join("");
  return `/obj/taskmgr/${segment}/${event}`;
}

/** Events of one kevent stream; `close` stops it. */
async function keventStream(patterns: string[]): Promise<{ events: JsonRecord[]; close: () => Promise<void> }> {
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
  const events: JsonRecord[] = [];
  let acked = false;
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
        if (!line) continue;
        const frame = JSON.parse(line) as JsonRecord;
        if (frame.type === "ack") acked = true;
        if (frame.type === "event") events.push(frame.event as JsonRecord);
      }
    }
  })().catch(() => {});
  const deadline = Date.now() + 5000;
  while (!acked && Date.now() < deadline) await new Promise((r) => setTimeout(r, 100));
  assert(acked, "kevent stream should ack");
  return { events, close: async () => { controller.abort(); await pump; } };
}

async function waitEvent(events: JsonRecord[], eventId: string, taskId: string): Promise<JsonRecord> {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const event = events.find((e) => e.eventid === eventId && (e.data as JsonRecord)?.task_id === taskId);
    if (event) return event.data as JsonRecord;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`no ${eventId} for task ${taskId}; got ${JSON.stringify(events.map((e) => e.eventid))}`);
}

const root = await createTask(`task-changed-signal-${Date.now()}`);
const rootId = String(root.task_id);
const changed = taskEventId(rootId, "changed");
const tree = taskEventId(rootId, "tree");
const stream = await keventStream([changed, tree]);
const created: JsonRecord[] = [root];
try {
  // 1. A mutation of the task is its `changed` event.
  await taskMgr("update_assignees", { task_id: rootId, add: [], remove: [], expected_revision: Number(root.revision) });
  const data = await waitEvent(stream.events, changed, rootId);
  assert(data.root_id === rootId && typeof data.revision === "number", `changed payload ${JSON.stringify(data)}`);
  assert(!("runner" in data), "task-changed payload should not carry a runner field");

  // 2. A child's mutation is also the root's `tree` event.
  const child = await createTask(`task-changed-child-${Date.now()}`, rootId);
  created.push(child);
  const childId = String(child.task_id);
  const treeData = await waitEvent(stream.events, tree, childId);
  assert(treeData.root_id === rootId, `tree payload ${JSON.stringify(treeData)}`);

  console.log(JSON.stringify({ status: "passed", case: "DV-07", events: [changed, tree], task_ids: created.map((t) => t.task_id) }));
} finally {
  await stream.close();
  for (const task of created.reverse()) {
    const latest = await taskMgr("get_task", { task_id: task.task_id }).catch(() => task);
    await taskMgr("request_control", {
      task_id: task.task_id,
      action: "Cancel",
      request_id: crypto.randomUUID(),
      expected_revision: latest.revision,
    }).catch((e) => console.log(`[warn] cleanup cancel ${task.task_id}: ${e}`));
  }
}
