import {
  ASSET_ENV,
  ASSET_FILE,
  ASSET_LABEL,
  type AssetKey,
  type Scenario,
  type ScenarioStep,
  SCENARIOS,
} from "./scenarios.ts";
import {
  type FlatToml,
  parseToml,
  tomlBoolean,
  tomlNumber,
  tomlString,
  tomlStrings,
} from "./config.ts";
import {
  TelegramDvClient,
  type TelegramObservedMessage,
} from "./telegram_client.ts";
import {
  queryUsageEvents,
  queryRouteTraces,
  providerCoverage,
  providerInstanceFromExactModel,
  usageEventFinance,
} from "../aicc_test/acceptance/usage_audit.ts";
import {
  applyProviderTokens,
  configuredProviderTokens,
  providerTokenDrivers,
  selectProviderTokens,
  type ProviderTokens,
} from "../aicc_test/acceptance/provider_credentials.ts";
import { withAiccSettingsOverride } from "../aicc_test/acceptance/settings_transaction.ts";
import {
  type ArtifactAudit,
  validateArtifactBytes,
  validateNamedArtifact,
} from "../aicc_test/acceptance/artifact_validation.ts";
import type { ProductDefect } from "../aicc_test/acceptance/types.ts";

type JsonObject = Record<string, unknown>;

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

type NdmProxyClient = {
  putChunk: (objId: string, bytes: Uint8Array) => Promise<unknown>;
  openReader: (request: { obj_id: string; inner_path?: string | null }) => Promise<{
    body: ReadableStream<Uint8Array> | null;
    totalSize: number | null;
  }>;
  removeChunk: (request: { chunk_id: string }) => Promise<unknown>;
  removeObject: (request: { obj_id: string }) => Promise<unknown>;
};

type PasswordLoginResponse = {
  session_token?: unknown;
  user_info?: { user_id?: unknown };
};

type RefItem = {
  role: string;
  target: { type: string; obj_id?: string };
  label?: string;
};

type MsgObject = {
  from: string;
  source?: string;
  to: string[];
  kind: string;
  thread?: { topic?: string; reply_to?: string; correlation_id?: string };
  created_at_ms: number;
  content: {
    format?: string;
    content: string;
    refs?: RefItem[];
  };
  [key: string]: unknown;
};

type SessionMessageItem = {
  record_id: string;
  msg_id: string;
  direction: "in" | "out";
  sort_key: number;
  from: string;
  to: string;
  msg?: MsgObject | null;
};

type SessionMessagePage = {
  items?: SessionMessageItem[];
  next_cursor_sort_key?: number;
  next_cursor_record_id?: string;
};

class ReplyWaitError extends Error {
  readonly items: SessionMessageItem[];
  readonly checks: string[];

  constructor(
    message: string,
    items: SessionMessageItem[],
    checks: string[],
  ) {
    super(message);
    this.name = "ReplyWaitError";
    this.items = items;
    this.checks = checks;
  }
}

class JudgeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "JudgeError";
  }
}

type AiMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: unknown;
};

const T3_JUDGE_RUBRIC_VERSION = "2026-08-27.1";

type StepStatus = "passed" | "failed" | "review" | "skipped" | "dispatched" |
  "not_applicable" | "platform_limitation";

export type StepResult = {
  transport?: Transport;
  scenario_id: string;
  step_id: string;
  status: StepStatus;
  started_at: string;
  elapsed_ms: number;
  prompt: string;
  attachment?: AssetKey;
  attachments?: AssetKey[];
  reply_texts: string[];
  reply_refs: RefItem[];
  artifact_audits?: ArtifactAudit[];
  automatic_checks: string[];
  review: string[];
  failure_class?: "message_transport_failed" | "attachment_failed" | "usage_failed" |
    "judge_failed" | "assertion_failed" | "cleanup_failed" | "platform_limitation";
  notes?: string;
  error?: string;
  judge?: {
    rubric_version: string;
    model: string;
    task_id: string;
    passed: boolean;
    score: number;
    reason: string;
    input_summary: {
      reply_text_count: number;
      attachment_count: number;
      attachment_types: string[];
    };
    exact_model?: string;
    provider_instance?: string;
    provider_driver?: string;
    distinct_provider_or_family?: boolean;
  };
};

export type EntryMessageKind = "text" | "image" | "video" | "audio" | "document" |
  "archive" | "multi_attachment";

export type EntryCoverageRecord = {
  transport: Transport;
  direction: "inbound" | "outbound";
  kind: EntryMessageKind;
  status: "covered" | "review" | "failed" | "skipped" | "missing" | "not_applicable" |
    "platform_limitation";
  planned_case_ids: string[];
  covered_case_ids: string[];
};

type RunReport = {
  run_id: string;
  started_at: string;
  finished_at?: string;
  commit: string;
  baseline_revision: string;
  transports: Transport[];
  expected_providers: string[];
  suite: string;
  gateway_url?: string;
  user_did?: string;
  jarvis_did?: string;
  telegram_bot?: string;
  selected_scenarios: string[];
  entry_coverage?: EntryCoverageRecord[];
  provider_credential_override?: {
    drivers: string[];
    cleanup: "restored";
  };
  resource_cleanup?: {
    status: "passed" | "failed";
    removed_fixture_ids: string[];
    removed_artifact_ids: string[];
    residual_ids: string[];
  };
  conversation_cleanup?: {
    status: "passed" | "failed" | "platform_limitation";
    removed_external_message_ids: string[];
    residual_session_ids: string[];
    details: string[];
  };
  results: StepResult[];
  judge: {
    enabled: boolean;
    model: string;
    task_ids: string[];
  };
  finance?: {
    status: "passed" | "failed";
    attribution: "caller_app_and_time_window_plus_judge_task_ids";
    attribution_limitation: string;
    caller_app_ids: string[];
    budget_usd: number;
    planned_max_calls: number;
    planned_max_cost_usd: number;
    observed_provider_instances: string[];
    observed_provider_drivers: string[];
    missing_expected_providers: string[];
    event_count: number;
    workflow_event_count: number;
    judge_event_count: number;
    input_tokens: number;
    output_tokens: number;
    total_tokens: number;
    request_units: number;
    actual_cost_usd: number;
    raw_cost_usd: number;
    credit_applied_usd: number;
    unknown_cost_events: number;
    estimated_unknown_cost_usd: number;
    total_exposure_usd: number;
    remaining_budget_usd: number;
    budget_exceeded: boolean;
    step_correlation: {
      status: "passed" | "failed";
      expected_step_ids: string[];
      correlated_step_ids: string[];
      uncorrelated_step_ids: string[];
      trace_count: number;
      defect?: string;
    };
    events: Array<{
      event_id: string;
      task_id: string;
      caller_app_id?: string;
      api_type: string;
      request_model: string;
      exact_model: string;
      provider_instance?: string;
      provider_driver?: string;
      actual_cost_usd?: number;
      input_tokens?: number;
      output_tokens?: number;
      total_tokens?: number;
      request_units?: number;
    }>;
    error?: string;
  };
  totals?: Record<StepStatus, number>;
  product_defects?: ProductDefect[];
  targeted_retest_command?: string;
};

type Transport = "msg-center" | "telegram";

const GATEWAY_REQUIRED_ERROR =
  "gateway URL is required; use --gateway-url, msg_center.gateway_url, or BUCKYOS_TEST_GATEWAY_URL";

type CliOptions = {
  transports: Transport[];
  expectedProviders: string[];
  financeCallerAppIds: string[];
  financeBudgetUsd: number;
  estimatedCostPerAiccCallUsd: number;
  maxAiccCallsPerStep: number;
  judgeEnabled: boolean;
  judgeModel: string;
  assumeYes: boolean;
  suite: "smoke" | "linked" | "matrix" | "all";
  caseIds: string[];
  configPath?: string;
  gatewayUrl: string;
  sessionToken?: string;
  username?: string;
  password?: string;
  userId: string;
  userDid?: string;
  zoneDid?: string;
  jarvisDid?: string;
  groupDid?: string;
  telegramApiId?: number;
  telegramApiHash?: string;
  telegramPhone?: string;
  telegramCode?: string;
  telegramPassword?: string;
  telegramBotUsername?: string;
  telegramSession?: string;
  telegramSessionFile: string;
  telegramConnectionRetries: number;
  assets: Partial<Record<AssetKey, string>>;
  telegramAssets: Partial<Record<AssetKey, string>>;
  interactiveReview: boolean;
  allowReview: boolean;
  dryRun: boolean;
  list: boolean;
  reportDir: string;
  settleMs: number;
  scenarioConcurrency: number;
  parameterSources: Record<string, string>;
  providerTokens: ProviderTokens;
  providerInstances: Record<string, string>;
  applyProviderCredentials: boolean;
  allowCredentialMutationCli: boolean;
};

const FLAG_TO_ASSET: Record<string, AssetKey> = {
  "image-primary-id": "image_primary",
  "image-secondary-id": "image_secondary",
  "image-ocr-id": "image_ocr",
  "audio-sfx-id": "audio_sfx",
  "audio-speech-id": "audio_speech",
  "video-fresh-id": "video_fresh",
  "video-subtitle-id": "video_subtitle",
  "document-facts-id": "document_facts",
  "archive-mixed-id": "archive_mixed",
  "archive-single-document-id": "archive_single_document",
  "archive-multiple-documents-id": "archive_multiple_documents",
  "archive-nested-id": "archive_nested",
  "archive-empty-id": "archive_empty",
  "archive-corrupt-id": "archive_corrupt",
  "archive-encrypted-id": "archive_encrypted",
  "archive-path-traversal-id": "archive_path_traversal",
  "archive-many-files-id": "archive_many_files",
  "archive-large-expansion-id": "archive_large_expansion",
  "archive-deep-nesting-id": "archive_deep_nesting",
};

const FLAG_TO_TELEGRAM_ASSET: Record<string, AssetKey> = {
  "image-primary-file": "image_primary",
  "image-secondary-file": "image_secondary",
  "image-ocr-file": "image_ocr",
  "audio-sfx-file": "audio_sfx",
  "audio-speech-file": "audio_speech",
  "video-fresh-file": "video_fresh",
  "video-subtitle-file": "video_subtitle",
  "document-facts-file": "document_facts",
  "archive-mixed-file": "archive_mixed",
  "archive-single-document-file": "archive_single_document",
  "archive-multiple-documents-file": "archive_multiple_documents",
  "archive-nested-file": "archive_nested",
  "archive-empty-file": "archive_empty",
  "archive-corrupt-file": "archive_corrupt",
  "archive-encrypted-file": "archive_encrypted",
  "archive-path-traversal-file": "archive_path_traversal",
  "archive-many-files-file": "archive_many_files",
  "archive-large-expansion-file": "archive_large_expansion",
  "archive-deep-nesting-file": "archive_deep_nesting",
};

function env(name: string): string | undefined {
  const value = Deno.env.get(name)?.trim();
  return value ? value : undefined;
}

function requiredValue(args: string[], index: number, flag: string): string {
  const value = args[index + 1]?.trim();
  if (!value || value.startsWith("--")) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function parsePositiveInt(raw: string | undefined, fallback: number, name: string): number {
  if (!raw) return fallback;
  const value = Number(raw);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

function parseBoolean(raw: string | undefined, fallback: boolean, name: string): boolean {
  if (!raw) return fallback;
  if (/^(1|true|yes|on)$/i.test(raw)) return true;
  if (/^(0|false|no|off)$/i.test(raw)) return false;
  throw new Error(`${name} must be true or false`);
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await Deno.stat(path);
    return true;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return false;
    throw error;
  }
}

function configPathFromArgs(args: string[]): { path?: string; explicit: boolean } {
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--config") {
      return { path: requiredValue(args, index, "--config"), explicit: true };
    }
  }
  return { path: "jarvis_media_dv.local.toml", explicit: false };
}

async function loadLocalConfig(args: string[]): Promise<{ path?: string; config: FlatToml }> {
  const selected = configPathFromArgs(args);
  if (!selected.path || !await pathExists(selected.path)) {
    if (selected.explicit) throw new Error(`config file not found: ${selected.path}`);
    return { config: {} };
  }
  return {
    path: selected.path,
    config: parseToml(await Deno.readTextFile(selected.path)),
  };
}

function transportValue(raw: string): Transport {
  if (raw !== "msg-center" && raw !== "telegram") {
    throw new Error("transport must be msg-center or telegram");
  }
  return raw;
}

function listValue(raw: string | undefined): string[] | undefined {
  if (!raw) return undefined;
  return raw.split(",").map((value) => value.trim()).filter(Boolean);
}

function modelFamily(exactModel: string): string {
  return exactModel.split("@")[0].split(":")[0].replace(/[-_]20\d{2}.*$/, "").replace(/[-_]v?\d+(?:\.\d+)*$/, "");
}

function uniqueValues<T>(values: T[]): T[] {
  return [...new Set(values)];
}

function parameterSource(
  args: string[],
  config: FlatToml,
  configPath: string | undefined,
  cliFlags: string[],
  configKey: string,
  envNames: string[],
): string {
  if (cliFlags.some((flag) => args.includes(flag))) return "command line";
  if (configKey in config) return `TOML:${configPath ?? "local config"}`;
  const environmentName = envNames.find((name) => env(name) !== undefined);
  return environmentName ? `environment:${environmentName}` : "default";
}

function suiteValue(raw: string | undefined): CliOptions["suite"] {
  const value = raw ?? "all";
  if (value !== "smoke" && value !== "linked" && value !== "matrix" && value !== "all") {
    throw new Error("suite must be smoke, linked, matrix, or all");
  }
  return value;
}

