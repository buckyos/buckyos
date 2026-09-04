type JsonObject = Record<string, unknown>;

function object(value: unknown): JsonObject {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as JsonObject;
}

function provider(input: {
  name: string;
  profile: string;
  adapter: string;
  baseUrl: string;
  token: string;
  timeoutMs: number;
  discovery?: JsonObject;
  providerRulesId?: string | null;
}): JsonObject {
  return {
    provider_instance_name: input.name,
    provider_type: "cloud_api",
    provider_profile_id: input.profile,
    protocol_adapter_id: input.adapter,
    ...(input.providerRulesId === null
      ? {}
      : { provider_rules_id: input.providerRulesId ?? input.profile }),
    base_url: input.baseUrl,
    credentials: { api_token: { locked: input.token } },
    enabled: true,
    timeout_ms: input.timeoutMs,
    auto_sync_models: true,
    ...(input.discovery ? { discovery: input.discovery } : {}),
  };
}

function customDiscovery(
  revision: string,
  protocol: "openai" | "claude" | "google-gemini" | "fal",
  selectedModels: Record<string, string>,
): JsonObject {
  const models = new Map<string, { apiTypes: string[]; remoteMethods: string[] }>();
  const operation = (apiType: string): string => {
    if (protocol === "claude") return "messages.create";
    if (protocol === "google-gemini") {
      if (apiType.startsWith("embedding.")) return "models.embedContent";
      if (apiType.startsWith("video.")) return "models.predictLongRunning";
      return "interactions.create";
    }
    if (protocol === "fal") return "queue.submit";
    if (apiType === "embedding.text") return "embeddings.create";
    if (apiType === "image.txt2img") return "images.generate";
    if (apiType === "image.img2img" || apiType === "image.inpaint") return "images.edit";
    if (apiType === "audio.tts") return "audio.speech.create";
    if (apiType === "audio.asr") return "audio.transcriptions.create";
    if (apiType.startsWith("video.")) return "videos.create";
    return "responses.create";
  };
  for (const [apiType, modelId] of Object.entries(selectedModels)) {
    const model = models.get(modelId) ?? { apiTypes: [], remoteMethods: [] };
    model.apiTypes.push(apiType);
    model.remoteMethods.push(operation(apiType));
    models.set(modelId, model);
  }
  return {
    revision,
    discovered_at_ms: Date.now(),
    health: "healthy",
    models: [...models].map(([provider_model_id, model]) => ({
      provider_model_id,
      api_types: model.apiTypes,
      remote_methods: [...new Set(model.remoteMethods)],
      availability: "available",
      deprecated: false,
    })),
  };
}

function installRoutingFixtures(settings: JsonObject, suffix: string): void {
  const session = object(settings.session_config);
  const logicalTree = object(session.logical_tree);
  const llm = object(logicalTree.llm);
  const llmChildren = object(llm.children);
  const acceptance = object(llmChildren.dv_acceptance);
  const acceptanceChildren = object(acceptance.children);
  acceptanceChildren.manual = { items: {}, source: "dv_system_routing_fixture" };
  acceptanceChildren.disable_line = {
    items: {
      primary: {
        target: `gpt-5.6@dv-openai-a-${suffix}`,
        weight: 1,
      },
    },
    disable_line: { web_search: true },
    source: "dv_system_routing_fixture",
  };
  acceptanceChildren.system_overlay = {
    items: {
      system: {
        target: `gpt-5.6@dv-openai-a-${suffix}`,
        weight: 1,
      },
    },
    source: "dv_system_routing_fixture",
  };
  acceptance.children = acceptanceChildren;
  llmChildren.dv_acceptance = acceptance;
  llm.children = llmChildren;
  logicalTree.llm = llm;
  session.logical_tree = logicalTree;
  session.revision = `dv-routing-${suffix}`;
  settings.session_config = session;
}

function falDiscovery(): JsonObject {
  return {
    revision: "t1-fal-v1",
    discovered_at_ms: Date.now(),
    health: "healthy",
    models: [
      ["fal-ai/esrgan", ["image.upscale"]],
      ["fal-ai/imageutils/rembg", ["image.bg_remove"]],
      ["fal-ai/deepfilternet3", ["audio.enhance"]],
      ["fal-ai/video-upscaler", ["video.upscale"]],
    ].map(([provider_model_id, api_types]) => ({
      provider_model_id,
      api_types,
      availability: "available",
      deprecated: false,
    })),
  };
}

