type JsonObject = Record<string, unknown>;

function object(value: unknown): JsonObject {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as JsonObject;
}

function instances(value: unknown): JsonObject[] {
  return Array.isArray(value) ? value.filter((item) => item && typeof item === "object") as JsonObject[] : [];
}

function appendSection(
  settings: JsonObject,
  key: string,
  additions: JsonObject[],
): void {
  const current = object(settings[key]);
  settings[key] = {
    ...current,
    enabled: true,
    instances: [...instances(current.instances), ...additions],
  };
}

function installRoutingFixtures(settings: JsonObject, suffix: string): void {
  const routing = object(settings.routing_config);
  const definitions = Array.isArray(routing.logical_definitions)
    ? routing.logical_definitions.filter((item) => item && typeof item === "object") as JsonObject[]
    : [];
  const fixtureDefinitions: JsonObject[] = [
    { path: "llm.dv_acceptance.auto", api_type: "llm", mount_mode: "auto" },
    { path: "llm.dv_acceptance.manual", api_type: "llm", mount_mode: "manual" },
    {
      path: "llm.dv_acceptance.min_line",
      api_type: "llm",
      mount_mode: "auto",
      min_line: { min_context_tokens: 1_000_000_000 },
    },
    {
      path: "llm.dv_acceptance.disable_line",
      api_type: "llm",
      mount_mode: "auto",
      disable_line: { web_search: true },
    },
    { path: "llm.dv_acceptance.system_overlay", api_type: "llm", mount_mode: "manual" },
  ];
  const fixturePaths = new Set(fixtureDefinitions.map((item) => String(item.path)));
  routing.logical_definitions = [
    ...definitions.filter((item) => !fixturePaths.has(String(item.path))),
    ...fixtureDefinitions,
  ];

  const logicalTree = object(routing.logical_tree);
  const llm = object(logicalTree.llm);
  const llmChildren = object(llm.children);
  const acceptance = object(llmChildren.dv_acceptance);
  const acceptanceChildren = object(acceptance.children);
  acceptanceChildren.system_overlay = {
    items: {
      system: {
        target: `gpt-4o-mini@dv-openai-a-${suffix}`,
        weight: 1,
      },
    },
    source: "dv_system_routing_fixture",
  };
  acceptance.children = acceptanceChildren;
  llmChildren.dv_acceptance = acceptance;
  llm.children = llmChildren;
  logicalTree.llm = llm;
  routing.logical_tree = logicalTree;
  routing.revision = `dv-routing-${suffix}`;
  settings.routing_config = routing;
}

export function buildMockSettings(
  original: unknown,
  input: { baseUrl: string; runId: string; timeoutMs?: number },
): JsonObject {
  const settings = structuredClone(object(original));
  const baseUrl = input.baseUrl.replace(/\/+$/, "");
  if (!/^https?:\/\//.test(baseUrl)) throw new Error("mock base URL must be HTTP(S)");
  const suffix = input.runId.replace(/[^a-zA-Z0-9_-]/g, "-");
  if (!suffix) throw new Error("run_id is required");
  const timeout_ms = input.timeoutMs ?? 5_000;
  const common = { provider_type: "cloud_api", api_token: `mock-${suffix}`, timeout_ms };

  appendSection(settings, "openai", [
    {
      ...common,
      api_token: `mock-a-${suffix}`,
      provider_instance_name: `dv-openai-a-${suffix}`,
      provider_driver: "openai",
      base_url: `${baseUrl}/instance-a/v1`,
      models: ["gpt-4o-mini", "gpt-5.4", "text-embedding-3-small", "gpt-image-1", "whisper-1", "tts-1", "sora-2", "sora-mock-pattern"],
    },
    {
      ...common,
      api_token: `mock-b-${suffix}`,
      provider_instance_name: `dv-openai-b-${suffix}`,
      provider_driver: "openai",
      base_url: `${baseUrl}/instance-b/v1`,
      models: ["gpt-4o-mini", "gpt-5-mini", "text-embedding-3-small"],
    },
  ]);
  appendSection(settings, "claude", [{
    ...common,
    provider_instance_name: `dv-claude-${suffix}`,
    provider_driver: "claude",
    base_url: `${baseUrl}/v1`,
    models: ["claude-3-7-sonnet-20250219"],
  }]);
  appendSection(settings, "gemini", [{
    ...common,
    provider_instance_name: `dv-gemini-${suffix}`,
    provider_driver: "google-gemini",
    base_url: `${baseUrl}/v1beta`,
  }]);
  appendSection(settings, "minimax", [{
    ...common,
    provider_instance_name: `dv-minimax-${suffix}`,
    provider_driver: "minimax",
    base_url: `${baseUrl}/v1`,
    models: ["MiniMax-M2.5"],
  }]);
  appendSection(settings, "fal", [{
    ...common,
    provider_instance_name: `dv-fal-${suffix}`,
    base_url: baseUrl,
    image_upscale_models: ["fal-ai/esrgan"],
    image_bg_remove_models: ["fal-ai/imageutils/rembg"],
    audio_enhance_models: ["fal-ai/deepfilternet3"],
    video_upscale_models: ["fal-ai/video-upscaler"],
  }]);
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