async function parseArgs(args: string[]): Promise<CliOptions> {
  const loaded = await loadLocalConfig(args);
  const config = loaded.config;
  const configuredRetries = tomlNumber(config, "telegram.connection_retries");
  const configuredSettleMs = tomlNumber(config, "common.settle_ms");
  const providerInstances: Record<string, string> = {};
  for (const [key, value] of Object.entries(config)) {
    const match = /^provider_credentials\.([^.]+)\.instance_name$/.exec(key);
    if (match && typeof value === "string" && value.trim()) providerInstances[match[1]] = value.trim();
  }
  for (const [key, value] of Object.entries(config)) {
    const match = /^instances\.([^.]+)\.name$/.exec(key);
    if (match && typeof value === "string" && value.trim()) providerInstances[match[1]] = value.trim();
  }
  const options: CliOptions = {
    transports: uniqueValues(
      (tomlStrings(config, "common.transports") ??
        listValue(env("JARVIS_DV_TRANSPORTS")) ??
        ["msg-center"]).map((value) => transportValue(value.trim())),
    ),
    expectedProviders: uniqueValues(
      (tomlStrings(config, "environment.providers") ??
        listValue(env("JARVIS_DV_PROVIDERS")) ?? [])
        .map((value) => value.trim()).filter(Boolean),
    ),
    financeCallerAppIds: uniqueValues(
      (tomlStrings(config, "finance.caller_app_ids") ??
        listValue(env("JARVIS_DV_FINANCE_CALLER_APP_IDS")) ?? [])
        .map((value) => value.trim()).filter(Boolean),
    ),
    financeBudgetUsd: tomlNumber(config, "finance.max_cost_usd") ??
      Number(env("JARVIS_DV_MAX_COST_USD") ?? "100"),
    estimatedCostPerAiccCallUsd: tomlNumber(config, "finance.estimated_cost_per_aicc_call_usd") ??
      Number(env("JARVIS_DV_ESTIMATED_COST_PER_AICC_CALL_USD") ?? "0.05"),
    maxAiccCallsPerStep: tomlNumber(config, "finance.max_aicc_calls_per_step") ??
      Number(env("JARVIS_DV_MAX_AICC_CALLS_PER_STEP") ?? "4"),
    judgeEnabled: tomlBoolean(config, "judge.enabled") ??
      parseBoolean(env("JARVIS_DV_JUDGE_ENABLED"), true, "JARVIS_DV_JUDGE_ENABLED"),
    judgeModel: tomlString(config, "judge.model") ?? env("JARVIS_DV_JUDGE_MODEL") ?? "llm.plan.default",
    assumeYes: tomlBoolean(config, "common.yes") ??
      parseBoolean(env("JARVIS_DV_YES"), false, "JARVIS_DV_YES"),
    suite: suiteValue(tomlString(config, "common.suite") ?? env("JARVIS_DV_SUITE")),
    caseIds: tomlStrings(config, "common.cases") ?? [],
    configPath: loaded.path,
    gatewayUrl: tomlString(config, "msg_center.gateway_url") ??
      env("BUCKYOS_TEST_GATEWAY_URL") ?? "",
    sessionToken: tomlString(config, "msg_center.session_token") ??
      env("BUCKYOS_APPCLIENT_SESSION_TOKEN"),
    username: tomlString(config, "msg_center.username") ?? env("BUCKYOS_TEST_USERNAME"),
    password: tomlString(config, "msg_center.password") ?? env("BUCKYOS_TEST_PASSWORD"),
    userId: tomlString(config, "msg_center.user_id") ?? env("BUCKYOS_TEST_USER_ID") ?? "",
    userDid: tomlString(config, "msg_center.user_did") ?? env("JARVIS_DV_USER_DID"),
    zoneDid: tomlString(config, "msg_center.zone_did") ?? env("JARVIS_DV_ZONE_DID"),
    jarvisDid: tomlString(config, "msg_center.jarvis_did") ?? env("JARVIS_DV_AGENT_DID"),
    groupDid: tomlString(config, "msg_center.group_did") ?? env("JARVIS_DV_GROUP_DID"),
    telegramApiId: tomlNumber(config, "telegram.api_id") ??
      (parsePositiveInt(env("TELEGRAM_API_ID"), 0, "TELEGRAM_API_ID") || undefined),
    telegramApiHash: tomlString(config, "telegram.api_hash") ?? env("TELEGRAM_API_HASH"),
    telegramPhone: tomlString(config, "telegram.phone") ?? env("TELEGRAM_PHONE"),
    telegramCode: tomlString(config, "telegram.code") ?? env("TELEGRAM_CODE"),
    telegramPassword: tomlString(config, "telegram.password") ?? env("TELEGRAM_PASSWORD"),
    telegramBotUsername: tomlString(config, "telegram.bot_username") ??
      env("JARVIS_TELEGRAM_BOT_USERNAME"),
    telegramSession: tomlString(config, "telegram.session") ?? env("TELEGRAM_SESSION"),
    telegramSessionFile: tomlString(config, "telegram.session_file") ??
      env("TELEGRAM_SESSION_FILE") ?? ".jarvis_media_dv.telegram.session",
    telegramConnectionRetries: configuredRetries !== undefined
      ? parsePositiveInt(String(configuredRetries), 5, "telegram.connection_retries")
      : parsePositiveInt(env("TELEGRAM_CONNECTION_RETRIES"), 5, "TELEGRAM_CONNECTION_RETRIES"),
    assets: {},
    telegramAssets: {},
    interactiveReview: tomlBoolean(config, "common.interactive_review") ??
      parseBoolean(env("JARVIS_DV_INTERACTIVE_REVIEW"), false, "JARVIS_DV_INTERACTIVE_REVIEW"),
    allowReview: tomlBoolean(config, "common.allow_review") ??
      parseBoolean(env("JARVIS_DV_ALLOW_REVIEW"), false, "JARVIS_DV_ALLOW_REVIEW"),
    dryRun: false,
    list: false,
    reportDir: tomlString(config, "common.report_dir") ??
      env("JARVIS_DV_REPORT_DIR") ?? "reports/jarvis_media_dv",
    settleMs: configuredSettleMs !== undefined
      ? parsePositiveInt(String(configuredSettleMs), 10_000, "common.settle_ms")
      : parsePositiveInt(env("JARVIS_DV_SETTLE_MS"), 10_000, "JARVIS_DV_SETTLE_MS"),
    scenarioConcurrency: parsePositiveInt(
      tomlNumber(config, "common.scenario_concurrency")?.toString() ??
        env("JARVIS_DV_SCENARIO_CONCURRENCY"),
      2,
      "common.scenario_concurrency",
    ),
    parameterSources: {},
    providerTokens: configuredProviderTokens(config, env),
    providerInstances,
    applyProviderCredentials: tomlBoolean(config, "provider_credentials.apply_to_aicc_settings") ?? false,
    allowCredentialMutationCli: false,
  };

  for (const asset of Object.keys(ASSET_ENV) as AssetKey[]) {
    const value = tomlString(config, `msg_center.assets.${asset}_id`) ?? env(ASSET_ENV[asset]);
    if (value) options.assets[asset] = value;
    options.telegramAssets[asset] = tomlString(
      config,
      `telegram.assets.${asset}_file`,
    ) ?? env(`JARVIS_DV_${asset.toUpperCase()}_FILE`) ?? ASSET_FILE[asset];
  }

  let commandLineCases = false;
  let commandLineTransports = false;
  let commandLineProviders = false;
  let commandLineFinanceCallers = false;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--") {
      continue;
    } else if (arg === "--config") {
      index += 1;
    } else if (arg === "--transport") {
      if (!commandLineTransports) {
        options.transports = [];
        commandLineTransports = true;
      }
      options.transports.push(transportValue(requiredValue(args, index, arg)));
      options.transports = uniqueValues(options.transports);
      index += 1;
    } else if (arg === "--provider") {
      if (!commandLineProviders) {
        options.expectedProviders = [];
        commandLineProviders = true;
      }
      options.expectedProviders.push(requiredValue(args, index, arg));
      options.expectedProviders = uniqueValues(options.expectedProviders);
      index += 1;
    } else if (arg === "--finance-caller-app-id") {
      if (!commandLineFinanceCallers) {
        options.financeCallerAppIds = [];
        commandLineFinanceCallers = true;
      }
      options.financeCallerAppIds.push(requiredValue(args, index, arg));
      options.financeCallerAppIds = uniqueValues(options.financeCallerAppIds);
      index += 1;
    } else if (arg === "--max-cost-usd") {
      options.financeBudgetUsd = Number(requiredValue(args, index, arg));
      index += 1;
    } else if (arg === "--estimated-cost-per-aicc-call-usd") {
      options.estimatedCostPerAiccCallUsd = Number(requiredValue(args, index, arg));
      index += 1;
    } else if (arg === "--max-aicc-calls-per-step") {
      options.maxAiccCallsPerStep = Number(requiredValue(args, index, arg));
      index += 1;
    } else if (arg === "--judge-model") {
      options.judgeModel = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--judge") {
      options.judgeEnabled = true;
    } else if (arg === "--no-judge") {
      options.judgeEnabled = false;
    } else if (arg === "--suite") {
      options.suite = suiteValue(requiredValue(args, index, arg));
      index += 1;
    } else if (arg === "--case") {
      if (!commandLineCases) {
        options.caseIds = [];
        commandLineCases = true;
      }
      options.caseIds.push(requiredValue(args, index, arg));
      index += 1;
    } else if (arg === "--gateway-url") {
      options.gatewayUrl = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--session-token") {
      options.sessionToken = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--username") {
      options.username = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--password") {
      options.password = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--user-id") {
      options.userId = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--user-did") {
      options.userDid = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--zone-did") {
      options.zoneDid = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--jarvis-did") {
      options.jarvisDid = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--group-did") {
      options.groupDid = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-api-id") {
      options.telegramApiId = parsePositiveInt(requiredValue(args, index, arg), 0, arg);
      index += 1;
    } else if (arg === "--telegram-api-hash") {
      options.telegramApiHash = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-phone") {
      options.telegramPhone = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-code") {
      options.telegramCode = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-password") {
      options.telegramPassword = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-bot") {
      options.telegramBotUsername = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-session") {
      options.telegramSession = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-session-file") {
      options.telegramSessionFile = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--telegram-connection-retries") {
      options.telegramConnectionRetries = parsePositiveInt(requiredValue(args, index, arg), 5, arg);
      index += 1;
    } else if (arg === "--report-dir") {
      options.reportDir = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--settle-ms") {
      options.settleMs = parsePositiveInt(requiredValue(args, index, arg), 10_000, arg);
      index += 1;
    } else if (arg === "--scenario-concurrency") {
      options.scenarioConcurrency = parsePositiveInt(requiredValue(args, index, arg), 2, arg);
      index += 1;
    } else if (arg === "--interactive-review") {
      options.interactiveReview = true;
    } else if (arg === "--no-interactive-review") {
      options.interactiveReview = false;
    } else if (arg === "--allow-review") {
      options.allowReview = true;
    } else if (arg === "--allow-credential-mutation") {
      options.allowCredentialMutationCli = true;
    } else if (arg === "--no-allow-review") {
      options.allowReview = false;
    } else if (arg === "--yes") {
      options.assumeYes = true;
    } else if (arg === "--no") {
      options.assumeYes = false;
    } else if (arg === "--dry-run") {
      options.dryRun = true;
    } else if (arg === "--list") {
      options.list = true;
    } else if (arg === "--help" || arg === "-h") {
      printUsage();
      Deno.exit(0);
    } else if (arg.startsWith("--")) {
      const key = arg.slice(2);
      const asset = FLAG_TO_ASSET[key];
      const telegramAsset = FLAG_TO_TELEGRAM_ASSET[key];
      if (asset) options.assets[asset] = requiredValue(args, index, arg);
      else if (telegramAsset) {
        options.telegramAssets[telegramAsset] = requiredValue(args, index, arg);
      } else throw new Error(`unknown option: ${arg}`);
      index += 1;
    } else {
      throw new Error(`unexpected argument: ${arg}`);
    }
  }

  options.gatewayUrl = options.gatewayUrl.replace(/\/+$/, "");
  if (!Number.isFinite(options.financeBudgetUsd) || options.financeBudgetUsd <= 0) {
    throw new Error("finance.max_cost_usd must be positive");
  }
  if (!Number.isFinite(options.estimatedCostPerAiccCallUsd) || options.estimatedCostPerAiccCallUsd < 0) {
    throw new Error("finance.estimated_cost_per_aicc_call_usd must be non-negative");
  }
  if (!Number.isInteger(options.maxAiccCallsPerStep) || options.maxAiccCallsPerStep < 1) {
    throw new Error("finance.max_aicc_calls_per_step must be a positive integer");
  }
  options.providerTokens = selectProviderTokens(options.providerTokens, options.expectedProviders);
  if (options.transports.length === 0) throw new Error("at least one transport is required");
  const source = (
    cliFlags: string[],
    configKey: string,
    envNames: string[] = [],
  ) => parameterSource(args, config, loaded.path, cliFlags, configKey, envNames);
  options.parameterSources = {
    transports: source(["--transport"], "common.transports", ["JARVIS_DV_TRANSPORTS"]),
    providers: source(["--provider"], "environment.providers", ["JARVIS_DV_PROVIDERS"]),
    financeCallerAppIds: source(
      ["--finance-caller-app-id"],
      "finance.caller_app_ids",
      ["JARVIS_DV_FINANCE_CALLER_APP_IDS"],
    ),
    financeBudgetUsd: source(["--max-cost-usd"], "finance.max_cost_usd", ["JARVIS_DV_MAX_COST_USD"]),
    estimatedCostPerAiccCallUsd: source(
      ["--estimated-cost-per-aicc-call-usd"],
      "finance.estimated_cost_per_aicc_call_usd",
      ["JARVIS_DV_ESTIMATED_COST_PER_AICC_CALL_USD"],
    ),
    maxAiccCallsPerStep: source(
      ["--max-aicc-calls-per-step"],
      "finance.max_aicc_calls_per_step",
      ["JARVIS_DV_MAX_AICC_CALLS_PER_STEP"],
    ),
    judgeEnabled: source(["--judge", "--no-judge"], "judge.enabled", ["JARVIS_DV_JUDGE_ENABLED"]),
    judgeModel: source(["--judge-model"], "judge.model", ["JARVIS_DV_JUDGE_MODEL"]),
    suite: source(["--suite"], "common.suite", ["JARVIS_DV_SUITE"]),
    cases: source(["--case"], "common.cases"),
    reportDir: source(["--report-dir"], "common.report_dir", ["JARVIS_DV_REPORT_DIR"]),
    settleMs: source(["--settle-ms"], "common.settle_ms", ["JARVIS_DV_SETTLE_MS"]),
    scenarioConcurrency: source(
      ["--scenario-concurrency"],
      "common.scenario_concurrency",
      ["JARVIS_DV_SCENARIO_CONCURRENCY"],
    ),
    interactiveReview: source(
      ["--interactive-review", "--no-interactive-review"],
      "common.interactive_review",
      ["JARVIS_DV_INTERACTIVE_REVIEW"],
    ),
    allowReview: source(
      ["--allow-review", "--no-allow-review"],
      "common.allow_review",
      ["JARVIS_DV_ALLOW_REVIEW"],
    ),
    assumeYes: source(["--yes", "--no"], "common.yes", ["JARVIS_DV_YES"]),
    gatewayUrl: source(["--gateway-url"], "msg_center.gateway_url", ["BUCKYOS_TEST_GATEWAY_URL"]),
    sessionToken: source(["--session-token"], "msg_center.session_token", ["BUCKYOS_APPCLIENT_SESSION_TOKEN"]),
    username: source(["--username"], "msg_center.username", ["BUCKYOS_TEST_USERNAME"]),
    password: source(["--password"], "msg_center.password", ["BUCKYOS_TEST_PASSWORD"]),
    userId: source(["--user-id"], "msg_center.user_id", ["BUCKYOS_TEST_USER_ID"]),
    userDid: source(["--user-did"], "msg_center.user_did", ["JARVIS_DV_USER_DID"]),
    zoneDid: source(["--zone-did"], "msg_center.zone_did", ["JARVIS_DV_ZONE_DID"]),
    jarvisDid: source(["--jarvis-did"], "msg_center.jarvis_did", ["JARVIS_DV_AGENT_DID"]),
    groupDid: source(["--group-did"], "msg_center.group_did", ["JARVIS_DV_GROUP_DID"]),
    telegramApiId: source(["--telegram-api-id"], "telegram.api_id", ["TELEGRAM_API_ID"]),
    telegramApiHash: source(["--telegram-api-hash"], "telegram.api_hash", ["TELEGRAM_API_HASH"]),
    telegramPhone: source(["--telegram-phone"], "telegram.phone", ["TELEGRAM_PHONE"]),
    telegramCode: source(["--telegram-code"], "telegram.code", ["TELEGRAM_CODE"]),
    telegramPassword: source(["--telegram-password"], "telegram.password", ["TELEGRAM_PASSWORD"]),
    telegramBotUsername: source(["--telegram-bot"], "telegram.bot_username", ["JARVIS_TELEGRAM_BOT_USERNAME"]),
    telegramSession: source(["--telegram-session"], "telegram.session", ["TELEGRAM_SESSION"]),
    telegramSessionFile: source(["--telegram-session-file"], "telegram.session_file", ["TELEGRAM_SESSION_FILE"]),
    telegramConnectionRetries: source(["--telegram-connection-retries"], "telegram.connection_retries", ["TELEGRAM_CONNECTION_RETRIES"]),
  };
  for (const asset of Object.keys(ASSET_ENV) as AssetKey[]) {
    options.parameterSources[`msg:${asset}`] = source(
      [`--${Object.entries(FLAG_TO_ASSET).find(([, value]) => value === asset)?.[0]}`],
      `msg_center.assets.${asset}_id`,
      [ASSET_ENV[asset]],
    );
    options.parameterSources[`telegram:${asset}`] = source(
      [`--${Object.entries(FLAG_TO_TELEGRAM_ASSET).find(([, value]) => value === asset)?.[0]}`],
      `telegram.assets.${asset}_file`,
      [`JARVIS_DV_${asset.toUpperCase()}_FILE`],
    );
  }
  return options;
}

