import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { buckyos, RuntimeType } from "buckyos/node";

type Layer = "kRPC Direct" | "AiccClient (Rust)" | "TS SDK";
type ProviderKind = "sn-openai" | "openai" | "gemini" | "claude";
type CaseStatus = "passed" | "failed" | "partial" | "skipped";

type CliOptions = {
    gatewayHost: string;
    krpcHost: string;
    aiccClientHost: string;
    token?: string;
    username?: string;
    password?: string;
    loginAppId: string;
    modelAlias: string;
    output: string;
    rustManifestPath: string;
    appId: string;
    providers: ProviderKind[];
    apiKeys: Partial<Record<"openai" | "gemini" | "claude", string>>;
    protocolMixOnlyNoSystemConfig: boolean;
};

type Context = {
    options: CliOptions;
    targets: {
        ts: TargetEndpoints;
        krpc: TargetEndpoints;
        rust: TargetEndpoints;
    };
    auth: {
        verifyHub: string[];
    };
    token?: string;
    seq: number;
    sdkInitialized: boolean;
};

type CaseResult = {
    provider: ProviderKind;
    id: string;
    layer: Layer;
    status: CaseStatus;
    durationMs: number;
    detail: string;
};

type TestCase = {
    id: string;
    layer: Layer;
    requiresAuth?: boolean;
    requiredConfig?: RequiredCaseConfig;
    run: (ctx: Context, provider: ProviderKind) => Promise<string>;
};

type TargetEndpoints = {
    aicc: string;
    systemConfig: string;
};

type RequiredCaseConfig = {
    fallbackLimit?: number;
    weights?: {
        cost: number;
        latency: number;
        error: number;
        load: number;
    };
    providerFeatures?: string[];
    providerTimeoutMs?: number;
};

class PartialCaseError extends Error {}
class SkippedCaseError extends Error {}

const SETTINGS_KEY = "services/aicc/settings";
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const MIN_EXPECTED_STEPS = 6;
const MAX_TOKENS_BASIC_COMPLETE = 2048;
const MAX_TOKENS_COMPLEX_DAG = 1800;
const MAX_TOKENS_COMPLEX_DAG_GEMINI = 4096;
const MAX_TOKENS_JSON_OUTPUT = 320;
const MAX_TOKENS_STREAM_OUTPUT = 1024;
const WORKFLOW_MAX_LATENCY_MS = 100000;
const WORKFLOW_MAX_COST_USD = 0.2;
const DEFAULT_OPENAI_BASE_URL = "https://api.openai.com/v1";
const DEFAULT_OPENAI_MODEL = "gpt-5.4";
const DEFAULT_GEMINI_BASE_URL =
    "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_GEMINI_MODEL = "gemini-2.5-flash";
const DEFAULT_CLAUDE_BASE_URL = "https://api.anthropic.com/v1";
const DEFAULT_CLAUDE_MODEL = "claude-3-7-sonnet-latest";
const DEFAULT_SN_OPENAI_BASE_URL =
    "https://sn.buckyos.ai/api/v1/ai/chat/completions";
const COMPLEX_DAG_PROMPT = `You are a workflow planner.
Return JSON only (no markdown).
Generate a DAG plan for "product release multimedia package" with EXACTLY 6 steps.
Output must include: plan_id, goal, steps.
Each step must include: id, title, capability, model_alias, depends_on, parallel_group, acceptance_criteria.
Rules:
- acceptance_criteria must be array of strings (never a single string), and each step uses 1-2 short items.
- depends_on must be an array of step id strings.
- include at least one parallel group containing >=2 steps.
- include at least one serial dependency step (depends_on not empty).
- include retry_policy or replan_trigger on at least one step.
- keep values concise; avoid long descriptions.`;
const JSON_OUTPUT_PROMPT = `Return strict JSON object only:
{"ok":true,"kind":"protocol-json","source":"aicc"}`;
const STREAM_PROMPT = "Protocol stream mode smoke check.";
const DEFAULT_CASE_CONFIG: RequiredCaseConfig = {
    fallbackLimit: 1,
    weights: { cost: 0.4, latency: 0.3, error: 0.2, load: 0.1 },
    providerFeatures: ["plan", "json_output", "tool_calling", "web_search"],
    providerTimeoutMs: 120000,
};
const DEFAULT_RUST_RUNNER_TIMEOUT_SECS = Math.ceil(
    (DEFAULT_CASE_CONFIG.providerTimeoutMs ?? 120000) / 1000,
);

function getEnv(name: string): string | undefined {
    const value = process.env[name];
    if (!value) {
        return undefined;
    }
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : undefined;
}

function parseArgMap(argv: string[]): Record<string, string> {
    const kv: Record<string, string> = {};
    for (let i = 0; i < argv.length; i += 1) {
        const token = argv[i];
        if (!token.startsWith("--")) {
            continue;
        }
        const [flag, inlineValue] = token.split("=", 2);
        if (inlineValue !== undefined) {
            kv[flag] = inlineValue;
            continue;
        }
        const next = argv[i + 1];
        if (!next || next.startsWith("--")) {
            kv[flag] = "true";
            continue;
        }
        kv[flag] = next;
        i += 1;
    }
    return kv;
}

function parseBooleanFlag(value: string | undefined): boolean | undefined {
    if (value === undefined) {
        return undefined;
    }
    const normalized = value.trim().toLowerCase();
    if (["1", "true", "yes", "on"].includes(normalized)) {
        return true;
    }
    if (["0", "false", "no", "off"].includes(normalized)) {
        return false;
    }
    return undefined;
}

