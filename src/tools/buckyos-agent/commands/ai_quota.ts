// ai_quota — quota.query
// Doc: aicc_agent_cli_tools.md §7.2
// 纯元数据查询；详细注释见 ai_provider.ts。

import { COMMON_OPTIONS_HELP, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import {
  bailAiccError, bailRuntimeError,
  emitAndExit, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";
import { JsonValue } from "../lib/types.ts";

const TOOL = "ai_quota";
const METHOD = "quota.query";

export const HELP = `Usage: ai_quota [options]

Options:
  --capability <llm|embedding|rerank|image|vision|audio|video|agent>
  --method <aicc_method>
${COMMON_OPTIONS_HELP}`;

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);

  const params: Record<string, unknown> = {};
  const cap = requireString(parsed.flags, "capability");
  if (cap !== undefined) params.capability = cap;
  const method = requireString(parsed.flags, "method");
  if (method !== undefined) params.method = method;

  let runtime;
  try { runtime = await initRuntime(); } catch (err) { bailRuntimeError(TOOL, err); }

  let response: JsonValue;
  try {
    // deno-lint-ignore no-explicit-any
    const aiccRpc = (runtime.buckyos as any).getServiceRpcClient("aicc");
    response = await aiccRpc.call(METHOD, params) as JsonValue;
  } catch (err) { bailAiccError(TOOL, METHOD, err); }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, "quota query", {
      method: METHOD,
      params: params as JsonValue,
      response,
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
