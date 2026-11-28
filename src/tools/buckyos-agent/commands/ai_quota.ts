// ai_quota — policy.status
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
const METHOD = "policy.status";

export const HELP = `Usage: ai_quota [options]

Your AI Credit balance (decimal string, may be negative) and the usage limits
that apply to your calls (yours and your Apps' / Agents'): period (5h / 24h /
7d), unit (credit / token), amount, used, status (normal / near_limit /
exhausted / forbidden) and reset_at_ms, the end of the current window (absent
when no window is open: it starts at the next use).

Options:
  --app <app_id>   Also the limits on calls through this App / Agent
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
    response = await runtime.buckyos.getAiccClient().policyStatus(
      params,
    ) as unknown as JsonValue;
  } catch (err) {
    bailAiccError(TOOL, METHOD, err);
  }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, "AI Credit and usage limits", {
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