function printUsage(): void {
  console.log(`Usage:
  deno task test -- [options]

Options:
  --config <path>                       Default: jarvis_media_dv.local.toml
  --transport <msg-center|telegram>     Repeat to enable transports; default: msg-center
  --provider <name>                     Repeat to declare expected providers
  --finance-caller-app-id <id>          Repeat to scope AICC usage accounting
  --max-cost-usd <usd>                  Abort when planned maximum exceeds budget
  --estimated-cost-per-aicc-call-usd <usd>
  --max-aicc-calls-per-step <count>
  --suite <smoke|linked|matrix|all>     Default: all
  --case <scenario_id>                  Repeat to select specific scenarios
  --gateway-url <url>                   Zone Gateway base URL (required)
  --username <name>                     Prefer BUCKYOS_TEST_USERNAME
  --password <password>                 Prefer BUCKYOS_TEST_PASSWORD
  --session-token <token>               Optional login override for debugging
  --user-id <id>                        Normally read from login response
  --user-did <did>
  --zone-did <did>
  --jarvis-did <did>
  --group-did <did>                    Optional msg-center group conversation target
  --judge-model <alias>                Default: llm.plan.default
  --judge / --no-judge                 Enable/disable automatic semantic Judge
  --telegram-api-id <id>
  --telegram-api-hash <hash>
  --telegram-phone <phone>
  --telegram-code <login-code>
  --telegram-password <2fa-password>
  --telegram-bot <@username>
  --telegram-session <string-session>
  --telegram-session-file <path>
  --telegram-connection-retries <count>
  --image-primary-id <obj_id>
  --image-secondary-id <obj_id>
  --image-ocr-id <obj_id>
  --audio-sfx-id <obj_id>
  --audio-speech-id <obj_id>
  --video-fresh-id <obj_id>
  --document-facts-id <obj_id>
  --archive-mixed-id <obj_id>
  --image-primary-file <path>
  --image-secondary-file <path>
  --image-ocr-file <path>
  --audio-sfx-file <path>
  --audio-speech-file <path>
  --video-fresh-file <path>
  --document-facts-file <path>
  --archive-mixed-file <path>
  --report-dir <path>
  --settle-ms <milliseconds>
  --scenario-concurrency <count>        Parallel isolated msg-center conversations; default: 2
  --interactive-review                  Ask the operator to judge semantics
  --no-interactive-review
  --allow-review                        Exit 0 when only manual review remains
  --no-allow-review
  --allow-credential-mutation           Allow temporary AICC credential override
  --yes                                 Start after preflight without confirmation
  --no
  --dry-run
  --list
`);
}

function selectScenarios(options: CliOptions): Scenario[] {
  let selected = SCENARIOS.filter((scenario) =>
    options.suite === "all" || scenario.suite === options.suite
  );
  if (options.caseIds.length > 0) {
    const requested = new Set(options.caseIds);
    const known = new Set(SCENARIOS.map((scenario) => scenario.id));
    for (const id of requested) {
      if (!known.has(id)) throw new Error(`unknown scenario: ${id}`);
    }
    selected = selected.filter((scenario) => requested.has(scenario.id));
  }
  if (selected.length === 0) throw new Error("no scenarios selected");
  return selected;
}

function printScenarioList(): void {
  for (const scenario of SCENARIOS) {
    console.log(`${scenario.id}\t${scenario.suite}\t${scenario.title}`);
    console.log(`  ${scenario.purpose}`);
    console.log(`  assets: ${scenario.requiredAssets.join(", ") || "none"}`);
  }
}

