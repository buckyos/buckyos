// ask_user — ask the user the Agent works for, as an asynchronous tool call.
// 配方骨架同 gen_image.ts；(2)(4)(5) 换成 lib/ask.ts：发一条 System Notify 的 `input`
// 通知（msg-center notify.send），它在 Zone TaskMgr 上带一个 `human.input/v1` 任务，
// 回答就是任务结果。命令立即返回 pending + 该 task_id，不在这里等。

import { bailArgError, HelpRequested } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { askCall, askUser, parseAskArgv } from "../lib/ask.ts";
import type { AskSpec } from "../lib/ask.ts";
import {
  bailRuntimeError,
  emitAndExit,
  errorResult,
  EXIT_ARG_ERROR,
  EXIT_SUCCESS,
} from "../lib/result.ts";

const TOOL = "ask_user";

export const HELP =
  `Usage: ask_user <question> [--option <id>=<label>]... [--allow-text] [--expires <seconds>]

Asks the user this Agent works for (the owner of its App) through a
notification. Without options it is a text question; with options (at least
two, unique ids and labels) the user picks one, and --allow-text also takes
free text. The question expires after --expires seconds (default 10800 = 3h).

The result is pending on the question's task (task_id, pending_reason
user_input): follow it with wait_task / get_task_state; the task's result is
the answer. Run ask_user as a shell command of its own: the question belongs
to that tool call ($XLLM_RUN_ID + $XLLM_CALL_ID), and running the same call
again finds the question already asked (or its answer) instead of asking twice.

Options:
  --option <id>=<label>   a choice; repeat for each option
  --allow-text            with --option: the user may answer in text instead
  --expires <seconds>     how long the question stays open (default 10800)

Exit codes: 0 asked (pending) or answered, 1 bad arguments / not a tool call
of a run, 2 sending failed, no recipient, dismissed or canceled, 4 expired.`;

export async function run(argv: string[]): Promise<never> {
  // ── 1. 解析参数 ────────────────────────────────────────────────────────
  let spec: AskSpec;
  try {
    spec = parseAskArgv(argv);
  } catch (err) {
    if (err instanceof HelpRequested) {
      console.error(HELP);
      Deno.exit(EXIT_SUCCESS);
    }
    bailArgError(TOOL, err as Error);
  }

  // ── 2. 这个问题属于哪次工具调用（幂等键 ask:<run_id>:<call_id>）─────────
  const call = askCall(Deno.env.toObject());
  if (!call) {
    const msg =
      "ask_user is a tool call of a run ($XLLM_RUN_ID and $XLLM_CALL_ID are not set); run it as its own shell command";
    emitAndExit(
      errorResult(TOOL, `${TOOL} => not_in_run`, msg, { error: msg }),
      EXIT_ARG_ERROR,
    );
  }

  // ── 3. 拿到 BuckyOS 运行时 ────────────────────────────────────────────
  let runtime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }

  // ── 4/5. 发通知（或找回这次调用已发的那条）→ 6. emit AgentToolResult ───
  const { result, exitCode } = await askUser(runtime, spec, call);
  emitAndExit(result, exitCode);
}

if (import.meta.main) {
  await run(Deno.args);
}
