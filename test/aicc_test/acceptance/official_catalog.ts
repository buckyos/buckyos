import type {
  OfficialCatalogConfig,
  ProviderBaseline,
  ProviderInventory,
} from "./types.ts";

type CatalogProfile = ProviderBaseline["providers"][number];
type Fetcher = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;

function curlConfigValue(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

async function fetchAnthropicCatalogWithCurl(url: URL, init: RequestInit): Promise<Response> {
  const headers = new Headers(init.headers);
  const config = [
    "silent",
    "show-error",
    `url = "${curlConfigValue(url.toString())}"`,
    `request = "${curlConfigValue(init.method ?? "GET")}"`,
    ...[...headers.entries()].map(([name, value]) =>
      `header = "${curlConfigValue(`${name}: ${value}`)}"`
    ),
    'write-out = "\\n%{http_code}"',
    "",
  ].join("\n");
  const child = new Deno.Command("curl", {
    args: ["--config", "-"],
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const writer = child.stdin.getWriter();
  await writer.write(new TextEncoder().encode(config));
  await writer.close();
  const output = await child.output();
  if (!output.success) throw new Error("Anthropic catalog curl transport failed");
  const text = new TextDecoder().decode(output.stdout);
  const separator = text.lastIndexOf("\n");
  const status = Number(text.slice(separator + 1));
  if (separator < 0 || !Number.isInteger(status)) {
    throw new Error("Anthropic catalog curl transport returned invalid status");
  }
  return new Response(text.slice(0, separator), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function object(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function stringField(value: unknown, field: string): string | undefined {
  const result = object(value)?.[field];
  return typeof result === "string" && result.trim() ? result.trim() : undefined;
}

function requireToken(profile: CatalogProfile, token: string | undefined): string {
  if (profile.official_catalog.authentication === "none") return "";
  if (!token?.trim()) {
    throw new Error(`official catalog credential is required for ${profile.provider_driver}`);
  }
  return token.trim();
}

function configureRequest(
  profile: CatalogProfile,
  token: string,
  cursor: string | undefined,
): { url: URL; init: RequestInit } {
  const catalog = profile.official_catalog;
  const url = new URL(catalog.endpoint);
  const headers = new Headers({ accept: "application/json" });
  const pageSize = String(catalog.page_size ?? 1000);
  if (catalog.format === "anthropic") {
    url.searchParams.set("limit", pageSize);
    if (cursor) url.searchParams.set("after_id", cursor);
    headers.set("anthropic-version", "2023-06-01");
    headers.set("user-agent", "aicc-acceptance/1.0");
  } else if (catalog.format === "gemini") {
    url.searchParams.set("pageSize", pageSize);
    if (cursor) url.searchParams.set("pageToken", cursor);
  } else if (catalog.format === "fal") {
    url.searchParams.set("limit", pageSize);
    if (cursor) url.searchParams.set("cursor", cursor);
  }
  if (catalog.authentication === "bearer") headers.set("authorization", `Bearer ${token}`);
  else if (catalog.authentication === "x-api-key") headers.set("x-api-key", token);
  else if (catalog.authentication === "query-key") url.searchParams.set("key", token);
  else if (catalog.authentication === "fal-key") headers.set("authorization", `Key ${token}`);
  return { url, init: { method: "GET", headers } };
}

function parsePage(
  config: OfficialCatalogConfig,
  body: unknown,
): { ids: string[]; cursor?: string } {
  const root = object(body);
  if (!root) throw new Error("official catalog returned a non-object response");
  if (config.format === "gemini") {
    const models = Array.isArray(root.models) ? root.models : [];
    return {
      ids: models.flatMap((entry) => {
        const name = stringField(entry, "name");
        return name ? [name.replace(/^models\//, "")] : [];
      }),
      cursor: stringField(root, "nextPageToken"),
    };
  }
  if (config.format === "fal") {
    const models = Array.isArray(root.models) ? root.models : [];
    return {
      ids: models.flatMap((entry) => {
        const id = stringField(entry, "endpoint_id");
        return id ? [id] : [];
      }),
      cursor: root.has_more === true ? stringField(root, "next_cursor") : undefined,
    };
  }
  if (config.format === "sn" && Array.isArray(root.models)) {
    return {
      ids: root.models.flatMap((entry) => {
        const id = stringField(entry, "provider_actual_model_id") ?? stringField(entry, "provider_model_id");
        return id ? [id] : [];
      }),
    };
  }
  if (config.format === "sn" && Array.isArray(root.items)) {
    return {
      ids: root.items.flatMap((entry) => {
        const id = stringField(entry, "model") ?? stringField(entry, "id");
        return id ? [id] : [];
      }),
    };
  }
  const data = Array.isArray(root.data) ? root.data : [];
  const result = {
    ids: data.flatMap((entry) => {
      const id = stringField(entry, "id") ?? stringField(entry, "model");
      return id ? [id] : [];
    }),
    cursor: undefined as string | undefined,
  };
  if (config.format === "anthropic" && root.has_more === true) {
    result.cursor = stringField(root, "last_id");
  }
  return result;
}

export async function fetchOfficialModelIds(input: {
  profile: CatalogProfile;
  token?: string;
  timeoutMs: number;
  fetcher?: Fetcher;
}): Promise<string[]> {
  const token = requireToken(input.profile, input.token);
  const fetcher = input.fetcher ?? fetch;
  const ids = new Map<string, string>();
  const seenCursors = new Set<string>();
  let cursor: string | undefined;
  for (let page = 0; page < 100; page += 1) {
    const request = configureRequest(input.profile, token, cursor);
    let response: Response | undefined;
    const maxAttempts = input.fetcher ? 1 : 5;
    for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
      try {
        response = input.profile.official_catalog.format === "anthropic" && !input.fetcher
          ? await fetchAnthropicCatalogWithCurl(request.url, request.init)
          : await fetcher(request.url, {
            ...request.init,
            signal: AbortSignal.timeout(input.timeoutMs),
          });
      } catch {
        if (attempt === maxAttempts) {
          throw new Error(`official catalog request failed for ${input.profile.provider_driver}: network error`);
        }
      }
      if (response && (response.ok || ![403, 429].includes(response.status) && response.status < 500)) break;
      if (attempt < maxAttempts) await new Promise((resolve) => setTimeout(resolve, attempt * 500));
    }
    if (!response) throw new Error(`official catalog request failed for ${input.profile.provider_driver}: network error`);
    if (!response.ok) {
      throw new Error(
        `official catalog request failed for ${input.profile.provider_driver}: HTTP ${response.status}`,
      );
    }
    let body: unknown;
    try {
      body = await response.json();
    } catch {
      throw new Error(`official catalog returned invalid JSON for ${input.profile.provider_driver}`);
    }
    const parsed = parsePage(input.profile.official_catalog, body);
    for (const rawId of parsed.ids) {
      const id = rawId.trim();
      if (id) ids.set(id.toLowerCase(), id);
    }
    if (!parsed.cursor) break;
    if (seenCursors.has(parsed.cursor)) {
      throw new Error(`official catalog pagination loop for ${input.profile.provider_driver}`);
    }
    seenCursors.add(parsed.cursor);
    cursor = parsed.cursor;
    if (page === 99) {
      throw new Error(`official catalog pagination exceeded 100 pages for ${input.profile.provider_driver}`);
    }
  }
  if (ids.size === 0) {
    throw new Error(`official catalog returned no models for ${input.profile.provider_driver}`);
  }
  return [...ids.values()].sort((left, right) => left.localeCompare(right));
}

export async function fetchOfficialCatalogs(input: {
  baseline: ProviderBaseline;
  drivers: string[];
  instanceNames: Record<string, string>;
  tokens: Record<string, string | undefined>;
  timeoutMs: number;
  fetcher?: Fetcher;
}): Promise<ProviderInventory[]> {
  const profiles = new Map(input.baseline.providers.map((profile) => [profile.provider_driver, profile]));
  const fetchedAt = new Date().toISOString();
  const catalogs: ProviderInventory[] = [];
  for (const driver of input.drivers) {
    const profile = profiles.get(driver);
    if (!profile) throw new Error(`missing provider baseline for ${driver}`);
    const instance = input.instanceNames[driver] || `${driver}-unresolved`;
    const ids = await fetchOfficialModelIds({
      profile,
      token: input.tokens[driver],
      timeoutMs: input.timeoutMs,
      fetcher: input.fetcher,
    });
    catalogs.push({
      provider_driver: driver,
      provider_instance_name: instance,
      inventory_revision: `official-snapshot-${fetchedAt}`,
      models: ids.map((id) => ({
        exact_model: `${id}@${instance}`,
        provider_model_id: id,
        api_types: [],
        logical_mounts: [],
      })),
    });
  }
  return catalogs;
}

export function bindOfficialCatalogInstances(
  catalogs: ProviderInventory[],
  selectedInventories: ProviderInventory[],
): ProviderInventory[] {
  const instances = new Map(
    selectedInventories.map((inventory) => [inventory.provider_driver, inventory.provider_instance_name]),
  );
  return catalogs.map((catalog) => {
    const instance = instances.get(catalog.provider_driver) ?? catalog.provider_instance_name;
    return {
      ...catalog,
      provider_instance_name: instance,
      models: catalog.models.map((model) => ({
        ...model,
        exact_model: `${model.provider_model_id}@${instance}`,
      })),
    };
  });
}