function isObject(value: unknown): value is JsonObject {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

async function resolveZoneDid(systemConfig: RpcClient): Promise<string> {
  const raw = await systemConfig.call("sys_config_get", { key: "boot/config" });
  if (!isObject(raw) || typeof raw.value !== "string") {
    throw new Error(`boot/config response is invalid: ${JSON.stringify(raw)}`);
  }
  const boot = JSON.parse(raw.value) as JsonObject;
  if (typeof boot.zone_document !== "string") {
    throw new Error("boot/config does not contain a zone_document string");
  }
  const zoneDocument = JSON.parse(boot.zone_document) as JsonObject;
  const zoneDid = zoneDocument.id;
  if (typeof zoneDid !== "string" || !zoneDid.startsWith("did:")) {
    throw new Error("boot/config zone_document does not contain a valid DID id");
  }
  return zoneDid;
}

function deriveJarvisDid(zoneDid: string): string {
  const match = /^did:([^:]+):(.+)$/.exec(zoneDid);
  if (!match) throw new Error(`invalid zone DID: ${zoneDid}`);
  return `did:${match[1]}:jarvis.${match[2]}`;
}

function assetRef(asset: AssetKey, objectId: string): RefItem {
  return {
    role: "input",
    target: { type: "data_obj", obj_id: objectId },
    label: ASSET_LABEL[asset],
  };
}

async function repositoryCommit(): Promise<string> {
  try {
    const gitDir = new URL("../../.git/", import.meta.url);
    const head = (await Deno.readTextFile(new URL("HEAD", gitDir))).trim();
    if (!head.startsWith("ref: ")) return head;
    return (await Deno.readTextFile(new URL(head.slice(5), gitDir))).trim();
  } catch {
    return "unknown";
  }
}

async function providerBaselineRevision(): Promise<string> {
  try {
    const raw = JSON.parse(await Deno.readTextFile(
      new URL("../aicc_test/acceptance/provider_capability_baseline.json", import.meta.url),
    )) as { baseline_revision?: unknown };
    return typeof raw.baseline_revision === "string" ? raw.baseline_revision : "unknown";
  } catch {
    return "unknown";
  }
}

function stepAssetKeys(step: ScenarioStep): AssetKey[] {
  return step.attachments ?? (step.attachment ? [step.attachment] : []);
}

function makeMessage(input: {
  userDid: string;
  targetDid: string;
  topic: string;
  prompt: string;
  traceId: string;
  kind?: "chat" | "group_msg";
  sourceDid?: string;
  replyTo?: string;
  attachments?: { asset: AssetKey; objectId: string }[];
}): MsgObject {
  const refs = input.attachments?.length
    ? input.attachments.map((item) => assetRef(item.asset, item.objectId))
    : undefined;
  return {
    from: input.userDid,
    ...(input.sourceDid ? { source: input.sourceDid } : {}),
    to: [input.targetDid],
    kind: input.kind ?? "chat",
    thread: {
      topic: input.topic,
      correlation_id: input.traceId,
      ...(input.replyTo ? { reply_to: input.replyTo } : {}),
    },
    created_at_ms: Date.now(),
    nonce: crypto.getRandomValues(new Uint32Array(1))[0],
    content: {
      format: "text/plain",
      content: input.prompt,
      ...(refs ? { refs } : {}),
    },
    dv_trace_id: input.traceId,
  };
}

async function listSession(
  msgCenter: RpcClient,
  owner: string,
  topic: string,
): Promise<SessionMessageItem[]> {
  const raw = await msgCenter.call("msg.list_session", {
    owner,
    session_id: topic,
    limit: 500,
    descending: false,
    with_object: true,
  });
  if (!isObject(raw)) return [];
  return Array.isArray(raw.items) ? raw.items as SessionMessageItem[] : [];
}

function replyTexts(items: SessionMessageItem[], jarvisDid: string): string[] {
  return items
    .filter((item) => item.direction === "in" && item.from === jarvisDid)
    .map((item) => item.msg?.content?.content?.trim() ?? "")
    .filter(Boolean);
}

function replyRefs(items: SessionMessageItem[], jarvisDid: string): RefItem[] {
  return items
    .filter((item) => item.direction === "in" && item.from === jarvisDid)
    .flatMap((item) => item.msg?.content?.refs ?? []);
}

function explicitFailureReply(
  items: SessionMessageItem[],
  jarvisDid: string,
): string | undefined {
  const prefixes = [
    "任务失败：",
    "任務失敗：",
    "Task failed:",
    "La tarea falló:",
    "La tâche a échoué",
    "Aufgabe fehlgeschlagen:",
    "작업 실패:",
    "タスクに失敗しました:",
    "Ошибка задачи:",
  ];
  for (const item of items) {
    if (item.direction !== "in" || item.from !== jarvisDid) continue;
    const meta = item.msg?.meta;
    if (isObject(meta) && meta.delivery_failure_fallback === true) {
      return item.msg?.content?.content?.trim() || "Jarvis reply delivery failed";
    }
    const text = item.msg?.content?.content?.trim() ?? "";
    if (prefixes.some((prefix) => text.startsWith(prefix))) return text;
  }
  return undefined;
}

function artifactMatches(label: string | undefined, prefix: string): boolean {
  const value = (label ?? "").toLowerCase();
  if (value.startsWith(prefix)) return true;
  if (prefix === "image/") return /\.(png|jpe?g|webp|gif)$/.test(value);
  if (prefix === "audio/") return /\.(wav|mp3|ogg|aac|flac)$/.test(value);
  if (prefix === "video/") return /\.(mp4|webm|mov|mkv)$/.test(value);
  return false;
}

function evaluateAutomatic(
  step: ScenarioStep,
  texts: string[],
  refs: RefItem[],
): { ready: boolean; failures: string[]; checks: string[] } {
  const combined = texts.join("\n");
  const failures: string[] = [];
  const checks: string[] = [];

  if (step.expect.textRequired) {
    if (combined.trim()) checks.push("received non-empty Jarvis text");
    else failures.push("missing Jarvis text reply");
  }
  if (step.expect.textAny?.length) {
    const matched = step.expect.textAny.some((value) => combined.includes(value));
    if (matched) checks.push(`text matched one of: ${step.expect.textAny.join(", ")}`);
    else failures.push(`text did not match any of: ${step.expect.textAny.join(", ")}`);
  }
  for (const forbidden of step.expect.textNone ?? []) {
    if (combined.toLowerCase().includes(forbidden.toLowerCase())) {
      failures.push(`reply contained forbidden hallucinated text: ${forbidden}`);
    } else if (combined) {
      checks.push(`reply excluded: ${forbidden}`);
    }
  }
  const expectedArtifacts = step.expect.artifacts ??
    (step.expect.artifact ? [step.expect.artifact] : []);
  for (const expected of expectedArtifacts) {
    const matched = refs.some((ref) =>
      Boolean(ref.target.obj_id) && artifactMatches(ref.label, expected)
    );
    if (matched) checks.push(`received ${expected} artifact`);
    else failures.push(`missing ${expected} artifact`);
  }
  if (step.expect.textAll?.length) {
    const missing = step.expect.textAll.filter((value) => !combined.includes(value));
    if (missing.length === 0) checks.push(`text contained all: ${step.expect.textAll.join(", ")}`);
    else failures.push(`text omitted required values: ${missing.join(", ")}`);
  }
  const countRange = step.expect.attachmentCount ??
    (expectedArtifacts.length > 0
      ? { min: expectedArtifacts.length, max: expectedArtifacts.length }
      : { min: 0, max: 0 });
  if (refs.length < countRange.min || refs.length > countRange.max) {
    failures.push(
      `outbound attachment count ${refs.length} outside ${countRange.min}..${countRange.max}`,
    );
  } else {
    checks.push(`outbound attachment count ${refs.length} matched ${countRange.min}..${countRange.max}`);
  }

  const hasReply = texts.length > 0 || refs.length > 0;
  const ready = hasReply && failures.length === 0;
  return { ready, failures, checks };
}

async function waitForReply(input: {
  msgCenter: RpcClient;
  userDid: string;
  jarvisDid: string;
  topic: string;
  afterSortKey: number;
  step: ScenarioStep;
  settleMs: number;
}): Promise<{ items: SessionMessageItem[]; checks: string[] }> {
  const deadline = Date.now() + (input.step.maxWaitMs ?? 180_000);
  let lastSignature = "";
  let lastChangeAt = Date.now();
  let lastFailures: string[] = [];
  let lastItems: SessionMessageItem[] = [];
  let lastChecks: string[] = [];

  while (Date.now() < deadline) {
    const all = await listSession(input.msgCenter, input.userDid, input.topic);
    const items = all.filter((item) => item.sort_key > input.afterSortKey);
    lastItems = items;
    const texts = replyTexts(items, input.jarvisDid);
    const refs = replyRefs(items, input.jarvisDid);
    const signature = JSON.stringify({
      records: items.map((item) => item.record_id),
      texts,
      refs,
    });
    if (signature !== lastSignature) {
      lastSignature = signature;
      lastChangeAt = Date.now();
    }

    const evaluation = evaluateAutomatic(input.step, texts, refs);
    lastFailures = evaluation.failures;
    lastChecks = evaluation.checks;
    const explicitFailure = explicitFailureReply(items, input.jarvisDid);
    if (explicitFailure && Date.now() - lastChangeAt >= input.settleMs) {
      throw new ReplyWaitError(
        `Jarvis reported failure: ${explicitFailure}`,
        items,
        evaluation.checks,
      );
    }
    if (evaluation.ready) {
      if (input.step.expect.artifact || input.step.expect.artifacts?.length ||
        Date.now() - lastChangeAt >= input.settleMs) {
        return { items, checks: evaluation.checks };
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }

  throw new ReplyWaitError(
    `timed out after ${input.step.maxWaitMs ?? 180_000} ms: ${lastFailures.join("; ") || "no final reply"}`,
    lastItems,
    lastChecks,
  );
}

function maxSortKey(items: SessionMessageItem[]): number {
  return items.reduce((max, item) => Math.max(max, item.sort_key ?? 0), 0);
}

function cleanSessionId(items: SessionMessageItem[], jarvisDid: string): string {
  const texts = replyTexts(items, jarvisDid);
  for (const text of texts.toReversed()) {
    const match = /new session `([^`]+)` created/.exec(text);
    if (match?.[1]) return match[1];
  }
  throw new Error("/clean reply did not contain the new Jarvis session id");
}

async function postMessage(
  msgCenter: RpcClient,
  msg: MsgObject,
  idempotencyKey: string,
): Promise<string | undefined> {
  const result = await msgCenter.call("msg.post_send", {
    msg,
    idempotency_key: idempotencyKey,
  });
  if (isObject(result) && result.ok === false) {
    throw new Error(`msg.post_send failed: ${JSON.stringify(result)}`);
  }
  return isObject(result) && typeof result.msg_id === "string"
    ? result.msg_id
    : undefined;
}

function reviewStep(step: ScenarioStep): { status: "passed" | "failed" | "review"; notes?: string } {
  if (step.review.length === 0) return { status: "passed" };
  const answer = globalThis.prompt(
    `人工检查：\n- ${step.review.join("\n- ")}\n判定 [p=pass, f=fail, Enter=review]:`,
  )?.trim().toLowerCase();
  if (answer === "p" || answer === "pass") return { status: "passed" };
  if (answer === "f" || answer === "fail") {
    const notes = globalThis.prompt("失败说明：")?.trim();
    return { status: "failed", notes };
  }
  return { status: "review" };
}

async function promptValue(label: string, secret = false): Promise<string> {
  const suffix = secret ? "（输入会显示在终端，请确认周围环境安全）" : "";
  const value = globalThis.prompt(`${label}${suffix}:`)?.trim();
  if (!value) throw new Error(`${label} is required`);
  return value;
}

function responseText(value: unknown, depth = 0): string[] {
  if (depth > 8 || value === null || value === undefined) return [];
  if (Array.isArray(value)) return value.flatMap((item) => responseText(item, depth + 1));
  if (typeof value !== "object") return [];
  const object = value as Record<string, unknown>;
  const values: string[] = [];
  for (const key of ["text", "output_text"] as const) {
    if (typeof object[key] === "string") values.push(object[key] as string);
  }
  if (typeof object.content === "string") values.push(object.content);
  for (const [key, nested] of Object.entries(object)) {
    if (["text", "output_text", "content"].includes(key)) continue;
    values.push(...responseText(nested, depth + 1));
  }
  return [...new Set(values.filter(Boolean))];
}

function bytesBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 32_768) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 32_768));
  }
  return btoa(binary);
}

async function waitForAiccResult(
  taskManager: RpcClient,
  initial: AiMethodResponse,
  timeoutMs: number,
): Promise<unknown> {
  if (initial.status === "failed") throw new JudgeError("LLM Judge request failed immediately");
  if (initial.status === "succeeded") return initial.result;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const raw = await taskManager.call("get_task", { task_id: initial.task_id }) as Record<string, unknown>;
    const task = (isObject(raw.task) ? raw.task : raw) as {
      phase?: string;
      outcome?: string;
      result?: { result?: { output?: unknown } };
      error?: unknown;
    };
    if (task.phase === "Terminal") {
      if (task.outcome !== "Succeeded") {
        throw new JudgeError(`LLM Judge task ended ${String(task.outcome)}: ${JSON.stringify(task.error ?? {})}`);
      }
      return task.result?.result?.output;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new JudgeError(`LLM Judge task timed out after ${timeoutMs} ms`);
}

async function judgeStep(input: {
  aicc: RpcClient;
  taskManager: RpcClient;
  model: string;
  traceId: string;
  step: ScenarioStep;
  texts: string[];
  refs: RefItem[];
  resourceBlocks?: Array<Record<string, unknown>>;
}): Promise<{ taskId: string; passed: boolean; score: number; reason: string }> {
  const rubric = input.step.review.join("\n- ");
  const observed = JSON.stringify({
    reply_texts: input.texts,
    reply_attachments: input.refs.map((ref) => ({ label: ref.label, type: ref.target.type, has_obj_id: Boolean(ref.target.obj_id) })),
  });
  const judgeContent: Array<Record<string, unknown>> = [{
    type: "text",
    text: `You are a strict test judge using rubric version ${T3_JUDGE_RUBRIC_VERSION}. Evaluate only the supplied observations and attached outputs against every rubric item. Return JSON only: {"pass":boolean,"score":number,"reason":string}. score must be between 0 and 1, and pass must be false below 0.7.\nRubric:\n- ${rubric}\nObservations:\n${observed}`,
  }];
  for (const ref of input.refs) {
    if (!ref.target.obj_id || ref.target.obj_id.startsWith("telegram:")) continue;
    const label = ref.label?.toLowerCase() ?? "";
    judgeContent.push({
      type: label.includes("image/") ? "image" : "document",
      source: { kind: "named_object", obj_id: ref.target.obj_id },
    });
  }
  judgeContent.push(...(input.resourceBlocks ?? []));
  const request = {
    capability: "llm",
    model: { alias: input.model },
    requirements: { must_features: ["json_output"], resp_format: "json" },
    payload: {
      input_json: {
        messages: [{
          role: "user",
          content: judgeContent,
        }],
        max_output_tokens: 256,
        response_format: {
          type: "json_schema",
          name: "aicc_t3_judge_verdict",
          strict: true,
          schema: {
            type: "object",
            properties: {
              pass: { type: "boolean" },
              score: { type: "number", minimum: 0, maximum: 1 },
              reason: { type: "string" },
            },
            required: ["pass", "score", "reason"],
            additionalProperties: false,
          },
        },
      },
      resources: [],
      tool_specs: [],
      options: { session_id: `${input.traceId}:judge`, rootid: input.traceId },
    },
    idempotency_key: `${input.traceId}:judge`,
  };
  const initial = await input.aicc.call("llm.chat", request) as AiMethodResponse;
  if (!initial.task_id) throw new JudgeError("LLM Judge response omitted task_id");
  const result = await waitForAiccResult(input.taskManager, initial, 180_000);
  const text = responseText(result).join("\n");
  const match = /\{[\s\S]*\}/.exec(text);
  if (!match) throw new JudgeError(`LLM Judge returned no JSON object: ${text.slice(0, 500)}`);
  let verdict: unknown;
  try {
    verdict = JSON.parse(match[0]);
  } catch (error) {
    throw new JudgeError(`LLM Judge returned invalid JSON: ${String(error)}`);
  }
  if (!isObject(verdict) || typeof verdict.pass !== "boolean" || typeof verdict.score !== "number" ||
    !Number.isFinite(verdict.score) || verdict.score < 0 || verdict.score > 1 ||
    typeof verdict.reason !== "string" || (verdict.score < 0.7 && verdict.pass)) {
    throw new JudgeError(`LLM Judge verdict has invalid schema: ${JSON.stringify(verdict)}`);
  }
  return { taskId: initial.task_id, passed: verdict.pass, score: verdict.score, reason: verdict.reason };
}

async function gatewaySession(
  options: CliOptions,
  forcePasswordLogin = false,
): Promise<{ token: string; userId: string }> {
  if (!forcePasswordLogin && options.sessionToken) {
    return { token: options.sessionToken, userId: options.userId };
  }
  if (!options.username || !options.password) {
    throw new Error("Gateway username/password or session token is required");
  }
  const { buckyos } = await import("buckyos");
  const nonce = Date.now();
  const loginRpc = new buckyos.kRPCClient(
    `${options.gatewayUrl}/kapi/control-panel`,
    null,
    nonce,
  ) as RpcClient;
  const login = await loginRpc.call("auth.login", {
    username: options.username,
    password: buckyos.hashPassword(options.username, options.password, nonce),
    appid: "control-panel",
    target: { kind: "system", service_id: "control-panel" },
    login_nonce: nonce,
  }) as PasswordLoginResponse;
  const token = typeof login.session_token === "string" ? login.session_token.trim() : "";
  const userId = typeof login.user_info?.user_id === "string"
    ? login.user_info.user_id.trim()
    : "";
  if (!token) throw new Error("auth.login succeeded without a session_token");
  options.sessionToken = token;
  console.log(`[probe] password login succeeded for ${userId || options.username}`);
  return { token, userId };
}

async function auditRunFinance(options: CliOptions, report: RunReport): Promise<void> {
  const plannedSteps = SCENARIOS.filter((scenario) => report.selected_scenarios.includes(scenario.id))
    .reduce((sum, scenario) => sum + scenario.steps.length, 0) * report.transports.length;
  const plannedMaxCalls = plannedSteps * options.maxAiccCallsPerStep;
  const plannedMaxCostUsd = plannedMaxCalls * options.estimatedCostPerAiccCallUsd;
  const base: Omit<NonNullable<RunReport["finance"]>, "status" | "error"> = {
    attribution: "caller_app_and_time_window_plus_judge_task_ids" as const,
    attribution_limitation: "Jarvis workflow events are isolated by caller_app_id and run time window; Judge events are isolated by their returned task IDs. Per-step workflow attribution additionally requires AICC route traces carrying the DV trace ID.",
    caller_app_ids: options.financeCallerAppIds,
    budget_usd: options.financeBudgetUsd,
    planned_max_calls: plannedMaxCalls,
    planned_max_cost_usd: plannedMaxCostUsd,
    observed_provider_instances: [],
    observed_provider_drivers: [],
    missing_expected_providers: options.expectedProviders,
    event_count: 0,
    workflow_event_count: 0,
    judge_event_count: 0,
    input_tokens: 0,
    output_tokens: 0,
    total_tokens: 0,
    request_units: 0,
    actual_cost_usd: 0,
    raw_cost_usd: 0,
    credit_applied_usd: 0,
    unknown_cost_events: 0,
    estimated_unknown_cost_usd: 0,
    total_exposure_usd: 0,
    remaining_budget_usd: options.financeBudgetUsd,
    budget_exceeded: false,
    step_correlation: {
      status: "failed",
      expected_step_ids: [],
      correlated_step_ids: [],
      uncorrelated_step_ids: [],
      trace_count: 0,
      defect: "correlation audit did not run",
    },
    events: [],
  };
  if (options.financeCallerAppIds.length === 0) {
    report.finance = {
      ...base,
      status: "failed",
      error: "finance.caller_app_ids is required for isolated T3 accounting",
    };
    return;
  }
  try {
    const session = await gatewaySession(options);
    const { buckyos } = await import("buckyos");
    const aicc = new buckyos.kRPCClient(
      `${options.gatewayUrl}/kapi/aicc`,
      session.token,
    ) as RpcClient;
    const [workflowEvents, judgeEvents, inventoryResponse] = await Promise.all([
      queryUsageEvents({
        aicc,
        startTimeMs: new Date(report.started_at).getTime() - 1_000,
        endTimeMs: Date.now() + 1_000,
        callerAppIds: options.financeCallerAppIds,
      }),
      report.judge.task_ids.length > 0
        ? queryUsageEvents({
          aicc,
          startTimeMs: new Date(report.started_at).getTime() - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds: report.judge.task_ids,
        })
        : Promise.resolve([]),
      aicc.call("models.list", {}),
    ]);
    const eventById = new Map([...workflowEvents, ...judgeEvents].map((event) => [event.event_id, event]));
    const events = [...eventById.values()];
    const workflowTaskIds = uniqueValues(workflowEvents.map((event) => event.task_id));
    let routeTraces: Awaited<ReturnType<typeof queryRouteTraces>> = [];
    let traceQueryError: string | undefined;
    if (workflowTaskIds.length > 0) {
      try {
        routeTraces = await queryRouteTraces({
          aicc,
          startTimeMs: new Date(report.started_at).getTime() - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds: workflowTaskIds,
        });
      } catch (error) {
        traceQueryError = String(error);
      }
    }
    const providers = inventoryResponse && typeof inventoryResponse === "object" &&
        Array.isArray((inventoryResponse as { providers?: unknown }).providers)
      ? (inventoryResponse as { providers: unknown[] }).providers
      : [];
    const inventoryMappings: Array<{ provider_instance_name: string; provider_driver: string }> = [];
    for (const provider of providers) {
      if (!provider || typeof provider !== "object") continue;
      const value = provider as Record<string, unknown>;
      if (typeof value.provider_instance_name === "string" &&
        typeof value.provider_driver === "string") {
        inventoryMappings.push({
          provider_instance_name: value.provider_instance_name,
          provider_driver: value.provider_driver,
        });
      }
    }
    const entries = events.map((event) => {
      const finance = usageEventFinance(event);
      const providerInstance = providerInstanceFromExactModel(event.provider_model);
      return {
        event_id: event.event_id,
        task_id: event.task_id,
        caller_app_id: event.caller_app_id,
        api_type: event.capability,
        request_model: event.request_model,
        exact_model: event.provider_model,
        provider_instance: providerInstance,
        provider_driver: providerInstance
          ? inventoryMappings.find((inventory) =>
            inventory.provider_instance_name === providerInstance
          )?.provider_driver
          : undefined,
        actual_cost_usd: finance.actualCostUsd,
        ...finance.usage,
      };
    });
    const entryByTask = new Map(entries.map((entry) => [entry.task_id, entry]));
    for (const result of report.results) {
      if (!result.judge) continue;
      const entry = entryByTask.get(result.judge.task_id);
      if (!entry) continue;
      result.judge.exact_model = entry.exact_model;
      result.judge.provider_instance = entry.provider_instance;
      result.judge.provider_driver = entry.provider_driver;
      const stepTraceId = `${report.run_id}:${result.transport ?? "unknown"}:${result.scenario_id}:${result.step_id}`;
      const workflowTrace = routeTraces.find((trace) => trace.trace_id === stepTraceId);
      const workflowEntry = workflowTrace ? entryByTask.get(workflowTrace.task_id) : undefined;
      if (workflowEntry) {
        result.judge.distinct_provider_or_family = entry.provider_instance !== workflowEntry.provider_instance ||
          modelFamily(entry.exact_model) !== modelFamily(workflowEntry.exact_model);
      }
    }
    const coverage = providerCoverage({
      exactModels: workflowEvents.map((event) => event.provider_model),
      inventories: inventoryMappings,
      expectedDrivers: options.expectedProviders,
    });
    const actualCostUsd = events.reduce((sum, event) => sum + (usageEventFinance(event).actualCostUsd ?? 0), 0);
    const unknownCostEvents = events.filter((event) => usageEventFinance(event).actualCostUsd === undefined).length;
    const estimatedUnknownCostUsd = unknownCostEvents * options.estimatedCostPerAiccCallUsd;
    const totalExposureUsd = actualCostUsd + estimatedUnknownCostUsd;
    const budgetExceeded = totalExposureUsd > options.financeBudgetUsd;
    const expectedStepIds = report.results
      .filter((result) => ["passed", "review", "dispatched"].includes(result.status))
      .map((result) => `${report.run_id}:${result.transport ?? "unknown"}:${result.scenario_id}:${result.step_id}`);
    const traceIds = new Set(routeTraces.map((trace) => trace.trace_id));
    const correlatedStepIds = expectedStepIds.filter((stepId) => traceIds.has(stepId));
    const uncorrelatedStepIds = expectedStepIds.filter((stepId) => !traceIds.has(stepId));
    const correlationPassed = expectedStepIds.length > 0 && uncorrelatedStepIds.length === 0;
    const correlationDefect = correlationPassed
      ? undefined
      : traceQueryError
      ? `AICC trace.query failed: ${traceQueryError}`
      : `AICC/Jarvis did not expose DV trace IDs for ${uncorrelatedStepIds.length}/${expectedStepIds.length} executed steps`;
    report.finance = {
      ...base,
      status: coverage.missingExpectedDrivers.length === 0 && !budgetExceeded && correlationPassed
        ? "passed"
        : "failed",
      observed_provider_instances: coverage.observedInstances,
      observed_provider_drivers: coverage.observedDrivers,
      missing_expected_providers: coverage.missingExpectedDrivers,
      event_count: entries.length,
      workflow_event_count: workflowEvents.length,
      judge_event_count: judgeEvents.length,
      input_tokens: entries.reduce((sum, item) => sum + (item.input_tokens ?? 0), 0),
      output_tokens: entries.reduce((sum, item) => sum + (item.output_tokens ?? 0), 0),
      total_tokens: entries.reduce((sum, item) => sum + (item.total_tokens ?? 0), 0),
      request_units: entries.reduce((sum, item) => sum + (item.request_units ?? 0), 0),
      actual_cost_usd: actualCostUsd,
      raw_cost_usd: events.reduce((sum, event) => sum + (usageEventFinance(event).rawCostUsd ?? 0), 0),
      credit_applied_usd: events.reduce((sum, event) => sum + (usageEventFinance(event).creditAppliedUsd ?? 0), 0),
      unknown_cost_events: unknownCostEvents,
      estimated_unknown_cost_usd: estimatedUnknownCostUsd,
      total_exposure_usd: totalExposureUsd,
      remaining_budget_usd: Math.max(0, options.financeBudgetUsd - totalExposureUsd),
      budget_exceeded: budgetExceeded,
      step_correlation: {
        status: correlationPassed ? "passed" : "failed",
        expected_step_ids: expectedStepIds,
        correlated_step_ids: correlatedStepIds,
        uncorrelated_step_ids: uncorrelatedStepIds,
        trace_count: routeTraces.length,
        ...(correlationDefect ? { defect: correlationDefect } : {}),
      },
      events: entries,
      ...(coverage.missingExpectedDrivers.length > 0 || budgetExceeded || !correlationPassed
        ? { error: [
          coverage.missingExpectedDrivers.length > 0
            ? `expected providers not observed in AICC usage: ${coverage.missingExpectedDrivers.join(", ")}`
            : "",
          budgetExceeded ? `financial exposure ${totalExposureUsd} exceeds budget ${options.financeBudgetUsd}` : "",
          correlationDefect ?? "",
        ].filter(Boolean).join("; ") }
        : {}),
    };
  } catch (error) {
    report.finance = { ...base, status: "failed", error: String(error) };
  }
}

async function runMsgCenter(
  options: CliOptions,
  scenarios: Scenario[],
  report: RunReport,
): Promise<void> {
  if (!options.sessionToken && (!options.username || !options.password)) {
    options.username ||= await promptValue("BuckyOS username");
    options.password ||= await promptValue("BuckyOS password", true);
  }
  const { buckyos, ndm_proxy, ndn } = await import("buckyos");
  const gatewayAuth = await gatewaySession(options);
  const sessionToken = gatewayAuth.token;
  const loginUserId = gatewayAuth.userId;
  const msgCenter = new buckyos.kRPCClient(
    `${options.gatewayUrl}/kapi/msg-center`,
    sessionToken,
  ) as RpcClient;
  const aicc = new buckyos.kRPCClient(
    `${options.gatewayUrl}/kapi/aicc`,
    sessionToken,
  ) as RpcClient;
  const taskManager = new buckyos.kRPCClient(
    `${options.gatewayUrl}/kapi/task-manager`,
    sessionToken,
  ) as RpcClient;
  const systemConfig = new buckyos.kRPCClient(
    `${options.gatewayUrl}/kapi/system_config`,
    sessionToken,
  ) as RpcClient;
  report.gateway_url = options.gatewayUrl;
  const zoneDid = options.zoneDid ?? await resolveZoneDid(systemConfig);
  const userId = options.userId || loginUserId || options.username;
  if (!userId) throw new Error("cannot resolve logged-in user id");
  const userDid = options.userDid ?? (userId.startsWith("did:") ? userId : `did:bns:${userId}`);
  const jarvisDid = options.jarvisDid ?? deriveJarvisDid(zoneDid);
  report.user_did = userDid;
  report.jarvis_did = jarvisDid;

  console.log(`[probe] gateway=${options.gatewayUrl}`);
  console.log(`[probe] user=${userDid} jarvis=${jarvisDid}`);
  await msgCenter.call("msg.list_sessions", {
    owner: userDid,
    limit: 1,
    with_object: false,
  });
  console.log("[probe] authenticated msg-center path is ready");
  const ndmProxy = ndm_proxy.createNdmProxyClient({
    endpoint: options.gatewayUrl,
    sessionToken,
    fetcher: (request: RequestInfo | URL, init?: RequestInit) => {
      const target = typeof request === "string"
        ? request.replaceAll("%3A", ":").replaceAll("%3a", ":")
        : request instanceof URL
        ? new URL(request.toString().replaceAll("%3A", ":").replaceAll("%3a", ":"))
        : request;
      return fetch(target, init);
    },
  }) as NdmProxyClient;
  const uploadedFixtureIds: string[] = [];
  const generatedArtifactIds = new Set<string>();
  const createdSessionIds = new Set<string>();
  const defaultAssets = selectedAssets(scenarios).filter((asset) => !options.assets[asset]);
  if (defaultAssets.length > 0) {
    for (const asset of defaultAssets) {
      const path = ASSET_FILE[asset];
      if (!await pathExists(path)) {
        console.warn(`[asset] project asset is missing: ${path}`);
        continue;
      }
      try {
        const bytes = await Deno.readFile(path);
        const objId = ndn.ChunkId.fromMix256Result(
          bytes.byteLength,
          ndn.sha256Bytes(bytes),
        ).toString();
        await ndmProxy.putChunk(objId, bytes);
        uploadedFixtureIds.push(objId);
        options.assets[asset] = objId;
        options.parameterSources[`msg:${asset}`] = `project asset:${path}`;
        console.log(`[asset] uploaded ${asset}: ${path} -> ${objId}`);
      } catch (error) {
        console.error(`[asset] failed to upload ${asset}: ${path}: ${String(error)}`);
      }
    }
  }

  let sessionCreationTail = Promise.resolve();
  const createScenarioSession = async (scenario: Scenario): Promise<string> => {
    const previous = sessionCreationTail;
    let release!: () => void;
    sessionCreationTail = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      const cleanTopic = `jarvis-dv:${report.run_id}:${scenario.id}:clean`;
      const commandReplyTopic = `dm:${jarvisDid}`;
      const cleanTrace = `${report.run_id}:msg-center:${scenario.id}:clean`;
      const beforeClean = await listSession(msgCenter, userDid, commandReplyTopic);
      await postMessage(
        msgCenter,
        makeMessage({ userDid, targetDid: jarvisDid, topic: cleanTopic, prompt: "/clean", traceId: cleanTrace }),
        cleanTrace,
      );
      const cleanReply = await waitForReply({
        msgCenter,
        userDid,
        jarvisDid,
        topic: commandReplyTopic,
        afterSortKey: maxSortKey(beforeClean),
        step: {
          id: "clean",
          prompt: "/clean",
          expect: { textRequired: true },
          maxWaitMs: 60_000,
          review: [],
        },
        settleMs: 1_500,
      });
      const sessionId = cleanSessionId(cleanReply.items, jarvisDid);
      createdSessionIds.add(cleanTopic);
      createdSessionIds.add(commandReplyTopic);
      createdSessionIds.add(sessionId);
      return sessionId;
    } finally {
      release();
    }
  };

  const runScenario = async (scenario: Scenario): Promise<void> => {
    if (scenario.coverage) {
      for (const step of scenario.steps) {
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: scenario.coverage.status,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          prompt: step.prompt,
          attachment: step.attachment,
          attachments: stepAssetKeys(step),
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: step.review,
          notes: scenario.coverage.reason,
        });
      }
      return;
    }
    const missing = scenario.requiredAssets.filter((asset) => !options.assets[asset]);
    if (scenario.requiresGroup && !options.groupDid) {
      for (const step of scenario.steps) {
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: "skipped",
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          prompt: step.prompt,
          attachment: step.attachment,
          attachments: stepAssetKeys(step),
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: step.review,
          notes: "msg_center.group_did / JARVIS_DV_GROUP_DID is not configured",
        });
      }
      return;
    }
    if (missing.length > 0) {
      console.log(`[skip] ${scenario.id}: missing ${missing.map((asset) => ASSET_ENV[asset]).join(", ")}`);
      for (const step of scenario.steps) {
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: "skipped",
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          prompt: step.prompt,
          attachment: step.attachment,
          attachments: stepAssetKeys(step),
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: step.review,
          notes: `missing assets: ${missing.join(", ")}`,
        });
      }
      return;
    }

    try {
    const sentMessageIds = new Map<string, string>();
    console.log(`\n[scenario] ${scenario.id} — ${scenario.title}`);
    const topic = scenario.requiresGroup ? options.groupDid! : await createScenarioSession(scenario);
    console.log(`[session] ${scenario.id}: ${topic}`);

    for (const step of scenario.steps) {
      const started = Date.now();
      const traceId = `${report.run_id}:msg-center:${scenario.id}:${step.id}`;
      const before = await listSession(msgCenter, userDid, topic);
      const afterSortKey = maxSortKey(before);
      console.log(`[send] ${scenario.id}/${step.id}: ${step.prompt}`);
      const stepAssets = stepAssetKeys(step);
      const attachments = stepAssets.map((asset) => ({
        asset,
        objectId: options.assets[asset],
      })).filter((item): item is { asset: AssetKey; objectId: string } => Boolean(item.objectId));
      const replyTo = step.replyToStep
        ? sentMessageIds.get(step.replyToStep)
        : undefined;
      if (step.replyToStep && !replyTo) {
        throw new Error(
          `${scenario.id}/${step.id} cannot resolve reply_to step ${step.replyToStep}`,
        );
      }
      const outboundMessage = makeMessage({
        userDid,
        targetDid: scenario.requiresGroup ? options.groupDid! : jarvisDid,
        topic,
        prompt: step.prompt,
        traceId,
        kind: step.messageKind,
        sourceDid: step.sourceDid,
        replyTo,
        attachments,
      });
      const sentMsgId = await postMessage(msgCenter, outboundMessage, traceId);
      if (step.duplicateInbound) {
        const duplicateMsgId = await postMessage(msgCenter, outboundMessage, traceId);
        if (sentMsgId && duplicateMsgId && sentMsgId !== duplicateMsgId) {
          throw new Error(`duplicate inbound returned different message IDs: ${sentMsgId} / ${duplicateMsgId}`);
        }
      }
      if (sentMsgId) sentMessageIds.set(step.id, sentMsgId);

      if (step.delayAfterSendMs) {
        await new Promise((resolve) => setTimeout(resolve, step.delayAfterSendMs));
      }
      if (step.sendWithoutWaiting) {
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: "dispatched",
          started_at: new Date(started).toISOString(),
          elapsed_ms: Date.now() - started,
          prompt: step.prompt,
          attachment: step.attachment,
          attachments: stepAssets,
          reply_texts: [],
          reply_refs: [],
          automatic_checks: ["message dispatched without waiting to exercise batching"],
          review: step.review,
        });
        continue;
      }

      try {
        const waited = await waitForReply({
          msgCenter,
          userDid,
          jarvisDid,
          topic,
          afterSortKey,
          step,
          settleMs: options.settleMs,
        });
        const texts = replyTexts(waited.items, jarvisDid);
        const refs = replyRefs(waited.items, jarvisDid);
        const artifactAudits = await Promise.all(refs.map(async (ref) => {
          if (!ref.target.obj_id) throw new Error(`outbound attachment ${ref.label ?? "<unnamed>"} omitted obj_id`);
          if (!uploadedFixtureIds.includes(ref.target.obj_id)) generatedArtifactIds.add(ref.target.obj_id);
          return await validateNamedArtifact(ndmProxy, {
            obj_id: ref.target.obj_id,
            label: ref.label,
          });
        }));
        for (const audit of artifactAudits) {
          waited.checks.push(
            `readable artifact ${audit.obj_id}: ${audit.size} bytes sha256=${audit.sha256}` +
              (audit.archive_entries ? ` entries=${audit.archive_entries.join(",")}` : ""),
          );
        }
        if (step.duplicateInbound) {
          const storedInputs = waited.items.filter((item) =>
            item.direction === "out" && item.from === userDid
          );
          const visibleReplies = waited.items.filter((item) =>
            item.direction === "in" && item.from === jarvisDid
          );
          if (storedInputs.length !== 1 || visibleReplies.length !== 1) {
            throw new ReplyWaitError(
              `duplicate inbound produced ${storedInputs.length} stored inputs and ${visibleReplies.length} user-visible replies`,
              waited.items,
              waited.checks,
            );
          }
          waited.checks.push("duplicate inbound produced one stored input and one user-visible reply");
        }
        if (step.assertUniqueOutbound) {
          const visibleReplies = waited.items.filter((item) =>
            item.direction === "in" && item.from === jarvisDid
          );
          if (visibleReplies.length !== 1 ||
            new Set(visibleReplies.map((item) => item.msg_id)).size !== visibleReplies.length ||
            new Set(visibleReplies.map((item) => item.record_id)).size !== visibleReplies.length) {
            throw new ReplyWaitError(
              `expected one unique outbound reply, observed ${visibleReplies.length}`,
              waited.items,
              waited.checks,
            );
          }
          waited.checks.push("one unique outbound msg_id/record_id was visible after settlement");
        }
        let judge: StepResult["judge"];
        let review: ReturnType<typeof reviewStep> = step.review.length
          ? { status: "review" }
          : { status: "passed" };
        if (step.review.length && options.judgeEnabled) {
          const verdict = await judgeStep({
            aicc,
            taskManager,
            model: options.judgeModel,
            traceId,
            step,
            texts,
            refs,
          });
          report.judge.task_ids.push(verdict.taskId);
          judge = {
            rubric_version: T3_JUDGE_RUBRIC_VERSION,
            model: options.judgeModel,
            task_id: verdict.taskId,
            passed: verdict.passed,
            score: verdict.score,
            reason: verdict.reason,
            input_summary: {
              reply_text_count: texts.length,
              attachment_count: refs.length,
              attachment_types: uniqueValues(refs.map((ref) => ref.target.type)),
            },
          };
          review = verdict.passed
            ? { status: "passed", notes: `LLM Judge: ${verdict.reason}` }
            : { status: "failed", notes: `LLM Judge: ${verdict.reason}` };
          waited.checks.push(`LLM Judge ${verdict.passed ? "passed" : "failed"}: ${verdict.reason}`);
        }
        if (options.interactiveReview) review = reviewStep(step);
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: review.status,
          started_at: new Date(started).toISOString(),
          elapsed_ms: Date.now() - started,
          prompt: step.prompt,
          attachment: step.attachment,
          attachments: stepAssets,
          reply_texts: texts,
          reply_refs: refs,
          artifact_audits: artifactAudits,
          automatic_checks: waited.checks,
          review: step.review,
          notes: review.notes,
          judge,
        });
        console.log(`[${review.status}] ${scenario.id}/${step.id}`);
      } catch (error) {
        const observedItems = error instanceof ReplyWaitError ? error.items : [];
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: "failed",
          started_at: new Date(started).toISOString(),
          elapsed_ms: Date.now() - started,
          prompt: step.prompt,
          attachment: step.attachment,
          attachments: stepAssets,
          reply_texts: replyTexts(observedItems, jarvisDid),
          reply_refs: replyRefs(observedItems, jarvisDid),
          automatic_checks: error instanceof ReplyWaitError ? error.checks : [],
          review: step.review,
          failure_class: error instanceof JudgeError ? "judge_failed" : undefined,
          error: String(error),
        });
        console.error(`[failed] ${scenario.id}/${step.id}: ${String(error)}`);
      }
    }
    } catch (error) {
      const recordedStepIds = new Set(
        report.results.filter((result) => result.scenario_id === scenario.id)
          .map((result) => result.step_id),
      );
      for (const step of scenario.steps) {
        if (recordedStepIds.has(step.id)) continue;
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: "failed",
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          prompt: step.prompt,
          attachment: step.attachment,
          attachments: stepAssetKeys(step),
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: step.review,
          error: `scenario execution failed before step result: ${String(error)}`,
        });
      }
      console.error(`[failed] ${scenario.id}: ${String(error)}`);
    }
  };

  let nextScenario = 0;
  const workerCount = Math.min(
    options.interactiveReview ? 1 : options.scenarioConcurrency,
    scenarios.length,
  );
  await Promise.all(Array.from({ length: workerCount }, async () => {
    while (true) {
      const index = nextScenario++;
      if (index >= scenarios.length) return;
      await runScenario(scenarios[index]);
    }
  }));
  const removedFixtures: string[] = [];
  const removedArtifacts: string[] = [];
  const residual: string[] = [];
  for (const objId of uploadedFixtureIds) {
    try {
      await ndmProxy.removeChunk({ chunk_id: objId });
      removedFixtures.push(objId);
    } catch {
      residual.push(objId);
    }
  }
  for (const objId of generatedArtifactIds) {
    try {
      if (objId.startsWith("mix256:")) await ndmProxy.removeChunk({ chunk_id: objId });
      else await ndmProxy.removeObject({ obj_id: objId });
      removedArtifacts.push(objId);
    } catch {
      residual.push(objId);
    }
  }
  report.resource_cleanup = {
    status: residual.length === 0 ? "passed" : "failed",
    removed_fixture_ids: removedFixtures,
    removed_artifact_ids: removedArtifacts,
    residual_ids: residual,
  };
  report.conversation_cleanup = {
    status: createdSessionIds.size > 0 ? "platform_limitation" : "passed",
    removed_external_message_ids: report.conversation_cleanup?.removed_external_message_ids ?? [],
    residual_session_ids: [...createdSessionIds].sort(),
    details: createdSessionIds.size > 0
      ? ["Current msg-center public KAPI has list/post methods but no scoped delete-session or delete-message method; test-created session IDs are reported for operator cleanup."]
      : [],
  };
  if (createdSessionIds.size > 0) {
    report.results.push({
      transport: "msg-center",
      scenario_id: "_cleanup",
      step_id: "conversation_history",
      status: "platform_limitation",
      started_at: new Date().toISOString(),
      elapsed_ms: 0,
      prompt: "",
      reply_texts: [],
      reply_refs: [],
      automatic_checks: [],
      review: [],
      failure_class: "platform_limitation",
      notes: "Test-created msg-center sessions cannot be deleted through the current public KAPI; residual IDs are listed in conversation_cleanup.",
    });
  }
  if (residual.length > 0) {
    report.results.push({
      scenario_id: "_cleanup",
      step_id: "named_data",
      status: "failed",
      started_at: new Date().toISOString(),
      elapsed_ms: 0,
      prompt: "",
      reply_texts: [],
      reply_refs: [],
      automatic_checks: [],
      review: [],
      failure_class: "cleanup_failed",
      error: `failed to remove ${residual.length} test-created Named Data objects`,
    });
  }
}

function telegramTexts(messages: TelegramObservedMessage[]): string[] {
  return messages.map((message) => message.text.trim()).filter(Boolean);
}

function telegramRefs(messages: TelegramObservedMessage[]): RefItem[] {
  return messages.flatMap((message) => message.media
    ? [{
      role: "output",
      target: {
        type: "telegram_media",
        obj_id: `telegram:${message.media.messageId}`,
      },
      label: [message.media.fileName, message.media.mimeType].filter(Boolean).join(" "),
    }]
    : []);
}

async function waitForTelegramReply(input: {
  telegram: TelegramDvClient;
  afterMessageId: number;
  step: ScenarioStep;
  settleMs: number;
}): Promise<{ messages: TelegramObservedMessage[]; checks: string[] }> {
  const deadline = Date.now() + (input.step.maxWaitMs ?? 180_000);
  let lastSignature = "";
  let lastChangeAt = Date.now();
  let lastFailures: string[] = [];
  let latest: TelegramObservedMessage[] = [];

  while (Date.now() < deadline) {
    latest = await input.telegram.messagesAfter(input.afterMessageId);
    const texts = telegramTexts(latest);
    const refs = telegramRefs(latest);
    const signature = JSON.stringify(latest);
    if (signature !== lastSignature) {
      lastSignature = signature;
      lastChangeAt = Date.now();
    }
    const evaluation = evaluateAutomatic(input.step, texts, refs);
    lastFailures = evaluation.failures;
    if (evaluation.ready) {
      if (input.step.expect.artifact || input.step.expect.artifacts?.length ||
        Date.now() - lastChangeAt >= input.settleMs) {
        return { messages: latest, checks: evaluation.checks };
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }

  throw new Error(
    `timed out after ${input.step.maxWaitMs ?? 180_000} ms: ${lastFailures.join("; ") || "no final Telegram reply"}`,
  );
}

async function runTelegram(
  options: CliOptions,
  scenarios: Scenario[],
  report: RunReport,
): Promise<void> {
  let judgeAicc: RpcClient | undefined;
  let judgeTaskManager: RpcClient | undefined;
  if (options.judgeEnabled && scenarios.some((scenario) =>
    scenario.steps.some((step) => step.review.length > 0)
  )) {
    const session = await gatewaySession(options);
    const { buckyos } = await import("buckyos");
    judgeAicc = new buckyos.kRPCClient(
      `${options.gatewayUrl}/kapi/aicc`,
      session.token,
    ) as RpcClient;
    judgeTaskManager = new buckyos.kRPCClient(
      `${options.gatewayUrl}/kapi/task-manager`,
      session.token,
    ) as RpcClient;
  }
  const apiId = options.telegramApiId ?? Number(await promptValue("Telegram API ID"));
  if (!Number.isSafeInteger(apiId) || apiId <= 0) {
    throw new Error("Telegram API ID must be a positive integer");
  }
  const apiHash = options.telegramApiHash ?? await promptValue("Telegram API hash", true);
  const botUsername = options.telegramBotUsername ?? await promptValue("Jarvis Telegram bot username");
  const telegramPrompt = options.assumeYes
    ? async (label: string): Promise<string> => {
      throw new Error(`${label} is required; automated mode does not accept interactive input`);
    }
    : promptValue;
  report.telegram_bot = botUsername.startsWith("@") ? botUsername : `@${botUsername}`;
  const telegram = new TelegramDvClient({
    apiId,
    apiHash,
    phoneNumber: options.telegramPhone,
    phoneCode: options.telegramCode,
    password: options.telegramPassword,
    session: options.telegramSession,
    sessionFile: options.telegramSessionFile,
    botUsername,
    connectionRetries: options.telegramConnectionRetries,
    promptValue: telegramPrompt,
  });

  console.log(`[probe] connecting Telegram user client to ${report.telegram_bot}`);
  await telegram.connect();
  console.log(`[probe] Telegram user session ready; persisted at ${options.telegramSessionFile}`);
  const baselineMessageId = await telegram.latestMessageId();
  const testMessageIds = new Set<number>();
  try {
    for (const scenario of scenarios) {
      if (scenario.coverage) {
        for (const step of scenario.steps) {
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: scenario.coverage.status,
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssetKeys(step),
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            notes: scenario.coverage.reason,
          });
        }
        continue;
      }
      if (scenario.requiresGroup || scenario.steps.some((step) => step.sourceDid)) {
        for (const step of scenario.steps) {
          report.results.push({
            transport: "telegram",
            scenario_id: scenario.id,
            step_id: step.id,
            status: "platform_limitation",
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssetKeys(step),
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            notes: scenario.requiresGroup
              ? "Telegram DV is configured for a direct bot dialog; no parameterized group chat target is available"
              : "Telegram client submission does not expose a deterministic original forward source in this runner",
          });
        }
        continue;
      }
      const missing: AssetKey[] = [];
      for (const asset of scenario.requiredAssets) {
        const file = options.telegramAssets[asset];
        if (!file || !await pathExists(file)) missing.push(asset);
      }
      if (missing.length > 0) {
        console.log(`[skip] ${scenario.id}: missing local files ${missing.join(", ")}`);
        for (const step of scenario.steps) {
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: "skipped",
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssetKeys(step),
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            notes: `missing Telegram asset files: ${missing.join(", ")}`,
          });
        }
        continue;
      }

      const scenarioResultStart = report.results.length;
      try {
      const sentMessageIds = new Map<string, number>();
      console.log(`\n[scenario] ${scenario.id} — ${scenario.title}`);
      const cleanId = await telegram.send({ text: "/clean" });
      testMessageIds.add(cleanId);
      const cleanReply = await waitForTelegramReply({
        telegram,
        afterMessageId: cleanId,
        step: {
          id: "clean",
          prompt: "/clean",
          expect: { textRequired: true },
          maxWaitMs: 60_000,
          review: [],
        },
        settleMs: 1_500,
      });
      for (const message of cleanReply.messages) testMessageIds.add(message.messageId);

      for (const step of scenario.steps) {
        const started = Date.now();
        if (step.duplicateInbound) {
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: "platform_limitation",
            started_at: new Date(started).toISOString(),
            elapsed_ms: 0,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssetKeys(step),
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            notes: "Telegram client submission does not expose a controllable duplicate external update ID; msg-center covers deterministic inbound idempotency.",
          });
          continue;
        }
        const replyTo = step.replyToStep ? sentMessageIds.get(step.replyToStep) : undefined;
        if (step.replyToStep && !replyTo) {
          throw new Error(
            `${scenario.id}/${step.id} cannot resolve Telegram reply_to step ${step.replyToStep}`,
          );
        }
        const stepAssets = stepAssetKeys(step);
        const files = stepAssets.map((asset) => options.telegramAssets[asset])
          .filter((file): file is string => Boolean(file));
        console.log(`[telegram-send] ${scenario.id}/${step.id}: ${step.prompt}`);
        const sentMessageId = await telegram.send({
          text: step.prompt,
          file: files.length > 0 ? files : undefined,
          replyTo,
        });
        testMessageIds.add(sentMessageId);
        sentMessageIds.set(step.id, sentMessageId);

        if (step.delayAfterSendMs) {
          await new Promise((resolve) => setTimeout(resolve, step.delayAfterSendMs));
        }
        if (step.sendWithoutWaiting) {
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: "dispatched",
            started_at: new Date(started).toISOString(),
            elapsed_ms: Date.now() - started,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssets,
            reply_texts: [],
            reply_refs: [],
            automatic_checks: ["message dispatched through Telegram without waiting"],
            review: step.review,
          });
          continue;
        }

        try {
          const waited = await waitForTelegramReply({
            telegram,
            afterMessageId: sentMessageId,
            step,
            settleMs: options.settleMs,
          });
          const texts = telegramTexts(waited.messages);
          for (const message of waited.messages) testMessageIds.add(message.messageId);
          const refs = telegramRefs(waited.messages);
          const artifactAudits: ArtifactAudit[] = [];
          const judgeResourceBlocks: Array<Record<string, unknown>> = [];
          for (const message of waited.messages.filter((item) => item.media)) {
            const bytes = await telegram.downloadMedia(message.messageId);
            const label = [message.media?.fileName, message.media?.mimeType].filter(Boolean).join(" ");
            artifactAudits.push(await validateArtifactBytes(bytes, {
              id: `telegram:${message.messageId}`,
              label,
            }));
            judgeResourceBlocks.push({
              type: message.media?.mimeType.startsWith("image/") ? "image" : "document",
              source: {
                kind: "base64",
                mime: message.media?.mimeType ?? "application/octet-stream",
                data_base64: bytesBase64(bytes),
              },
            });
          }
          if (artifactAudits.length > 0) waited.checks.push(`validated ${artifactAudits.length} Telegram media payload(s) byte-for-byte`);
          const traceId = `${report.run_id}:telegram:${scenario.id}:${step.id}`;
          let judge: StepResult["judge"];
          let review: ReturnType<typeof reviewStep> = step.review.length
            ? { status: "review" }
            : { status: "passed" };
          if (step.review.length && options.judgeEnabled) {
            if (!judgeAicc || !judgeTaskManager) {
              throw new JudgeError("Telegram LLM Judge clients were not initialized");
            }
            const verdict = await judgeStep({
              aicc: judgeAicc,
              taskManager: judgeTaskManager,
              model: options.judgeModel,
              traceId,
              step,
              texts,
              refs,
              resourceBlocks: judgeResourceBlocks,
            });
            report.judge.task_ids.push(verdict.taskId);
            judge = {
              rubric_version: T3_JUDGE_RUBRIC_VERSION,
              model: options.judgeModel,
              task_id: verdict.taskId,
              passed: verdict.passed,
              score: verdict.score,
              reason: verdict.reason,
              input_summary: {
                reply_text_count: texts.length,
                attachment_count: refs.length,
                attachment_types: uniqueValues(refs.map((ref) => ref.target.type)),
              },
            };
            review = verdict.passed
              ? { status: "passed", notes: `LLM Judge: ${verdict.reason}` }
              : { status: "failed", notes: `LLM Judge: ${verdict.reason}` };
            waited.checks.push(`LLM Judge ${verdict.passed ? "passed" : "failed"}: ${verdict.reason}`);
          }
          if (options.interactiveReview) review = reviewStep(step);
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: review.status,
            started_at: new Date(started).toISOString(),
            elapsed_ms: Date.now() - started,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssetKeys(step),
            reply_texts: texts,
            reply_refs: refs,
            artifact_audits: artifactAudits,
            automatic_checks: waited.checks,
            review: step.review,
            notes: review.notes,
            judge,
          });
          console.log(`[${review.status}] ${scenario.id}/${step.id}`);
        } catch (error) {
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: "failed",
            started_at: new Date(started).toISOString(),
            elapsed_ms: Date.now() - started,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssets,
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            failure_class: error instanceof JudgeError ? "judge_failed" : undefined,
            error: String(error),
          });
          console.error(`[failed] ${scenario.id}/${step.id}: ${String(error)}`);
        }
      }
      } catch (error) {
        const recordedStepIds = new Set(
          report.results.slice(scenarioResultStart).map((result) => result.step_id),
        );
        for (const step of scenario.steps) {
          if (recordedStepIds.has(step.id)) continue;
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: "failed",
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
            prompt: step.prompt,
            attachment: step.attachment,
            attachments: stepAssetKeys(step),
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            error: `scenario execution failed before step result: ${String(error)}`,
          });
        }
        console.error(`[failed] ${scenario.id}: ${String(error)}`);
      }
    }
  } finally {
    try {
      for (const message of await telegram.messagesAfter(baselineMessageId)) testMessageIds.add(message.messageId);
      await telegram.deleteMessages([...testMessageIds]);
      const previous = report.conversation_cleanup;
      report.conversation_cleanup = {
        status: previous?.status === "platform_limitation" ? "platform_limitation" : "passed",
        removed_external_message_ids: [
          ...(previous?.removed_external_message_ids ?? []),
          ...[...testMessageIds].map((id) => `telegram:${id}`),
        ],
        residual_session_ids: previous?.residual_session_ids ?? [],
        details: [...(previous?.details ?? []), `Removed ${testMessageIds.size} test-created Telegram messages with revoke=true.`],
      };
    } catch (error) {
      const previous = report.conversation_cleanup;
      report.conversation_cleanup = {
        status: "failed",
        removed_external_message_ids: previous?.removed_external_message_ids ?? [],
        residual_session_ids: previous?.residual_session_ids ?? [],
        details: [...(previous?.details ?? []), `Telegram cleanup failed: ${String(error)}`],
      };
      report.results.push({
        transport: "telegram",
        scenario_id: "_cleanup",
        step_id: "telegram_messages",
        status: "failed",
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
        prompt: "",
        reply_texts: [],
        reply_refs: [],
        automatic_checks: [],
        review: [],
        failure_class: "cleanup_failed",
        error: String(error),
      });
    } finally {
      await telegram.disconnect();
    }
  }
}

function selectedAssets(scenarios: Scenario[]): AssetKey[] {
  return uniqueValues(scenarios.flatMap((scenario) => scenario.requiredAssets));
}

function readiness(configured: boolean): string {
  return configured ? "ready" : "prompt required";
}

function sourceSuffix(options: CliOptions, key: string): string {
  return ` [${options.parameterSources[key] ?? "derived"}]`;
}

function configuredSecret(value: string | undefined): string {
  return value ? "<configured>" : "<missing>";
}

async function collectPreflightInputs(options: CliOptions): Promise<void> {
  if (!options.gatewayUrl && !options.assumeYes) {
    options.gatewayUrl = (await promptValue("Zone Gateway base URL")).replace(/\/+$/, "");
    options.parameterSources.gatewayUrl = "interactive input";
  }
  if (options.dryRun || options.assumeYes) return;
  if (!options.sessionToken) {
    if (!options.username) {
      options.username = await promptValue("BuckyOS username");
      options.parameterSources.username = "interactive input";
    }
    if (!options.password) {
      options.password = await promptValue("BuckyOS password", true);
      options.parameterSources.password = "interactive input";
    }
  }
  if (options.transports.includes("telegram")) {
    if (!options.telegramApiId) {
      options.telegramApiId = parsePositiveInt(
        await promptValue("Telegram API ID"),
        0,
        "Telegram API ID",
      );
      options.parameterSources.telegramApiId = "interactive input";
    }
    if (!options.telegramApiHash) {
      options.telegramApiHash = await promptValue("Telegram API hash", true);
      options.parameterSources.telegramApiHash = "interactive input";
    }
    if (!options.telegramBotUsername) {
      options.telegramBotUsername = await promptValue("Jarvis Telegram bot username");
      options.parameterSources.telegramBotUsername = "interactive input";
    }
    const hasSession = Boolean(options.telegramSession) || await pathExists(options.telegramSessionFile);
    if (!hasSession && !options.telegramPhone) {
      options.telegramPhone = await promptValue("Telegram phone number");
      options.parameterSources.telegramPhone = "interactive input";
    }
  }
}

async function requiredParameterErrors(options: CliOptions): Promise<string[]> {
  const errors: string[] = [];
  const tokenDrivers = providerTokenDrivers(options.providerTokens);
  if (!options.gatewayUrl) {
    errors.push(GATEWAY_REQUIRED_ERROR);
  }
  if (options.assumeYes && options.interactiveReview) {
    errors.push("automated mode cannot use --interactive-review");
  }
  if (!options.sessionToken && !options.username) {
    errors.push("Gateway usage audit requires --session-token or --username with --password");
  }
  if (!options.sessionToken && !options.password) {
    errors.push("Gateway usage audit requires --session-token or --username with --password");
  }
  if (options.financeCallerAppIds.length === 0) {
    errors.push("finance requires at least one --finance-caller-app-id");
  }
  if (tokenDrivers.length > 0 && !options.applyProviderCredentials) {
    errors.push(
      "provider API tokens are configured but provider_credentials.apply_to_aicc_settings is false",
    );
  }
  if (
    tokenDrivers.length > 0 && options.applyProviderCredentials &&
    !options.allowCredentialMutationCli
  ) {
    errors.push("applying Provider credentials requires --allow-credential-mutation");
  }
  if (options.transports.includes("telegram")) {
    if (!options.telegramApiId) errors.push("telegram requires --telegram-api-id");
    if (!options.telegramApiHash) errors.push("telegram requires --telegram-api-hash");
    if (!options.telegramBotUsername) errors.push("telegram requires --telegram-bot");
    const hasSession = Boolean(options.telegramSession) || await pathExists(options.telegramSessionFile);
    if (!hasSession && !options.telegramPhone) {
      errors.push("telegram requires --telegram-phone when no saved session is available");
    }
    if (options.assumeYes && !hasSession && !options.telegramCode) {
      errors.push("automated telegram login requires --telegram-code when no saved session is available");
    }
  }
  return uniqueValues(errors);
}

async function printEnvironmentChecklist(
  options: CliOptions,
  scenarios: Scenario[],
): Promise<void> {
  const assets = selectedAssets(scenarios);
  const stepCount = scenarios.reduce((total, scenario) => total + scenario.steps.length, 0);
  console.log("\n=== Jarvis Media DV 测试环境清单 ===");
  console.log(`配置文件: ${options.configPath ?? "未使用（默认值/环境变量/命令行）"}`);
  console.log(`测试范围: ${options.suite}${sourceSuffix(options, "suite")}; cases=${options.caseIds.length ? options.caseIds.join(",") : "全部"}${sourceSuffix(options, "cases")}`);
  console.log(`场景规模: ${scenarios.length} 个场景; ${stepCount} 个步骤; 最多 ${stepCount * options.transports.length} 次步骤执行`);
  console.log(`参数规则: 通道选择可选，默认 msg-center；已启用通道的连接参数必须；场景素材缺少时跳过相关场景`);
  console.log(`消息出入口: ${options.transports.join(" -> ")}${sourceSuffix(options, "transports")}`);
  if (options.transports.includes("msg-center")) {
    const authReady = Boolean(options.sessionToken || (options.username && options.password));
    console.log(`  msg-center.gateway: ${options.gatewayUrl}${sourceSuffix(options, "gatewayUrl")}`);
    console.log(`  msg-center.auth: ${options.sessionToken ? "session token" : "username/password"}; ${readiness(authReady)}`);
    console.log(`  msg-center.session_token: ${configuredSecret(options.sessionToken)}${sourceSuffix(options, "sessionToken")}`);
    console.log(`  msg-center.username: ${options.username ?? "<missing>"}${sourceSuffix(options, "username")}`);
    console.log(`  msg-center.password: ${configuredSecret(options.password)}${sourceSuffix(options, "password")}`);
    console.log(`  msg-center.user_id: ${options.userId || "<login result>"}${sourceSuffix(options, "userId")}`);
    console.log(`  msg-center.user_did: ${options.userDid ?? "<derived>"}${sourceSuffix(options, "userDid")}`);
    console.log(`  msg-center.zone_did: ${options.zoneDid ?? "<runtime lookup>"}${sourceSuffix(options, "zoneDid")}`);
    console.log(`  msg-center.jarvis_did: ${options.jarvisDid ?? "<derived>"}${sourceSuffix(options, "jarvisDid")}`);
    console.log(`  msg-center.group_did: ${options.groupDid ?? "<missing; group scenario skipped>"}${sourceSuffix(options, "groupDid")}`);
    for (const asset of assets) {
      const configured = options.assets[asset];
      const path = ASSET_FILE[asset];
      const defaultStatus = await pathExists(path)
        ? `${path}; ready; uploads on start`
        : `${path}; missing`;
      console.log(`  msg-center.asset.${asset}: ${configured ?? defaultStatus}${sourceSuffix(options, `msg:${asset}`)}`);
    }
  }
  if (options.transports.includes("telegram")) {
    const sessionReady = Boolean(options.telegramSession) || await pathExists(options.telegramSessionFile);
    console.log(`  telegram.bot: ${options.telegramBotUsername ?? "<missing>"}${sourceSuffix(options, "telegramBotUsername")}`);
    console.log(`  telegram.api_id: ${options.telegramApiId ?? "<missing>"}${sourceSuffix(options, "telegramApiId")}`);
    console.log(`  telegram.api_hash: ${configuredSecret(options.telegramApiHash)}${sourceSuffix(options, "telegramApiHash")}`);
    console.log(`  telegram.phone: ${configuredSecret(options.telegramPhone)}${sourceSuffix(options, "telegramPhone")}`);
    console.log(`  telegram.login_code: ${options.telegramCode ? "<configured>" : options.assumeYes && !sessionReady ? "<missing; required for new automated login>" : "<requested during login>"}${sourceSuffix(options, "telegramCode")}`);
    console.log(`  telegram.2fa_password: ${options.telegramPassword ? "<configured>" : options.assumeYes ? "<required if account uses 2FA>" : "<requested during login if required>"}${sourceSuffix(options, "telegramPassword")}`);
    console.log(`  telegram.string_session: ${configuredSecret(options.telegramSession)}${sourceSuffix(options, "telegramSession")}`);
    console.log(`  telegram.session_file: ${options.telegramSessionFile}; ${sessionReady ? "ready" : "new login required"}${sourceSuffix(options, "telegramSessionFile")}`);
    console.log(`  telegram.connection_retries: ${options.telegramConnectionRetries}${sourceSuffix(options, "telegramConnectionRetries")}`);
    for (const asset of assets) {
      const path = options.telegramAssets[asset] ?? "";
      console.log(`  telegram.asset.${asset}: ${path || "<missing>"}; ${path && await pathExists(path) ? "ready" : "missing; related scenarios will be skipped"}${sourceSuffix(options, `telegram:${asset}`)}`);
    }
  }
  console.log(
    options.expectedProviders.length > 0
      ? `期望 Provider: ${options.expectedProviders.join(", ")}${sourceSuffix(options, "providers")}（声明的覆盖目标；实际以 AICC 路由与日志为准）`
      : `期望 Provider: 未限定${sourceSuffix(options, "providers")}（由 AICC 动态路由；实际以运行日志为准）`,
  );
  const tokenDrivers = providerTokenDrivers(options.providerTokens);
  console.log(
    `Provider credentials: ${tokenDrivers.join(", ") || "<none>"}; ` +
      `temporary_override=${options.applyProviderCredentials}; ` +
      `mutation_opt_in=${options.allowCredentialMutationCli}`,
  );
  console.log(`结果判定: interactive_review=${options.interactiveReview}${sourceSuffix(options, "interactiveReview")}; allow_review=${options.allowReview}${sourceSuffix(options, "allowReview")}; settle_ms=${options.settleMs}${sourceSuffix(options, "settleMs")}`);
  console.log(`对话并发: msg-center=${options.interactiveReview ? 1 : options.scenarioConcurrency}${sourceSuffix(options, "scenarioConcurrency")}; interactive review 与 Telegram 保持串行`);
  console.log(`Finance caller_app_ids: ${options.financeCallerAppIds.join(", ") || "<missing>"}${sourceSuffix(options, "financeCallerAppIds")}`);
  console.log(`Finance budget: $${options.financeBudgetUsd.toFixed(6)}; estimated_call=$${options.estimatedCostPerAiccCallUsd.toFixed(6)}; max_calls_per_step=${options.maxAiccCallsPerStep}`);
  console.log(`LLM Judge: enabled=${options.judgeEnabled}${sourceSuffix(options, "judgeEnabled")}; model=${options.judgeModel}${sourceSuffix(options, "judgeModel")}`);
  console.log(`开始策略: ${options.assumeYes ? "全自动非交互，立即开始" : "等待 10 秒，可确认或取消"}${sourceSuffix(options, "assumeYes")}`);
  console.log(`报告目录: ${options.reportDir}${sourceSuffix(options, "reportDir")}`);
  console.log("====================================\n");
}

async function confirmStart(options: CliOptions): Promise<boolean> {
  if (options.assumeYes) {
    console.log("[start] --yes 已跳过 10 秒确认等待。");
    return true;
  }
  console.log("[start] 10 秒后自动开始；输入 c 后按 Enter 可取消，直接按 Enter 可立即开始。");
  let finish!: (start: boolean) => void;
  let settled = false;
  const decision = new Promise<boolean>((resolve) => {
    finish = (start) => {
      if (settled) return;
      settled = true;
      resolve(start);
    };
  });
  let remaining = 10;
  console.log(`[start] ${remaining}s`);
  const ticker = setInterval(() => {
    remaining -= 1;
    if (remaining > 0) console.log(`[start] ${remaining}s`);
  }, 1_000);
  const timeout = setTimeout(() => finish(true), 10_000);
  const controller = new AbortController();
  let inputTask: Promise<void> | undefined;
  if (Deno.stdin.isTerminal()) {
    const decoder = new TextDecoder();
    inputTask = Deno.stdin.readable.pipeTo(
      new WritableStream<Uint8Array>({
        write(chunk) {
          const input = decoder.decode(chunk).trim().toLowerCase();
          if (input === "c" || input === "cancel") finish(false);
          else if (input === "") finish(true);
        },
      }),
      { signal: controller.signal, preventCancel: true, preventClose: true },
    ).catch((error) => {
      if (!(error instanceof DOMException && error.name === "AbortError")) throw error;
    });
  }
  const start = await decision;
  clearInterval(ticker);
  clearTimeout(timeout);
  controller.abort();
  await inputTask;
  return start;
}

function printDryRun(options: CliOptions, scenarios: Scenario[]): void {
  console.log(`[dry-run] transports=${options.transports.join(",")} suite=${options.suite}`);
  if (options.configPath) console.log(`[dry-run] config=${options.configPath}`);
  for (const scenario of scenarios) {
    console.log(`\n${scenario.id}: ${scenario.title}`);
    for (const asset of scenario.requiredAssets) {
      if (options.transports.includes("msg-center")) {
        console.log(
          `  msg-center asset ${asset}: ${options.assets[asset] ?? `${ASSET_FILE[asset]} (project default; uploads on start)`}`,
        );
      }
      if (options.transports.includes("telegram")) {
        console.log(`  telegram asset ${asset}: ${options.telegramAssets[asset] ?? "missing"}`);
      }
    }
    for (const step of scenario.steps) {
      console.log(`  - ${step.id}: ${step.prompt}`);
    }
  }
}

const ENTRY_MESSAGE_KINDS: EntryMessageKind[] = [
  "text",
  "image",
  "video",
  "audio",
  "document",
  "archive",
  "multi_attachment",
];

function mimeMessageKind(mime: string): Exclude<EntryMessageKind, "text" | "multi_attachment"> {
  if (mime.startsWith("image/")) return "image";
  if (mime.startsWith("video/")) return "video";
  if (mime.startsWith("audio/")) return "audio";
  if (mime === "application/zip") return "archive";
  return "document";
}

function stepCoverageKinds(step: ScenarioStep, direction: "inbound" | "outbound"): Set<EntryMessageKind> {
  const kinds = new Set<EntryMessageKind>();
  if (direction === "inbound") {
    kinds.add("text");
    const assets = stepAssetKeys(step);
    for (const asset of assets) kinds.add(mimeMessageKind(ASSET_LABEL[asset]));
    if (assets.length > 1) kinds.add("multi_attachment");
    return kinds;
  }
  if (step.expect.textRequired) kinds.add("text");
  const artifacts = step.expect.artifacts ?? (step.expect.artifact ? [step.expect.artifact] : []);
  for (const artifact of artifacts) kinds.add(mimeMessageKind(artifact));
  if ((step.expect.attachmentCount?.min ?? artifacts.length) > 1) kinds.add("multi_attachment");
  return kinds;
}

export function buildEntryCoverage(input: {
  transports: Transport[];
  scenarios: Scenario[];
  results: Array<Pick<StepResult, "transport" | "scenario_id" | "step_id" | "status">>;
}): EntryCoverageRecord[] {
  const records: EntryCoverageRecord[] = [];
  for (const transport of input.transports) {
    for (const direction of ["inbound", "outbound"] as const) {
      for (const kind of ENTRY_MESSAGE_KINDS) {
        const planned = input.scenarios.flatMap((scenario) =>
          scenario.steps
            .filter((step) => stepCoverageKinds(step, direction).has(kind))
            .map((step) => `${scenario.id}/${step.id}`)
        );
        const matching = input.results.filter((result) =>
          result.transport === transport &&
          planned.includes(`${result.scenario_id}/${result.step_id}`)
        );
        const covered = matching.filter((result) => result.status === "passed");
        const reviews = matching.filter((result) => result.status === "review");
        const limitations = matching.filter((result) => result.status === "platform_limitation");
        const notApplicable = matching.filter((result) => result.status === "not_applicable");
        const skipped = matching.filter((result) => result.status === "skipped");
        const status: EntryCoverageRecord["status"] = planned.length === 0
          ? "missing"
          : covered.length > 0
          ? "covered"
          : reviews.length > 0
          ? "review"
          : limitations.length > 0
          ? "platform_limitation"
          : notApplicable.length > 0
          ? "not_applicable"
          : skipped.length === matching.length && matching.length > 0
          ? "skipped"
          : "failed";
        records.push({
          transport,
          direction,
          kind,
          status,
          planned_case_ids: planned,
          covered_case_ids: [...covered, ...reviews].map((result) =>
            `${result.scenario_id}/${result.step_id}`
          ),
        });
      }
    }
  }
  return records;
}

function summarize(report: RunReport): Record<StepStatus, number> {
  const totals: Record<StepStatus, number> = {
    passed: 0,
    failed: 0,
    review: 0,
    skipped: 0,
    dispatched: 0,
    not_applicable: 0,
    platform_limitation: 0,
  };
  for (const result of report.results) totals[result.status] += 1;
  return totals;
}

const STEP_STATUSES: StepStatus[] = [
  "passed",
  "failed",
  "review",
  "skipped",
  "dispatched",
  "not_applicable",
  "platform_limitation",
];

function markdownTableCell(value: unknown): string {
  return String(value ?? "")
    .replaceAll("|", "\\|")
    .replace(/\r?\n/g, "<br>");
}

function markdownCodeBlock(value: string, language = "text"): string {
  const longestRun = [...value.matchAll(/`+/g)]
    .reduce((longest, match) => Math.max(longest, match[0].length), 0);
  const fence = "`".repeat(Math.max(3, longestRun + 1));
  return `${fence}${language}\n${value}\n${fence}`;
}

