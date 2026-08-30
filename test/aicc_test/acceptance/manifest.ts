import {
  FAILURE_CLASSES,
  RESULT_STATUSES,
  type AcceptanceCase,
  type CapabilityRule,
  type DocumentFormatCoverageRecord,
  type MatrixCell,
  type ProviderBaseline,
  type ProviderInventory,
} from "./types.ts";
import { CANONICAL_API_TYPES, methodsForApiType } from "./canonical.ts";
import { reconcileOfficialAndAiccInventories } from "./inventory_reconciliation.ts";

const CASE_ID_PATTERN = /^[a-z0-9][a-z0-9._-]*$/;
export const DOCUMENT_FORMAT_CANDIDATES = [
  "txt", "md", "pdf", "doc", "docx", "xls", "xlsx", "csv", "tsv", "ppt", "pptx",
  "html", "xml", "json", "yaml", "rtf", "epub", "source",
] as const;

function isObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return value;
}

function requireStringArray(value: unknown, field: string): string[] {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    throw new Error(`${field} must be a string array`);
  }
  return value as string[];
}

export function validateCaseManifest(value: unknown): AcceptanceCase[] {
  if (!Array.isArray(value)) throw new Error("case manifest must be an array");
  const seen = new Set<string>();
  return value.map((raw, index) => {
    if (!isObject(raw)) throw new Error(`case[${index}] must be an object`);
    const caseId = requireString(raw.case_id, `case[${index}].case_id`);
    if (!CASE_ID_PATTERN.test(caseId)) throw new Error(`invalid case_id ${caseId}`);
    if (seen.has(caseId)) throw new Error(`duplicate case_id ${caseId}`);
    seen.add(caseId);
    if (!(["T1", "T2", "T3"] as unknown[]).includes(raw.layer)) {
      throw new Error(`${caseId}.layer is invalid`);
    }
    if (!(["P0", "P1", "P2"] as unknown[]).includes(raw.priority)) {
      throw new Error(`${caseId}.priority is invalid`);
    }
    requireStringArray(raw.tags, `${caseId}.tags`);
    requireString(raw.input_entry, `${caseId}.input_entry`);
    requireString(raw.user, `${caseId}.user`);
    requireString(raw.session, `${caseId}.session`);
    requireString(raw.method, `${caseId}.method`);
    requireStringArray(raw.required_capabilities, `${caseId}.required_capabilities`);
    requireStringArray(raw.disabled_capabilities, `${caseId}.disabled_capabilities`);
    requireStringArray(raw.fixtures, `${caseId}.fixtures`);
    requireStringArray(raw.semantic_rubric, `${caseId}.semantic_rubric`);
    requireStringArray(raw.cleanup, `${caseId}.cleanup`);
    if (!isObject(raw.expected_output)) {
      throw new Error(`${caseId}.expected_output must be an object`);
    }
    requireStringArray(raw.expected_output.kinds, `${caseId}.expected_output.kinds`);
    requireStringArray(
      raw.expected_output.mime_types,
      `${caseId}.expected_output.mime_types`,
    );
    if (!isObject(raw.expected_output.attachment_count)) {
      throw new Error(`${caseId}.expected_output.attachment_count is required`);
    }
    const min = raw.expected_output.attachment_count.min;
    const max = raw.expected_output.attachment_count.max;
    if (!Number.isInteger(min) || !Number.isInteger(max) || Number(min) < 0 || Number(max) < Number(min)) {
      throw new Error(`${caseId}.expected_output.attachment_count is invalid`);
    }
    if (!Number.isFinite(raw.timeout_ms) || Number(raw.timeout_ms) <= 0) {
      throw new Error(`${caseId}.timeout_ms must be positive`);
    }
    if (!Number.isInteger(raw.max_attempts) || Number(raw.max_attempts) < 1) {
      throw new Error(`${caseId}.max_attempts must be at least one`);
    }
    if (!Number.isFinite(raw.estimated_cost_usd) || Number(raw.estimated_cost_usd) < 0) {
      throw new Error(`${caseId}.estimated_cost_usd must be non-negative`);
    }
    return raw as AcceptanceCase;
  });
}

