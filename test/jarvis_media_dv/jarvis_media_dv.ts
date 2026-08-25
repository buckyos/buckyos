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

type JsonObject = Record<string, unknown>;

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
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

type StepStatus = "passed" | "failed" | "review" | "skipped" | "dispatched";

type StepResult = {
  transport?: Transport;
  scenario_id: string;
  step_id: string;
  status: StepStatus;
  started_at: string;
  elapsed_ms: number;
  prompt: string;
  attachment?: AssetKey;
  reply_texts: string[];
  reply_refs: RefItem[];
  automatic_checks: string[];
  review: string[];
  notes?: string;
  error?: string;
};

type RunReport = {
  run_id: string;
  started_at: string;
  finished_at?: string;
  transports: Transport[];
  expected_providers: string[];
  suite: string;
  gateway_url?: string;
  user_did?: string;
  jarvis_did?: string;
  telegram_bot?: string;
  selected_scenarios: string[];
  results: StepResult[];
  totals?: Record<StepStatus, number>;
};

type Transport = "msg-center" | "telegram";

type CliOptions = {
  transports: Transport[];
  expectedProviders: string[];
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
  parameterSources: Record<string, string>;
};

const FLAG_TO_ASSET: Record<string, AssetKey> = {
  "image-primary-id": "image_primary",
  "image-secondary-id": "image_secondary",
  "image-ocr-id": "image_ocr",
  "audio-sfx-id": "audio_sfx",
  "audio-speech-id": "audio_speech",
  "video-fresh-id": "video_fresh",
};