function resultStatusSummary(results: StepResult[]): string {
  return STEP_STATUSES
    .map((status) => [status, results.filter((result) => result.status === status).length] as const)
    .filter(([, count]) => count > 0)
    .map(([status, count]) => `${status}: ${count}`)
    .join(", ");
}

function classifyFailure(result: StepResult): StepResult["failure_class"] {
  if (result.status === "platform_limitation") return "platform_limitation";
  if (result.status !== "failed") return undefined;
  if (result.scenario_id === "_finance") return "usage_failed";
  if (result.scenario_id === "_environment" || /transport|login|gateway|telegram/i.test(result.error ?? "")) {
    return "message_transport_failed";
  }
  if (/attachment|artifact|mime|object|file|zip/i.test(result.error ?? "")) {
    return "attachment_failed";
  }
  if (/review|judge|semantic/i.test(result.error ?? "")) return "judge_failed";
  return "assertion_failed";
}

export function productDefects(results: StepResult[]): ProductDefect[] {
  const productFailureClasses = new Set<NonNullable<StepResult["failure_class"]>>([
    "attachment_failed",
    "usage_failed",
    "assertion_failed",
  ]);
  const defects: ProductDefect[] = [];
  for (const result of results) {
    const failureClass = result.failure_class;
    if (result.status !== "failed" || !failureClass || !productFailureClasses.has(failureClass)) {
      continue;
    }
    defects.push({
      defect_id: `T3-${result.scenario_id}-${result.step_id}-${result.failure_class}`,
      component: result.failure_class === "usage_failed" ? "AICC" : "Jarvis",
      case_id: `${result.scenario_id}:${result.step_id}`,
      expected: result.automatic_checks.length > 0
        ? result.automatic_checks.join("; ")
        : "the T3 scenario step completes and satisfies its automatic assertions",
      observed: result.error ?? result.notes ?? "scenario assertion failed",
      evidence_paths: [
        "summary.json",
        `conversations/${reportFileSegment(result.transport ?? "unknown")}-${reportFileSegment(result.scenario_id)}.md`,
      ],
      failure_class: failureClass,
    });
  }
  return defects;
}

