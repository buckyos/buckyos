import type { RpcClient } from "./gateway.ts";

const METADATA_KEY = "services/aicc/driver_metadata";

function falModelDriver(): Record<string, unknown> {
  const models = [
    ["fal-ai/esrgan", "image.upscale"],
    ["fal-ai/imageutils/rembg", "image.bg_remove"],
    ["fal-ai/deepfilternet3", "audio.enhance"],
    ["fal-ai/video-upscaler", "video.upscale"],
  ].map(([id, apiType]) => ({
    id,
    api_types: [apiType],
    logical_mounts: [`${apiType}.fal`, `${apiType}.{model}`],
    capabilities: {},
  }));
  return {
    format: "buckyos.aicc.model-driver-catalog",
    schema_version: 1,
    schema_revision: 0,
    model_driver_id: "aicc-t1-fal",
    revision_seq: 1,
    required_features: [],
    models,
    patterns: [],
    defaults: { api_types: [], capabilities: {} },
    variants: [],
    version_rules: [],
  };
}

function parseMetadata(serialized: string | null): Record<string, unknown> {
  if (serialized === null) {
    return { schema_version: 1, model_drivers: [], provider_rules: [], known_providers: [] };
  }
  const value = JSON.parse(serialized);
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${METADATA_KEY} is not an object`);
  }
  return value as Record<string, unknown>;
}

async function readOptional(systemConfig: RpcClient): Promise<string | null> {
  try {
    const raw = await systemConfig.call("sys_config_get", { key: METADATA_KEY }) as { value?: unknown } | null;
    if (raw === null) return null;
    if (typeof raw.value !== "string") throw new Error(`${METADATA_KEY} has no string value`);
    return raw.value;
  } catch (error) {
    if (/not.?found|key.?not.?found/i.test(String(error))) return null;
    throw error;
  }
}

export async function installFalTestMetadata(input: {
  systemConfig: RpcClient;
  aicc: RpcClient;
}): Promise<(clients?: { systemConfig: RpcClient; aicc: RpcClient }) => Promise<void>> {
  const backup = await readOptional(input.systemConfig);
  const document = parseMetadata(backup);
  const current = Array.isArray(document.model_drivers) ? document.model_drivers : [];
  document.model_drivers = [
    ...current.filter((item) =>
      !item || typeof item !== "object" ||
      (item as Record<string, unknown>).model_driver_id !== "aicc-t1-fal"
    ),
    falModelDriver(),
  ];
  document.provider_rules = Array.isArray(document.provider_rules) ? document.provider_rules : [];
  document.known_providers = Array.isArray(document.known_providers) ? document.known_providers : [];
  await input.systemConfig.call("sys_config_set", {
    key: METADATA_KEY,
    value: JSON.stringify(document),
  });
  await input.aicc.call("service.reload_settings", {});
  let restored = false;
  return async (clients) => {
    if (restored) return;
    restored = true;
    const systemConfig = clients?.systemConfig ?? input.systemConfig;
    const aicc = clients?.aicc ?? input.aicc;
    if (backup === null) {
      await systemConfig.call("sys_config_delete", { key: METADATA_KEY });
    } else {
      await systemConfig.call("sys_config_set", { key: METADATA_KEY, value: backup });
    }
    await aicc.call("service.reload_settings", {});
  };
}
