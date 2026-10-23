// ai_provider — provider.list / provider.health / models.list
// Doc: aicc_agent_cli_tools.md §7.1
//
// 这些管理方法不创建 TaskMgr 任务，直接使用 SDK canonical client。

import {
  COMMON_OPTIONS_HELP,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
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
const MAX_PROVIDER_SUMMARY_LINES = 20;

export const HELP = `Usage:
  ai_provider list      # list configured providers
  ai_provider health <exact_model>  # health for one exact model
  ai_provider models [--api-type <type>]  # models that serve an api_type
                                          # (e.g. video.generate), with price
${COMMON_OPTIONS_HELP}`;

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  const sub = parsed.positional[0];
  let method: string;
  if (sub === "list") method = "provider.list";
  else if (sub === "health") method = "provider.health";
  else if (sub === "models") method = "models.list";
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
    } else if (sub === "models") {
      response = filterModels(
        await client.listModels() as unknown as JsonValue,
        requireString(parsed.flags, "api-type"),
      );
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

  const summary = sub === "list"
    ? formatProviderListSummary(response)
    : sub === "models"
    ? formatModelsSummary(response)
    : "provider health";
  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      summary,
      {
        method,
        response,
      },
      sub === "health" ? undefined : summary,
    ),
    EXIT_SUCCESS,
  );
}

export function formatProviderListSummary(response: JsonValue): string {
  if (!isRecord(response)) {
    return "provider list: invalid response";
  }
  const providers: Record<string, unknown>[] = Array.isArray(response.providers)
    ? (response.providers as unknown[]).filter(isRecord)
    : [];
  const settingsRevision = formatScalar(response.settings_revision);
  const inventoryRevision = formatScalar(response.inventory_revision);
  const lines = [
    `provider list: ${providers.length} provider(s), settings_revision=${settingsRevision}, inventory_revision=${inventoryRevision}`,
  ];

  for (const provider of providers.slice(0, MAX_PROVIDER_SUMMARY_LINES)) {
    const name = formatScalar(provider.provider_instance_name);
    const profile = formatScalar(provider.provider_profile_id);
    const type = formatScalar(provider.provider_type);
    const adapter = formatScalar(provider.protocol_adapter_id);
    const enabled = formatScalar(provider.enabled);
    const auth = isRecord(provider.auth) ? provider.auth : {};
    const inventory = isRecord(provider.inventory) ? provider.inventory : {};
    const health = isRecord(provider.health) ? provider.health : {};
    const authMode = formatScalar(auth.mode);
    const authConfigured = formatScalar(auth.configured);
    const inventoryState = formatScalar(inventory.state);
    const modelCount = formatScalar(inventory.model_count);
    const healthState = formatScalar(health.state);
    lines.push(
      `- ${name}: profile=${profile}, type=${type}, adapter=${adapter}, enabled=${enabled}, auth=${authMode}/${authConfigured}, inventory=${inventoryState}/${modelCount} models, health=${healthState}`,
    );
  }

  if (providers.length > MAX_PROVIDER_SUMMARY_LINES) {
    lines.push(
      `... ${
        providers.length - MAX_PROVIDER_SUMMARY_LINES
      } provider(s) omitted`,
    );
  }

  return lines.join("\n");
}

// One line per model: exact model, the media api_types it serves, the
// logical names that select it (`--model <name>`), and its list price.
export function filterModels(
  response: JsonValue,
  apiType: string | undefined,
): JsonValue {
  const models = isRecord(response) && Array.isArray(response.models)
    ? (response.models as unknown[]).filter(isRecord)
    : [];
  return models.flatMap((model) => {
    const apiTypes = Array.isArray(model.api_types)
      ? (model.api_types as unknown[]).filter((t): t is string =>
        typeof t === "string"
      )
      : [];
    const served = apiType ? apiTypes.filter((t) => t === apiType) : apiTypes;
    if (!served.length) return [];
    const mounts = Array.isArray(model.logical_mounts)
      ? (model.logical_mounts as unknown[]).filter((m): m is string =>
        typeof m === "string" &&
        served.some((t) => m === t || m.startsWith(`${t}.`))
      )
      : [];
    const attributes = isRecord(model.attributes) ? model.attributes : {};
    const pricing = isRecord(attributes.pricing) ? attributes.pricing : {};
    const price = isRecord(pricing.value) ? pricing.value : null;
    return [{
      exact_model: formatScalar(model.exact_model),
      api_types: served,
      logical_names: mounts,
      price: price && typeof price.amount === "number"
        ? `${price.amount} ${formatScalar(price.currency)}/${
          formatScalar(price.unit)
        }`
        : price
        ? "token-priced"
        : "unknown",
    }];
  }) as JsonValue;
}

export function formatModelsSummary(response: JsonValue): string {
  const models = Array.isArray(response)
    ? (response as unknown[]).filter(isRecord)
    : [];
  const lines = [`${models.length} model(s)`];
  for (const model of models.slice(0, 40)) {
    lines.push(
      `- ${formatScalar(model.exact_model)}: ${
        (model.api_types as string[]).join(",")
      }; price=${formatScalar(model.price)}; names=${
        (model.logical_names as string[]).join(",")
      }`,
    );
  }
  if (models.length > 40) lines.push(`... ${models.length - 40} omitted`);
  return lines.join("\n");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function formatScalar(value: unknown): string {
  if (value === null || value === undefined) return "unknown";
  if (
    typeof value === "string" || typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return String(value);
  }
  return "unknown";
}

if (import.meta.main) {
  await run(Deno.args);
}