function reportFileSegment(value: string): string {
  return value.replace(/[^A-Za-z0-9._-]+/g, "_");
}

function groupedResults(report: RunReport): StepResult[][] {
  const groups = new Map<string, StepResult[]>();
  for (const result of report.results) {
    const key = `${result.transport ?? "unknown"}\0${result.scenario_id}`;
    const group = groups.get(key) ?? [];
    group.push(result);
    groups.set(key, group);
  }
  return [...groups.values()];
}

function renderConversationMarkdown(report: RunReport, results: StepResult[]): string {
  const first = results[0];
  const transport = first.transport ?? "unknown";
  const scenario = SCENARIOS.find((item) => item.id === first.scenario_id);
  const lines = [
    `# ${first.scenario_id} 对话详情`,
    "",
    `[返回测试报告](../summary.md)`,
    "",
    `- Transport: \`${transport}\``,
    `- 场景: ${scenario?.title ?? first.scenario_id}`,
    `- 运行 ID: \`${report.run_id}\``,
    `- 结果: ${resultStatusSummary(results)}`,
    "",
  ];
  for (const result of results) {
    lines.push(
      `## ${result.step_id} — ${result.status}`,
      "",
      `- 开始时间: \`${result.started_at}\``,
      `- 耗时: ${result.elapsed_ms} ms`,
    );
    if (result.failure_class) lines.push(`- Failure class: \`${result.failure_class}\``);
    const inputAttachments = result.attachments ?? (result.attachment ? [result.attachment] : []);
    if (inputAttachments.length > 0) {
      lines.push(`- 输入附件 (${inputAttachments.length}): ${inputAttachments.map((item) => `\`${item}\``).join(", ")}`);
    }
    lines.push("", "### 用户", "", markdownCodeBlock(result.prompt || "<empty>"), "");
    if (result.reply_texts.length > 0) {
      for (const [index, reply] of result.reply_texts.entries()) {
        lines.push(
          `### Jarvis ${index + 1}`,
          "",
          markdownCodeBlock(reply || "<empty>"),
          "",
        );
      }
    } else {
      lines.push("### Jarvis", "", "_无文本回复_", "");
    }
    if (result.reply_refs.length > 0) {
      lines.push(
        "### Jarvis 附件",
        "",
        markdownCodeBlock(JSON.stringify(result.reply_refs, null, 2), "json"),
        "",
      );
    }
    if (result.automatic_checks.length > 0) {
      lines.push("### 自动检查", "");
      for (const check of result.automatic_checks) lines.push(`- ${check}`);
      lines.push("");
    }
    if (result.review.length > 0) {
      lines.push("### 人工检查项", "");
      for (const review of result.review) lines.push(`- ${review}`);
      lines.push("");
    }
    if (result.judge) {
      lines.push(
        "### LLM Judge",
        "",
        markdownCodeBlock(JSON.stringify(result.judge, null, 2), "json"),
        "",
      );
    }
    if (result.notes) lines.push("### 备注", "", markdownCodeBlock(result.notes), "");
    if (result.error) lines.push("### 错误", "", markdownCodeBlock(result.error), "");
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

