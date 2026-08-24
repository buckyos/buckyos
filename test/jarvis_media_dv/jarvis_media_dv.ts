import {
  ASSET_DESCRIPTION,
  ASSET_ENV,
  ASSET_FILE,
  ASSET_LABEL,
  type AssetKey,
  type Scenario,
  type ScenarioStep,
  SCENARIOS,
} from "./scenarios.ts";

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
  transport: string;
  suite: string;
  gateway_url?: string;
  user_did?: string;
  jarvis_did?: string;
  selected_scenarios: string[];
  results: StepResult[];
  totals?: Record<StepStatus, number>;
};

type CliOptions = {
  transport: "native" | "telegram-manual";
  suite: "smoke" | "linked" | "matrix" | "all";
  caseIds: string[];
  gatewayUrl: string;
  sessionToken?: string;
  username?: string;
  password?: string;
  userId: string;
  userDid?: string;
  zoneDid?: string;
  jarvisDid?: string;
  telegramBotToken?: string;
  assets: Partial<Record<AssetKey, string>>;
  interactiveReview: boolean;
  allowReview: boolean;
  dryRun: boolean;
  list: boolean;
  reportDir: string;
  settleMs: number;
};

const FLAG_TO_ASSET: Record<string, AssetKey> = {
  "image-primary-id": "image_primary",
  "image-secondary-id": "image_secondary",
  "image-ocr-id": "image_ocr",
  "audio-sfx-id": "audio_sfx",
  "audio-speech-id": "audio_speech",
  "video-fresh-id": "video_fresh",
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

function parseArgs(args: string[]): CliOptions {
  const zoneHost = env("BUCKYOS_TEST_ZONE_HOST") ?? "test.buckyos.io";
  const options: CliOptions = {
    transport: "native",
    suite: "all",
    caseIds: [],
    gatewayUrl: env("BUCKYOS_TEST_GATEWAY_URL") ?? `https://${zoneHost}`,
    sessionToken: env("BUCKYOS_APPCLIENT_SESSION_TOKEN"),
    username: env("BUCKYOS_TEST_USERNAME"),
    password: env("BUCKYOS_TEST_PASSWORD"),
    userId: env("BUCKYOS_TEST_USER_ID") ?? "",
    userDid: env("JARVIS_DV_USER_DID"),
    zoneDid: env("JARVIS_DV_ZONE_DID"),
    jarvisDid: env("JARVIS_DV_AGENT_DID"),
    telegramBotToken: env("JARVIS_TELEGRAM_BOT_TOKEN"),
    assets: {},
    interactiveReview: false,
    allowReview: false,
    dryRun: false,
    list: false,
    reportDir: env("JARVIS_DV_REPORT_DIR") ?? "reports/jarvis_media_dv",
    settleMs: parsePositiveInt(env("JARVIS_DV_SETTLE_MS"), 10_000, "JARVIS_DV_SETTLE_MS"),
  };

  for (const asset of Object.keys(ASSET_ENV) as AssetKey[]) {
    const value = env(ASSET_ENV[asset]);
    if (value) options.assets[asset] = value;
  }

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--transport") {
      const value = requiredValue(args, index, arg);
      if (value !== "native" && value !== "telegram-manual") {
        throw new Error("--transport must be native or telegram-manual");
      }
      options.transport = value;
      index += 1;
    } else if (arg === "--suite") {
      const value = requiredValue(args, index, arg);
      if (value !== "smoke" && value !== "linked" && value !== "matrix" && value !== "all") {
        throw new Error("--suite must be smoke, linked, matrix, or all");
      }
      options.suite = value;
      index += 1;
    } else if (arg === "--case") {
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
    } else if (arg === "--telegram-bot-token") {
      options.telegramBotToken = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--report-dir") {
      options.reportDir = requiredValue(args, index, arg);
      index += 1;
    } else if (arg === "--settle-ms") {
      options.settleMs = parsePositiveInt(requiredValue(args, index, arg), 10_000, arg);
      index += 1;
    } else if (arg === "--interactive-review") {
      options.interactiveReview = true;
    } else if (arg === "--allow-review") {
      options.allowReview = true;
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
      if (!asset) throw new Error(`unknown option: ${arg}`);
      options.assets[asset] = requiredValue(args, index, arg);
      index += 1;
    } else {
      throw new Error(`unexpected argument: ${arg}`);
    }
  }

  options.gatewayUrl = options.gatewayUrl.replace(/\/+$/, "");
  return options;
}