const FLAG_TO_TELEGRAM_ASSET: Record<string, AssetKey> = {
  "image-primary-file": "image_primary",
  "image-secondary-file": "image_secondary",
  "image-ocr-file": "image_ocr",
  "audio-sfx-file": "audio_sfx",
  "audio-speech-file": "audio_speech",
  "video-fresh-file": "video_fresh",
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
  const zoneHost = tomlString(config, "msg_center.zone_host") ??
    env("BUCKYOS_TEST_ZONE_HOST") ?? "test.buckyos.io";
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
    assumeYes: tomlBoolean(config, "common.yes") ??
      parseBoolean(env("JARVIS_DV_YES"), false, "JARVIS_DV_YES"),
    suite: suiteValue(tomlString(config, "common.suite") ?? env("JARVIS_DV_SUITE")),
    caseIds: tomlStrings(config, "common.cases") ?? [],
    configPath: loaded.path,
    gatewayUrl: tomlString(config, "msg_center.gateway_url") ??
      env("BUCKYOS_TEST_GATEWAY_URL") ?? `https://${zoneHost}`,
    sessionToken: tomlString(config, "msg_center.session_token") ??
      env("BUCKYOS_APPCLIENT_SESSION_TOKEN"),
    username: tomlString(config, "msg_center.username") ?? env("BUCKYOS_TEST_USERNAME"),
    password: tomlString(config, "msg_center.password") ?? env("BUCKYOS_TEST_PASSWORD"),
    userId: tomlString(config, "msg_center.user_id") ?? env("BUCKYOS_TEST_USER_ID") ?? "",
    userDid: tomlString(config, "msg_center.user_did") ?? env("JARVIS_DV_USER_DID"),
    zoneDid: tomlString(config, "msg_center.zone_did") ?? env("JARVIS_DV_ZONE_DID"),
    jarvisDid: tomlString(config, "msg_center.jarvis_did") ?? env("JARVIS_DV_AGENT_DID"),
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
    parameterSources: {},
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
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--config") {
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
    } else if (arg === "--interactive-review") {
      options.interactiveReview = true;
    } else if (arg === "--no-interactive-review") {
      options.interactiveReview = false;
    } else if (arg === "--allow-review") {
      options.allowReview = true;
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
  if (options.transports.length === 0) throw new Error("at least one transport is required");
  const source = (
    cliFlags: string[],
    configKey: string,
    envNames: string[] = [],
  ) => parameterSource(args, config, loaded.path, cliFlags, configKey, envNames);
  options.parameterSources = {
    transports: source(["--transport"], "common.transports", ["JARVIS_DV_TRANSPORTS"]),
    providers: source(["--provider"], "environment.providers", ["JARVIS_DV_PROVIDERS"]),
    suite: source(["--suite"], "common.suite", ["JARVIS_DV_SUITE"]),
    cases: source(["--case"], "common.cases"),
    reportDir: source(["--report-dir"], "common.report_dir", ["JARVIS_DV_REPORT_DIR"]),
    settleMs: source(["--settle-ms"], "common.settle_ms", ["JARVIS_DV_SETTLE_MS"]),
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
    gatewayUrl: source(["--gateway-url"], "msg_center.gateway_url", ["BUCKYOS_TEST_GATEWAY_URL", "BUCKYOS_TEST_ZONE_HOST"]),
    sessionToken: source(["--session-token"], "msg_center.session_token", ["BUCKYOS_APPCLIENT_SESSION_TOKEN"]),
    username: source(["--username"], "msg_center.username", ["BUCKYOS_TEST_USERNAME"]),
    password: source(["--password"], "msg_center.password", ["BUCKYOS_TEST_PASSWORD"]),
    userId: source(["--user-id"], "msg_center.user_id", ["BUCKYOS_TEST_USER_ID"]),
    userDid: source(["--user-did"], "msg_center.user_did", ["JARVIS_DV_USER_DID"]),
    zoneDid: source(["--zone-did"], "msg_center.zone_did", ["JARVIS_DV_ZONE_DID"]),
    jarvisDid: source(["--jarvis-did"], "msg_center.jarvis_did", ["JARVIS_DV_AGENT_DID"]),
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
  --suite <smoke|linked|matrix|all>     Default: all
  --case <scenario_id>                  Repeat to select specific scenarios
  --gateway-url <url>                   Zone Gateway base URL
  --username <name>                     Prefer BUCKYOS_TEST_USERNAME
  --password <password>                 Prefer BUCKYOS_TEST_PASSWORD
  --session-token <token>               Optional login override for debugging
  --user-id <id>                        Normally read from login response
  --user-did <did>
  --zone-did <did>
  --jarvis-did <did>
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
  --image-primary-file <path>
  --image-secondary-file <path>
  --image-ocr-file <path>
  --audio-sfx-file <path>
  --audio-speech-file <path>
  --video-fresh-file <path>
  --report-dir <path>
  --settle-ms <milliseconds>
  --interactive-review                  Ask the operator to judge semantics
  --no-interactive-review
  --allow-review                        Exit 0 when only manual review remains
  --no-allow-review
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
  const zoneDid = boot.zone_name;
  if (typeof zoneDid !== "string" || !zoneDid.startsWith("did:")) {
    throw new Error("boot/config does not contain a valid zone_name DID");
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

function makeMessage(input: {
  userDid: string;
  jarvisDid: string;
  topic: string;
  prompt: string;
  traceId: string;
  replyTo?: string;
  attachment?: { asset: AssetKey; objectId: string };
}): MsgObject {
  const refs = input.attachment
    ? [assetRef(input.attachment.asset, input.attachment.objectId)]
    : undefined;
  return {
    from: input.userDid,
    to: [input.jarvisDid],
    kind: "chat",
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
  if (step.expect.artifact) {
    const matched = refs.some((ref) =>
      Boolean(ref.target.obj_id) &&
      artifactMatches(ref.label, step.expect.artifact!)
    );
    if (matched) checks.push(`received ${step.expect.artifact} artifact`);
    else failures.push(`missing ${step.expect.artifact} artifact`);
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

  while (Date.now() < deadline) {
    const all = await listSession(input.msgCenter, input.userDid, input.topic);
    const items = all.filter((item) => item.sort_key > input.afterSortKey);
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
    if (evaluation.ready) {
      if (input.step.expect.artifact || Date.now() - lastChangeAt >= input.settleMs) {
        return { items, checks: evaluation.checks };
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }

  throw new Error(
    `timed out after ${input.step.maxWaitMs ?? 180_000} ms: ${lastFailures.join("; ") || "no final reply"}`,
  );
}

function maxSortKey(items: SessionMessageItem[]): number {
  return items.reduce((max, item) => Math.max(max, item.sort_key ?? 0), 0);
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

async function runMsgCenter(
  options: CliOptions,
  scenarios: Scenario[],
  report: RunReport,
): Promise<void> {
  if (!options.sessionToken && (!options.username || !options.password)) {
    options.username ||= await promptValue("BuckyOS username");
    options.password ||= await promptValue("BuckyOS password", true);
  }
  const { buckyos } = await import("buckyos");
  let sessionToken = options.sessionToken;
  let loginUserId = "";
  if (!sessionToken) {
    const nonce = Date.now();
    const loginRpc = new buckyos.kRPCClient(
      `${options.gatewayUrl}/kapi/control-panel`,
      null,
      nonce,
    ) as RpcClient;
    const login = await loginRpc.call("auth.login", {
      username: options.username!,
      password: buckyos.hashPassword(options.username!, options.password!, nonce),
      appid: "control-panel",
      login_nonce: nonce,
    }) as PasswordLoginResponse;
    sessionToken = typeof login.session_token === "string"
      ? login.session_token.trim()
      : "";
    loginUserId = typeof login.user_info?.user_id === "string"
      ? login.user_info.user_id.trim()
      : "";
    if (!sessionToken) {
      throw new Error("auth.login succeeded without a session_token");
    }
    console.log(`[probe] password login succeeded for ${loginUserId || options.username}`);
  } else {
    console.log("[probe] using explicit session-token override");
  }
  const msgCenter = new buckyos.kRPCClient(
    `${options.gatewayUrl}/kapi/msg-center`,
    sessionToken,
  ) as RpcClient;
  const systemConfig = new buckyos.kRPCClient(
    `${options.gatewayUrl}/kapi/system_config`,
    sessionToken,
  ) as RpcClient;
  const zoneDid = options.zoneDid ?? await resolveZoneDid(systemConfig);
  const userId = options.userId || loginUserId || options.username;
  if (!userId) throw new Error("cannot resolve logged-in user id");
  const userDid = options.userDid ?? (userId.startsWith("did:") ? userId : `did:bns:${userId}`);
  const jarvisDid = options.jarvisDid ?? deriveJarvisDid(zoneDid);
  report.gateway_url = options.gatewayUrl;
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

  for (const scenario of scenarios) {
    const missing = scenario.requiredAssets.filter((asset) => !options.assets[asset]);
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
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: step.review,
          notes: `missing assets: ${missing.join(", ")}`,
        });
      }
      continue;
    }

    const topic = `jarvis-dv:${report.run_id}:${scenario.id}`;
    const sentMessageIds = new Map<string, string>();
    console.log(`\n[scenario] ${scenario.id} — ${scenario.title}`);
    const cleanTrace = `${report.run_id}:${scenario.id}:clean`;
    const beforeClean = await listSession(msgCenter, userDid, topic);
    await postMessage(
      msgCenter,
      makeMessage({ userDid, jarvisDid, topic, prompt: "/clean", traceId: cleanTrace }),
      cleanTrace,
    );
    await waitForReply({
      msgCenter,
      userDid,
      jarvisDid,
      topic,
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

    for (const step of scenario.steps) {
      const started = Date.now();
      const traceId = `${report.run_id}:${scenario.id}:${step.id}`;
      const before = await listSession(msgCenter, userDid, topic);
      const afterSortKey = maxSortKey(before);
      console.log(`[send] ${scenario.id}/${step.id}: ${step.prompt}`);
      const objectId = step.attachment ? options.assets[step.attachment] : undefined;
      const replyTo = step.replyToStep
        ? sentMessageIds.get(step.replyToStep)
        : undefined;
      if (step.replyToStep && !replyTo) {
        throw new Error(
          `${scenario.id}/${step.id} cannot resolve reply_to step ${step.replyToStep}`,
        );
      }
      const sentMsgId = await postMessage(
        msgCenter,
        makeMessage({
          userDid,
          jarvisDid,
          topic,
          prompt: step.prompt,
          traceId,
          replyTo,
          attachment: step.attachment && objectId
            ? { asset: step.attachment, objectId }
            : undefined,
        }),
        traceId,
      );
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
        let review: ReturnType<typeof reviewStep> = step.review.length
          ? { status: "review" }
          : { status: "passed" };
        if (options.interactiveReview) review = reviewStep(step);
        report.results.push({
          scenario_id: scenario.id,
          step_id: step.id,
          status: review.status,
          started_at: new Date(started).toISOString(),
          elapsed_ms: Date.now() - started,
          prompt: step.prompt,
          attachment: step.attachment,
          reply_texts: texts,
          reply_refs: refs,
          automatic_checks: waited.checks,
          review: step.review,
          notes: review.notes,
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
          reply_texts: [],
          reply_refs: [],
          automatic_checks: [],
          review: step.review,
          error: String(error),
        });
        console.error(`[failed] ${scenario.id}/${step.id}: ${String(error)}`);
      }
    }
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
      label: message.media.mimeType || message.media.fileName,
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
      if (input.step.expect.artifact || Date.now() - lastChangeAt >= input.settleMs) {
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
  try {
    for (const scenario of scenarios) {
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
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            notes: `missing Telegram asset files: ${missing.join(", ")}`,
          });
        }
        continue;
      }

      const sentMessageIds = new Map<string, number>();
      console.log(`\n[scenario] ${scenario.id} — ${scenario.title}`);
      const cleanId = await telegram.send({ text: "/clean" });
      await waitForTelegramReply({
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

      for (const step of scenario.steps) {
        const started = Date.now();
        const replyTo = step.replyToStep ? sentMessageIds.get(step.replyToStep) : undefined;
        if (step.replyToStep && !replyTo) {
          throw new Error(
            `${scenario.id}/${step.id} cannot resolve Telegram reply_to step ${step.replyToStep}`,
          );
        }
        const file = step.attachment ? options.telegramAssets[step.attachment] : undefined;
        console.log(`[telegram-send] ${scenario.id}/${step.id}: ${step.prompt}`);
        const sentMessageId = await telegram.send({
          text: step.prompt,
          file,
          replyTo,
        });
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
          const refs = telegramRefs(waited.messages);
          let review: ReturnType<typeof reviewStep> = step.review.length
            ? { status: "review" }
            : { status: "passed" };
          if (options.interactiveReview) review = reviewStep(step);
          report.results.push({
            scenario_id: scenario.id,
            step_id: step.id,
            status: review.status,
            started_at: new Date(started).toISOString(),
            elapsed_ms: Date.now() - started,
            prompt: step.prompt,
            attachment: step.attachment,
            reply_texts: texts,
            reply_refs: refs,
            automatic_checks: waited.checks,
            review: step.review,
            notes: review.notes,
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
            reply_texts: [],
            reply_refs: [],
            automatic_checks: [],
            review: step.review,
            error: String(error),
          });
          console.error(`[failed] ${scenario.id}/${step.id}: ${String(error)}`);
        }
      }
    }
  } finally {
    await telegram.disconnect();
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
  if (options.dryRun || options.assumeYes) return;
  if (options.transports.includes("msg-center") && !options.sessionToken) {
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
  if (options.assumeYes && options.interactiveReview) {
    errors.push("automated mode cannot use --interactive-review");
  }
  if (options.transports.includes("msg-center")) {
    if (!options.sessionToken && !options.username) {
      errors.push("msg-center requires --session-token or --username with --password");
    }
    if (!options.sessionToken && !options.password) {
      errors.push("msg-center requires --session-token or --username with --password");
    }
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
    for (const asset of assets) {
      console.log(`  msg-center.asset.${asset}: ${options.assets[asset] ?? "<missing; related scenarios will be skipped>"}${sourceSuffix(options, `msg:${asset}`)}`);
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
  console.log(`结果判定: interactive_review=${options.interactiveReview}${sourceSuffix(options, "interactiveReview")}; allow_review=${options.allowReview}${sourceSuffix(options, "allowReview")}; settle_ms=${options.settleMs}${sourceSuffix(options, "settleMs")}`);
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
        const supplied = options.assets[asset] ? "configured" : "missing";
        console.log(`  msg-center asset ${asset}: ${supplied} (${ASSET_ENV[asset]})`);
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

function summarize(report: RunReport): Record<StepStatus, number> {
  const totals: Record<StepStatus, number> = {
    passed: 0,
    failed: 0,
    review: 0,
    skipped: 0,
    dispatched: 0,
  };
  for (const result of report.results) totals[result.status] += 1;
  return totals;
}

async function writeReport(options: CliOptions, report: RunReport): Promise<string> {
  report.finished_at = new Date().toISOString();
  report.totals = summarize(report);
  const dir = `${options.reportDir}/${report.run_id}`;
  await Deno.mkdir(dir, { recursive: true });
  const path = `${dir}/summary.json`;
  await Deno.writeTextFile(path, `${JSON.stringify(report, null, 2)}\n`);
  return path;
}

async function main(): Promise<void> {
  const options = await parseArgs(Deno.args);
  if (options.list) {
    printScenarioList();
    return;
  }
  const scenarios = selectScenarios(options);
  await collectPreflightInputs(options);
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
    transports: options.transports,
    expected_providers: options.expectedProviders,
    suite: options.suite,
    selected_scenarios: scenarios.map((scenario) => scenario.id),
    results: [],
  };

  try {
    for (const transport of options.transports) {
      const resultStart = report.results.length;
      console.log(`\n[transport] ${transport}`);
      try {
        if (transport === "msg-center") {
          await runMsgCenter(options, scenarios, report);
        } else {
          await runTelegram(options, scenarios, report);
        }
      } catch (error) {
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
      }
    }
  } finally {
    const path = await writeReport(options, report);
    console.log(`\n[report] ${path}`);
  }

  const totals = report.totals ?? summarize(report);
  console.log(`[done] ${JSON.stringify(totals)}`);
  if (totals.failed > 0) Deno.exitCode = 1;
  else if (totals.review > 0 && !options.allowReview) Deno.exitCode = 2;
}

main().catch((error) => {
  console.error(`[fatal] ${String(error)}`);
  Deno.exitCode = 1;
});
