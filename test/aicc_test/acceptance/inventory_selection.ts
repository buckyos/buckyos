import type { ProviderInventory } from "./types.ts";

export function selectSingleProviderInstances(input: {
  inventories: ProviderInventory[];
  drivers: string[];
  configured: Record<string, string>;
}): ProviderInventory[] {
  const selected: ProviderInventory[] = [];
  for (const driver of input.drivers) {
    const candidates = input.inventories.filter((item) => item.provider_driver === driver);
    const configured = input.configured[driver];
    if (configured) {
      const match = candidates.find((item) => item.provider_instance_name === configured);
      if (!match) {
        throw new Error(`configured T2 instance ${configured} was not found for provider ${driver}`);
      }
      selected.push(match);
      continue;
    }
    if (candidates.length > 1) {
      throw new Error(
        `provider ${driver} has multiple instances (${candidates.map((item) => item.provider_instance_name).join(", ")}); configure instances.${driver}.name or --provider-instance ${driver}:<name>`,
      );
    }
    if (candidates.length === 1) selected.push(candidates[0]);
  }
  return selected;
}
