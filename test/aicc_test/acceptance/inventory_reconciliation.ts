import { filterPhysicalModels } from "./model_coverage.ts";
import type {
  ModelCoverageRecord,
  ProviderBaseline,
  ProviderInventory,
  ProviderModel,
} from "./types.ts";

function globMatches(pattern: string, value: string): boolean {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*");
  return new RegExp(`^${escaped}$`, "i").test(value);
}

function physicalIds(coverage: ModelCoverageRecord[]): Map<string, string> {
  return new Map(coverage.map((record) => [record.exact_model, record.physical_model_id.toLowerCase()]));
}

function membershipId(model: ProviderModel, filteredPhysicalId: string): string {
  if (model.provider_actual_model_id?.trim()) return model.provider_actual_model_id.trim().toLowerCase();
  if (model.provider_options && Object.keys(model.provider_options).length > 0) {
    return model.provider_model_id.replace(/:[^:]+$/, "").toLowerCase();
  }
  return filteredPhysicalId;
}

export function reconcileOfficialAndAiccInventories(input: {
  baseline: ProviderBaseline;
  officialInventories: ProviderInventory[];
  aiccInventories: ProviderInventory[];
}): {
  inventories: ProviderInventory[];
  coverage: ModelCoverageRecord[];
  mismatches: string[];
} {
  const profiles = new Map(input.baseline.providers.map((profile) => [profile.provider_driver, profile]));
  const official = filterPhysicalModels({
    baseline: input.baseline,
    inventories: input.officialInventories,
    source: "official_catalog",
  });
  const aicc = filterPhysicalModels({
    baseline: input.baseline,
    inventories: input.aiccInventories,
    source: "aicc_inventory",
  });
  const officialPhysical = physicalIds(official.coverage);
  const aiccPhysical = physicalIds(aicc.coverage);
  const mismatches: string[] = [];
  const retained: ProviderInventory[] = [];

  for (const officialInventory of official.inventories) {
    const profile = profiles.get(officialInventory.provider_driver);
    const capabilityProfile = profile?.capability_source_provider
      ? profiles.get(profile.capability_source_provider)
      : profile;
    if (!capabilityProfile) continue;
    const expected = new Map<string, ProviderModel>();
    for (const model of officialInventory.models) {
      const rule = capabilityProfile.rules.find((candidate) =>
        globMatches(candidate.model_pattern, model.provider_model_id)
      );
      const taskTypeApiTypes = model.official_task_types?.flatMap((taskType) =>
        profile?.official_catalog.task_type_api_types?.[taskType] ?? []
      ) ?? [];
      if (!rule && taskTypeApiTypes.length === 0) {
        mismatches.push(
          `missing official capability rule for ${officialInventory.provider_driver}/${model.provider_model_id}`,
        );
        continue;
      }
      expected.set(officialPhysical.get(model.exact_model) ?? model.provider_model_id.toLowerCase(), model);
    }
    const actualInventory = aicc.inventories.find((candidate) =>
      candidate.provider_driver === officialInventory.provider_driver &&
      candidate.provider_instance_name === officialInventory.provider_instance_name
    );
    const actualModels = actualInventory?.models ?? [];
    const actualMembership = new Map<string, ProviderModel[]>();
    for (const model of actualModels) {
      const id = membershipId(
        model,
        aiccPhysical.get(model.exact_model) ?? model.provider_model_id.toLowerCase(),
      );
      const models = actualMembership.get(id) ?? [];
      models.push(model);
      actualMembership.set(id, models);
      if (!expected.has(id)) {
        mismatches.push(
          `aicc_advertised_but_official_missing ${officialInventory.provider_driver}/${model.provider_model_id}`,
        );
      }
    }
    for (const [id, model] of expected) {
      if (!actualMembership.has(id)) {
        mismatches.push(
          `official_supported_but_aicc_missing ${officialInventory.provider_driver}/${model.provider_model_id}`,
        );
      }
    }
    if (actualInventory) {
      retained.push({
        ...actualInventory,
        models: actualModels.flatMap((model) => {
          const id = membershipId(
            model,
            aiccPhysical.get(model.exact_model) ?? model.provider_model_id.toLowerCase(),
          );
          const officialModel = expected.get(id);
          return officialModel
            ? [{
              ...model,
              official_lifecycle_status: officialModel.official_lifecycle_status,
              official_task_types: officialModel.official_task_types,
            }]
            : [];
        }),
      });
    }
  }
  return {
    inventories: retained,
    coverage: [...official.coverage, ...aicc.coverage],
    mismatches,
  };
}
