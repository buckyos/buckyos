// Ask the user the Agent works for: an `input` notification of System Notify
// (msg-center `notify.send`). msg-center gives it a Zone TaskMgr task
// (`human.input/v1`, NotifyView.task_id) whose result is the answer, so a
// question is an ordinary asynchronous tool call: `pending` on that task id,
// followed with wait_task / get_task_state like any long command.
// Wire types: src/kernel/buckyos-api/src/msg_notify.rs (websdk msg_center_client.ts).

import { inputPrompt, parseSessionTokenClaims, promptOptions } from "buckyos";

import { ArgError, flagInt, parseArgs, takeRepeatedFlags } from "./cli.ts";
import { errorResult, pendingResult, successResult } from "./result.ts";
import type { AiccRuntime } from "./runtime.ts";
import {
  type AgentToolResult,
  EXIT_AICC_FAILED,
  EXIT_SUCCESS,
  EXIT_TIMEOUT,
  type JsonValue,
} from "./types.ts";

export const ASK_TOOL = "ask_user";
export const ASK_PENDING_REASON = "user_input";
export const DEFAULT_ASK_EXPIRES_SECS = 3 * 60 * 60;
const TITLE_CHARS = 120;

export interface AskOption {
  id: string;
  label: string;
}

export interface AskSpec {
  question: string;
  // Empty: a text question. Otherwise a choice of at least two.
  options: AskOption[];
  // A choice that also takes free text.
  allowText: boolean;
  expiresSecs: number;
}

// The tool call a question belongs to (set by the `shell` tool).
export interface AskCall {
  runId: string;
  callId: string;
  sessionId?: string;
}

export interface AskOutcome {
  result: AgentToolResult;
  exitCode: number;
}

export type AskRuntime = Pick<AiccRuntime, "buckyos" | "sessionToken">;

// The wire shapes used here (msg_notify.rs NotifySendReq / NotifyView); the
// SDK's declared types are not visible through its .mjs entry.
interface PromptOption {
  value: string;
  label: string;
}

interface HumanTaskPrompt {
  title?: string;
  message?: string;
  controls?: unknown[];
}

export interface AskNotifyRequest {
  to_user: string;
  kind: "input";
  level: "timely";
  prompt: HumanTaskPrompt;
  source: { call_id: string; session_id?: string };
  expires_at_ms: number;
  idempotency_key: string;
}

export interface AskNotifyView {
  notify_id: string;
  url: string;
  status: string;
  prompt: HumanTaskPrompt;
  task_id?: string;
  expires_at_ms?: number;
  closed_reason?: string;
  result?: {
    action: string;
    values?: Record<string, unknown>;
    submitted_by?: string;
    submitted_at?: number;
  };
}

function parseOption(value: string): AskOption {
  const at = value.indexOf("=");
  const id = at < 0 ? "" : value.slice(0, at).trim();
  const label = at < 0 ? "" : value.slice(at + 1).trim();
  if (!id || !label) {
    throw new ArgError(`--option expects <id>=<label>, got ${value}`);
  }
  return { id, label };
}

export function askSpec(
  question: string,
  optionArgs: string[],
  allowText: boolean,
  expiresSecs: number = DEFAULT_ASK_EXPIRES_SECS,
): AskSpec {
  const text = question.trim();
  if (!text) throw new ArgError("the question is empty");
  const options = optionArgs.map(parseOption);
  if (options.length === 1) {
    throw new ArgError("a choice needs at least two --option");
  }
  for (const key of ["id", "label"] as const) {
    const seen = new Set<string>();
    for (const option of options) {
      if (seen.has(option[key])) {
        throw new ArgError(`two options have the ${key} ${option[key]}`);
      }
      seen.add(option[key]);
    }
  }
  if (allowText && !options.length) {
    throw new ArgError(
      "--allow-text goes with --option: a question without options is answered in text",
    );
  }
  if (!Number.isInteger(expiresSecs) || expiresSecs <= 0) {
    throw new ArgError(
      `--expires must be a positive number of seconds, got ${expiresSecs}`,
    );
  }
  return { question: text, options, allowText, expiresSecs };
}

