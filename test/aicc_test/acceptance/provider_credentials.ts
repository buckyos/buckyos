const SUPPORTED_TOKEN_DRIVERS = [
  "openai",
  "claude",
  "google-gemini",
  "fal",
  "minimax",
  "openrouter",
] as const;

export type ProviderTokenDriver = (typeof SUPPORTED_TOKEN_DRIVERS)[number];
export type ProviderTokens = Partial<Record<ProviderTokenDriver, string>>;

type JsonObject = Record<string, unknown>;

function object(value: unknown): JsonObject | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as JsonObject
    : undefined;
}

function sectionKey(driver: ProviderTokenDriver): string {
  if (driver === "google-gemini") return "google";
  if (driver === "openrouter") return "openai";
  return driver;
}

function sectionKeys(driver: ProviderTokenDriver): string[] {
  return driver === "google-gemini" ? ["google", "gemini", "google_gemini"] : [sectionKey(driver)];
}

function defaultInstance(driver: ProviderTokenDriver, name: string, token: string): JsonObject {
  const endpoints: Record<ProviderTokenDriver, string> = {
    openai: "https://api.openai.com/v1",
    claude: "https://api.anthropic.com/v1",
    "google-gemini": "https://generativelanguage.googleapis.com/v1beta",
    fal: "https://fal.run",
    minimax: "https://api.minimax.io/v1",
    openrouter: "https://openrouter.ai/api/v1",
  };
  return {
    provider_instance_name: name,
    provider_type: "cloud_api",
    provider_driver: driver,
    api_token: token,
    base_url: endpoints[driver],
    timeout_ms: 300_000,
  };
}

function instanceDriver(instance: JsonObject, section: string): string {
  const value = instance.provider_driver;
  if (typeof value === "string" && value.trim()) return value.trim();
  return section === "gemini" ? "google-gemini" : section;
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
  for (const [rawDriver, rawToken] of Object.entries(tokens)) {
    const driver = rawDriver as ProviderTokenDriver;
    const token = rawToken?.trim();
    if (!token) continue;
    if (!SUPPORTED_TOKEN_DRIVERS.includes(driver)) {
      throw new Error(`provider credential driver ${driver} is not supported`);
    }
    const existingKey = sectionKeys(driver).find((candidate) => object(settings[candidate]));
    const key = existingKey ?? sectionKey(driver);
    const section = object(settings[key]) ?? { enabled: true, instances: [] };
    settings[key] = section;
    section.enabled = true;
    const instances = Array.isArray(section.instances)
      ? section.instances.flatMap((value) => object(value) ? [value as JsonObject] : [])
      : [];
    const candidates = instances.filter((instance) => instanceDriver(instance, key) === driver);
    const selectedName = selectedInstances[driver]?.trim();
    if (selectedName) {
      const selected = candidates.find((instance) =>
        instance.provider_instance_name === selectedName || instance.instance_id === selectedName
      );
      if (selected) {
        selected.api_token = token;
        continue;
      }
      if (candidates.length > 0) {
        throw new Error(`configured provider instance ${selectedName} was not found for ${driver}`);
      }
    }
    if (candidates.length === 1) {
      candidates[0].api_token = token;
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
      };
      const created = defaultInstance(
        driver,
        selectedName || defaultNames[driver],
        token,
      );
      section.instances = [...instances, created];
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
