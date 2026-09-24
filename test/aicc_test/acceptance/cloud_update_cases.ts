import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type {
  CloudCatalogFile,
  CloudCatalogTombstone,
} from "./cloud_update_fixture_service.ts";

const here = dirname(fileURLToPath(import.meta.url));
const metadataRoot = join(here, "../../../src/frame/aicc/driver_metadata");

async function json(path: string): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(path, "utf8")) as Record<string, unknown>;
}

function objects(
  value: unknown,
  field: string,
): Array<Record<string, unknown>> {
  if (!Array.isArray(value)) throw new Error(`${field} must be an array`);
  return value.map((item) => {
    if (!item || typeof item !== "object" || Array.isArray(item)) {
      throw new Error(`${field} entry must be an object`);
    }
    return item as Record<string, unknown>;
  });
}

export const CLOUD_TEST_PROFILE_ID = "aicc-cloud-update-openai";
export const CLOUD_TEST_RULES_ID = "aicc-cloud-update-openai";
export const CLOUD_TEST_MOUNT_V1 = "llm.aicc-cloud-update.v1";
export const CLOUD_TEST_MOUNT_V2 = "llm.aicc-cloud-update.v2";

async function openAiModelDriver(
  revisionSeq: number,
  phase: "v1" | "v2",
): Promise<Record<string, unknown>> {
  const catalog = await json(join(metadataRoot, "models/openai.model.json"));
  catalog.revision_seq = revisionSeq;
  const models = objects(catalog.models, "openai.models");
  const gpt = models.find((model) => model.id === "gpt-5.6");
  if (!gpt || !Array.isArray(gpt.logical_mounts)) {
    throw new Error("builtin OpenAI catalog has no gpt-5.6 logical mounts");
  }
  gpt.logical_mounts = [
    ...gpt.logical_mounts.filter((mount) =>
      mount !== CLOUD_TEST_MOUNT_V1 && mount !== CLOUD_TEST_MOUNT_V2
    ),
    phase === "v1" ? CLOUD_TEST_MOUNT_V1 : CLOUD_TEST_MOUNT_V2,
  ];
  catalog.models = phase === "v1"
    ? models.filter((model) => model.id !== "text-embedding-3-small")
    : models;
  return catalog;
}

async function cloudProviderRules(
  revisionSeq: number,
  _marker: "v1" | "v2",
): Promise<Record<string, unknown>> {
  const openAiRules = await json(
    join(metadataRoot, "providers/openai.provider.json"),
  );
  return {
    format: "buckyos.aicc.provider-rules-catalog",
    schema_version: 1,
    schema_revision: 0,
    revision_seq: revisionSeq,
    provider_profile_id: CLOUD_TEST_PROFILE_ID,
    metadata_drivers: ["openai"],
    origin_provider_aliases: {},
    origin_mappings: [],
    models: [],
    patterns: objects(openAiRules.patterns, "openai.provider.patterns"),
    variants: objects(openAiRules.variants, "openai.provider.variants"),
  };
}

async function cloudKnownProvider(
  revisionSeq: number,
  marker: "v1" | "v2",
): Promise<Record<string, unknown>> {
  const openAi = await json(
    join(metadataRoot, "known-providers/openai.known-provider.json"),
  );
  const provider = structuredClone(
    objects(openAi.providers, "openai.known.providers")[0],
  );
  provider.provider_profile_id = CLOUD_TEST_PROFILE_ID;
  provider.provider_rules_id = CLOUD_TEST_RULES_ID;
  provider.display_name = `AICC Cloud Update ${marker.toUpperCase()}`;
  return {
    format: "buckyos.aicc.known-provider-catalog",
    schema_version: 1,
    schema_revision: 0,
    revision_seq: revisionSeq,
    catalog_id: CLOUD_TEST_PROFILE_ID,
    providers: [provider],
  };
}

export async function buildCloudUpdateFiles(
  revisionSeq: number,
  phase: "v1" | "v2",
): Promise<CloudCatalogFile[]> {
  return [
    {
      catalog_kind: "model_driver",
      catalog_id: "openai",
      revision_seq: revisionSeq,
      contents: await openAiModelDriver(revisionSeq, phase),
    },
    {
      catalog_kind: "provider_rules",
      catalog_id: CLOUD_TEST_RULES_ID,
      revision_seq: revisionSeq,
      contents: await cloudProviderRules(revisionSeq, phase),
    },
    {
      catalog_kind: "known_provider",
      catalog_id: CLOUD_TEST_PROFILE_ID,
      revision_seq: revisionSeq,
      contents: await cloudKnownProvider(revisionSeq, phase),
    },
  ];
}

export function cloudUpdateTombstones(
  revisionSeq: number,
): CloudCatalogTombstone[] {
  return [
    {
      catalog_kind: "model_driver",
      catalog_id: "openai",
      revision_seq: revisionSeq,
    },
    {
      catalog_kind: "provider_rules",
      catalog_id: CLOUD_TEST_RULES_ID,
      revision_seq: revisionSeq,
    },
    {
      catalog_kind: "known_provider",
      catalog_id: CLOUD_TEST_PROFILE_ID,
      revision_seq: revisionSeq,
    },
  ];
}

export async function buildT15OpenAiRules(
  revisionSeq: number,
): Promise<CloudCatalogFile[]> {
  const rules = await json(
    join(metadataRoot, "providers/openai.provider.json"),
  );
  rules.revision_seq = revisionSeq;
  const models = objects(rules.models, "openai.provider.models");
  const basePattern = objects(rules.patterns, "openai.provider.patterns").find((
    pattern,
  ) => pattern.match === "gpt-5*");
  if (!basePattern) {
    throw new Error("builtin OpenAI rules have no gpt-5* pattern");
  }
  const { match: _match, ...baseAction } = structuredClone(basePattern);
  const baseOptions = baseAction.provider_options &&
      typeof baseAction.provider_options === "object" &&
      !Array.isArray(baseAction.provider_options)
    ? baseAction.provider_options as Record<string, unknown>
    : {};
  models.unshift({
    id: "gpt-5.6",
    ...baseAction,
    provider_options: { ...baseOptions, service_tier: "default" },
  });
  rules.models = models;
  return [{
    catalog_kind: "provider_rules",
    catalog_id: "openai",
    revision_seq: revisionSeq,
    contents: rules,
  }];
}

export function t15OpenAiTombstone(
  revisionSeq: number,
): CloudCatalogTombstone[] {
  return [{
    catalog_kind: "provider_rules",
    catalog_id: "openai",
    revision_seq: revisionSeq,
  }];
}
