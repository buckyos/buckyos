import { parseToml, tomlString } from "../../jarvis_media_dv/config.ts";
import { loginGateway } from "./gateway.ts";
import { configValue } from "./mock_settings.ts";

type JsonObject = Record<string, unknown>;

function object(value: unknown): JsonObject | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as JsonObject
    : undefined;
}

function requiredArg(args: string[], index: number, flag: string): string {
  const value = args[index + 1]?.trim();
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`);
  return value;
}

async function main(): Promise<void> {
  let configPath = "aicc_acceptance.local.toml";
  for (let index = 0; index < Deno.args.length; index += 1) {
    if (Deno.args[index] === "--config") configPath = requiredArg(Deno.args, index, "--config");
  }
  const config = parseToml(await Deno.readTextFile(configPath));
  const gatewayUrl = tomlString(config, "gateway.url") ?? "";
  const session = await loginGateway({
    gatewayUrl,
    sessionToken: tomlString(config, "auth.session_token"),
    username: tomlString(config, "auth.username"),
    password: tomlString(config, "auth.password"),
    appId: "control-panel",
  });
  const settings = configValue(await session.systemConfig.call("sys_config_get", {
    key: "services/aicc/settings",
  })).parsed;
  const sections = Object.entries(settings).map(([name, raw]) => {
    const section = object(raw);
    const instances = Array.isArray(section?.instances) ? section.instances : [];
    return {
      name,
      enabled: section?.enabled,
      instance_count: instances.length,
      instances: instances.flatMap((rawInstance) => {
        const instance = object(rawInstance);
        if (!instance) return [];
        return [{
          name: instance.provider_instance_name ?? instance.instance_id ?? "<unnamed>",
          driver: instance.provider_driver ?? name,
          has_token: typeof instance.api_token === "string" && instance.api_token.length > 0,
          has_base_url: typeof instance.base_url === "string" && instance.base_url.length > 0,
        }];
      }),
    };
  });
  const rawInventory = await session.aicc.call("models.list", {}) as { providers?: unknown };
  const providers = Array.isArray(rawInventory.providers)
    ? rawInventory.providers.flatMap((raw) => {
      const provider = object(raw);
      if (!provider) return [];
      return [{
        instance: provider.provider_instance_name,
        driver: provider.provider_driver,
        model_count: Array.isArray(provider.models) ? provider.models.length : 0,
      }];
    })
    : [];
  console.log(JSON.stringify({ sections, providers }, null, 2));
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(`AICC runtime inspection failed: ${String(error)}`);
    Deno.exitCode = 1;
  });
}