// `ask_user <question> [--option <id>=<label>]... [--allow-text] [--expires <seconds>]`.
// Throws ArgError, or HelpRequested for -h / --help.
export function parseAskArgv(argv: string[]): AskSpec {
  const { rest, values } = takeRepeatedFlags(argv, ["option"]);
  const { positional, flags } = parseArgs(rest, new Set(["allow-text"]));
  for (const name of flags.keys()) {
    if (name !== "allow-text" && name !== "expires") {
      throw new ArgError(`unknown option --${name}`);
    }
  }
  if (positional.length !== 1) {
    throw new ArgError(
      positional.length
        ? `the question is one argument (quote it), got ${positional.length}`
        : "missing <question>",
    );
  }
  return askSpec(
    positional[0],
    values.get("option") ?? [],
    flags.get("allow-text") === true,
    flagInt(flags, "expires") ?? DEFAULT_ASK_EXPIRES_SECS,
  );
}

function envValue(
  env: Record<string, string | undefined>,
  name: string,
): string | undefined {
  const value = env[name]?.trim();
  return value ? value : undefined;
}

// `null` outside a tool call of a run.
export function askCall(
  env: Record<string, string | undefined>,
): AskCall | null {
  const runId = envValue(env, "XLLM_RUN_ID");
  const callId = envValue(env, "XLLM_CALL_ID");
  if (!runId || !callId) return null;
  const sessionId = envValue(env, "OPENDAN_SESSION_ID");
  return sessionId ? { runId, callId, sessionId } : { runId, callId };
}

// msg-center keeps one notification per requester + key: a re-run of the
// same call finds the question it already asked.
export function askIdempotencyKey(call: AskCall): string {
  return `ask:${call.runId}:${call.callId}`;
}

// The zone user the session token speaks for. An Agent's tools carry its
// App's verify-hub session (principal `app`), whose `sub` is the owner of the
// App instance (verify-hub refuses any other); msg-center lets such a caller
// notify only that user.
export function askRecipient(sessionToken: string | undefined): string {
  const claims = parseSessionTokenClaims(sessionToken) as
    | Record<string, unknown>
    | null;
  if (!claims) {
    throw new Error("the BuckyOS session token is missing or unreadable");
  }
  const sub = typeof claims.sub === "string" ? claims.sub.trim() : "";
  const kind = typeof claims.principal_kind === "string"
    ? claims.principal_kind
    : undefined;
  if (kind === "app") {
    const owner = typeof claims.app_owner_user_id === "string"
      ? claims.app_owner_user_id.trim()
      : "";
    if (!sub) throw new Error("the App session token has no subject");
    if (owner && owner !== sub) {
      throw new Error(
        `the App session token's subject ${sub} is not its app_owner_user_id ${owner}`,
      );
    }
    return sub;
  }
  if (kind === "user" && claims.external !== true && sub) return sub;
  throw new Error(
    `a ${
      kind ?? "non verify-hub"
    } session token does not name the zone user this agent works for (need an App or zone user session)`,
  );
}

function firstLine(text: string, max: number): string {
  const line = text.split("\n", 1)[0].trim();
  const chars = Array.from(line);
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : line;
}

export function askNotifyRequest(
  spec: AskSpec,
  call: AskCall,
  toUser: string,
  nowMs: number,
): AskNotifyRequest {
  const title = firstLine(spec.question, TITLE_CHARS);
  const message = spec.question === title ? "" : spec.question;
  const options: PromptOption[] = spec.options.map((option) => ({
    value: option.id,
    label: option.label,
  }));
  const source: AskNotifyRequest["source"] = { call_id: call.callId };
  if (call.sessionId) source.session_id = call.sessionId;
  return {
    to_user: toUser,
    kind: "input",
    level: "timely",
    prompt: inputPrompt(
      title,
      message,
      options,
      spec.allowText,
    ) as HumanTaskPrompt,
    source,
    expires_at_ms: nowMs + spec.expiresSecs * 1000,
    idempotency_key: askIdempotencyKey(call),
  };
}

