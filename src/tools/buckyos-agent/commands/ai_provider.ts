// ai_provider — provider.list / provider.health
// Doc: aicc_agent_cli_tools.md §7.1
//
// 这两个管理方法不创建 TaskMgr 任务，直接使用 SDK canonical client。

import { COMMON_OPTIONS_HELP, parseArgvOrExit } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import {
  bailAiccError,
  bailRuntimeError,
  emitAndExit,
  errorResult,
  EXIT_ARG_ERROR,
  EXIT_SUCCESS,
  successResult,
} from "../lib/result.ts";
import { JsonValue } from "../lib/types.ts";

const TOOL = "ai_provider";

export const HELP = `Usage:
  ai_provider list      # list configured providers
  ai_provider health <exact_model>  # health for one exact model
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
      errorResult(TOOL, `${TOOL} => arg_error`, msg, {
        error: msg,
        help: HELP,
      }),
      EXIT_ARG_ERROR,
    );
  }

  let runtime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }

  let response: JsonValue;
  try {
    const client = runtime.buckyos.getAiccClient();
    if (sub === "list") {
      response = await client.listProviders() as unknown as JsonValue;
    } else {
      const exactModel = parsed.positional[1];
      if (!exactModel) {
        emitAndExit(
          errorResult(
            TOOL,
            `${TOOL} => arg_error`,
            "health requires <exact_model>",
            {
              error: "health requires <exact_model>",
              help: HELP,
            },
          ),
          EXIT_ARG_ERROR,
        );
      }
      response = await client.providerHealth({
        exact_model: exactModel,
      }) as unknown as JsonValue;
    }
  } catch (err) {
    bailAiccError(TOOL, method, err);
  }

  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      sub === "list" ? "provider list" : "provider health",
      {
        method,
        response,
      },
    ),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
