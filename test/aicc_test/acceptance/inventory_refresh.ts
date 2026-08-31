import type { RpcClient } from "./gateway.ts";
import type { ProviderInventory } from "./types.ts";

export type ProviderInventoryRefreshEvidence = {
  provider_driver: string;
  provider_instance_name: string;
  attempt_count: number;
  before_inventory_revision?: string;
  refresh_inventory_revision?: string;
  after_inventory_revision?: string;
};

function object(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export async function refreshProviderInventoriesUntilSuccess(input: {
  aicc: RpcClient;
  selectedInventories: ProviderInventory[];
  readInventories: () => Promise<ProviderInventory[]>;
  maxAttempts: number;
  retryDelayMs: number;
  providerRetryDelayMs?: Record<string, number>;
  sleep?: (delayMs: number) => Promise<void>;
}): Promise<{
  inventories: ProviderInventory[];
  evidence: ProviderInventoryRefreshEvidence[];
}> {
  if (!Number.isInteger(input.maxAttempts) || input.maxAttempts < 1) {
    throw new Error("inventory refresh maxAttempts must be a positive integer");
  }
  if (!Number.isFinite(input.retryDelayMs) || input.retryDelayMs < 0) {
    throw new Error("inventory refresh retryDelayMs must be non-negative");
  }
  const sleep = input.sleep ?? ((delayMs: number) =>
    new Promise((resolve) => setTimeout(resolve, delayMs)));
  const refreshed = await Promise.all(input.selectedInventories.map(async (inventory) => {
    const baseDelayMs = Math.max(
      input.retryDelayMs,
      input.providerRetryDelayMs?.[inventory.provider_driver] ?? 0,
    );
    for (let attempt = 1; attempt <= input.maxAttempts; attempt += 1) {
      try {
        const raw = object(await input.aicc.call("provider.refresh_models", {
          provider_instance_name: inventory.provider_instance_name,
        }));
        if (!raw || raw.ok !== true) throw new Error("refresh rejected");
        const returnedInstance = optionalString(raw.provider_instance_name);
        if (returnedInstance !== inventory.provider_instance_name) {
          throw new Error("refresh returned unexpected instance");
        }
        return {
          provider_driver: inventory.provider_driver,
          provider_instance_name: inventory.provider_instance_name,
          attempt_count: attempt,
          before_inventory_revision: optionalString(inventory.inventory_revision),
          refresh_inventory_revision: optionalString(raw.inventory_revision),
        };
      } catch {
        if (attempt === input.maxAttempts) {
          throw new Error(
            `provider inventory refresh failed after ${attempt} attempt(s) for ${inventory.provider_driver}/${inventory.provider_instance_name}`,
          );
        }
        const delayMs = Math.min(baseDelayMs * 2 ** (attempt - 1), 30_000);
        if (delayMs > 0) await sleep(delayMs);
      }
    }
    throw new Error("provider inventory refresh exhausted unexpectedly");
  }));
  const inventories = await input.readInventories();
  const evidence = refreshed.map((item) => {
    const after = inventories.find((inventory) =>
      inventory.provider_driver === item.provider_driver &&
      inventory.provider_instance_name === item.provider_instance_name
    );
    if (!after) {
      throw new Error(
        `refreshed provider inventory is absent from models.list for ${item.provider_driver}/${item.provider_instance_name}`,
      );
    }
    return {
      ...item,
      after_inventory_revision: optionalString(after.inventory_revision),
    };
  });
  return { inventories, evidence };
}