export function validateProviderBaseline(value: unknown): ProviderBaseline {
  if (!isObject(value)) throw new Error("provider baseline must be an object");
  if (value.schema_version !== 2) throw new Error("unsupported baseline schema_version");
  requireString(value.baseline_revision, "baseline_revision");
  requireString(value.checked_at, "checked_at");
  const canonical = requireStringArray(value.canonical_api_types, "canonical_api_types");
  if (new Set(canonical).size !== canonical.length) {
    throw new Error("canonical_api_types contains duplicates");
  }
  if (!Array.isArray(value.providers)) throw new Error("providers must be an array");
  const drivers = new Set<string>();
  for (const rawProvider of value.providers) {
    if (!isObject(rawProvider)) throw new Error("provider entry must be an object");
    const driver = requireString(rawProvider.provider_driver, "provider_driver");
    if (drivers.has(driver)) throw new Error(`duplicate provider baseline ${driver}`);
    drivers.add(driver);
    if (!isObject(rawProvider.official_catalog)) {
      throw new Error(`${driver}.official_catalog must be an object`);
    }
    requireString(rawProvider.official_catalog.endpoint, `${driver}.official_catalog.endpoint`);
    if (!["openai", "anthropic", "gemini", "fal", "sn"].includes(String(rawProvider.official_catalog.format))) {
      throw new Error(`${driver}.official_catalog.format is invalid`);
    }
    if (!["bearer", "x-api-key", "query-key", "fal-key", "none"].includes(String(rawProvider.official_catalog.authentication))) {
      throw new Error(`${driver}.official_catalog.authentication is invalid`);
    }
    if (rawProvider.official_catalog.page_size !== undefined &&
      (!Number.isInteger(rawProvider.official_catalog.page_size) || Number(rawProvider.official_catalog.page_size) < 1)) {
      throw new Error(`${driver}.official_catalog.page_size must be a positive integer`);
    }
    requireStringArray(rawProvider.source_urls, `${driver}.source_urls`);
    if (!isObject(rawProvider.protocol_evidence)) {
      throw new Error(`${driver}.protocol_evidence must be an object`);
    }
    for (const field of [
      "documentation_version",
      "streaming_semantics",
      "async_operation_semantics",
      "usage_semantics",
      "input_output_formats",
      "item_request_batch_limits",
      "image_audio_video_limits",
      "context_output_limits",
      "region_restrictions",
      "account_tier_restrictions",
      "preview_allowlist",
    ] as const) {
      requireString(rawProvider.protocol_evidence[field], `${driver}.protocol_evidence.${field}`);
    }
    if (rawProvider.coverage_rules !== undefined) {
      if (!Array.isArray(rawProvider.coverage_rules)) {
        throw new Error(`${driver}.coverage_rules must be an array`);
      }
      for (const rule of rawProvider.coverage_rules) {
        if (!isObject(rule)) throw new Error(`${driver}.coverage_rule must be an object`);
        requireString(rule.model_pattern, `${driver}.coverage_rule.model_pattern`);
        if (!["exclude", "alias"].includes(String(rule.action))) {
          throw new Error(`${driver}.coverage_rule.action is invalid`);
        }
        if (!["deprecated_or_retiring", "logical_alias", "not_physical_model", "unsupported_canonical_protocol"].includes(String(rule.reason))) {
          throw new Error(`${driver}.coverage_rule.reason is invalid`);
        }
        if (rule.action === "alias") requireString(rule.physical_model_id, `${driver}.coverage_rule.physical_model_id`);
        requireStringArray(rule.source_urls, `${driver}.coverage_rule.source_urls`);
        requireString(rule.evidence_summary, `${driver}.coverage_rule.evidence_summary`);
      }
    }
    const capabilitySource = rawProvider.capability_source_provider;
    if (capabilitySource !== undefined && typeof capabilitySource !== "string") {
      throw new Error(`${driver}.capability_source_provider must be a string`);
    }
    if (!Array.isArray(rawProvider.rules) ||
      (rawProvider.rules.length === 0 && !capabilitySource)) {
      throw new Error(`${driver}.rules must not be empty without a capability source provider`);
    }
    for (const rule of rawProvider.rules) {
      if (!isObject(rule)) throw new Error(`${driver}.rule must be an object`);
      requireString(rule.model_pattern, `${driver}.model_pattern`);
      if (!["active", "preview", "deprecated", "removed"].includes(String(rule.status))) {
        throw new Error(`${driver}.${String(rule.model_pattern)} has invalid normalized status ${String(rule.status)}`);
      }
      if (rule.source_status !== undefined && typeof rule.source_status !== "string") {
        throw new Error(`${driver}.${String(rule.model_pattern)}.source_status must be a string`);
      }
      requireStringArray(rule.api_types, `${driver}.api_types`);
      requireStringArray(rule.methods, `${driver}.methods`);
      requireStringArray(rule.input_kinds, `${driver}.input_kinds`);
      requireStringArray(rule.output_kinds, `${driver}.output_kinds`);
      if ((rule.input_kinds as string[]).includes("document")) {
        const formats = requireStringArray(rule.document_formats, `${driver}.document_formats`);
        if (formats.length === 0 || new Set(formats).size !== formats.length) {
          throw new Error(`${driver}.document_formats must be non-empty and unique for document input`);
        }
      }
      if (rule.api_io !== undefined) {
        if (!isObject(rule.api_io)) throw new Error(`${driver}.api_io must be an object`);
        for (const [apiType, rawIo] of Object.entries(rule.api_io)) {
          if (!isObject(rawIo)) throw new Error(`${driver}.api_io.${apiType} must be an object`);
          for (const field of ["input_combinations", "output_combinations"] as const) {
            const combinations = rawIo[field];
            if (!Array.isArray(combinations) || combinations.length === 0 ||
              combinations.some((combination) => !Array.isArray(combination) || combination.length === 0 ||
                combination.some((kind) => typeof kind !== "string"))) {
              throw new Error(`${driver}.api_io.${apiType}.${field} must contain non-empty string arrays`);
            }
          }
        }
      }
      requireStringArray(rule.source_urls, `${driver}.rule.source_urls`);
      requireString(rule.evidence_summary, `${driver}.evidence_summary`);
      for (const apiType of rule.api_types as string[]) {
        if (!CANONICAL_API_TYPES.includes(apiType as never)) {
          throw new Error(`${driver} uses unknown api_type ${apiType}`);
        }
      }
    }
  }
  for (const rawProvider of value.providers) {
    const provider = rawProvider as Record<string, unknown>;
    if (typeof provider.capability_source_provider === "string" &&
      !drivers.has(provider.capability_source_provider)) {
      throw new Error(
        `${String(provider.provider_driver)} references missing capability source ${provider.capability_source_provider}`,
      );
    }
  }
  return value as unknown as ProviderBaseline;
}