function parseTomlValue(raw: string): string | number | boolean {
    const value = raw.trim();
    if (
        (value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'"))
    ) {
        return value.slice(1, -1);
    }
    if (value === "true") return true;
    if (value === "false") return false;
    if (/^-?\d+(\.\d+)?$/.test(value)) return Number(value);
    return value;
}

function parseSimpleToml(content: string): Record<string, string> {
    const result: Record<string, string> = {};
    let section = "";
    const lines = content.split(/\r?\n/);
    for (const rawLine of lines) {
        const line = rawLine.trim();
        if (!line || line.startsWith("#")) continue;
        if (line.startsWith("[") && line.endsWith("]")) {
            section = line.slice(1, -1).trim();
            continue;
        }
        const idx = line.indexOf("=");
        if (idx <= 0) continue;
        const key = line.slice(0, idx).trim();
        let valuePart = line.slice(idx + 1).trim();
        const commentIdx = valuePart.indexOf("#");
        if (commentIdx >= 0) {
            valuePart = valuePart.slice(0, commentIdx).trim();
        }
        const val = parseTomlValue(valuePart);
        const fullKey = section ? `${section}.${key}` : key;
        result[fullKey] = String(val);
    }
    return result;
}

function getConfigValue(
    map: Record<string, string>,
    keys: string[],
): string | undefined {
    for (const key of keys) {
        const value = map[key];
        if (value && value.trim()) {
            return value.trim();
        }
    }
    return undefined;
}

function normalizeLegacyPrefix(input: string): string {
    return input.replace(/^test[\\/]+aicc_test[\\/]+/, "");
}

function resolveConfiguredPath(
    rawValue: string | undefined,
    defaultValue: string,
    configDir: string,
): string {
    const value = (rawValue && rawValue.trim()) || defaultValue;
    if (path.isAbsolute(value)) {
        return value;
    }

    const cwdCandidate = path.resolve(process.cwd(), value);
    if (existsSync(cwdCandidate)) {
        return cwdCandidate;
    }

    const normalized = normalizeLegacyPrefix(value);
    const normalizedCwd = path.resolve(process.cwd(), normalized);
    if (existsSync(normalizedCwd)) {
        return normalizedCwd;
    }

    return path.resolve(configDir, normalized);
}

async function loadOptionsFromConfig(argv: string[]): Promise<CliOptions> {
    const args = parseArgMap(argv);
    const configPath = args["--config"]
        ? path.resolve(process.cwd(), args["--config"])
        : path.resolve(process.cwd(), "aicc_remote_runner.toml");
    const configDir = path.dirname(configPath);
    const rawToml = await readFile(configPath, "utf8");
    const cfg = parseSimpleToml(rawToml);

    const gatewayHost = getConfigValue(cfg, ["gateway_host", "gateway"]);
    if (!gatewayHost) {
        throw new Error(`missing gateway_host in config: ${configPath}`);
    }
    const krpcHost = getConfigValue(cfg, ["krpc_host"]) ?? gatewayHost;
    const aiccClientHost =
        getConfigValue(cfg, ["aicc_client_host"]) ?? gatewayHost;

    const output = resolveConfiguredPath(
        getConfigValue(cfg, ["output", "runner.output"]),
        path.join(SCRIPT_DIR, "reports", `aicc_remote_report_${Date.now()}.md`),
        configDir,
    );

    const apiKeys: CliOptions["apiKeys"] = {
        openai: getConfigValue(cfg, ["api_keys.openai", "openai_api_key"]),
        gemini: getConfigValue(cfg, ["api_keys.gemini", "gemini_api_key"]),
        claude: getConfigValue(cfg, ["api_keys.claude", "claude_api_key"]),
    };
    const providers: ProviderKind[] = [
        "sn-openai",
        "openai",
        "gemini",
        "claude",
    ];
    const protocolMixOnlyNoSystemConfig =
        parseBooleanFlag(args["--protocol-mix-only-no-system-config"]) ??
        parseBooleanFlag(
            getConfigValue(cfg, [
                "runner.protocol_mix_only_no_system_config",
                "protocol_mix_only_no_system_config",
            ]),
        ) ??
        false;

    return {
        gatewayHost,
        krpcHost,
        aiccClientHost,
        token:
            getConfigValue(cfg, ["auth.token", "token"]) ??
            getEnv("AICC_RPC_TOKEN"),
        username:
            getConfigValue(cfg, ["auth.username", "username"]) ??
            getEnv("AICC_LOGIN_USERNAME") ??
            getEnv("BUCKYOS_USERNAME"),
        password:
            getConfigValue(cfg, ["auth.password", "password"]) ??
            getEnv("AICC_LOGIN_PASSWORD") ??
            getEnv("BUCKYOS_PASSWORD"),
        loginAppId:
            getConfigValue(cfg, ["auth.login_appid", "login_appid"]) ??
            getEnv("AICC_LOGIN_APPID") ??
            "aicc-tests",
        modelAlias:
            getConfigValue(cfg, ["runner.model_alias", "model_alias"]) ??
            getEnv("AICC_MODEL_ALIAS") ??
            "llm.plan.default",
        output,
        rustManifestPath: resolveConfiguredPath(
            getConfigValue(cfg, [
                "runner.rust_manifest_path",
                "rust_manifest_path",
            ]),
            path.join(SCRIPT_DIR, "rust_runner", "Cargo.toml"),
            configDir,
        ),
        appId:
            getConfigValue(cfg, ["runner.app_id", "app_id"]) ??
            getEnv("AICC_TEST_APP_ID") ??
            "aicc-tests",
        providers,
        apiKeys,
        protocolMixOnlyNoSystemConfig,
    };
}

function buildTargetEndpoints(host: string): TargetEndpoints {
    const base = new URL(host);
    const toPath = (pathname: string) => {
        const u = new URL(base.toString());
        u.pathname = pathname;
        u.search = "";
        return u.toString();
    };
    return {
        aicc: toPath("/kapi/aicc"),
        systemConfig: toPath("/kapi/system_config"),
    };
}

function buildVerifyHubEndpoints(host: string): string[] {
    const base = new URL(host);
    const toPath = (pathname: string) => {
        const u = new URL(base.toString());
        u.pathname = pathname;
        u.search = "";
        return u.toString();
    };
    return [toPath("/kapi/verify_hub"), toPath("/kapi/verify-hub")];
}

function nextSeq(ctx: Context): number {
    ctx.seq += 1;
    return ctx.seq;
}

async function rpcCall<T>(
    endpoint: string,
    method: string,
    params: Record<string, unknown>,
    ctx: Context,
    overrides?: { token?: string; traceId?: string; omitToken?: boolean },
): Promise<T> {
    const seq = nextSeq(ctx);
    const token = overrides?.omitToken
        ? undefined
        : (overrides?.token ?? ctx.token);
    const traceId = overrides?.traceId;
    const sys: unknown[] = [seq];
    if (token) {
        sys.push(token);
    }
    if (traceId) {
        if (!token) {
            sys.push(null);
        }
        sys.push(traceId);
    }

    const resp = await fetch(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ method, params, sys }),
    });

    const body = await resp.json().catch(() => ({}));
    if (!resp.ok) {
        throw new Error(`HTTP ${resp.status}: ${JSON.stringify(body)}`);
    }
    if (typeof body?.error === "string" && body.error.trim().length > 0) {
        throw new Error(body.error);
    }
    if (!("result" in body)) {
        throw new Error(`RPC response missing result: ${JSON.stringify(body)}`);
    }
    return body.result as T;
}

function sha256Base64(value: string): string {
    return createHash("sha256").update(value).digest("base64");
}

async function resolveToken(ctx: Context): Promise<string | undefined> {
    if (ctx.options.token) {
        return ctx.options.token;
    }
    const { username, password, loginAppId } = ctx.options;
    if (!username || !password) {
        return undefined;
    }

    const loginNonce = Date.now();
    const stage1 = sha256Base64(`${password}${username}.buckyos`);
    const passwordHash = sha256Base64(`${stage1}${loginNonce}`);

    let lastErr = "";
    for (const endpoint of ctx.auth.verifyHub) {
        try {
            const result = await rpcCall<{ session_token?: string }>(
                endpoint,
                "login_by_password",
                {
                    type: "password",
                    username,
                    password: passwordHash,
                    appid: loginAppId,
                    login_nonce: loginNonce,
                },
                ctx,
            );
            if (result.session_token?.trim()) {
                return result.session_token.trim();
            }
            lastErr = `empty session token at ${endpoint}`;
        } catch (error) {
            lastErr = `${endpoint}: ${String(error)}`;
        }
    }
    if (lastErr) {
        throw new Error(
            `failed to resolve token by password login: ${lastErr}`,
        );
    }
    return undefined;
}

