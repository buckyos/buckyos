import { parseToml, tomlString } from "../../jarvis_media_dv/config.ts";
import { loginGateway } from "./gateway.ts";
import { configValue } from "./mock_settings.ts";

const MOCK_SECTIONS = ["openai", "claude", "gemini", "minimax", "fal"];

function argument(name: string): string | undefined {
  const index = Deno.args.indexOf(name);
  return index >= 0 ? Deno.args[index + 1]?.trim() : undefined;
}

async function main(): Promise<void> {
  if (!Deno.args.includes("--confirm-remove-mock-run")) {
    throw new Error("--confirm-remove-mock-run is required");
  }
  const configPath = argument("--config") ?? "aicc_acceptance.local.toml";
  const runId = argument("--run-id");
  if (!runId || !/^aicc-t1-[A-Za-z0-9_-]+$/.test(runId)) {
    throw new Error("--run-id must be an exact aicc-t1 run id");
  }
  const suffix = runId.replace(/[^a-zA-Z0-9_-]/g, "-");
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
  for (const sectionName of MOCK_SECTIONS) {
    const section = next[sectionName];
    if (section === undefined) continue;
    if (!section || typeof section !== "object" || Array.isArray(section)) {
      throw new Error(`refusing to modify non-object section ${sectionName}`);
    }
    const object = section as Record<string, unknown>;
    if (!Array.isArray(object.instances)) {
      throw new Error(`refusing to modify ${sectionName}: instances is not an array`);
    }
    const retained = object.instances.filter((value) => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return true;
      const name = (value as Record<string, unknown>).provider_instance_name;
      const matches = typeof name === "string" && name.startsWith("dv-") && name.endsWith(suffix);
      if (matches) removed.push(`${sectionName}/${name}`);
      return !matches;
    });
    if (retained.length === 0) delete next[sectionName];
    else object.instances = retained;
  }
  if (removed.length !== 6) {
    throw new Error(`refusing cleanup: expected exactly 6 run-scoped instances, found ${removed.length}`);
  }
  await session.systemConfig.call("sys_config_set", {
    key: "services/aicc/settings",
    value: JSON.stringify(next),
  });
  await session.aicc.call("service.reload_settings", {});
  console.log(JSON.stringify({ cleanup: "restored", run_id: runId, removed }, null, 2));
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(`AICC T1 mock cleanup failed: ${String(error)}`);
    Deno.exitCode = 1;
  });
}