function globMatches(pattern: string, value: string): boolean {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*");
  return new RegExp(`^${escaped}$`, "i").test(value);
}

function normalizedStatus(
  rule: ProviderBaseline["providers"][number]["rules"][number],
  modelId: string,
): CapabilityRule["status"] {
  if (/(?:^|[-_.])preview(?:$|[-_.])/i.test(modelId)) return "preview";
  return rule.status;
}

function defaultInputCombinations(apiType: string, declared: string[]): string[][] {
  const available = new Set(declared);
  if (apiType === "llm") {
    const singles = ["text", "code", "document", "image", "audio", "video"]
      .filter((kind) => available.has(kind)).map((kind) => [kind]);
    const paired = ["document", "image", "audio", "video"]
      .filter((kind) => available.has("text") && available.has(kind))
      .map((kind) => ["text", kind]);
    return [...singles, ...paired].length > 0 ? [...singles, ...paired] : [["text"]];
  }
  const canonical: Record<string, string[][]> = {
    "embedding.text": [["text"], ["code"], ["document_chunk"]],
    "embedding.multimodal": [["text"], ["image"], ["text", "image"]],
    rerank: [["query", "documents"]],
    "image.txt2img": [["text"]],
    "image.img2img": [["text", "image"]],
    "image.inpaint": [["text", "image", "mask"]],
    "image.upscale": [["image"]],
    "image.bg_remove": [["image"]],
    "vision.ocr": [["image"]],
    "vision.caption": [["image"]],
    "vision.detect": [["image"]],
    "vision.segment": [["image"]],
    "audio.tts": [["text"]],
    "audio.asr": [["audio"]],
    "audio.music": available.has("audio") ? [["text"], ["text", "audio"]] : [["text"]],
    "audio.enhance": [["audio"]],
    "video.txt2video": [["text"]],
    "video.img2video": [["text", "image"]],
    "video.video2video": [["text", "video"]],
    "video.extend": [["video"]],
    "video.upscale": [["video"]],
    "agent.computer_use": [["observation", "action", "environment_state", "session_state"]],
  };
  return canonical[apiType] ?? [declared.length > 0 ? declared : ["structured"]];
}