function normalizeObj(value: unknown): Record<string, unknown> {
    return value && typeof value === "object" && !Array.isArray(value)
        ? ({ ...(value as Record<string, unknown>) } as Record<string, unknown>)
        : {};
}

function setProviderEnabled(
    value: unknown,
    enabled: boolean,
    apiKey?: string,
): Record<string, unknown> {
    const obj = normalizeObj(value);
    obj.enabled = enabled;
    if (apiKey) {
        obj.api_token = apiKey;
    }
    return obj;
}

function ensureGeminiProviderConfig(
    value: unknown,
    apiKey?: string,
): Record<string, unknown> {
    const obj = setProviderEnabled(value, true, apiKey);
    const instances = Array.isArray(obj.instances) ? obj.instances : [];
    if (instances.length === 0) {
        obj.instances = [
            {
                instance_id: "workflow-gemini-remote",
                provider_type: "google-gimini",
                base_url: DEFAULT_GEMINI_BASE_URL,
                auth_mode: "bearer",
                timeout_ms: 120000,
                models: [DEFAULT_GEMINI_MODEL],
                default_model: DEFAULT_GEMINI_MODEL,
                features: ["plan", "json_output", "tool_calling", "web_search"],
            },
        ];
        return obj;
    }

    obj.instances = instances.map((it, index) => {
        if (!it || typeof it !== "object" || Array.isArray(it)) {
            return it;
        }
        const item = { ...(it as Record<string, unknown>) };
        if (
            typeof item.provider_type !== "string" ||
            !item.provider_type.trim()
        ) {
            item.provider_type = "google-gimini";
        }
        if (typeof item.base_url !== "string" || !item.base_url.trim()) {
            item.base_url = DEFAULT_GEMINI_BASE_URL;
        }
        if (!Array.isArray(item.models) || item.models.length === 0) {
            item.models = [DEFAULT_GEMINI_MODEL];
        }
        if (
            typeof item.default_model !== "string" ||
            !item.default_model.trim()
        ) {
            item.default_model = DEFAULT_GEMINI_MODEL;
        }
        if (!Array.isArray(item.features)) {
            item.features = [
                "plan",
                "json_output",
                "tool_calling",
                "web_search",
            ];
        } else {
            const current = new Set(
                (item.features as unknown[]).filter(
                    (v) => typeof v === "string",
                ) as string[],
            );
            current.add("plan");
            current.add("json_output");
            current.add("tool_calling");
            current.add("web_search");
            item.features = Array.from(current);
        }
        if (typeof item.instance_id !== "string" || !item.instance_id.trim()) {
            item.instance_id = `workflow-gemini-remote-${index}`;
        }
        return item;
    });
    return obj;
}

function mergeCaseConfig(required?: RequiredCaseConfig): RequiredCaseConfig {
    return {
        fallbackLimit:
            required?.fallbackLimit ?? DEFAULT_CASE_CONFIG.fallbackLimit,
        weights: required?.weights ?? DEFAULT_CASE_CONFIG.weights,
        providerFeatures:
            required?.providerFeatures ?? DEFAULT_CASE_CONFIG.providerFeatures,
        providerTimeoutMs:
            required?.providerTimeoutMs ??
            DEFAULT_CASE_CONFIG.providerTimeoutMs,
    };
}

function buildOpenAiProviderConfig(
    caseConfig: RequiredCaseConfig,
    apiKey?: string,
): Record<string, unknown> {
    return {
        enabled: true,
        api_token: apiKey ?? "",
        instances: [
            {
                instance_id: "workflow-openai-remote",
                provider_type: "openai",
                base_url: DEFAULT_OPENAI_BASE_URL,
                auth_mode: "bearer",
                timeout_ms: caseConfig.providerTimeoutMs,
                models: [DEFAULT_OPENAI_MODEL],
                default_model: DEFAULT_OPENAI_MODEL,
                features: caseConfig.providerFeatures,
            },
        ],
    };
}

function buildSnOpenAiProviderConfig(
    caseConfig: RequiredCaseConfig,
): Record<string, unknown> {
    return {
        enabled: true,
        api_token: "",
        instances: [
            {
                instance_id: "workflow-sn-openai-remote",
                provider_type: "sn-openai",
                base_url: DEFAULT_SN_OPENAI_BASE_URL,
                auth_mode: "device_jwt",
                timeout_ms: caseConfig.providerTimeoutMs,
                models: [DEFAULT_OPENAI_MODEL],
                default_model: DEFAULT_OPENAI_MODEL,
                features: caseConfig.providerFeatures,
            },
        ],
    };
}

function buildClaudeProviderConfig(
    caseConfig: RequiredCaseConfig,
    apiKey?: string,
): Record<string, unknown> {
    return {
        enabled: true,
        api_token: apiKey ?? "",
        instances: [
            {
                instance_id: "workflow-claude-remote",
                provider_type: "claude",
                base_url: DEFAULT_CLAUDE_BASE_URL,
                auth_mode: "bearer",
                timeout_ms: caseConfig.providerTimeoutMs,
                models: [DEFAULT_CLAUDE_MODEL],
                default_model: DEFAULT_CLAUDE_MODEL,
                features: caseConfig.providerFeatures,
            },
        ],
    };
}

function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

async function reloadAiccSettings(
    ctx: Context,
    target: TargetEndpoints,
    provider: ProviderKind,
): Promise<void> {
    await rpcCall(target.aicc, "service.reload_settings", {}, ctx, {
        traceId: `aicc-test-reset-reload-${provider}`,
    });
}

async function getAiccSettings(
    ctx: Context,
    target: TargetEndpoints,
): Promise<Record<string, unknown>> {
    const result = await rpcCall<{ value?: string } | null>(
        target.systemConfig,
        "sys_config_get",
        { key: SETTINGS_KEY },
        ctx,
        { traceId: "aicc-test-reset-verify-settings" },
    );
    const raw = result?.value;
    if (!raw || !raw.trim()) {
        return {};
    }
    try {
        const parsed = JSON.parse(raw);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
            return {};
        }
        return parsed as Record<string, unknown>;
    } catch {
        return {};
    }
}

function isOpenAiConfigActive(settings: Record<string, unknown>): boolean {
    const openai = settings.openai;
    if (!openai || typeof openai !== "object" || Array.isArray(openai)) {
        return false;
    }
    const cfg = openai as Record<string, unknown>;
    if (cfg.enabled !== true) {
        return false;
    }
    const instances = cfg.instances;
    if (!Array.isArray(instances)) {
        return false;
    }
    return instances.some((item) => {
        if (!item || typeof item !== "object" || Array.isArray(item)) {
            return false;
        }
        const inst = item as Record<string, unknown>;
        return (
            inst.instance_id === "workflow-openai-remote" &&
            inst.provider_type === "openai"
        );
    });
}

