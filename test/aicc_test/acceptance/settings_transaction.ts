import type { RpcClient } from "./gateway.ts";
import { buildMockSettings, configValue } from "./mock_settings.ts";

const SETTINGS_KEY = "services/aicc/settings";

export class SettingsCleanupError<T> extends AggregateError {
  readonly executionResult?: T;

  constructor(
    errors: unknown[],
    message: string,
    executionResult?: T,
  ) {
    super(errors, message);
    this.name = "SettingsCleanupError";
    this.executionResult = executionResult;
  }
}

async function setSettings(systemConfig: RpcClient, serialized: string): Promise<void> {
  await systemConfig.call("sys_config_set", { key: SETTINGS_KEY, value: serialized });
}

async function waitForSettings(
  systemConfig: RpcClient,
  serialized: string,
  timeoutMs = 15_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let observed = "";
  while (Date.now() < deadline) {
    const raw = await systemConfig.call("sys_config_get", { key: SETTINGS_KEY });
    const value = (raw && typeof raw === "object" && "value" in raw)
      ? (raw as { value?: unknown }).value
      : undefined;
    observed = typeof value === "string" ? value : "";
    if (observed === serialized) return;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(
    `system-config did not expose the requested AICC settings within ${timeoutMs}ms (observed_bytes=${observed.length}, expected_bytes=${serialized.length})`,
  );
}

async function reload(aicc: RpcClient): Promise<void> {
  await aicc.call("service.reload_settings", {});
}

export async function withMockSettings<T>(input: {
  systemConfig: RpcClient;
  aicc: RpcClient;
  baseUrl: string;
  runId: string;
  execute: () => Promise<T>;
  refreshClients?: () => Promise<{ systemConfig: RpcClient; aicc: RpcClient }>;
}): Promise<{ result: T; cleanup: "restored" }> {
  return await withAiccSettingsOverride({
    systemConfig: input.systemConfig,
    aicc: input.aicc,
    description: "AICC mock settings",
    patch: (settings) => buildMockSettings(settings, {
      baseUrl: input.baseUrl,
      runId: input.runId,
    }),
    execute: input.execute,
    refreshClients: input.refreshClients,
  });
}

export async function withAiccSettingsOverride<T>(input: {
  systemConfig: RpcClient;
  aicc: RpcClient;
  description: string;
  patch: (settings: Record<string, unknown>) => Record<string, unknown>;
  execute: () => Promise<T>;
  refreshClients?: () => Promise<{ systemConfig: RpcClient; aicc: RpcClient }>;
}): Promise<{ result: T; cleanup: "restored" }> {
  const raw = await input.systemConfig.call("sys_config_get", { key: SETTINGS_KEY });
  const backup = configValue(raw);
  const patched = JSON.stringify(input.patch(backup.parsed));
  let mutationAttempted = false;
  let executionError: unknown;
  let executionResult: T | undefined;
  let executionCompleted = false;
  try {
    mutationAttempted = true;
    await setSettings(input.systemConfig, patched);
    await waitForSettings(input.systemConfig, patched);
    await reload(input.aicc);
    executionResult = await input.execute();
    executionCompleted = true;
    return { result: executionResult, cleanup: "restored" };
  } catch (error) {
    executionError = error;
    throw error;
  } finally {
    if (mutationAttempted) {
      try {
        await setSettings(input.systemConfig, backup.serialized);
        await waitForSettings(input.systemConfig, backup.serialized);
        await reload(input.aicc);
      } catch (initialCleanupError) {
        let cleanupError: unknown = initialCleanupError;
        if (input.refreshClients) {
          try {
            const refreshed = await input.refreshClients();
            await setSettings(refreshed.systemConfig, backup.serialized);
            await waitForSettings(refreshed.systemConfig, backup.serialized);
            await reload(refreshed.aicc);
            cleanupError = undefined;
          } catch (refreshedCleanupError) {
            cleanupError = new AggregateError(
              [initialCleanupError, refreshedCleanupError],
              "cleanup failed with both original and refreshed authentication",
            );
          }
        }
        if (cleanupError !== undefined) {
          throw new SettingsCleanupError(
            executionError ? [executionError, cleanupError] : [cleanupError],
            `${input.description} cleanup failed; manual restoration is required`,
            executionCompleted ? executionResult : undefined,
          );
        }
      }
    }
  }
}
