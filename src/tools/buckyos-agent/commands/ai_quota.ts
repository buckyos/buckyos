// ai_quota — quota.query
// Doc: aicc_agent_cli_tools.md §7.2
// 纯元数据查询；详细注释见 ai_provider.ts。

import { COMMON_OPTIONS_HELP, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import {
  bailAiccError,
  bailRuntimeError,
  emitAndExit,
  EXIT_SUCCESS,
  successResult,
} from "../lib/result.ts";
import { JsonValue } from "../lib/types.ts";

const TOOL = "ai_quota";
const METHOD = "quota.query";

export const HELP = `Usage: ai_quota [options]

Token quota rules that limit your AI calls (yours and your Apps' / Agents'), each
with its current window or total account: limit or grant, settled, reserved,
available, reset_at_ms (null for a total account).

Options:
  --app <app_id>   Only the rules that apply to calls of this App / Agent
${COMMON_OPTIONS_HELP}`;

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);

  const params: { app_id?: string } = {};
  const app = requireString(parsed.flags, "app");
  if (app !== undefined) params.app_id = app;

  let runtime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }

  let response: JsonValue;
  try {
    response = await runtime.buckyos.getAiccClient().queryQuota(
      params,
    ) as unknown as JsonValue;
  } catch (err) {
    bailAiccError(TOOL, METHOD, err);
  }

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
