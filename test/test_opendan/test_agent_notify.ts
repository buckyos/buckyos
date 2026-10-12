/**
 * System Notify acceptance against a running zone (notepads/opendan-agent-notify-todo.md
 * V20 / V21): msg-center `notify.*` driven by the owner's session token, answers
 * committed to TaskMgr the way MessageHub does.
 *
 *   deno run --config ../deno.json --allow-net --allow-env --allow-read --allow-write \
 *     --unsafely-ignore-certificate-errors test_agent_notify.ts <command> ...
 *
 *   send notice|input "<title>" [--message <t>] [--option <id>=<label>]... [--level <l>] [--expires-in <secs>]
 *                         notify the owner as an app (category app); prints the view
 *   list [--open] [--category todo|agent|app] [--agent]   the owner's notifications (--agent: of the test agent)
 *   show <notify_id>
 *   answer <notify_id> (--option <id> | --text <t> | --dismiss)   as the owner, through TaskMgr (CAS)
 *   ack|dismiss|cancel <notify_id> [--reason <r>]
 *   wait <notify_id> [--secs <n>]          until closed or the deadline; exit 0 closed, 3 timeout
 *   ask "<request>"                        sends the request to the agent and waits for the
 *                                          agent's next notification (exit 10, printed), 3 timeout
 *
 * Env as test_agent_loader.ts / agent_target.ts.
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
const timeoutMs = Number(Deno.env.get("OPENDAN_REPLY_TIMEOUT_S") || "600") * 1000;
let lastNonce = Date.now();

function nextNonce(): number {
  lastNonce = Math.max(Date.now(), lastNonce + 1);
  return lastNonce;
}

function record(value: unknown, label: string): JsonRecord {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object: ${JSON.stringify(value)}`);
  }
  return value as JsonRecord;
}

async function zone(service: string, method: string, params: JsonRecord, token: string | null): Promise<unknown> {
  return await new KRpcClient(`https://${zoneHost}/kapi/${service}`, token, nextNonce()).call(method, params);
}

const args = [...Deno.args];
const command = args.shift() ?? "";
const flag = (name: string): string | undefined => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : undefined;
};
const all = (name: string): string[] =>
  args.flatMap((a, i) => (a === `--${name}` && args[i + 1] ? [args[i + 1]] : []));
const has = (name: string) => args.includes(`--${name}`);
const positional = args.filter((a, i) => !a.startsWith("--") && !(i > 0 && args[i - 1].startsWith("--") && !["--open", "--agent", "--dismiss"].includes(args[i - 1])));

const nonce = nextNonce();
const login = record(
  await zone("control-panel", "auth.login", {
    username: adminUser,
    password: hashPassword(adminUser, adminPassword, nonce),
    appid: "control-panel",
    target: { kind: "system", service_id: "control-panel" },
    login_nonce: nonce,
  }, null),
  "auth.login",
);
const token = String(login.session_token);
const msg = (method: string, params: JsonRecord) => zone("msg-center", method, params, token);
const tasks = (method: string, params: JsonRecord) => zone("task-manager", method, params, token);

function brief(v: JsonRecord): JsonRecord {
  return {
    notify_id: v.notify_id,
    kind: v.kind,
    category: v.category,
    level: v.level,
    status: v.status,
    title: (v.prompt as JsonRecord | undefined)?.title,
    source: v.source,
    task_id: v.task_id,
    result: v.result,
    closed_reason: v.closed_reason,
    home: v.home,
    deliveries: v.deliveries,
    url: v.url,
  };
}

function print(v: unknown) {
  console.log(JSON.stringify(v, null, 2));
}

async function get(id: string): Promise<JsonRecord> {
  return record(await msg("notify.get", { notify_id: id }), "notify.get");
}

/// The owner answers the way MessageHub does: latest snapshot, then
/// `commit_result` with `expected_revision`; a lost race shows the result.
async function answer(id: string, a: { option?: string; text?: string; dismiss?: boolean }): Promise<JsonRecord> {
  const view = await get(id);
  if (view.kind === "notice") {
    return record(await msg(a.dismiss ? "notify.dismiss" : "notify.ack", { notify_id: id }), "notice");
  }
  const taskId = String(view.task_id);
  for (let i = 0; i < 3; i++) {
    const task = record(record(await tasks("get_task", { task_id: taskId }), "get_task").task, "task");
    if (task.phase === "Terminal") break;
    const values: JsonRecord = {};
    if (a.option) values.option = a.option;
    if (a.text) values.text = a.text;
    try {
      await tasks("commit_result", {
        task_id: taskId,
        result: { kind: "human", action: a.dismiss ? "dismiss" : "answer", values, submitted_by: adminUser, submitted_at: Date.now() },
        expected_revision: task.revision,
      });
      break;
    } catch (e) {
      const text = e instanceof Error ? e.message : String(e);
      if (text.includes("revision_conflict")) continue;
      if (text.includes("task_already_completed")) break;
      throw e;
    }
  }
  return await get(id);
}

async function wait(id: string, secs: number): Promise<number> {
  const deadline = Date.now() + secs * 1000;
  let last = "";
  while (Date.now() < deadline) {
    const v = await get(id);
    const line = `${v.status}${v.closed_reason ? ` (${v.closed_reason})` : ""}`;
    if (line !== last) {
      console.log(`${((Date.now() - started) / 1000).toFixed(1)}s ${id}: ${line}`);
      last = line;
    }
    if (!["open", "read"].includes(String(v.status))) {
      print(brief(v));
      return 0;
    }
    await new Promise((r) => setTimeout(r, 1500));
  }
  return 3;
}

const started = Date.now();
let code = 0;
switch (command) {
  case "send": {
    const [kind, title] = positional;
    const options = all("option").map((o) => {
      const [value, label] = o.split("=");
      return { value, label: label ?? value };
    });
    const controls: JsonRecord[] = [];
    if (kind === "input") {
      if (options.length) controls.push({ kind: "choice", id: "option", options, required: true });
      else controls.push({ kind: "text_input", id: "text", multiline: true, required: true });
    }
    const expires = flag("expires-in");
    const view = record(await msg("notify.send", {
      to_user: adminUser,
      kind,
      level: flag("level"),
      prompt: { title, message: flag("message"), controls },
      source: { operation: "System Notify acceptance (test_agent_notify.ts)" },
      expires_at_ms: expires ? Date.now() + Number(expires) * 1000 : undefined,
      idempotency_key: `test-notify-${Date.now()}`,
    }), "notify.send");
    print(brief(view));
    break;
  }
  case "list": {
    const req: JsonRecord = {};
    if (has("open")) req.status = ["open", "read"];
    if (flag("category")) req.category = flag("category");
    if (has("agent")) {
      const { agentDid } = await resolveAgent((m, p) => zone("control-panel", m, p, token), adminUser, zoneHost);
      req.agent_did = agentDid;
    }
    const page = record(await msg("notify.list", req), "notify.list");
    print(((page.items as JsonRecord[]) ?? []).map(brief));
    break;
  }
  case "show":
    print(await get(positional[0]));
    break;
  case "answer":
    print(brief(await answer(positional[0], { option: flag("option"), text: flag("text"), dismiss: has("dismiss") })));
    break;
  case "ack":
  case "dismiss":
    print(brief(record(await msg(`notify.${command}`, { notify_id: positional[0] }), command)));
    break;
  case "cancel":
    print(brief(record(await msg("notify.cancel", { notify_id: positional[0], reason: flag("reason") }), "cancel")));
    break;
  case "wait":
    code = await wait(positional[0], Number(flag("secs") ?? timeoutMs / 1000));
    break;
  case "ask": {
    const { agentDid, ownerDid } = await resolveAgent((m, p) => zone("control-panel", m, p, token), adminUser, zoneHost);
    const before = new Set(
      ((record(await msg("notify.list", { agent_did: agentDid }), "list").items as JsonRecord[]) ?? []).map((v) => String(v.notify_id)),
    );
    const at = Date.now();
    const sent = record(await msg("msg.post_send", {
      msg: {
        from: ownerDid,
        to: [agentDid],
        kind: "chat",
        created_at_ms: at,
        nonce: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER),
        content: { format: "text/plain", content: positional[0] },
      },
      idempotency_key: `agent-notify-${at}`,
    }), "post_send");
    console.log(`→ ${sent.msg_id}: ${positional[0]}`);
    code = 3;
    while (Date.now() < started + timeoutMs) {
      await new Promise((r) => setTimeout(r, 2000));
      const items = (record(await msg("notify.list", { agent_did: agentDid }), "list").items as JsonRecord[]) ?? [];
      const fresh = items.find((v) => !before.has(String(v.notify_id)));
      if (fresh) {
        console.log(`${((Date.now() - started) / 1000).toFixed(1)}s the agent notified:`);
        print(brief(fresh));
        code = 10;
        break;
      }
    }
    break;
  }
  default:
    console.error("usage: test_agent_notify.ts send|list|show|answer|ack|dismiss|cancel|wait|ask ...");
    code = 2;
}
Deno.exit(code);