function renderSummaryMarkdown(report: RunReport, groups: StepResult[][]): string {
  const totals = report.totals ?? summarize(report);
  const lines = [
    "# Jarvis Media DV 测试报告",
    "",
    "## 运行信息",
    "",
    "| 字段 | 值 |",
    "|---|---|",
    `| Run ID | \`${markdownTableCell(report.run_id)}\` |`,
    `| Commit | \`${markdownTableCell(report.commit)}\` |`,
    `| Capability baseline | \`${markdownTableCell(report.baseline_revision)}\` |`,
    `| 开始时间 | \`${markdownTableCell(report.started_at)}\` |`,
    `| 结束时间 | \`${markdownTableCell(report.finished_at)}\` |`,
    `| Transport | ${markdownTableCell(report.transports.join(", "))} |`,
    `| Suite | ${markdownTableCell(report.suite)} |`,
    `| Gateway | ${markdownTableCell(report.gateway_url ?? "-")} |`,
    `| User DID | ${markdownTableCell(report.user_did ?? "-")} |`,
    `| Jarvis DID | ${markdownTableCell(report.jarvis_did ?? "-")} |`,
    `| Telegram Bot | ${markdownTableCell(report.telegram_bot ?? "-")} |`,
    `| 期望 Provider | ${markdownTableCell(report.expected_providers.join(", ") || "未限定")} |`,
    "",
    "## 汇总",
    "",
    "| Passed | Failed | Review | Skipped | Dispatched | Not applicable | Platform limitation |",
    "|---:|---:|---:|---:|---:|---:|---:|",
    `| ${totals.passed} | ${totals.failed} | ${totals.review} | ${totals.skipped} | ${totals.dispatched} | ${totals.not_applicable} | ${totals.platform_limitation} |`,
  ];
  if (report.entry_coverage?.length) {
    lines.push(
      "",
      "## 入口消息覆盖",
      "",
      "| Transport | Direction | Kind | Status | Planned cases | Covered cases |",
      "|---|---|---|---|---:|---:|",
      ...report.entry_coverage.map((coverage) =>
        `| ${coverage.transport} | ${coverage.direction} | ${coverage.kind} | ${coverage.status} | ${coverage.planned_case_ids.length} | ${coverage.covered_case_ids.length} |`
      ),
    );
  }
  lines.push(
    "",
    "## 场景用例",
    "",
    "对话详情默认不在主报告中展开；点击每行的“查看对话”打开独立详情页。",
    "",
    "| Transport | 场景 | 结果 | 步骤数 | 耗时 | 对话详情 |",
    "|---|---|---|---:|---:|---|",
  );
  for (const results of groups) {
    const first = results[0];
    const transport = first.transport ?? "unknown";
    const scenario = SCENARIOS.find((item) => item.id === first.scenario_id);
    const title = scenario ? `${first.scenario_id} — ${scenario.title}` : first.scenario_id;
    const elapsedMs = results.reduce((total, result) => total + result.elapsed_ms, 0);
    const conversation = first.scenario_id === "_environment"
      ? "-"
      : `[查看对话](conversations/${reportFileSegment(transport)}-${reportFileSegment(first.scenario_id)}.md)`;
    lines.push(
      `| ${markdownTableCell(transport)} | ${markdownTableCell(title)} | ${markdownTableCell(resultStatusSummary(results))} | ${results.length} | ${elapsedMs} ms | ${conversation} |`,
    );
  }
  if (report.finance) {
    lines.push(
      "",
      "## AICC Finance",
      "",
      "| Status | Events | Input tokens | Output tokens | Request units | Actual USD | Raw USD | Credits USD | Unknown cost events |",
      "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
      `| ${report.finance.status} | ${report.finance.event_count} | ${report.finance.input_tokens} | ${report.finance.output_tokens} | ${report.finance.request_units} | ${report.finance.actual_cost_usd} | ${report.finance.raw_cost_usd} | ${report.finance.credit_applied_usd} | ${report.finance.unknown_cost_events} |`,
      "",
      `Workflow / Judge events: ${report.finance.workflow_event_count} / ${report.finance.judge_event_count}. Judge model: ${report.judge.model}; Judge task IDs: ${report.judge.task_ids.join(", ") || "none"}.`,
      "",
      `Planned maximum: ${report.finance.planned_max_calls} AICC calls / $${report.finance.planned_max_cost_usd.toFixed(6)}; budget: $${report.finance.budget_usd.toFixed(6)}.`,
      "",
      `Known actual / unknown estimated / total exposure: $${report.finance.actual_cost_usd.toFixed(6)} / $${report.finance.estimated_unknown_cost_usd.toFixed(6)} / $${report.finance.total_exposure_usd.toFixed(6)}; remaining budget: $${report.finance.remaining_budget_usd.toFixed(6)}; exceeded: ${report.finance.budget_exceeded}.`,
      "",
      `Attribution: ${report.finance.attribution}; callers: ${report.finance.caller_app_ids.join(", ")}.`,
      "",
      `Observed provider drivers: ${report.finance.observed_provider_drivers.join(", ") || "none"}.`,
      "",
      `Observed provider instances: ${report.finance.observed_provider_instances.join(", ") || "none"}.`,
      "",
      `Missing expected providers: ${report.finance.missing_expected_providers.join(", ") || "none"}.`,
      "",
      `Step correlation: ${report.finance.step_correlation.status}; ${report.finance.step_correlation.correlated_step_ids.length}/${report.finance.step_correlation.expected_step_ids.length} correlated; traces: ${report.finance.step_correlation.trace_count}.`,
      "",
      `Uncorrelated steps: ${report.finance.step_correlation.uncorrelated_step_ids.join(", ") || "none"}.`,
      "",
      ...(report.finance.step_correlation.defect
        ? [`Product defect: ${report.finance.step_correlation.defect}`, ""]
        : []),
      report.finance.attribution_limitation,
    );
    if (report.finance.error) lines.push("", `Error: ${report.finance.error}`);
  }
  if (report.product_defects && report.product_defects.length > 0) {
    lines.push("", "## Product defects", "");
    for (const defect of report.product_defects) {
      lines.push(
        `### ${defect.defect_id}`,
        "",
        `- Component: ${defect.component}`,
        `- Case: ${defect.case_id}`,
        `- Failure class: ${defect.failure_class}`,
        `- Expected: ${defect.expected}`,
        `- Observed: ${defect.observed}`,
        `- Evidence: ${defect.evidence_paths.join(", ")}`,
        "",
      );
    }
  }
  if (report.targeted_retest_command) {
    lines.push(
      "",
      "## Targeted retest",
      "",
      "Run after fixing the reported defect; repeat `--case` to select additional scenarios:",
      "",
      "```bash",
      report.targeted_retest_command,
      "```",
    );
  }
  lines.push("", "## 机器可读数据", "", "[summary.json](summary.json)", "");
  return `${lines.join("\n").trimEnd()}\n`;
}