function nonEmpty(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

// The result of the call from the state of its notification: `pending` on
// the task while open; the answer, or an error, once it is closed.
export function askResult(
  view: AskNotifyView,
  tool: string = ASK_TOOL,
): AskOutcome {
  const options = (promptOptions(view.prompt) as PromptOption[]).map((
    option,
  ) => ({ id: option.value, label: option.label }));
  const detail: Record<string, JsonValue> = {
    notify_id: view.notify_id,
    task_id: view.task_id ?? null,
    status: view.status,
    question: view.prompt.message ?? view.prompt.title ?? "",
    options,
    expires_at_ms: view.expires_at_ms ?? null,
    url: view.url,
  };
  const failed = (reason: string, summary: string, exitCode: number) => ({
    result: errorResult(tool, `${tool} => ${reason}`, summary, {
      ...detail,
      error: summary,
    }),
    exitCode,
  });
  switch (view.status) {
    case "open":
    case "read":
      if (!view.task_id) {
        return failed(
          "no_task",
          `notification ${view.notify_id} has no task to wait for`,
          EXIT_AICC_FAILED,
        );
      }
      return {
        result: pendingResult(
          tool,
          ASK_PENDING_REASON,
          view.task_id,
          `${tool} => waiting`,
          "waiting for the user's answer",
          detail,
        ),
        exitCode: EXIT_SUCCESS,
      };
    case "answered": {
      const values = view.result?.values ?? {};
      const option = nonEmpty(values.option);
      const label = options.find((o) => o.id === option)?.label;
      const text = nonEmpty(values.text);
      const output = [label ?? option, text].filter(Boolean).join("\n");
      const summary = `the user answered: ${firstLine(output, 200)}`;
      return {
        result: successResult(tool, `${tool} => answered`, summary, {
          ...detail,
          answer: {
            option: option ?? null,
            label: label ?? null,
            text: text ?? null,
            submitted_by: view.result?.submitted_by ?? null,
            submitted_at: view.result?.submitted_at ?? null,
          },
        }, output),
        exitCode: EXIT_SUCCESS,
      };
    }
    case "dismissed":
      return failed(
        "dismissed",
        "the user dismissed the question without answering",
        EXIT_AICC_FAILED,
      );
    case "expired":
      return failed(
        "expired",
        "the question expired before the user answered",
        EXIT_TIMEOUT,
      );
    case "canceled":
      return failed(
        "canceled",
        `the question was canceled${
          view.closed_reason ? ` (${view.closed_reason})` : ""
        }`,
        EXIT_AICC_FAILED,
      );
    default:
      return failed(
        "unknown_status",
        `notification ${view.notify_id} is ${view.status}`,
        EXIT_AICC_FAILED,
      );
  }
}

// Send the question (or find the one this call already sent) and map its state.
export async function askUser(
  runtime: AskRuntime,
  spec: AskSpec,
  call: AskCall,
  nowMs: number = Date.now(),
  tool: string = ASK_TOOL,
): Promise<AskOutcome> {
  let toUser: string;
  try {
    toUser = askRecipient(runtime.sessionToken);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    return {
      result: errorResult(tool, `${tool} => no_recipient`, msg, { error: msg }),
      exitCode: EXIT_AICC_FAILED,
    };
  }
  const request = askNotifyRequest(spec, call, toUser, nowMs);
  let view: AskNotifyView;
  try {
    view = await runtime.buckyos.getMsgCenterClient().notifySend(request);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    return {
      result: errorResult(tool, `${tool} => notify_failed`, msg, {
        method: "notify.send",
        to_user: toUser,
        idempotency_key: request.idempotency_key,
        error: msg,
      }),
      exitCode: EXIT_AICC_FAILED,
    };
  }
  return askResult(view, tool);
}
