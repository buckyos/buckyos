import type { RpcClient } from "./gateway.ts";

const CLOUD_CONFIG_KEY = "services/aicc/driver_metadata_update";

type CloudUpdateView = {
  enabled?: unknown;
  source_url?: unknown;
  interval_secs?: unknown;
  metadata_target_seq?: unknown;
  active_revision?: unknown;
  status?: unknown;
  last_error?: unknown;
  providers?: Array<{
    provider_instance_name?: unknown;
    metadata_applied_seq?: unknown;
  }>;
};

async function readOptional(systemConfig: RpcClient): Promise<string | null> {
  try {
    const raw = await systemConfig.call("sys_config_get", {
      key: CLOUD_CONFIG_KEY,
    }) as { value?: unknown } | null;
    if (raw === null) return null;
    if (typeof raw.value !== "string") {
      throw new Error(`${CLOUD_CONFIG_KEY} has no string value`);
    }
    return raw.value;
  } catch (error) {
    if (/not.?found|key.?not.?found/i.test(String(error))) return null;
    throw error;
  }
}

export async function setCloudUpdateSource(
  aicc: RpcClient,
  sourceUrl: string,
): Promise<void> {
  const response = await aicc.call("driver_metadata_update.set", {
    enabled: true,
    source_url: sourceUrl,
    interval_secs: 1,
  }) as { ok?: unknown; runtime_apply?: { ok?: unknown } };
  if (response.ok !== true || response.runtime_apply?.ok !== true) {
    throw new Error(
      `driver_metadata_update.set failed: ${JSON.stringify(response)}`,
    );
  }
}

export async function waitCloudUpdateConverged(
  aicc: RpcClient,
  revisionSeq: number,
  timeoutMs: number,
): Promise<CloudUpdateView> {
  const deadline = Date.now() + timeoutMs;
  let last: CloudUpdateView = {};
  while (Date.now() < deadline) {
    last = await aicc.call("driver_metadata_update.get", {}) as CloudUpdateView;
    const providers = Array.isArray(last.providers) ? last.providers : [];
    const converged = last.metadata_target_seq === revisionSeq &&
      last.active_revision === revisionSeq &&
      providers.every((provider) =>
        provider.metadata_applied_seq === revisionSeq
      );
    if (converged) return last;
    if (last.status === "error") {
      throw new Error(
        `cloud update failed before revision ${revisionSeq}: ${
          String(last.last_error)
        }`,
      );
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error(
    `cloud update revision ${revisionSeq} did not converge: ${
      JSON.stringify(last)
    }`,
  );
}

export async function backupCloudUpdateConfig(
  systemConfig: RpcClient,
): Promise<(refreshedSystemConfig?: RpcClient) => Promise<void>> {
  const backup = await readOptional(systemConfig);
  let restored = false;
  return async (refreshedSystemConfig?: RpcClient) => {
    if (restored) return;
    const client = refreshedSystemConfig ?? systemConfig;
    if (backup === null) {
      await client.call("sys_config_delete", { key: CLOUD_CONFIG_KEY });
    } else {
      await client.call("sys_config_set", {
        key: CLOUD_CONFIG_KEY,
        value: backup,
      });
    }
    restored = true;
  };
}

export async function disableCloudUpdate(aicc: RpcClient): Promise<void> {
  const response = await aicc.call("driver_metadata_update.set", {
    enabled: false,
  }) as { ok?: unknown };
  if (response.ok !== true) {
    throw new Error(
      `failed to disable cloud update: ${JSON.stringify(response)}`,
    );
  }
}