export function buildMockSettings(
  original: unknown,
  input: {
    baseUrl: string;
    runId: string;
    timeoutMs?: number;
    customModels?: Record<"openai" | "claude" | "google-gemini" | "fal", Record<string, string>>;
  },
): JsonObject {
  const settings = structuredClone(object(original));
  const baseUrl = input.baseUrl.replace(/\/+$/, "");
  if (!/^https?:\/\//.test(baseUrl)) throw new Error("mock base URL must be HTTP(S)");
  const suffix = input.runId.replace(/[^a-zA-Z0-9_-]/g, "-");
  if (!suffix) throw new Error("run_id is required");
  const timeoutMs = input.timeoutMs ?? 5_000;
  const customModels = input.customModels ?? {
    openai: { llm: "gpt-5.6-sol" },
    claude: { llm: "claude-sonnet-5" },
    "google-gemini": { llm: "gemini-3.8-flash" },
    fal: { "image.upscale": "fal-ai/esrgan" },
  };
  const currentProviders = Array.isArray(settings.providers)
    ? settings.providers.filter((item) => item && typeof item === "object")
    : [];
  settings.providers = [
    ...currentProviders,
    provider({
      name: `dv-openai-a-${suffix}`,
      profile: "openai",
      adapter: "openai-responses",
      baseUrl: `${baseUrl}/instance-a/v1`,
      token: `mock-a-${suffix}`,
      timeoutMs,
    }),
    provider({
      name: `dv-openai-b-${suffix}`,
      profile: "openai",
      adapter: "openai-responses",
      baseUrl: `${baseUrl}/instance-b/v1`,
      token: `mock-b-${suffix}`,
      timeoutMs,
    }),
    provider({
      name: `dv-claude-${suffix}`,
      profile: "claude",
      adapter: "claude-messages",
      baseUrl: `${baseUrl}/v1`,
      token: `mock-${suffix}`,
      timeoutMs,
    }),
    provider({
      name: `dv-gemini-${suffix}`,
      profile: "gemini",
      adapter: "gemini-interactions",
      baseUrl: `${baseUrl}/v1beta`,
      token: `mock-${suffix}`,
      timeoutMs,
    }),
    provider({
      name: `dv-minimax-${suffix}`,
      profile: "minimax",
      adapter: "minimax-messages",
      baseUrl: `${baseUrl}/v1`,
      token: `mock-${suffix}`,
      timeoutMs,
    }),
    provider({
      name: `dv-fal-${suffix}`,
      profile: "fal",
      adapter: "fal-queue",
      baseUrl,
      token: `mock-${suffix}`,
      timeoutMs,
      discovery: falDiscovery(),
    }),
    provider({
      name: `dv-custom-openai-${suffix}`,
      profile: "custom",
      adapter: "openai-responses",
      providerRulesId: null,
      baseUrl: `${baseUrl}/instance-custom-openai/v1`,
      token: `mock-custom-openai-${suffix}`,
      timeoutMs,
      discovery: customDiscovery(`t1-custom-openai-${suffix}`, "openai", customModels.openai),
    }),
    provider({
      name: `dv-custom-claude-${suffix}`,
      profile: "custom",
      adapter: "claude-messages",
      providerRulesId: null,
      baseUrl: `${baseUrl}/instance-custom-claude/v1`,
      token: `mock-custom-claude-${suffix}`,
      timeoutMs,
      discovery: customDiscovery(`t1-custom-claude-${suffix}`, "claude", customModels.claude),
    }),
    provider({
      name: `dv-custom-gemini-${suffix}`,
      profile: "custom",
      adapter: "gemini-interactions",
      providerRulesId: null,
      baseUrl: `${baseUrl}/instance-custom-gemini/v1beta`,
      token: `mock-custom-gemini-${suffix}`,
      timeoutMs,
      discovery: customDiscovery(`t1-custom-gemini-${suffix}`, "google-gemini", customModels["google-gemini"]),
    }),
    provider({
      name: `dv-custom-fal-${suffix}`,
      profile: "custom",
      adapter: "fal-queue",
      providerRulesId: null,
      baseUrl,
      token: `mock-custom-fal-${suffix}`,
      timeoutMs,
      discovery: customDiscovery(`t1-custom-fal-${suffix}`, "fal", customModels.fal),
    }),
  ];
  installRoutingFixtures(settings, suffix);
  return settings;
}

export function configValue(raw: unknown): { serialized: string; parsed: JsonObject } {
  const value = object(raw).value;
  if (typeof value !== "string") throw new Error("sys_config_get returned no string value");
  const parsed = JSON.parse(value);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("services/aicc/settings is not a JSON object");
  }
  return { serialized: value, parsed: parsed as JsonObject };
}
