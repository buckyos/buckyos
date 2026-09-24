import { CANONICAL_API_TYPES, methodsForApiType } from "./canonical.ts";
import type { RpcClient } from "./gateway.ts";
import type { ProviderInventory } from "./types.ts";

function quotaKeys(input: {
  userId: string;
  appId: string;
  inventories: readonly ProviderInventory[];
}): string[] {
  const providers = [...new Set(input.inventories.map((item) => item.provider_instance_name))];
  return CANONICAL_API_TYPES.flatMap((apiType) =>
    methodsForApiType(apiType).flatMap((method) =>
      providers.map((provider) =>
        [
          "services/aicc/quota",
          input.userId,
          input.userId,
          input.appId,
          apiType.split(".", 1)[0],
          method,
          provider,
        ].join("/")
      )
    )
  );
}

async function batches<T>(
  values: readonly T[],
  execute: (value: T) => Promise<void>,
): Promise<void> {
  for (let index = 0; index < values.length; index += 12) {
    await Promise.all(values.slice(index, index + 12).map(execute));
  }
}

export async function withMockQuotaTruth<T>(input: {
  systemConfig: RpcClient;
  userId: string;
  appId: string;
  inventories: readonly ProviderInventory[];
  execute: () => Promise<T>;
}): Promise<T> {
  const keys = quotaKeys(input);
  const now = Date.now();
  const value = JSON.stringify({
    period_start_ms: now - 60_000,
    period_end_ms: now + 3_600_000,
    max_request_units: 1_000_000,
    max_cost: null,
    reset_at: new Date(now + 3_600_000).toISOString(),
  });
  const created: string[] = [];
  try {
    await batches(keys, async (key) => {
      await input.systemConfig.call("sys_config_set", { key, value });
      created.push(key);
    });
    return await input.execute();
  } finally {
    await batches(created, async (key) => {
      await input.systemConfig.call("sys_config_delete", { key });
    });
  }
}