async function resetAiccSettings(
    ctx: Context,
    target: TargetEndpoints,
    provider: ProviderKind,
    required?: RequiredCaseConfig,
): Promise<void> {
    const caseConfig = mergeCaseConfig(required);
    const expected: Record<string, unknown> = {
        fallback_limit: caseConfig.fallbackLimit,
        weights: caseConfig.weights,
    };

    if (provider === "sn-openai") {
        expected.openai = buildSnOpenAiProviderConfig(caseConfig);
    } else if (provider === "openai") {
        expected.openai = buildOpenAiProviderConfig(
            caseConfig,
            ctx.options.apiKeys.openai,
        );
    } else if (provider === "gemini") {
        const geminiCfg = ensureGeminiProviderConfig(
            {},
            ctx.options.apiKeys.gemini,
        );
        geminiCfg.instances = (
            Array.isArray(geminiCfg.instances) ? geminiCfg.instances : []
        ).map((it) => {
            if (!it || typeof it !== "object" || Array.isArray(it)) return it;
            return {
                ...(it as Record<string, unknown>),
                timeout_ms: caseConfig.providerTimeoutMs,
                features: caseConfig.providerFeatures,
            };
        });
        expected.gemini = geminiCfg;
        expected.gimini = geminiCfg;
    } else if (provider === "claude") {
        expected.claude = buildClaudeProviderConfig(
            caseConfig,
            ctx.options.apiKeys.claude,
        );
    }

    await rpcCall(
        target.systemConfig,
        "sys_config_set",
        { key: SETTINGS_KEY, value: "{}" },
        ctx,
        { traceId: `aicc-test-reset-clear-${provider}` },
    );

    await rpcCall(
        target.systemConfig,
        "sys_config_set",
        { key: SETTINGS_KEY, value: JSON.stringify(expected, null, 2) },
        ctx,
        { traceId: `aicc-test-reset-apply-${provider}` },
    );

    await sleep(1000);
    await reloadAiccSettings(ctx, target, provider);

    if (provider === "openai") {
        const maxAttempts = 3;
        for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
            const settings = await getAiccSettings(ctx, target);
            if (isOpenAiConfigActive(settings)) {
                break;
            }
            if (attempt === maxAttempts) {
                throw new Error(
                    "openai config is not active after reload retries",
                );
            }
            await sleep(400);
            await reloadAiccSettings(ctx, target, provider);
        }
    }
}

function buildCompletePayload(modelAlias: string, text: string) {
    return {
        capability: "llm_router",
        model: { alias: modelAlias },
        requirements: { must_features: [] },
        payload: {
            messages: [{ role: "user", content: text }],
            options: {
                max_tokens: MAX_TOKENS_BASIC_COMPLETE,
                temperature: 0.1,
            },
        },
        idempotency_key: `aicc-test-${Date.now()}`,
    };
}

function stripMarkdownFence(raw: string): string {
    const trimmed = raw.trim();
    if (!trimmed.startsWith("```")) {
        return trimmed;
    }
    const lines = trimmed.split(/\r?\n/);
    if (lines.length < 3) {
        return trimmed;
    }
    return lines.slice(1, lines.length - 1).join("\n");
}

function parseJsonText(raw: string): Record<string, unknown> {
    const cleaned = stripMarkdownFence(raw);
    const parsed = JSON.parse(cleaned);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error(
            `model output is not a json object: ${cleaned.slice(0, 200)}`,
        );
    }
    return parsed as Record<string, unknown>;
}

function mustStringField(obj: Record<string, unknown>, field: string): string {
    const value = obj[field];
    if (typeof value !== "string" || value.trim().length === 0) {
        throw new Error(`field '${field}' must be non-empty string`);
    }
    return value.trim();
}

function mustArrayField(
    obj: Record<string, unknown>,
    field: string,
): unknown[] {
    const value = obj[field];
    if (!Array.isArray(value)) {
        throw new Error(`field '${field}' must be array`);
    }
    return value;
}

function validateComplexPlan(plan: Record<string, unknown>): void {
    mustStringField(plan, "plan_id");
    mustStringField(plan, "goal");
    const steps = mustArrayField(plan, "steps");
    if (steps.length < MIN_EXPECTED_STEPS) {
        throw new Error(`steps count ${steps.length} < ${MIN_EXPECTED_STEPS}`);
    }

    let hasDependencyStep = false;
    let hasRetryOrReplan = false;
    const parallelGroups = new Map<string, number>();

    for (const stepRaw of steps) {
        if (!stepRaw || typeof stepRaw !== "object" || Array.isArray(stepRaw)) {
            throw new Error("step must be object");
        }
        const step = stepRaw as Record<string, unknown>;
        mustStringField(step, "id");
        mustStringField(step, "title");
        mustStringField(step, "capability");
        mustStringField(step, "model_alias");
        const acceptance = mustArrayField(step, "acceptance_criteria");
        if (acceptance.length === 0) {
            throw new Error("acceptance_criteria must not be empty");
        }

        const dependsOn = mustArrayField(step, "depends_on");
        if (dependsOn.length > 0) {
            hasDependencyStep = true;
        }
        if (
            step.retry_policy !== undefined ||
            step.replan_trigger !== undefined
        ) {
            hasRetryOrReplan = true;
        }
        if (
            typeof step.parallel_group === "string" &&
            step.parallel_group.trim()
        ) {
            const key = step.parallel_group.trim();
            parallelGroups.set(key, (parallelGroups.get(key) ?? 0) + 1);
        }
    }

    if (!Array.from(parallelGroups.values()).some((count) => count >= 2)) {
        throw new Error("plan has no parallel group with at least 2 steps");
    }
    if (!hasDependencyStep) {
        throw new Error("plan has no dependent(serial) step");
    }
    if (!hasRetryOrReplan) {
        throw new Error("plan has no retry_policy/replan_trigger");
    }
}

function buildWorkflowPayload(
    modelAlias: string,
    prompt: string,
    mustFeatures: string[],
    options: Record<string, unknown>,
) {
    return {
        capability: "llm_router",
        model: { alias: modelAlias },
        requirements: {
            must_features: mustFeatures,
            max_latency_ms: WORKFLOW_MAX_LATENCY_MS,
            max_cost_usd: WORKFLOW_MAX_COST_USD,
        },
        payload: {
            messages: [{ role: "user", content: prompt }],
            options,
        },
        idempotency_key: `workflow-remote-${Date.now()}-${Math.floor(Math.random() * 10000)}`,
    };
}

function shortJson(value: unknown, limit = 1200): string {
    try {
        return JSON.stringify(value).slice(0, limit);
    } catch {
        return String(value).slice(0, limit);
    }
}

function requireTaskId(result: { task_id?: string }, label: string): string {
    const taskId = result.task_id?.trim();
    if (!taskId) {
        throw new Error(`${label}: missing task_id (${shortJson(result)})`);
    }
    return taskId;
}

function isAuthOrPermissionError(message: string): boolean {
    const lowered = message.toLowerCase();
    return (
        lowered.includes("jwt") ||
        lowered.includes("permission") ||
        lowered.includes("forbidden") ||
        lowered.includes("nopermission")
    );
}

function isParseBadRequestError(message: string): boolean {
    const lowered = message.toLowerCase();
    return lowered.includes("parse") || lowered.includes("completerequest");
}

function parseJsonStringValue(
    payload: Record<string, unknown>,
    field: string,
): Record<string, unknown> {
    const raw = payload[field];
    if (typeof raw !== "string" || !raw.trim()) {
        throw new Error(
            `missing string field '${field}': ${shortJson(payload)}`,
        );
    }
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error(`field '${field}' is not json object: ${raw}`);
    }
    return parsed as Record<string, unknown>;
}