function printUsage(): void {
  console.log(`Usage:
  deno task test -- [options]

Options:
  --transport <native|telegram-manual>  Default: native
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
  --telegram-bot-token <token>           Prefer JARVIS_TELEGRAM_BOT_TOKEN
  --image-primary-id <obj_id>
  --image-secondary-id <obj_id>
  --image-ocr-id <obj_id>
  --audio-sfx-id <obj_id>
  --audio-speech-id <obj_id>
  --video-fresh-id <obj_id>
  --interactive-review                   Ask the operator to judge semantics
  --allow-review                         Exit 0 when only manual review remains
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
      ref.target?.type === "data_obj" &&
      Boolean(ref.target.obj_id) &&
      artifactMatches(ref.label, step.expect.artifact!)
    );
    if (matched) checks.push(`received ${step.expect.artifact} named-object artifact`);
    else failures.push(`missing ${step.expect.artifact} named-object artifact`);
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

async function runNative(
  options: CliOptions,
  scenarios: Scenario[],
  report: RunReport,
): Promise<void> {
  if (!options.sessionToken && (!options.username || !options.password)) {
    throw new Error(
      "native transport requires username/password via arguments or BUCKYOS_TEST_USERNAME and BUCKYOS_TEST_PASSWORD",
    );
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

async function validateTelegramToken(token: string): Promise<string> {
  const response = await fetch(`https://api.telegram.org/bot${token}/getMe`);
  const body = await response.json() as JsonObject;
  if (!response.ok || body.ok !== true || !isObject(body.result)) {
    throw new Error("Telegram Bot API getMe failed; check the supplied token");
  }
  const username = body.result.username;
  return typeof username === "string" ? `@${username}` : "configured bot";
}

async function runTelegramManual(
  options: CliOptions,
  scenarios: Scenario[],
  report: RunReport,
): Promise<void> {
  if (!options.telegramBotToken) {
    throw new Error(
      "telegram-manual requires --telegram-bot-token or JARVIS_TELEGRAM_BOT_TOKEN",
    );
  }
  const botName = await validateTelegramToken(options.telegramBotToken);
  console.log(`[probe] Telegram bot token is valid for ${botName}`);
  console.log("[manual] Bot API cannot impersonate a user. Send each step from the owner account in the real Jarvis chat.");

  for (const scenario of scenarios) {
    console.log(`\n[scenario] ${scenario.id} — ${scenario.title}`);
    console.log(scenario.purpose);
    console.log("在 Telegram 中发送 /clean，然后等待新会话确认。按 Enter 继续。");
    globalThis.prompt("");
    for (const step of scenario.steps) {
      const started = Date.now();
      console.log(`\n[step] ${scenario.id}/${step.id}`);
      if (step.attachment) {
        console.log(`附件：${ASSET_DESCRIPTION[step.attachment]}`);
        console.log(`仓库文件：${ASSET_FILE[step.attachment]}`);
      }
      if (step.replyToStep) {
        console.log(`消息引用：请在 Telegram 中使用“回复”功能引用步骤 ${step.replyToStep} 的用户消息。`);
      }
      console.log(`指令：${step.prompt}`);
      console.log(`等待上限：${Math.ceil((step.maxWaitMs ?? 180_000) / 1000)} 秒`);
      globalThis.prompt("发送并等待最终回复后按 Enter：");
      const review = reviewStep(step);
      report.results.push({
        scenario_id: scenario.id,
        step_id: step.id,
        status: review.status,
        started_at: new Date(started).toISOString(),
        elapsed_ms: Date.now() - started,
        prompt: step.prompt,
        attachment: step.attachment,
        reply_texts: [],
        reply_refs: [],
        automatic_checks: ["executed through real Telegram tunnel by owner"],
        review: step.review,
        notes: review.notes,
      });
    }
  }
}

function printDryRun(options: CliOptions, scenarios: Scenario[]): void {
  console.log(`[dry-run] transport=${options.transport} suite=${options.suite}`);
  for (const scenario of scenarios) {
    console.log(`\n${scenario.id}: ${scenario.title}`);
    for (const asset of scenario.requiredAssets) {
      const supplied = options.assets[asset] ? "configured" : "missing";
      console.log(`  asset ${asset}: ${supplied} (${ASSET_ENV[asset]}; ${ASSET_FILE[asset]})`);
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
  const options = parseArgs(Deno.args);
  if (options.list) {
    printScenarioList();
    return;
  }
  const scenarios = selectScenarios(options);
  if (options.dryRun) {
    printDryRun(options, scenarios);
    return;
  }

  const report: RunReport = {
    run_id: `${Date.now()}-${crypto.randomUUID().slice(0, 8)}`,
    started_at: new Date().toISOString(),
    transport: options.transport,
    suite: options.suite,
    selected_scenarios: scenarios.map((scenario) => scenario.id),
    results: [],
  };

  try {
    if (options.transport === "native") {
      await runNative(options, scenarios, report);
    } else {
      await runTelegramManual(options, scenarios, report);
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
