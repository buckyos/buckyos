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

function representativeRank(model: ProviderModel, physicalModelId: string): string {
  const exactPhysical = model.provider_model_id.toLowerCase() === physicalModelId.toLowerCase() ? "0" : "1";
  const datedOrVariant = /:\w+|(?:^|-)\d{4}(?:-|$)|(?:^|-)latest$/i.test(model.provider_model_id) ? "1" : "0";
  return `${exactPhysical}${datedOrVariant}:${model.provider_model_id.toLowerCase()}:${model.exact_model.toLowerCase()}`;
}

export function filterPhysicalModels(args: {
  baseline: ProviderBaseline;
  inventories: ProviderInventory[];
}): { inventories: ProviderInventory[]; coverage: ModelCoverageRecord[] } {
  const profiles = new Map(args.baseline.providers.map((profile) => [profile.provider_driver, profile]));
  const coverage: ModelCoverageRecord[] = [];
  const inventories = args.inventories.map((inventory) => {
    const profile = profiles.get(inventory.provider_driver);
    const capabilityProfile = profile?.capability_source_provider
      ? profiles.get(profile.capability_source_provider)
      : profile;
    const candidates: Array<{
      model: ProviderModel;
      physicalModelId: string;
      sourceUrls: string[];
      evidenceSummary: string;
    }> = [];
    for (const model of inventory.models) {
      const coverageRule = capabilityProfile?.coverage_rules?.find((rule) =>
        globMatches(rule.model_pattern, model.provider_model_id)
      );
      const capabilityRule = capabilityProfile?.rules.find((rule) =>
        globMatches(rule.model_pattern, model.provider_model_id)
      );
      const isVariant = model.provider_model_id.includes(":") ||
        Boolean(model.provider_options && Object.keys(model.provider_options).length > 0);
      const physicalModelId = coverageRule?.action === "alias"
        ? coverageRule.physical_model_id!
        : isVariant
        ? model.provider_model_id
        : model.provider_actual_model_id ?? model.provider_model_id;
      if (coverageRule?.action === "exclude") {
        coverage.push({
          provider_driver: inventory.provider_driver,
          provider_instance: inventory.provider_instance_name,
          exact_model: model.exact_model,
          provider_model_id: model.provider_model_id,
          provider_actual_model_id: model.provider_actual_model_id,
          physical_model_id: physicalModelId,
          status: "filtered",
          reason: coverageRule.reason,
          source_urls: coverageRule.source_urls,
          evidence_summary: coverageRule.evidence_summary,
        });
        continue;
      }
      if (capabilityRule?.status === "deprecated" || capabilityRule?.status === "removed") {
        coverage.push({
          provider_driver: inventory.provider_driver,
          provider_instance: inventory.provider_instance_name,
          exact_model: model.exact_model,
          provider_model_id: model.provider_model_id,
          provider_actual_model_id: model.provider_actual_model_id,
          physical_model_id: physicalModelId,
          status: "filtered",
          reason: "deprecated_or_retiring",
          source_urls: capabilityRule.source_urls,
          evidence_summary: `Capability baseline status is ${capabilityRule.status}.`,
        });
        continue;
      }
      candidates.push({
        model,
        physicalModelId,
        sourceUrls: coverageRule?.source_urls ?? capabilityRule?.source_urls ?? [],
        evidenceSummary: coverageRule?.evidence_summary ?? "AICC inventory exposes this physical model.",
      });
    }
    const retained: ProviderModel[] = [];
    const groups = new Map<string, typeof candidates>();
    for (const candidate of candidates) {
      const key = candidate.physicalModelId.toLowerCase();
      const values = groups.get(key) ?? [];
      values.push(candidate);
      groups.set(key, values);
    }
    for (const group of groups.values()) {
      group.sort((left, right) =>
        representativeRank(left.model, left.physicalModelId).localeCompare(
          representativeRank(right.model, right.physicalModelId),
        )
      );
      const representative = group[0];
      retained.push(representative.model);
      coverage.push({
        provider_driver: inventory.provider_driver,
        provider_instance: inventory.provider_instance_name,
        exact_model: representative.model.exact_model,
        provider_model_id: representative.model.provider_model_id,
        provider_actual_model_id: representative.model.provider_actual_model_id,
        physical_model_id: representative.physicalModelId,
        status: "included",
        source_urls: representative.sourceUrls,
        evidence_summary: representative.evidenceSummary,
      });
      for (const duplicate of group.slice(1)) {
        coverage.push({
          provider_driver: inventory.provider_driver,
          provider_instance: inventory.provider_instance_name,
          exact_model: duplicate.model.exact_model,
          provider_model_id: duplicate.model.provider_model_id,
          provider_actual_model_id: duplicate.model.provider_actual_model_id,
          physical_model_id: duplicate.physicalModelId,
          status: "filtered",
          reason: "duplicate_physical_model",
          retained_exact_model: representative.model.exact_model,
          source_urls: duplicate.sourceUrls,
          evidence_summary: `Same physical model as retained representative ${representative.model.exact_model}.`,
        });
      }
    }
    retained.sort((left, right) => left.exact_model.localeCompare(right.exact_model));
    return { ...inventory, models: retained };
  });
  coverage.sort((left, right) =>
    `${left.provider_driver}/${left.provider_instance}/${left.provider_model_id}/${left.exact_model}`.localeCompare(
      `${right.provider_driver}/${right.provider_instance}/${right.provider_model_id}/${right.exact_model}`,
    )
  );
  return { inventories, coverage };
}
