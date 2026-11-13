import type { RpcClient } from "./gateway.ts";
import type { CloudCatalogTombstone } from "./cloud_update_fixture_service.ts";
import { randomUUID } from "node:crypto";
import { lstat, readFile, readdir, rename } from "node:fs/promises";
import { join } from "node:path";

const CLOUD_CONFIG_KEY = "services/aicc/driver_metadata_update";
const METADATA_KEY = "services/aicc/driver_metadata";

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

async function readOptional(systemConfig: RpcClient, key = CLOUD_CONFIG_KEY): Promise<string | null> {
  try {
    const raw = await systemConfig.call("sys_config_get", {
      key,
    }) as { value?: unknown } | null;
    if (raw === null) return null;
    if (typeof raw.value !== "string") {
      throw new Error(`${key} has no string value`);
    }
    return raw.value;
  } catch (error) {
    if (/not.?found|key.?not.?found/i.test(String(error))) return null;
    throw error;
  }
}

export async function isolateCloudMetadataOverrides(input: {
  systemConfig: RpcClient;
  localRoot: string;
  catalogs: Array<Pick<CloudCatalogTombstone, "catalog_kind" | "catalog_id">>;
}): Promise<(refreshedSystemConfig?: RpcClient) => Promise<void>> {
  const targets = new Set(input.catalogs.map((catalog) => `${catalog.catalog_kind}:${catalog.catalog_id}`));
  const kinds = [
    ["models", "model_drivers", "model_driver", "model_driver_id"],
    ["providers", "provider_rules", "provider_rules", "provider_profile_id"],
    ["known-providers", "known_providers", "known_provider", "catalog_id"],
  ] as const;
  const backup = await readOptional(input.systemConfig, METADATA_KEY);
  let patched = backup;
  if (backup !== null) {
    const document = JSON.parse(backup) as Record<string, unknown>;
    for (const [, field, kind, idField] of kinds) {
      if (document[field] === undefined) continue;
      if (!Array.isArray(document[field])) throw new Error(`${METADATA_KEY}.${field} must be an array`);
      document[field] = (document[field] as Array<Record<string, unknown>>).filter((catalog) =>
        !targets.has(`${kind}:${catalog[idField]}`)
      );
    }
    const original = JSON.parse(backup);
    if (JSON.stringify(document) !== JSON.stringify(original)) patched = JSON.stringify(document);
  }
  const candidates: Array<{ original: string; hidden: string }> = [];
  for (const [directory, , kind, idField] of kinds) {
    const folder = join(input.localRoot, directory);
    const entries = await readdir(folder, { withFileTypes: true }).catch((error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") return [];
      throw error;
    });
    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      if (!entry.isFile() || !entry.name.endsWith(".json")) throw new Error(`invalid local metadata entry: ${join(folder, entry.name)}`);
      const original = join(folder, entry.name);
      const catalog = JSON.parse(await readFile(original, "utf8")) as Record<string, unknown>;
      if (targets.has(`${kind}:${catalog[idField]}`)) {
        candidates.push({ original, hidden: join(folder, `.aicc-cloud-${randomUUID()}-${entry.name}`) });
      }
    }
  }
  const moved: typeof candidates = [];
  let configChanged = false;
  const restore = async (refreshedSystemConfig = input.systemConfig) => {
    const errors: unknown[] = [];
    for (const file of [...moved].reverse()) {
      try {
        const existing = await lstat(file.original).catch((error: NodeJS.ErrnoException) => {
          if (error.code === "ENOENT") return null;
          throw error;
        });
        if (existing) throw new Error(`cannot restore local metadata over an existing file: ${file.original}; backup: ${file.hidden}`);
        await rename(file.hidden, file.original);
        moved.splice(moved.indexOf(file), 1);
      } catch (error) {
        errors.push(error);
      }
    }
    if (configChanged) {
      try {
        await refreshedSystemConfig.call("sys_config_set", { key: METADATA_KEY, value: backup });
        configChanged = false;
      } catch (error) {
        errors.push(error);
      }
    }
    if (errors.length) throw new AggregateError(errors, "cloud metadata override restoration failed");
  };
  try {
    for (const file of candidates) {
      await rename(file.original, file.hidden);
      moved.push(file);
    }
    if (patched !== backup) {
      configChanged = true;
      await input.systemConfig.call("sys_config_set", { key: METADATA_KEY, value: patched });
    }
    return restore;
  } catch (error) {
    try {
      await restore();
    } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], "cloud metadata override isolation and rollback failed");
    }
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
      try {
        await client.call("sys_config_delete", { key: CLOUD_CONFIG_KEY });
      } catch (error) {
        if (!/not.?found|key.?not.?found/i.test(String(error))) throw error;
      }
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
