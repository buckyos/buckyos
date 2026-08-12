// ai_provider — provider.list / provider.health
// Doc: aicc_agent_cli_tools.md §7.1
//
// 注意：这俩 method 是纯元数据查询，不走 AiMethodRequest envelope，也不进
// task-manager；只调用一次 aicc kRPC，结果直接塞进 AgentToolResult.detail。

import { COMMON_OPTIONS_HELP, parseArgvOrExit } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import {
  bailAiccError, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";
import { JsonValue } from "../lib/types.ts";

const TOOL = "ai_provider";

export const HELP = `Usage:
  ai_provider list      # list configured providers
  ai_provider health    # provider health snapshot
${COMMON_OPTIONS_HELP}`;

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  const sub = parsed.positional[0];
  let method: string;
  if (sub === "list") method = "provider.list";
  else if (sub === "health") method = "provider.health";
  else {
    const msg = sub ? `unknown subcommand: ${sub}` : "missing subcommand";
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, msg, { error: msg, help: HELP }),
      EXIT_ARG_ERROR,
    );
  }

  let runtime;
  try { runtime = await initRuntime(); } catch (err) { bailRuntimeError(TOOL, err); }

  let response: JsonValue;
  try {
    // deno-lint-ignore no-explicit-any
    const aiccRpc = (runtime.buckyos as any).getServiceRpcClient("aicc");
    response = await aiccRpc.call(method, {}) as JsonValue;
  } catch (err) { bailAiccError(TOOL, method, err); }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, sub === "list" ? "provider list" : "provider health", {
      method, response,
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
