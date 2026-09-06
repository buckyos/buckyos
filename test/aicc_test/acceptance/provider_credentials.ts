const SUPPORTED_TOKEN_DRIVERS = [
  "openai",
  "claude",
  "google-gemini",
  "fal",
  "minimax",
  "openrouter",
  "glm",
] as const;

export type ProviderTokenDriver = (typeof SUPPORTED_TOKEN_DRIVERS)[number];
export type ProviderTokens = Partial<Record<ProviderTokenDriver, string>>;

type JsonObject = Record<string, unknown>;

function object(value: unknown): JsonObject | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as JsonObject
    : undefined;
}

function profileId(driver: ProviderTokenDriver): string {
  return driver === "google-gemini" ? "gemini" : driver;
}

function defaultInstance(driver: ProviderTokenDriver, name: string, token: string): JsonObject {
  const endpoints: Record<ProviderTokenDriver, string> = {
    openai: "https://api.openai.com/v1",
    claude: "https://api.anthropic.com/v1",
    "google-gemini": "https://generativelanguage.googleapis.com/v1beta",
    fal: "https://queue.fal.run",
    minimax: "https://api.minimax.io/anthropic",
    openrouter: "https://openrouter.ai/api/v1",
    glm: "https://api.z.ai/api/paas/v4",
  };
  const adapters: Record<ProviderTokenDriver, string> = {
    openai: "openai-responses",
    claude: "claude-messages",
    "google-gemini": "gemini-interactions",
    fal: "fal-queue",
    minimax: "minimax-messages",
    openrouter: "openrouter-openai",
    glm: "glm-chat",
  };
  const profile = profileId(driver);
  return {
    provider_instance_name: name,
    provider_type: "cloud_api",
    provider_profile_id: profile,
    protocol_adapter_id: adapters[driver],
    base_url: endpoints[driver],
    credentials: { api_token: { locked: token } },
    provider_rules_id: profile,
    enabled: true,
    timeout_ms: 300_000,
  };
}

function instanceDriver(instance: JsonObject): string {
  const value = instance.provider_profile_id;
  if (value === "gemini") return "google-gemini";
  return typeof value === "string" && value.trim() ? value.trim() : "";
}

export function configuredProviderTokens(
  values: Record<string, unknown>,
  environment: (name: string) => string | undefined,
): ProviderTokens {
  const result: ProviderTokens = {};
  for (const driver of SUPPORTED_TOKEN_DRIVERS) {
    const tomlValue = values[`provider_credentials.${driver}.api_token`];
    const envName = `AICC_${driver.replaceAll("-", "_").toUpperCase()}_API_TOKEN`;
    const token = (typeof tomlValue === "string" ? tomlValue.trim() : "") || environment(envName)?.trim();
    if (token) result[driver] = token;
  }
  return result;
}

export function applyProviderTokens(
  original: Record<string, unknown>,
  tokens: ProviderTokens,
  selectedInstances: Record<string, string>,
): Record<string, unknown> {
  const settings = structuredClone(original);
  const providers = Array.isArray(settings.providers)
    ? settings.providers.flatMap((value) => object(value) ? [value as JsonObject] : [])
    : [];
  settings.providers = providers;
  for (const [rawDriver, rawToken] of Object.entries(tokens)) {
    const driver = rawDriver as ProviderTokenDriver;
    const token = rawToken?.trim();
    if (!token) continue;
    if (!SUPPORTED_TOKEN_DRIVERS.includes(driver)) {
      throw new Error(`provider credential driver ${driver} is not supported`);
    }
    const candidates = providers.filter((instance) => instanceDriver(instance) === driver);
    const selectedName = selectedInstances[driver]?.trim();
    if (selectedName) {
      const selected = candidates.find((instance) =>
        instance.provider_instance_name === selectedName
      );
      if (selected) {
        selected.credentials = { api_token: { locked: token } };
        continue;
      }
      if (candidates.length > 0) {
        throw new Error(`configured provider instance ${selectedName} was not found for ${driver}`);
      }
    }
    if (candidates.length === 1) {
      candidates[0].credentials = { api_token: { locked: token } };
      continue;
    }
    if (candidates.length === 0) {
      const defaultNames: Record<ProviderTokenDriver, string> = {
        openai: "openai-main",
        claude: "claude-main",
        "google-gemini": "google-gemini-main",
        fal: "fal-main",
        minimax: "minimax-main",
        openrouter: "openrouter-main",
        glm: "glm-main",
      };
      const created = defaultInstance(
        driver,
        selectedName || defaultNames[driver],
        token,
      );
      providers.push(created);
      continue;
    }
    if (candidates.length > 1) {
      throw new Error(
        `provider ${driver} has multiple configured instances; provider_credentials.${driver}.instance_name is required`,
      );
    }
    throw new Error(`AICC settings has no unambiguous credential target for provider ${driver}`);
  }
  return settings;
}

export function providerTokenDrivers(tokens: ProviderTokens): string[] {
  return Object.entries(tokens).filter(([, token]) => Boolean(token?.trim())).map(([driver]) => driver).sort();
}

export function selectProviderTokens(
  tokens: ProviderTokens,
  selectedDrivers: string[],
): ProviderTokens {
  if (selectedDrivers.length === 0) return { ...tokens };
  const selected = new Set(selectedDrivers);
  return Object.fromEntries(
    Object.entries(tokens).filter(([driver, token]) => selected.has(driver) && Boolean(token?.trim())),
  ) as ProviderTokens;
}
