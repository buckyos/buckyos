import type { ProviderInventory, ProviderModel } from "./types.ts";

const DRIVER_BY_PROFILE: Record<string, string> = {
  gemini: "google-gemini",
  sn: "sn-ai-provider",
};

type ModelView = ProviderModel & {
  provider_instance_name?: unknown;
  provider_profile_id?: unknown;
  inventory_revision?: unknown;
};

export function inventoriesFromModelsList(value: unknown): ProviderInventory[] {
  const models = value && typeof value === "object" && !Array.isArray(value)
    ? (value as { models?: unknown }).models
    : undefined;
  if (!Array.isArray(models)) throw new Error("models.list.models must be an array");
  const grouped = new Map<string, ProviderInventory>();
  for (const raw of models) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      throw new Error("models.list.models entries must be objects");
    }
    const model = raw as ModelView;
    if (typeof model.provider_instance_name !== "string" || !model.provider_instance_name) {
      throw new Error("models.list model is missing provider_instance_name");
    }
    if (typeof model.provider_profile_id !== "string" || !model.provider_profile_id) {
      throw new Error("models.list model is missing provider_profile_id");
    }
    let inventory = grouped.get(model.provider_instance_name);
    if (!inventory) {
      inventory = {
        provider_instance_name: model.provider_instance_name,
        provider_driver: DRIVER_BY_PROFILE[model.provider_profile_id] ?? model.provider_profile_id,
        inventory_revision: typeof model.inventory_revision === "string" ? model.inventory_revision : undefined,
        models: [],
      };
      grouped.set(model.provider_instance_name, inventory);
    }
    inventory.models.push(model);
  }
  return [...grouped.values()];
}
