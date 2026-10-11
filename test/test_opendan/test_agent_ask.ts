/**
 * Intent analysis acceptance against a running zone: the owner sends a
 * request, the agent's analysis asks questions (`agent_ask` messages on the
 * Turn's placeholder), the owner answers them the way MessageHub does, and
 * the placeholder is edited into the result.
 *
 *   deno run --config ../deno.json --allow-net --allow-env --allow-read --allow-write \
 *     --unsafely-ignore-certificate-errors test_agent_ask.ts send "<request>"
 *   ... test_agent_ask.ts answer "<text>" [--option <id>]   (answers the open question)
 *   ... test_agent_ask.ts wait                              (keeps following the Turn)
 *   ... test_agent_ask.ts stop                              (`/stop`)
 *
 * Every command prints the agent's new messages and the Turn's task, and
 * returns when the agent asks (exit 10, the question is printed), when the
 * placeholder is edited into the result (exit 0), or on timeout (exit 3).
 * Between calls the conversation is kept in ASK_STATE (default
 * ./.agent_ask_state.json). Env as test_agent_loader.ts / agent_target.ts.
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
const statePath = Deno.env.get("ASK_STATE") || "./.agent_ask_state.json";
let lastNonce = Date.now();

type AskState = {
  sessionId?: string;
  sentAt: number;
  requestId?: string;
  anchorId?: string;
  taskId?: string;
  seen: string[];
  question?: { msgId: string; callId: string; anchor: string; kind: string; options: JsonRecord[]; allowText: boolean };
};

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

function items(value: unknown): JsonRecord[] {
  return ((value as JsonRecord | null)?.items as JsonRecord[]) ?? [];
}

async function zone(service: string, method: string, params: JsonRecord, token: string | null): Promise<unknown> {
  return await new KRpcClient(`https://${zoneHost}/kapi/${service}`, token, nextNonce()).call(method, params);
}

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
const { agentDid, ownerDid: selfDid } = await resolveAgent(
  (method, params) => zone("control-panel", method, params, token),
  adminUser,
  zoneHost,
);
const msg = (method: string, params: JsonRecord) => zone("msg-center", method, params, token);
const tasks = (method: string, params: JsonRecord) => zone("task-manager", method, params, token);

const load = (): AskState => {
  try {
    return JSON.parse(Deno.readTextFileSync(statePath));
  } catch {
    return { sentAt: Date.now(), seen: [] };
  }
};
const save = (s: AskState) => Deno.writeTextFileSync(statePath, JSON.stringify(s, null, 2));

function msgOf(item: JsonRecord): JsonRecord {
  return (item.msg ?? item.object ?? {}) as JsonRecord;
}
function textOf(m: JsonRecord): string {
  return String((m.content as JsonRecord | undefined)?.content ?? "");
}
function relation(m: JsonRecord): { rel?: string; target?: string } {
  const r = m.relates_to as JsonRecord | undefined;
  return { rel: r?.rel as string | undefined, target: r ? String(r.target) : undefined };
}

async function post(s: AskState, text: string, extra: JsonRecord = {}): Promise<string> {
  const at = Date.now();
  const sent = record(await msg("msg.post_send", {
    msg: {
      from: selfDid,
      to: [agentDid],
      kind: "chat",
      created_at_ms: at,
      nonce: Math.floor(Math.random() * Number.MAX_SAFE_INTEGER),
      content: { format: "text/plain", content: text, ...(extra.content as JsonRecord ?? {}) },
      ...(extra.relates_to ? { relates_to: extra.relates_to } : {}),
    },
    idempotency_key: `agent-ask-${at}`,
  }), "post_send");
  if (sent.ok !== true) throw new Error(`post_send failed: ${JSON.stringify(sent)}`);
  console.log(`→ ${sent.msg_id}: ${text}`);
  return String(sent.msg_id);
}

async function follow(s: AskState): Promise<number> {
  const deadline = Date.now() + timeoutMs;
  let lastTask = "";
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    const sessions = items(await msg("msg.list_sessions", { owner: selfDid, order_by: "activity", with_object: true }));
    for (const session of sessions) {
      const timeline = items(await msg("msg.list_session", {
        owner: selfDid,
        session_id: String(session.session_id),
        limit: 40,
        with_object: true,
      }));
      if (s.requestId && !timeline.some((i) => i.msg_id === s.requestId) && s.sessionId !== session.session_id) continue;
      s.sessionId = String(session.session_id);
      for (const item of timeline.reverse()) {
        const id = String(item.msg_id);
        if (item.direction !== "in" || Number(item.sort_key) < s.sentAt || s.seen.includes(id)) continue;
        s.seen.push(id);
        const m = msgOf(item);
        const r = relation(m);
        const ask = m.agent_ask as JsonRecord | undefined;
        const task = (m.agent_task as JsonRecord | undefined)?.task_id as string | undefined;
        const t = ((Date.now() - s.sentAt) / 1000).toFixed(1);
        console.log(`← ${t}s ${id}${r.rel ? ` ${r.rel}→${r.target}` : ""}${task ? ` agent_task=${task}` : ""}${ask ? " AGENT_ASK" : ""}`);
        console.log(`   ${textOf(m).replaceAll("\n", "\n   ")}`);
        if (task && !r.rel && !s.anchorId) {
          s.anchorId = id;
          s.taskId = task;
        }
        if (ask) {
          const input = (ask.input ?? {}) as JsonRecord;
          s.question = {
            msgId: id,
            callId: String(ask.call_id),
            anchor: String(ask.anchor || id),
            kind: String(input.kind ?? "text"),
            options: (input.options as JsonRecord[]) ?? [],
            allowText: input.allow_text === true,
          };
          console.log(`   question: ${JSON.stringify(ask)}`);
        }
        if (r.rel === "edit" && s.anchorId && r.target === s.anchorId) {
          console.log("RESULT (the placeholder was edited)");
          // The anchor for MessageHub's real e2e (messagehub-agent-task.real.spec.ts).
          console.log(`AGENT_ASK_MSG=${s.anchorId}`);
          s.question = undefined;
          save(s);
          return 0;
        }
        if (!r.rel && !ask && !task && s.question === undefined && s.anchorId === undefined) {
          console.log("RESULT (a reply of its own)");
          save(s);
          return 0;
        }
      }
    }
    if (s.taskId) {
      try {
        const t = record(record(await tasks("get_task", { task_id: s.taskId }), "get_task").task, "task");
        const line = `${t.phase}${t.outcome ? `/${t.outcome}` : ""}${t.wait_reason ? ` wait=${JSON.stringify(t.wait_reason)}` : ""} ${JSON.stringify(t.message ?? "")}`;
        if (line !== lastTask) {
          console.log(`   task ${s.taskId}: ${line}`);
          lastTask = line;
        }
      } catch (e) {
        console.log(`   task ${s.taskId}: ${e instanceof Error ? e.message : String(e)}`);
      }
    }
    save(s);
    if (s.question) {
      console.log(`QUESTION ${s.question.msgId} (${s.question.kind}${s.question.options.length ? `: ${s.question.options.map((o) => `${o.id}=${o.label}`).join(", ")}` : ""})`);
      return 10;
    }
  }
  console.log("timed out");
  return 3;
}

const [command, ...rest] = Deno.args;
let state = load();
switch (command) {
  case "send": {
    state = { sentAt: Date.now(), seen: [] };
    state.requestId = await post(state, rest.join(" ") || "帮我整理一下这个项目的文档");
    break;
  }
  case "answer": {
    const q = state.question;
    if (!q) throw new Error("no open question in the state file");
    const at = rest.indexOf("--option");
    const option = at >= 0 ? rest[at + 1] : undefined;
    const words = at >= 0 ? rest.slice(0, at) : rest;
    const label = option ? String(q.options.find((o) => o.id === option)?.label ?? option) : words.join(" ");
    await post(state, label, {
      relates_to: { rel: "thread", target: q.anchor },
      ...(option ? { content: { machine: { intent: "agent_ask.answer", data: { call_id: q.callId, option } } } } : {}),
    });
    state.question = undefined;
    break;
  }
  case "stop":
    await post(state, "/stop");
    state.question = undefined;
    break;
  case "wait":
    break;
  default:
    console.log("usage: test_agent_ask.ts send <text> | answer <text> [--option <id>] | wait | stop");
    Deno.exit(2);
}
save(state);
Deno.exit(await follow(state));