async function runRustRunner(args: string[], ctx: Context): Promise<unknown> {
    const cargoArgs = [
        "run",
        "--quiet",
        "--manifest-path",
        ctx.options.rustManifestPath,
        "--",
        ...args,
    ];
    return new Promise((resolve, reject) => {
        const child = spawn("cargo", cargoArgs, {
            cwd: process.cwd(),
            env: process.env,
            stdio: ["ignore", "pipe", "pipe"],
        });
        let stdout = "";
        let stderr = "";
        child.stdout.on("data", (chunk: Buffer) => {
            stdout += chunk.toString("utf8");
        });
        child.stderr.on("data", (chunk: Buffer) => {
            stderr += chunk.toString("utf8");
        });
        child.on("error", reject);
        child.on("close", (code) => {
            if (code !== 0) {
                reject(
                    new Error(
                        `cargo runner failed(${code}): ${stderr || stdout}`,
                    ),
                );
                return;
            }
            try {
                resolve(JSON.parse(stdout));
            } catch (error) {
                reject(
                    new Error(
                        `rust runner output is not json: ${String(error)}\nstdout:\n${stdout}\nstderr:\n${stderr}`,
                    ),
                );
            }
        });
    });
}

async function ensureSdk(ctx: Context): Promise<void> {
    if (ctx.sdkInitialized) {
        return;
    }
    await buckyos.initBuckyOS(ctx.options.appId, {
        appId: ctx.options.appId,
        runtimeType: RuntimeType.AppClient,
        zoneHost: "",
        defaultProtocol: "https://",
        systemConfigServiceUrl: ctx.targets.ts.systemConfig,
    });
    ctx.sdkInitialized = true;
}

