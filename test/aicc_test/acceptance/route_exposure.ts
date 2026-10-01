import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { CANONICAL_API_TYPES } from "./canonical.ts";
import type {
  ProviderBaseline,
  ProviderInventory,
  RouteExposureContract,
  RouteExposureDeclaration,
} from "./types.ts";

const here = dirname(fileURLToPath(import.meta.url));

function object(value: unknown, field: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function string(value: unknown, field: string): string {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return value;
}

function strings(value: unknown, field: string): string[] {
  if (
    !Array.isArray(value) ||
    value.some((item) => typeof item !== "string" || !item)
  ) {
    throw new Error(`${field} must be a non-empty string array`);
  }
  return value as string[];
}

function declaration(value: unknown, field: string): RouteExposureDeclaration {
  const raw = object(value, field);
  const mode = string(raw.mode, `${field}.mode`);
  if (
    !["logical_routable", "exact_only", "excluded", "not_applicable"].includes(
      mode,
    )
  ) {
    throw new Error(`${field}.mode is invalid`);
  }
  const logicalEntrypoint = raw.logical_entrypoint === undefined
    ? undefined
    : string(raw.logical_entrypoint, `${field}.logical_entrypoint`);
  const reason = raw.reason === undefined
    ? undefined
    : string(raw.reason, `${field}.reason`);
  if (mode === "logical_routable") {
    if (!logicalEntrypoint) {
      throw new Error(`${field}.logical_entrypoint is required`);
    }
    if (reason) {
      throw new Error(`${field}.reason is not valid for logical_routable`);
    }
  } else {
    if (logicalEntrypoint) {
      throw new Error(
        `${field}.logical_entrypoint is only valid for logical_routable`,
      );
    }
    if (!reason) throw new Error(`${field}.reason is required for ${mode}`);
  }
  return {
    mode: mode as RouteExposureDeclaration["mode"],
    logical_entrypoint: logicalEntrypoint,
    reason,
  };
}

export async function loadRouteExposureContract(): Promise<
  RouteExposureContract
> {
  return validateRouteExposureContract(JSON.parse(
    await readFile(join(here, "route_exposure_contracts.json"), "utf8"),
  ));
}

export function validateRouteExposureContract(
  value: unknown,
): RouteExposureContract {
  const raw = object(value, "route exposure contract");
  if (raw.schema_version !== 1) {
    throw new Error("unsupported route exposure schema_version");
  }
  string(raw.contract_revision, "contract_revision");
  string(raw.capability_baseline_revision, "capability_baseline_revision");
  const capabilityCellsSha256 = string(
    raw.capability_cells_sha256,
    "capability_cells_sha256",
  );
  if (!/^[0-9a-f]{64}$/.test(capabilityCellsSha256)) {
    throw new Error(
      "capability_cells_sha256 must be a lowercase SHA-256 digest",
    );
  }
  if (!Array.isArray(raw.profiles)) {
    throw new Error("route exposure profiles must be an array");
  }
  const profiles = raw.profiles.map((value, index) => {
    const profile = object(value, `profiles[${index}]`);
    const providerDriver = string(
      profile.provider_driver,
      `profiles[${index}].provider_driver`,
    );
    const providerProfileId = string(
      profile.provider_profile_id,
      `profiles[${index}].provider_profile_id`,
    );
    const patterns = strings(
      profile.covered_model_patterns,
      `${providerDriver}.covered_model_patterns`,
    );
    if (new Set(patterns).size !== patterns.length) {
      throw new Error(
        `${providerDriver}.covered_model_patterns contains duplicates`,
      );
    }
    const defaultExposure = declaration(
      profile.default_exposure,
      `${providerDriver}.default_exposure`,
    );
    if (!Array.isArray(profile.overrides)) {
      throw new Error(`${providerDriver}.overrides must be an array`);
    }
    const keys = new Set<string>();
    const overrides = profile.overrides.map((value, overrideIndex) => {
      const override = object(
        value,
        `${providerDriver}.overrides[${overrideIndex}]`,
      );
      const modelPattern = string(
        override.model_pattern,
        `${providerDriver}.override.model_pattern`,
      );
      const apiType = string(
        override.api_type,
        `${providerDriver}.override.api_type`,
      );
      const key = `${modelPattern}\u0000${apiType}`;
      if (keys.has(key)) {
        throw new Error(
          `${providerDriver} has duplicate override ${modelPattern}/${apiType}`,
        );
      }
      keys.add(key);
      return {
        model_pattern: modelPattern,
        api_type: apiType,
        ...declaration(
          override,
          `${providerDriver}.${modelPattern}.${apiType}`,
        ),
      };
    });
    return {
      provider_driver: providerDriver,
      provider_profile_id: providerProfileId,
      covered_model_patterns: patterns,
      default_exposure: defaultExposure,
      overrides,
    };
  });
  return {
    schema_version: 1,
    contract_revision: raw.contract_revision as string,
    capability_baseline_revision: raw.capability_baseline_revision as string,
    capability_cells_sha256: capabilityCellsSha256,
    profiles,
  };
}

function expandEntrypoint(template: string, apiType: string): string {
  return template.replaceAll("{api_type}", apiType);
}

export function exposureFor(
  contract: RouteExposureContract,
  providerDriver: string,
  modelPattern: string,
  apiType: string,
): RouteExposureDeclaration {
  const profile = contract.profiles.find((item) =>
    item.provider_driver === providerDriver
  );
  if (!profile || !profile.covered_model_patterns.includes(modelPattern)) {
    throw new Error(
      `unclassified route exposure ${providerDriver}/${modelPattern}/${apiType}`,
    );
  }
  const value =
    profile.overrides.find((item) =>
      item.model_pattern === modelPattern && item.api_type === apiType
    ) ?? profile.default_exposure;
  return value.logical_entrypoint
    ? {
      ...value,
      logical_entrypoint: expandEntrypoint(value.logical_entrypoint, apiType),
    }
    : value;
}

export type RouteExposureSummary = {
  logical_routable: number;
  exact_only: number;
  excluded: number;
  not_applicable: number;
  unclassified: number;
  by_provider_api: Record<
    string,
    Omit<RouteExposureSummary, "by_provider_api">
  >;
};

function capabilityCellsSha256(baseline: ProviderBaseline): string {
  const cells = baseline.providers.flatMap((provider) =>
    provider.rules.flatMap((rule) =>
      rule.api_types.map((apiType) =>
        [
          provider.provider_driver,
          provider.provider_profile_id,
          rule.model_pattern,
          apiType,
        ].join("\u0000")
      )
    )
  ).sort();
  return createHash("sha256").update(cells.join("\n")).digest("hex");
}

export function assertRouteExposureCompleteness(
  baseline: ProviderBaseline,
  contract: RouteExposureContract,
): RouteExposureSummary {
  if (contract.capability_baseline_revision !== baseline.baseline_revision) {
    throw new Error(
      `route exposure baseline revision ${contract.capability_baseline_revision} differs from ${baseline.baseline_revision}`,
    );
  }
  const cellsDigest = capabilityCellsSha256(baseline);
  if (contract.capability_cells_sha256 !== cellsDigest) {
    throw new Error(
      `route exposure capability cell snapshot differs: expected ${contract.capability_cells_sha256}, found ${cellsDigest}`,
    );
  }
  const expectedProfiles = new Map(
    baseline.providers.map((provider) => [provider.provider_driver, provider]),
  );
  const actualProfiles = new Map<
    string,
    RouteExposureContract["profiles"][number]
  >();
  for (const profile of contract.profiles) {
    if (actualProfiles.has(profile.provider_driver)) {
      throw new Error(
        `duplicate route exposure profile ${profile.provider_driver}`,
      );
    }
    actualProfiles.set(profile.provider_driver, profile);
  }
  const missingProfiles = [...expectedProfiles.keys()].filter((driver) =>
    !actualProfiles.has(driver)
  );
  const unknownProfiles = [...actualProfiles.keys()].filter((driver) =>
    !expectedProfiles.has(driver)
  );
  if (missingProfiles.length || unknownProfiles.length) {
    throw new Error(
      `route exposure profile mismatch: missing=${
        missingProfiles.join(",")
      } unknown=${unknownProfiles.join(",")}`,
    );
  }
  const summary: RouteExposureSummary = {
    logical_routable: 0,
    exact_only: 0,
    excluded: 0,
    not_applicable: 0,
    unclassified: 0,
    by_provider_api: {},
  };
  for (const provider of baseline.providers) {
    const profile = actualProfiles.get(provider.provider_driver)!;
    if (profile.provider_profile_id !== provider.provider_profile_id) {
      throw new Error(
        `${provider.provider_driver} route exposure provider_profile_id differs from baseline`,
      );
    }
    const baselinePatterns = provider.rules.map((rule) => rule.model_pattern);
    const missingPatterns = baselinePatterns.filter((pattern) =>
      !profile.covered_model_patterns.includes(pattern)
    );
    const unknownPatterns = profile.covered_model_patterns.filter((pattern) =>
      !baselinePatterns.includes(pattern)
    );
    if (missingPatterns.length || unknownPatterns.length) {
      throw new Error(
        `${provider.provider_driver} route exposure model rules mismatch: missing=${
          missingPatterns.join(",")
        } unknown=${unknownPatterns.join(",")}`,
      );
    }
    for (const override of profile.overrides) {
      const rule = provider.rules.find((item) =>
        item.model_pattern === override.model_pattern
      );
      if (!rule || !rule.api_types.includes(override.api_type)) {
        throw new Error(
          `${provider.provider_driver} route exposure override references unknown cell ${override.model_pattern}/${override.api_type}`,
        );
      }
    }
    for (const rule of provider.rules) {
      for (const apiType of rule.api_types) {
        const exposure = exposureFor(
          contract,
          provider.provider_driver,
          rule.model_pattern,
          apiType,
        );
        if (exposure.mode === "logical_routable") {
          const entrypoint = exposure.logical_entrypoint!;
          if (entrypoint !== apiType && !entrypoint.startsWith(`${apiType}.`)) {
            throw new Error(
              `${provider.provider_driver}/${rule.model_pattern}/${apiType} has cross-API logical entrypoint ${entrypoint}`,
            );
          }
        }
        if (
          (rule.status === "active" || rule.status === "preview") &&
          (exposure.mode === "excluded" || exposure.mode === "not_applicable")
        ) {
          throw new Error(
            `${provider.provider_driver}/${rule.model_pattern}/${apiType} is executable but declared ${exposure.mode}`,
          );
        }
        summary[exposure.mode] += 1;
        const aggregateKey = `${provider.provider_profile_id}/${apiType}`;
        const aggregate = summary.by_provider_api[aggregateKey] ?? {
          logical_routable: 0,
          exact_only: 0,
          excluded: 0,
          not_applicable: 0,
          unclassified: 0,
        };
        aggregate[exposure.mode] += 1;
        summary.by_provider_api[aggregateKey] = aggregate;
      }
    }
  }
  if (summary.unclassified !== 0) {
    throw new Error("route exposure contract contains unclassified cells");
  }
  return summary;
}

function globMatches(pattern: string, value: string): boolean {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(
    /\*/g,
    ".*",
  );
  return new RegExp(`^${escaped}$`, "i").test(value);
}

export type RouteExposureRuntimeCell = {
  provider_driver: string;
  provider_profile_id: string;
  provider_instance: string;
  model_pattern: string;
  api_type: string;
  exposure: RouteExposureDeclaration;
  exact_model: string;
  logical_mounts: string[];
};

export function buildRouteExposureRuntimeCells(args: {
  baseline: ProviderBaseline;
  contract: RouteExposureContract;
  inventories: ProviderInventory[];
}): RouteExposureRuntimeCell[] {
  const cells = new Map<string, RouteExposureRuntimeCell>();
  for (const inventory of args.inventories) {
    const provider = args.baseline.providers.find((item) =>
      item.provider_profile_id === inventory.provider_profile_id ||
      (!inventory.provider_profile_id &&
        item.provider_driver === inventory.provider_driver)
    );
    if (!provider) continue;
    const capabilityProvider = provider.capability_source_provider
      ? args.baseline.providers.find((item) =>
        item.provider_driver === provider.capability_source_provider
      )
      : provider;
    if (!capabilityProvider) {
      throw new Error(
        `missing capability source for ${provider.provider_driver}`,
      );
    }
    if (provider.capability_source_provider) continue;
    for (const model of inventory.models) {
      const rule = capabilityProvider.rules.find((item) =>
        globMatches(item.model_pattern, model.provider_model_id)
      );
      if (!rule) {
        throw new Error(
          `unclassified runtime model ${provider.provider_driver}/${model.provider_model_id}`,
        );
      }
      for (const apiType of model.api_types) {
        if (!(CANONICAL_API_TYPES as readonly string[]).includes(apiType)) {
          continue;
        }
        if (!rule.api_types.includes(apiType)) {
          throw new Error(
            `unclassified runtime API ${provider.provider_driver}/${model.provider_model_id}/${apiType}`,
          );
        }
        const modelPattern = rule.model_pattern;
        const exposure = exposureFor(
          args.contract,
          provider.provider_driver,
          modelPattern,
          apiType,
        );
        const key =
          `${provider.provider_driver}\u0000${modelPattern}\u0000${apiType}\u0000${inventory.provider_instance_name}`;
        if (!cells.has(key)) {
          cells.set(key, {
            provider_driver: provider.provider_driver,
            provider_profile_id: provider.provider_profile_id,
            provider_instance: inventory.provider_instance_name,
            model_pattern: modelPattern,
            api_type: apiType,
            exposure,
            exact_model: model.exact_model,
            logical_mounts: model.logical_mounts,
          });
        }
      }
    }
  }
  return [...cells.values()];
}

export function assertExactOnlyIsUnmounted(
  cell: RouteExposureRuntimeCell,
): void {
  if (cell.exposure.mode !== "exact_only") return;
  const mounts = cell.logical_mounts.filter((mount) =>
    mount === cell.api_type || mount.startsWith(`${cell.api_type}.`)
  );
  if (mounts.length) {
    throw new Error(
      `exact_only cell ${cell.provider_driver}/${cell.model_pattern}/${cell.api_type} has logical mounts: ${
        mounts.join(", ")
      }`,
    );
  }
}