function defaultOutputCombinations(apiType: string, declared: string[]): string[][] {
  if (apiType === "llm") {
    const supported = ["text", "json", "tool_call"].filter((kind) => declared.includes(kind));
    return (supported.length > 0 ? supported : ["text"]).map((kind) => [kind]);
  }
  const canonical: Record<string, string> = {
    "embedding.text": "embedding",
    "embedding.multimodal": "embedding",
    rerank: "rerank",
    "image.txt2img": "image",
    "image.img2img": "image",
    "image.inpaint": "image",
    "image.upscale": "image",
    "image.bg_remove": "image",
    "vision.ocr": "text",
    "vision.caption": "text",
    "vision.detect": "structured",
    "vision.segment": "structured",
    "audio.tts": "audio",
    "audio.asr": "text",
    "audio.music": "audio",
    "audio.enhance": "audio",
    "video.txt2video": "video",
    "video.img2video": "video",
    "video.video2video": "video",
    "video.extend": "video",
    "video.upscale": "video",
    "agent.computer_use": "structured",
  };
  return [[canonical[apiType] ?? declared[0] ?? "structured"]];
}

export function analyzeProviderMatrix(args: {
  baseline: ProviderBaseline;
  officialInventories: ProviderInventory[];
  aiccInventories: ProviderInventory[];
  selectedDrivers?: string[];
}): {
  cells: MatrixCell[];
  mismatches: string[];
  coverage: import("./types.ts").ModelCoverageRecord[];
  documentCoverage: DocumentFormatCoverageRecord[];
} {
  const selected = args.selectedDrivers?.length
    ? new Set(args.selectedDrivers)
    : null;
  const profiles = new Map(
    args.baseline.providers.map((profile) => [profile.provider_driver, profile]),
  );
  const cells: MatrixCell[] = [];
  const documentCoverage: DocumentFormatCoverageRecord[] = [];
  const errors: string[] = [];
  const reconciled = reconcileOfficialAndAiccInventories({
    baseline: args.baseline,
    officialInventories: args.officialInventories,
    aiccInventories: args.aiccInventories,
  });
  errors.push(...reconciled.mismatches);
  for (const inventory of reconciled.inventories) {
    if (selected && !selected.has(inventory.provider_driver)) continue;
    const profile = profiles.get(inventory.provider_driver);
    if (!profile) {
      errors.push(`missing provider baseline for ${inventory.provider_driver}`);
      continue;
    }
    const capabilityProfile = profile.capability_source_provider
      ? profiles.get(profile.capability_source_provider)
      : profile;
    if (!capabilityProfile) {
      errors.push(
        `missing capability source ${profile.capability_source_provider} for ${inventory.provider_driver}`,
      );
      continue;
    }
    for (const model of inventory.models) {
      const rule = capabilityProfile.rules.find((candidate) =>
        globMatches(candidate.model_pattern, model.provider_model_id)
      );
      if (!rule) {
        errors.push(
          `missing official capability rule for ${inventory.provider_driver}/${model.provider_model_id}`,
        );
        continue;
      }
      const declared = new Set(model.api_types);
      const official = new Set(rule.api_types);
      if (rule.input_kinds.includes("document")) {
        const supported = new Set(rule.document_formats ?? []);
        for (const format of DOCUMENT_FORMAT_CANDIDATES) {
          documentCoverage.push({
            provider_driver: inventory.provider_driver,
            provider_instance: inventory.provider_instance_name,
            exact_model: model.exact_model,
            provider_model_id: model.provider_model_id,
            format,
            status: supported.has(format) ? "supported" : "not_applicable",
            source_urls: rule.source_urls,
          });
        }
      }
      for (const apiType of declared) {
        if (!official.has(apiType)) {
          errors.push(
            `official_not_supported_but_aicc_advertised ${model.exact_model} ${apiType}`,
          );
        }
      }
      for (const apiType of official) {
        if (!declared.has(apiType)) {
          errors.push(
            `official_supported_but_aicc_missing ${model.exact_model} ${apiType}`,
          );
          continue;
        }
        const methods = rule.methods.length > 0
          ? rule.methods.filter((method) => methodsForApiType(apiType).includes(method))
          : [...methodsForApiType(apiType)];
        for (const method of methods) {
          const variants = apiType.startsWith("embedding.")
            ? ["default", "embedding_large_artifact"] as const
            : ["default"] as const;
          const ioProfile = rule.api_io?.[apiType];
          const inputVariants = ioProfile?.input_combinations ??
            defaultInputCombinations(apiType, rule.input_kinds);
          const outputVariants = ioProfile?.output_combinations ??
            defaultOutputCombinations(apiType, rule.output_kinds);
          for (const variant of variants) {
            const largeTextInput = inputVariants.find((combination) =>
              combination.length === 1 && combination[0] === "text"
            ) ?? ["text"];
            const variantInputs = variant === "embedding_large_artifact"
              ? [largeTextInput]
              : inputVariants;
            for (const inputKinds of variantInputs) {
            for (const outputKinds of outputVariants) {
            const documentFormats = inputKinds.includes("document")
              ? rule.document_formats ?? []
              : [undefined];
            const resourceRepresentations = inputKinds.some((kind) =>
              ["document", "image", "audio", "video", "mask"].includes(kind)
            ) ? ["url", "base64", "named_object"] as const : [undefined];
            for (const documentFormat of documentFormats) {
            for (const resourceRepresentation of resourceRepresentations) {
            cells.push({
            case_id: `t2.${inventory.provider_driver}.${inventory.provider_instance_name}.${model.provider_model_id}.${method}.${variant}.in-${inputKinds.join("-")}.out-${outputKinds.join("-")}${documentFormat ? `.doc-${documentFormat}` : ""}${resourceRepresentation ? `.res-${resourceRepresentation}` : ""}`
              .toLowerCase().replace(/[^a-z0-9._-]+/g, "-"),
            provider_driver: inventory.provider_driver,
            provider_instance: inventory.provider_instance_name,
            exact_model: model.exact_model,
            provider_model_id: model.provider_model_id,
            api_type: apiType,
            method,
            variant,
            baseline_status: normalizedStatus(rule, model.provider_model_id),
            input_kinds: inputKinds,
            output_kinds: outputKinds,
            resource_representation: resourceRepresentation,
            document_format: documentFormat,
            source_urls: rule.source_urls,
            estimated_cost_usd: model.pricing?.currency?.toUpperCase() === "USD" &&
                typeof model.pricing.estimated_cost === "number" &&
                Number.isFinite(model.pricing.estimated_cost) &&
                model.pricing.estimated_cost >= 0
              ? model.pricing.estimated_cost
              : undefined,
          });
            }
            }
            }
            }
          }
        }
      }
    }
  }
  return { cells, mismatches: errors, coverage: reconciled.coverage, documentCoverage };
}

export function buildProviderMatrix(args: {
  baseline: ProviderBaseline;
  officialInventories: ProviderInventory[];
  aiccInventories: ProviderInventory[];
  selectedDrivers?: string[];
}): MatrixCell[] {
  const result = analyzeProviderMatrix(args);
  if (result.mismatches.length > 0) {
    throw new Error(result.mismatches.join("; "));
  }
  return result.cells;
}

export function assertTaxonomyConstants(): void {
  if (new Set(RESULT_STATUSES).size !== RESULT_STATUSES.length) {
    throw new Error("duplicate result status");
  }
  if (new Set(FAILURE_CLASSES).size !== FAILURE_CLASSES.length) {
    throw new Error("duplicate failure class");
  }
}
