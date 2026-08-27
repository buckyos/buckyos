import { parseToml, tomlString } from "../../jarvis_media_dv/config.ts";
import { loginGateway } from "./gateway.ts";
import { configValue } from "./mock_settings.ts";

const TEST_SECTIONS: Record<string, string> = {
  openai: "openai-main",
  claude: "claude-main",
  google: "google-gemini-main",
  fal: "fal-main",
};

async function main(): Promise<void> {
  if (!Deno.args.includes("--confirm-remove-test-sections")) {
    throw new Error("--confirm-remove-test-sections is required");
  }
  const configIndex = Deno.args.indexOf("--config");
  const configPath = configIndex >= 0 ? Deno.args[configIndex + 1] : "aicc_acceptance.local.toml";
  if (!configPath) throw new Error("--config requires a value");
  const config = parseToml(await Deno.readTextFile(configPath));
  const session = await loginGateway({
    gatewayUrl: tomlString(config, "gateway.url") ?? "",
    sessionToken: tomlString(config, "auth.session_token"),
    username: tomlString(config, "auth.username"),
    password: tomlString(config, "auth.password"),
    appId: "control-panel",
  });
  const current = configValue(await session.systemConfig.call("sys_config_get", {
    key: "services/aicc/settings",
  })).parsed;
  const next = structuredClone(current);
  const removed: string[] = [];
  for (const [sectionName, expectedInstance] of Object.entries(TEST_SECTIONS)) {
    const section = next[sectionName];
    if (section === undefined) continue;
    if (!section || typeof section !== "object" || Array.isArray(section)) {
      throw new Error(`refusing to remove non-object section ${sectionName}`);
    }
    const instances = (section as Record<string, unknown>).instances;
    if (!Array.isArray(instances) || instances.length !== 1) {
      throw new Error(`refusing to remove ${sectionName}: expected exactly one test instance`);
    }
    const instance = instances[0] as Record<string, unknown>;
    if (instance.provider_instance_name !== expectedInstance) {
      throw new Error(`refusing to remove ${sectionName}: unexpected instance name`);
    }
    delete next[sectionName];
    removed.push(`${sectionName}/${expectedInstance}`);
  }
  await session.systemConfig.call("sys_config_set", {
    key: "services/aicc/settings",
    value: JSON.stringify(next),
  });
  await session.aicc.call("service.reload_settings", {});
  console.log(JSON.stringify({ cleanup: "restored", removed }, null, 2));
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(`AICC test Provider cleanup failed: ${String(error)}`);
    Deno.exitCode = 1;
  });
}