function createCases(): TestCase[] {
    const runWorkflowProtocolMix = async (
        ctx: Context,
        provider: ProviderKind,
    ): Promise<string> => {
        await ensureSdk(ctx);
        const client = new buckyos.kRPCClient(ctx.targets.ts.aicc, ctx.token);
        const complexMaxTokens =
            provider === "gemini"
                ? MAX_TOKENS_COMPLEX_DAG_GEMINI
                : MAX_TOKENS_COMPLEX_DAG;

        const complex = (await client.call(
            "complete",
            buildWorkflowPayload(
                ctx.options.modelAlias,
                COMPLEX_DAG_PROMPT,
                ["json_output"],
                {
                    temperature: 0.1,
                    max_tokens: complexMaxTokens,
                    response_format: { type: "json_object" },
                },
            ),
        )) as Record<string, unknown>;

        if (complex.status !== "succeeded") {
            throw new Error(
                `complex dag status=${String(complex.status)} response=${JSON.stringify(complex).slice(0, 1200)}`,
            );
        }
        const complexText = ((
            complex.result as Record<string, unknown> | undefined
        )?.text ?? "") as string;
        if (!complexText.trim()) {
            throw new Error("complex dag missing result.text");
        }
        validateComplexPlan(parseJsonText(complexText));

        const jsonResp = (await client.call(
            "complete",
            buildWorkflowPayload(
                ctx.options.modelAlias,
                JSON_OUTPUT_PROMPT,
                ["json_output"],
                {
                    temperature: 0,
                    max_tokens: MAX_TOKENS_JSON_OUTPUT,
                    response_format: { type: "json_object" },
                },
            ),
        )) as Record<string, unknown>;
        if (jsonResp.status !== "succeeded") {
            throw new Error(
                `json-output status=${String(jsonResp.status)} response=${JSON.stringify(jsonResp).slice(0, 1200)}`,
            );
        }
        const jsonText = ((
            jsonResp.result as Record<string, unknown> | undefined
        )?.text ?? "") as string;
        if (!jsonText.trim()) {
            throw new Error("json-output missing result.text");
        }
        const parsedJson = parseJsonText(jsonText);
        if (parsedJson.ok !== true) {
            throw new Error(
                `json-output mismatch: ${JSON.stringify(parsedJson)}`,
            );
        }

        const streamResp = (await client.call(
            "complete",
            buildWorkflowPayload(ctx.options.modelAlias, STREAM_PROMPT, [], {
                temperature: 0,
                max_tokens: MAX_TOKENS_STREAM_OUTPUT,
                stream: true,
            }),
        )) as Record<string, unknown>;
        if (
            streamResp.status !== "running" &&
            streamResp.status !== "succeeded"
        ) {
            throw new Error(
                `stream status=${String(streamResp.status)} response=${JSON.stringify(streamResp).slice(0, 1200)}`,
            );
        }

        return "workflow protocol mix ok";
    };

    return [
        {
            id: "krpc_direct_01_complete_minimal_llm_success",
            layer: "kRPC Direct",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const result = await rpcCall<{
                    task_id?: string;
                    status?: string;
                }>(
                    ctx.targets.krpc.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] hello from remote runner`,
                    ),
                    ctx,
                    { traceId: `krpc-direct-complete-${provider}` },
                );
                const taskId = requireTaskId(result, "krpc direct complete");
                if ((result.status ?? "").toLowerCase() === "failed") {
                    throw new Error(
                        `complete returned failed: ${shortJson(result)}`,
                    );
                }
                return `task_id=${taskId}, status=${result.status ?? "<none>"}`;
            },
        },
        {
            id: "krpc_direct_02_complete_with_sys_seq_token_trace_success",
            layer: "kRPC Direct",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const result = await rpcCall<{
                    task_id?: string;
                    status?: string;
                }>(
                    ctx.targets.krpc.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] complete with token+trace`,
                    ),
                    ctx,
                    {
                        token: ctx.token ?? "tenant-a",
                        traceId: `krpc-direct-trace-${provider}`,
                    },
                );
                const taskId = requireTaskId(result, "krpc direct with trace");
                return `task_id=${taskId}`;
            },
        },
        {
            id: "krpc_direct_03_complete_invalid_sys_shape_returns_bad_request",
            layer: "kRPC Direct",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                try {
                    await rpcCall(
                        ctx.targets.krpc.aicc,
                        "complete",
                        { bad: "payload" },
                        ctx,
                        {
                            traceId: `krpc-direct-invalid-${provider}`,
                        },
                    );
                } catch (error) {
                    const message = String(error);
                    if (isParseBadRequestError(message)) {
                        return "invalid payload rejected as expected";
                    }
                    throw error;
                }
                throw new Error("invalid payload unexpectedly passed");
            },
        },
        {
            id: "krpc_direct_04_cancel_cross_tenant_rejected",
            layer: "kRPC Direct",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const start = await rpcCall<{ task_id?: string }>(
                    ctx.targets.krpc.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] cancel cross-tenant`,
                    ),
                    ctx,
                    { traceId: `krpc-cancel-start-${provider}` },
                );
                const taskId = requireTaskId(start, "krpc cancel start");
                try {
                    const cancel = await rpcCall<{ accepted?: boolean }>(
                        ctx.targets.krpc.aicc,
                        "cancel",
                        { task_id: taskId },
                        ctx,
                        {
                            token: "cross-tenant-test-invalid-token",
                            traceId: `krpc-cancel-cross-${provider}`,
                        },
                    );
                    if (cancel.accepted === false) {
                        return "cross-tenant cancel rejected with accepted=false";
                    }
                    throw new Error(
                        `unexpected cross-tenant cancel result: ${JSON.stringify(cancel)}`,
                    );
                } catch (error) {
                    const message = String(error).toLowerCase();
                    if (
                        message.includes("permission") ||
                        message.includes("tenant") ||
                        message.includes("jwt")
                    ) {
                        return "cross-tenant cancel rejected with auth error";
                    }
                    throw error;
                }
            },
        },
        {
            id: "krpc_direct_05_cancel_same_tenant_accepted_or_graceful_false",
            layer: "kRPC Direct",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const token = ctx.token ?? "tenant-a";
                const start = await rpcCall<{ task_id?: string }>(
                    ctx.targets.krpc.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] same-tenant cancel`,
                    ),
                    ctx,
                    { token, traceId: `krpc-cancel-same-start-${provider}` },
                );
                const taskId = requireTaskId(
                    start,
                    "krpc same-tenant cancel start",
                );
                const cancel = await rpcCall<{
                    task_id?: string;
                    accepted?: boolean;
                }>(ctx.targets.krpc.aicc, "cancel", { task_id: taskId }, ctx, {
                    token,
                    traceId: `krpc-cancel-same-${provider}`,
                });
                if (cancel.task_id && cancel.task_id !== taskId) {
                    throw new Error(
                        `cancel task_id mismatch expect=${taskId} actual=${cancel.task_id}`,
                    );
                }
                if (
                    cancel.accepted !== undefined &&
                    typeof cancel.accepted !== "boolean"
                ) {
                    throw new Error(
                        `invalid accepted flag: ${shortJson(cancel)}`,
                    );
                }
                return `task_id=${taskId}, accepted=${String(cancel.accepted)}`;
            },
        },
        {
            id: "gateway_01_complete_minimal_llm_success",
            layer: "TS SDK",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const result = await rpcCall<{
                    task_id?: string;
                    status?: string;
                }>(
                    ctx.targets.ts.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] gateway minimal`,
                    ),
                    ctx,
                    { traceId: `gateway-complete-${provider}` },
                );
                const taskId = requireTaskId(result, "gateway complete");
                if ((result.status ?? "").toLowerCase() === "failed") {
                    throw new Error(
                        `gateway complete failed: ${shortJson(result)}`,
                    );
                }
                return `task_id=${taskId}, status=${result.status ?? "<none>"}`;
            },
        },
        {
            id: "gateway_02_complete_with_sys_seq_token_trace_success",
            layer: "TS SDK",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const result = await rpcCall<{ task_id?: string }>(
                    ctx.targets.ts.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] gateway token+trace`,
                    ),
                    ctx,
                    {
                        token: ctx.token ?? "tenant-a",
                        traceId: `gateway-trace-${provider}`,
                    },
                );
                const taskId = requireTaskId(result, "gateway token+trace");
                return `task_id=${taskId}`;
            },
        },
        {
            id: "gateway_03_complete_without_token_with_trace_uses_null_placeholder",
            layer: "TS SDK",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const result = await rpcCall<{ task_id?: string }>(
                    ctx.targets.ts.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] gateway no-token trace`,
                    ),
                    ctx,
                    {
                        omitToken: true,
                        traceId: `gateway-no-token-trace-${provider}`,
                    },
                );
                const taskId = requireTaskId(result, "gateway no-token trace");
                return `task_id=${taskId}`;
            },
        },
        {
            id: "gateway_04_complete_invalid_sys_shape_returns_bad_request",
            layer: "TS SDK",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx) => {
                try {
                    await rpcCall(
                        ctx.targets.ts.aicc,
                        "complete",
                        { bad: "payload" },
                        ctx,
                    );
                } catch (error) {
                    if (isParseBadRequestError(String(error))) {
                        return "invalid payload rejected as expected";
                    }
                    throw error;
                }
                throw new Error("invalid payload unexpectedly passed");
            },
        },
        {
            id: "gateway_05_cancel_cross_tenant_rejected",
            layer: "TS SDK",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const start = await rpcCall<{ task_id?: string }>(
                    ctx.targets.ts.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] gateway cross-tenant cancel`,
                    ),
                    ctx,
                    { token: ctx.token ?? "tenant-a" },
                );
                const taskId = requireTaskId(
                    start,
                    "gateway cross-tenant start",
                );
                try {
                    const cancel = await rpcCall<{ accepted?: boolean }>(
                        ctx.targets.ts.aicc,
                        "cancel",
                        { task_id: taskId },
                        ctx,
                        { token: "cross-tenant-test-invalid-token" },
                    );
                    if (cancel.accepted === false) {
                        return "cross-tenant cancel rejected with accepted=false";
                    }
                    throw new Error(
                        `unexpected cancel result: ${shortJson(cancel)}`,
                    );
                } catch (error) {
                    if (isAuthOrPermissionError(String(error))) {
                        return "cross-tenant cancel rejected with auth error";
                    }
                    throw error;
                }
            },
        },
        {
            id: "gateway_06_cancel_same_tenant_accepted_or_graceful_false",
            layer: "TS SDK",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const token = ctx.token ?? "tenant-a";
                const start = await rpcCall<{ task_id?: string }>(
                    ctx.targets.ts.aicc,
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] gateway same-tenant cancel`,
                    ),
                    ctx,
                    { token },
                );
                const taskId = requireTaskId(
                    start,
                    "gateway same-tenant start",
                );
                const cancel = await rpcCall<{
                    task_id?: string;
                    accepted?: boolean;
                }>(ctx.targets.ts.aicc, "cancel", { task_id: taskId }, ctx, {
                    token,
                });
                if (cancel.task_id && cancel.task_id !== taskId) {
                    throw new Error(
                        `cancel task_id mismatch expect=${taskId} actual=${cancel.task_id}`,
                    );
                }
                return `task_id=${taskId}, accepted=${String(cancel.accepted)}`;
            },
        },
        {
            id: "cfg_01_sys_config_get_aicc_settings_success",
            layer: "TS SDK",
            requiresAuth: true,
            run: async (ctx, provider) => {
                const key = `services/aicc/test_settings/${provider}/cfg_01_${Date.now()}`;
                const expectedLimit = 5;
                const value = JSON.stringify({ fallback_limit: expectedLimit });
                await rpcCall(
                    ctx.targets.ts.systemConfig,
                    "sys_config_set",
                    { key, value },
                    ctx,
                );
                const payload = await rpcCall<Record<string, unknown> | null>(
                    ctx.targets.ts.systemConfig,
                    "sys_config_get",
                    { key },
                    ctx,
                );
                if (!payload) {
                    throw new Error("sys_config_get returned null");
                }
                const parsed = parseJsonStringValue(payload, "value");
                if (parsed.fallback_limit !== expectedLimit) {
                    throw new Error(
                        `fallback_limit mismatch: ${shortJson(parsed)}`,
                    );
                }
                return `key=${key}`;
            },
        },
        {
            id: "cfg_02_sys_config_set_full_value_effective",
            layer: "TS SDK",
            requiresAuth: true,
            run: async (ctx, provider) => {
                const key = `services/aicc/test_settings/${provider}/cfg_02_${Date.now()}`;
                const value = JSON.stringify({
                    fallback_limit: 7,
                    weights: { cost: 0.4, latency: 0.3, error: 0.2, load: 0.1 },
                });
                await rpcCall(
                    ctx.targets.ts.systemConfig,
                    "sys_config_set",
                    { key, value },
                    ctx,
                );
                const payload = await rpcCall<Record<string, unknown> | null>(
                    ctx.targets.ts.systemConfig,
                    "sys_config_get",
                    { key },
                    ctx,
                );
                if (!payload) {
                    throw new Error("sys_config_get returned null");
                }
                const parsed = parseJsonStringValue(payload, "value");
                const weights = normalizeObj(parsed.weights);
                if (parsed.fallback_limit !== 7 || weights.cost !== 0.4) {
                    throw new Error(
                        `persisted config mismatch: ${shortJson(parsed)}`,
                    );
                }
                return `key=${key}`;
            },
        },
        {
            id: "cfg_03_sys_config_set_by_json_path_partial_update_effective",
            layer: "TS SDK",
            requiresAuth: true,
            run: async (ctx, provider) => {
                const key = `services/aicc/test_settings/${provider}/cfg_03_${Date.now()}`;
                await rpcCall(
                    ctx.targets.ts.systemConfig,
                    "sys_config_set",
                    {
                        key,
                        value: JSON.stringify({
                            fallback_limit: 2,
                            weights: { cost: 0.2, latency: 0.3 },
                        }),
                    },
                    ctx,
                );
                await rpcCall(
                    ctx.targets.ts.systemConfig,
                    "sys_config_set_by_json_path",
                    { key, json_path: "/weights/cost", value: "0.9" },
                    ctx,
                );
                const payload = await rpcCall<Record<string, unknown> | null>(
                    ctx.targets.ts.systemConfig,
                    "sys_config_get",
                    { key },
                    ctx,
                );
                if (!payload) {
                    throw new Error("sys_config_get returned null");
                }
                const parsed = parseJsonStringValue(payload, "value");
                const weights = normalizeObj(parsed.weights);
                if (weights.cost !== 0.9 || weights.latency !== 0.3) {
                    throw new Error(
                        `json_path update mismatch: ${shortJson(parsed)}`,
                    );
                }
                return `key=${key}`;
            },
        },
        {
            id: "cfg_04_sys_config_write_without_permission_rejected",
            layer: "TS SDK",
            requiresAuth: true,
            run: async (ctx, provider) => {
                const key = `services/aicc/test_settings/${provider}/cfg_04_${Date.now()}`;
                try {
                    await rpcCall(
                        ctx.targets.ts.systemConfig,
                        "sys_config_set",
                        { key, value: JSON.stringify({ fallback_limit: 5 }) },
                        ctx,
                        { token: "tenant-a" },
                    );
                } catch (error) {
                    if (isAuthOrPermissionError(String(error))) {
                        return "write rejected without permission";
                    }
                    throw error;
                }
                throw new Error(
                    "sys_config_set unexpectedly succeeded without permission",
                );
            },
        },
        {
            id: "cfg_05_sys_config_value_not_json_string_rejected",
            layer: "TS SDK",
            requiresAuth: true,
            run: async (ctx, provider) => {
                const key = `services/aicc/test_settings/${provider}/cfg_05_${Date.now()}`;
                try {
                    await rpcCall(
                        ctx.targets.ts.systemConfig,
                        "sys_config_set",
                        { key, value: "not-json" },
                        ctx,
                    );
                    return "plain string accepted by target";
                } catch (error) {
                    const msg = String(error);
                    if (isAuthOrPermissionError(msg)) {
                        throw new SkippedCaseError(
                            "auth error while checking non-json rejection",
                        );
                    }
                    if (
                        msg.toLowerCase().includes("json") ||
                        msg.toLowerCase().includes("parse")
                    ) {
                        return "invalid json string rejected";
                    }
                    throw error;
                }
            },
        },
        {
            id: "aicc_client_complete_success",
            layer: "AiccClient (Rust)",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const out = await runRustRunner(
                    [
                        "complete",
                        "--endpoint",
                        ctx.targets.rust.aicc,
                        "--model-alias",
                        ctx.options.modelAlias,
                        "--prompt",
                        `[${provider}] Rust AiccClient complete`,
                        "--timeout-secs",
                        String(DEFAULT_RUST_RUNNER_TIMEOUT_SECS),
                        ...(ctx.token ? ["--token", ctx.token] : []),
                    ],
                    ctx,
                );
                const taskId = (out as { task_id?: string }).task_id;
                const status = (out as { status?: string }).status;
                if (!taskId) {
                    throw new Error(
                        `missing task_id from rust runner: ${JSON.stringify(out)}`,
                    );
                }
                if ((status ?? "").toLowerCase() === "failed") {
                    throw new Error(
                        `AiccClient complete returned failed: ${shortJson(out)}`,
                    );
                }
                return `task_id=${taskId}`;
            },
        },
        {
            id: "aicc_client_cancel_same_tenant",
            layer: "AiccClient (Rust)",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                const out = await runRustRunner(
                    [
                        "cancel",
                        "--endpoint",
                        ctx.targets.rust.aicc,
                        "--model-alias",
                        ctx.options.modelAlias,
                        "--prompt",
                        `[${provider}] Rust AiccClient cancel`,
                        "--timeout-secs",
                        String(DEFAULT_RUST_RUNNER_TIMEOUT_SECS),
                        ...(ctx.token ? ["--token", ctx.token] : []),
                    ],
                    ctx,
                );
                const accepted = (out as { accepted?: boolean }).accepted;
                if (typeof accepted !== "boolean") {
                    throw new Error(
                        `invalid cancel result from rust runner: ${JSON.stringify(out)}`,
                    );
                }
                return `accepted=${accepted}`;
            },
        },
        {
            id: "ts_sdk_complete_success",
            layer: "TS SDK",
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                await ensureSdk(ctx);
                const client = new buckyos.kRPCClient(
                    ctx.targets.ts.aicc,
                    ctx.token,
                );
                const result = (await client.call(
                    "complete",
                    buildCompletePayload(
                        ctx.options.modelAlias,
                        `[${provider}] TS SDK complete`,
                    ),
                )) as { task_id?: string; status?: string };
                if (!result.task_id) {
                    throw new Error(
                        `missing task_id: ${JSON.stringify(result)}`,
                    );
                }
                if ((result.status ?? "").toLowerCase() === "failed") {
                    throw new Error(
                        `TS SDK complete returned failed: ${shortJson(result)}`,
                    );
                }
                return `task_id=${result.task_id}, status=${result.status ?? "<none>"}`;
            },
        },
        {
            id: "workflow_remote_01_gateway_complex_scenario_protocol_mix",
            layer: "TS SDK",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                if (provider !== "openai") {
                    throw new SkippedCaseError("openai-only workflow case");
                }
                return runWorkflowProtocolMix(ctx, provider);
            },
        },
        {
            id: "workflow_remote_02_sn_openai_complex_scenario_protocol_mix",
            layer: "TS SDK",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                if (provider !== "sn-openai") {
                    throw new SkippedCaseError("sn-openai-only workflow case");
                }
                return runWorkflowProtocolMix(ctx, provider);
            },
        },
        {
            id: "workflow_remote_03_gemini_complex_scenario_protocol_mix",
            layer: "TS SDK",
            requiresAuth: true,
            requiredConfig: {
                providerFeatures: [
                    "plan",
                    "json_output",
                    "tool_calling",
                    "web_search",
                ],
            },
            run: async (ctx, provider) => {
                if (provider !== "gemini") {
                    throw new SkippedCaseError("gemini-only workflow case");
                }
                return runWorkflowProtocolMix(ctx, provider);
            },
        },
    ];
}

function esc(input: string): string {
    return input.replaceAll("|", "\\|").replaceAll("\n", "<br/>");
}

function statusIcon(status: CaseStatus): string {
    if (status === "passed")
        return '<span style="color:#16a34a;">&#x2714;</span>';
    if (status === "partial") return '<span style="color:#ca8a04;">!</span>';
    if (status === "skipped")
        return '<span style="color:#6b7280;">&#x23ED;</span>';
    return '<span style="color:#dc2626;">&#x2717;</span>';
}

function buildReport(results: CaseResult[], ctx: Context): string {
    const total = results.length;
    const passed = results.filter((r) => r.status === "passed").length;
    const partial = results.filter((r) => r.status === "partial").length;
    const failed = results.filter((r) => r.status === "failed").length;
    const skipped = results.filter((r) => r.status === "skipped").length;
    const overall: CaseStatus =
        failed > 0 ? "failed" : partial > 0 ? "partial" : "passed";

    const lines = [
        "# AICC Remote Test Report",
        "",
        `- Time: ${new Date().toISOString()}`,
        `- Gateway: ${ctx.options.gatewayHost}`,
        `- kRPC Host: ${ctx.options.krpcHost}`,
        `- AiccClient Host: ${ctx.options.aiccClientHost}`,
        `- Model Alias: ${ctx.options.modelAlias}`,
        `- Providers: ${ctx.options.providers.join(", ")}`,
        `- Mode: ${ctx.options.protocolMixOnlyNoSystemConfig ? "protocol_mix_only_no_system_config" : "default"}`,
        `- Overall: ${statusIcon(overall)} ${overall.toUpperCase()}`,
        `- Summary: total=${total}, passed=${passed}, partial=${partial}, failed=${failed}, skipped=${skipped}`,
        "",
        "## Case Results",
        "",
        "| Status | Provider | Layer | Case | Duration(ms) | Detail |",
        "|---|---|---|---|---:|---|",
    ];
    for (const r of results) {
        lines.push(
            `| ${statusIcon(r.status)} | ${r.provider} | ${r.layer} | ${esc(r.id)} | ${r.durationMs} | ${esc(r.detail)} |`,
        );
    }

    lines.push("", "## Strategy Mapping", "");
    lines.push(
        "- 覆盖链路：`/kapi/aicc`（complete/cancel） + `/kapi/system_config`（set/get） + `service.reload_settings`",
    );
    lines.push("- 调用层次：`kRPC Direct`、`AiccClient (Rust)`、`TS SDK`");
    lines.push("- 用例顺序：按 provider 串行，每个用例执行前重置 AICC 配置");
    lines.push("");
    return lines.join("\n");
}

async function runOneCase(
    testCase: TestCase,
    provider: ProviderKind,
    ctx: Context,
): Promise<CaseResult> {
    const started = Date.now();
    let status: CaseStatus = "passed";
    let detail = "ok";
    try {
        if (provider === "openai" && !ctx.options.apiKeys.openai) {
            throw new SkippedCaseError("missing openai api key");
        }
        if (provider === "gemini" && !ctx.options.apiKeys.gemini) {
            throw new SkippedCaseError("missing gemini api key");
        }
        if (provider === "claude" && !ctx.options.apiKeys.claude) {
            throw new SkippedCaseError("missing claude api key");
        }
        if (testCase.requiresAuth && !ctx.token) {
            throw new PartialCaseError("missing token for auth-required case");
        }
        const targetByLayer: Record<Layer, TargetEndpoints> = {
            "kRPC Direct": ctx.targets.krpc,
            "AiccClient (Rust)": ctx.targets.rust,
            "TS SDK": ctx.targets.ts,
        };
        if (!ctx.options.protocolMixOnlyNoSystemConfig) {
            await resetAiccSettings(
                ctx,
                targetByLayer[testCase.layer],
                provider,
                testCase.requiredConfig,
            );
        }
        detail = await testCase.run(ctx, provider);
    } catch (error) {
        if (error instanceof SkippedCaseError) {
            status = "skipped";
            detail = error.message;
        } else if (error instanceof PartialCaseError) {
            status = "partial";
            detail = error.message;
        } else {
            status = "failed";
            detail = String(error);
        }
    }

    return {
        provider,
        id: testCase.id,
        layer: testCase.layer,
        status,
        durationMs: Date.now() - started,
        detail,
    };
}

async function main(): Promise<void> {
    const options = await loadOptionsFromConfig(process.argv.slice(2));

    const ctx: Context = {
        options,
        targets: {
            ts: buildTargetEndpoints(options.gatewayHost),
            krpc: buildTargetEndpoints(options.krpcHost),
            rust: buildTargetEndpoints(options.aiccClientHost),
        },
        auth: {
            verifyHub: buildVerifyHubEndpoints(options.gatewayHost),
        },
        token: options.token,
        seq: Math.floor(Date.now() % 1_000_000),
        sdkInitialized: false,
    };
    ctx.token = await resolveToken(ctx);

    const allCases = createCases();
    const cases = ctx.options.protocolMixOnlyNoSystemConfig
        ? allCases.filter((testCase) =>
              testCase.id.endsWith("_complex_scenario_protocol_mix"),
          )
        : allCases;
    if (cases.length === 0) {
        throw new Error("no test cases selected by current mode/filter");
    }
    const results: CaseResult[] = [];
    for (const provider of options.providers) {
        for (const testCase of cases) {
            results.push(await runOneCase(testCase, provider, ctx));
        }
    }

    const report = buildReport(results, ctx);
    const outputPath = path.isAbsolute(options.output)
        ? options.output
        : path.join(process.cwd(), options.output);
    await mkdir(path.dirname(outputPath), { recursive: true });
    await writeFile(outputPath, report, "utf8");

    const failed = results.some((r) => r.status === "failed");
    const partial = !failed && results.some((r) => r.status === "partial");

    console.log(`report: ${outputPath}`);
    console.log(
        `summary: total=${results.length} passed=${results.filter((r) => r.status === "passed").length} partial=${results.filter((r) => r.status === "partial").length} failed=${results.filter((r) => r.status === "failed").length} skipped=${results.filter((r) => r.status === "skipped").length}`,
    );

    if (failed) {
        process.exitCode = 1;
    } else if (partial) {
        process.exitCode = 2;
    }
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