async function writeReport(
  options: CliOptions,
  report: RunReport,
  finished = true,
): Promise<string> {
  if (finished) report.finished_at = new Date().toISOString();
  for (const result of report.results) result.failure_class ??= classifyFailure(result);
  report.totals = summarize(report);
  report.product_defects = productDefects(report.results);
  const failedScenarios = [...new Set(
    report.results
      .filter((result) =>
        ["failed", "not_applicable", "platform_limitation"].includes(result.status) &&
        !result.scenario_id.startsWith("_")
      )
      .map((result) => result.scenario_id),
  )];
  const retestScenarios = failedScenarios.length > 0
    ? failedScenarios
    : report.results.some((result) => result.status === "failed")
    ? report.selected_scenarios
    : [];
  report.targeted_retest_command = retestScenarios.length > 0
    ? [
      "pnpm test --",
      ...(options.configPath ? ["--config", JSON.stringify(options.configPath)] : []),
      "--yes",
      ...(options.applyProviderCredentials ? ["--allow-credential-mutation"] : []),
      ...retestScenarios.flatMap((scenarioId) => ["--case", scenarioId]),
    ].join(" ")
    : undefined;
  const dir = `${options.reportDir}/${report.run_id}`;
  await Deno.mkdir(dir, { recursive: true });
  await Deno.writeTextFile(`${dir}/summary.json`, `${JSON.stringify(report, null, 2)}\n`);
  const groups = groupedResults(report);
  const conversationsDir = `${dir}/conversations`;
  await Deno.mkdir(conversationsDir, { recursive: true });
  for (const results of groups) {
    const first = results[0];
    if (first.scenario_id === "_environment") continue;
    const transport = first.transport ?? "unknown";
    const fileName = `${reportFileSegment(transport)}-${reportFileSegment(first.scenario_id)}.md`;
    await Deno.writeTextFile(
      `${conversationsDir}/${fileName}`,
      renderConversationMarkdown(report, results),
    );
  }
  const path = `${dir}/summary.md`;
  await Deno.writeTextFile(path, renderSummaryMarkdown(report, groups));
  return path;
}

async function main(): Promise<void> {
  const options = await parseArgs(Deno.args);
  if (options.list) {
    printScenarioList();
    return;
  }
  const scenarios = selectScenarios(options);
  const plannedSteps = scenarios.reduce((sum, scenario) => sum + scenario.steps.length, 0) *
    options.transports.length;
  const plannedMaxCalls = plannedSteps * options.maxAiccCallsPerStep;
  const plannedMaxCostUsd = plannedMaxCalls * options.estimatedCostPerAiccCallUsd;
  console.log(`[plan] scenarios=${scenarios.length} step_executions=${plannedSteps} max_aicc_calls=${plannedMaxCalls}`);
  console.log(`[plan] estimated_max_cost_usd=${plannedMaxCostUsd.toFixed(6)} budget_usd=${options.financeBudgetUsd.toFixed(6)}`);
  if (plannedMaxCostUsd > options.financeBudgetUsd) {
    throw new Error(`planned T3 cost ${plannedMaxCostUsd} exceeds finance budget ${options.financeBudgetUsd}`);
  }
  await collectPreflightInputs(options);
  if (!options.gatewayUrl) {
    console.error(`[fatal] ${GATEWAY_REQUIRED_ERROR}`);
    Deno.exitCode = 1;
    return;
  }
  await printEnvironmentChecklist(options, scenarios);
  if (options.dryRun) {
    printDryRun(options, scenarios);
    return;
  }
  const parameterErrors = await requiredParameterErrors(options);
  if (parameterErrors.length > 0) {
    for (const error of parameterErrors) console.error(`[fatal] ${error}`);
    Deno.exitCode = 1;
    return;
  }
  if (!await confirmStart(options)) {
    console.log("[cancelled] 测试尚未开始。");
    return;
  }

  const report: RunReport = {
    run_id: `${Date.now()}-${crypto.randomUUID().slice(0, 8)}`,
    started_at: new Date().toISOString(),
    commit: await repositoryCommit(),
    baseline_revision: await providerBaselineRevision(),
    transports: options.transports,
    expected_providers: options.expectedProviders,
    suite: options.suite,
    selected_scenarios: scenarios.map((scenario) => scenario.id),
    results: [],
    judge: {
      enabled: options.judgeEnabled,
      model: options.judgeModel,
      task_ids: [],
    },
  };
  report.results.push({
    scenario_id: "_messagehub_public_entry",
    step_id: "separate_public_kapi",
    status: "platform_limitation",
    started_at: new Date().toISOString(),
    elapsed_ms: 0,
    prompt: "",
    reply_texts: [],
    reply_refs: [],
    automatic_checks: ["The msg-center scenarios target shareable Jarvis DIDs and therefore traverse the native MessageHub DeliveryExecutor."],
    review: [],
    failure_class: "platform_limitation",
    notes: "Current BuckyOS exposes MessageHub as an internal native DeliveryExecutor behind msg-center, not as a separate public Gateway KAPI. A distinct public-entry execution cannot be implemented without a product API.",
  });

  let checkpointWriting = false;
  let activeTransport: Transport | undefined;
  const checkpointId = setInterval(async () => {
    if (checkpointWriting) return;
    checkpointWriting = true;
    try {
      if (activeTransport) {
        for (const result of report.results) {
          result.transport ??= activeTransport;
        }
      }
      await writeReport(options, report, false);
    } catch (error) {
      console.error(`[report] checkpoint failed: ${String(error)}`);
    } finally {
      checkpointWriting = false;
    }
  }, 30_000);

  const executeTransports = async (): Promise<void> => {
    for (const transport of options.transports) {
      activeTransport = transport;
      const resultStart = report.results.length;
      console.log(`\n[transport] ${transport}`);
      try {
        if (transport === "msg-center") {
          await runMsgCenter(options, scenarios, report);
        } else {
          await runTelegram(options, scenarios, report);
        }
      } catch (error) {
        const recordedSteps = new Set(
          report.results.slice(resultStart).map((result) =>
            `${result.scenario_id}\u0000${result.step_id}`
          ),
        );
        for (const scenario of scenarios) {
          for (const step of scenario.steps) {
            const key = `${scenario.id}\u0000${step.id}`;
            if (recordedSteps.has(key)) continue;
            report.results.push({
              transport,
              scenario_id: scenario.id,
              step_id: step.id,
              status: "failed",
              started_at: new Date().toISOString(),
              elapsed_ms: 0,
              prompt: step.prompt,
              attachment: step.attachment,
              attachments: stepAssetKeys(step),
              reply_texts: [],
              reply_refs: [],
              automatic_checks: [],
              review: step.review,
              error: `transport initialization failed before scenario result: ${String(error)}`,
            });
          }
        }
        report.results.push({
          transport,
          scenario_id: "_environment",
          step_id: transport,
          status: "failed",
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          prompt: "",
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: [],
          error: String(error),
        });
        console.error(`[failed] ${transport}: ${String(error)}`);
      } finally {
        for (let index = resultStart; index < report.results.length; index += 1) {
          report.results[index].transport ??= transport;
        }
        activeTransport = undefined;
      }
    }
    report.entry_coverage = buildEntryCoverage({
      transports: options.transports,
      scenarios,
      results: report.results,
    });
    if (options.suite === "all" && options.caseIds.length === 0) {
      for (const coverage of report.entry_coverage.filter((item) => item.status === "missing")) {
        report.results.push({
          transport: coverage.transport,
          scenario_id: "_entry_coverage",
          step_id: `${coverage.direction}.${coverage.kind}`,
          status: "failed",
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          prompt: "",
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: [],
          error: `no T3 scenario declares ${coverage.transport} ${coverage.direction} ${coverage.kind} coverage`,
        });
      }
    }
    await auditRunFinance(options, report);
    if (report.finance?.status === "failed") {
      report.results.push({
        scenario_id: "_finance",
        step_id: "aicc_usage_audit",
        status: "failed",
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
        prompt: "",
        reply_texts: [],
        reply_refs: [],
        automatic_checks: [],
        review: [],
        error: report.finance.error,
      });
    }
  };

  try {
    const tokenDrivers = providerTokenDrivers(options.providerTokens);
    if (tokenDrivers.length === 0) {
      await executeTransports();
    } else {
      const session = await gatewaySession(options);
      const { buckyos } = await import("buckyos");
      const aicc = new buckyos.kRPCClient(
        `${options.gatewayUrl}/kapi/aicc`,
        session.token,
      ) as RpcClient;
      const systemConfig = new buckyos.kRPCClient(
        `${options.gatewayUrl}/kapi/system_config`,
        session.token,
      ) as RpcClient;
      const outcome = await withAiccSettingsOverride({
        systemConfig,
        aicc,
        description: "Jarvis DV Provider credential override",
        patch: (settings) =>
          applyProviderTokens(settings, options.providerTokens, options.providerInstances),
        execute: executeTransports,
        refreshClients: options.username && options.password
          ? async () => {
            const refreshed = await gatewaySession(options, true);
            return {
              aicc: new buckyos.kRPCClient(
                `${options.gatewayUrl}/kapi/aicc`,
                refreshed.token,
              ) as RpcClient,
              systemConfig: new buckyos.kRPCClient(
                `${options.gatewayUrl}/kapi/system_config`,
                refreshed.token,
              ) as RpcClient,
            };
          }
          : undefined,
      });
      report.provider_credential_override = {
        drivers: tokenDrivers,
        cleanup: outcome.cleanup,
      };
    }
  } finally {
    clearInterval(checkpointId);
    while (checkpointWriting) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    const path = await writeReport(options, report);
    console.log(`\n[report] ${path}`);
  }

  const totals = report.totals ?? summarize(report);
  console.log(`[done] ${JSON.stringify(totals)}`);
  if (totals.failed > 0) Deno.exitCode = 1;
  else if ((totals.review > 0 || totals.skipped > 0 || totals.not_applicable > 0 || totals.platform_limitation > 0) &&
    !options.allowReview) Deno.exitCode = 2;
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(`[fatal] ${String(error)}`);
    Deno.exitCode = 1;
  });
}
