import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { AcceptanceCase, ProviderModel } from "./types.ts";
import { methodsForApiType } from "./canonical.ts";

export const REQUIRED_T15_PROVIDER_DRIVERS = [
  "openai",
  "claude",
  "google-gemini",
  "fal",
  "minimax",
  "openrouter",
  "sn-ai-provider",
] as const;

export type ProtocolErrorFixture = {
  scenario: string;
  status: number;
  body: Record<string, unknown>;
  headers?: Record<string, string>;
};

export type ProviderProtocolContract = {
  id: string;
  protocol_adapter_id: string;
  base_contract_id?: string;
  api_version: string;
  api_types: string[];
  operation: string;
  http_method: string;
  path: string;
  auth: { kind: "header" | "query"; name: string; prefix: string };
  required_headers?: Record<string, string>;
  content_type: string;
  required_body_fields: string[];
  allowed_body_fields: string[];
  stream_protocol?: "openai_responses" | "claude_messages" | "gemini_interactions" | "openrouter_chat";
  async_protocol?: "fal_queue" | "minimax_video" | "google_lro" | "openai_video";
  success_content_type?: string;
  success_fixture?: Record<string, unknown>;
  async_result_fixture?: Record<string, unknown>;
  success_fixture_base64?: string;
  official_sources: string[];
  evidence_summary: string;
};

export type ProviderProtocolCatalog = {
  schema_version: number;
  revision: string;
  checked_at: string;
  providers: Array<{
    provider_driver: string;
    contracts: ProviderProtocolContract[];
  }>;
  error_evidence: Record<string, {
    applicable_api_versions: string[];
    official_sources: string[];
    evidence_summary: string;
  }>;
  error_fixtures: Record<string, ProtocolErrorFixture[]>;
};

export type CapturedProviderRequest = {
  method: string;
  pathname: string;
  query: URLSearchParams;
  headers: Headers;
  body: unknown;
};

const here = dirname(fileURLToPath(import.meta.url));

function object(value: unknown, field: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function nonEmptyString(value: unknown, field: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${field} must be a non-empty string`);
  return value;
}

function stringArray(value: unknown, field: string): string[] {
  if (!Array.isArray(value) || value.length === 0 || value.some((item) => typeof item !== "string" || !item)) {
    throw new Error(`${field} must be a non-empty string array`);
  }
  return value as string[];
}

export function validateProviderProtocolCatalog(value: unknown): ProviderProtocolCatalog {
  const root = object(value, "protocol catalog");
  if (root.schema_version !== 1) throw new Error("unsupported provider protocol contract schema");
  nonEmptyString(root.revision, "revision");
  nonEmptyString(root.checked_at, "checked_at");
  if (!Array.isArray(root.providers)) throw new Error("providers must be an array");
  const providers = new Set<string>();
  const ids = new Set<string>();
  for (const rawProvider of root.providers) {
    const provider = object(rawProvider, "provider");
    const driver = nonEmptyString(provider.provider_driver, "provider_driver");
    if (providers.has(driver)) throw new Error(`duplicate protocol provider ${driver}`);
    providers.add(driver);
    if (!Array.isArray(provider.contracts) || provider.contracts.length === 0) {
      throw new Error(`${driver}.contracts must not be empty`);
    }
    for (const rawContract of provider.contracts) {
      const contract = object(rawContract, `${driver}.contract`);
      const id = nonEmptyString(contract.id, `${driver}.contract.id`);
      if (ids.has(id)) throw new Error(`duplicate protocol contract ${id}`);
      ids.add(id);
      for (const field of ["protocol_adapter_id", "api_version", "operation", "http_method", "path", "content_type", "evidence_summary"]) {
        nonEmptyString(contract[field], `${id}.${field}`);
      }
      stringArray(contract.api_types, `${id}.api_types`);
      stringArray(contract.allowed_body_fields, `${id}.allowed_body_fields`);
      if (!Array.isArray(contract.required_body_fields) ||
          contract.required_body_fields.some((field) => typeof field !== "string")) {
        throw new Error(`${id}.required_body_fields must be a string array`);
      }
      const allowed = new Set(contract.allowed_body_fields as string[]);
      for (const required of contract.required_body_fields as string[]) {
        if (!allowed.has(required)) throw new Error(`${id} required field ${required} is not allowed`);
      }
      const sources = stringArray(contract.official_sources, `${id}.official_sources`);
      if (sources.some((source) => !/^https:\/\//.test(source))) {
        throw new Error(`${id}.official_sources must use HTTPS`);
      }
      const auth = object(contract.auth, `${id}.auth`);
      if (!["header", "query"].includes(String(auth.kind))) throw new Error(`${id}.auth.kind is invalid`);
      nonEmptyString(auth.name, `${id}.auth.name`);
      if (typeof auth.prefix !== "string") throw new Error(`${id}.auth.prefix must be a string`);
    }
  }
  for (const required of REQUIRED_T15_PROVIDER_DRIVERS) {
    if (!providers.has(required)) throw new Error(`protocol catalog missing Provider ${required}`);
  }
  const errors = object(root.error_fixtures, "error_fixtures");
  const errorEvidence = object(root.error_evidence, "error_evidence");
  for (const driver of providers) {
    const evidence = object(errorEvidence[driver], `${driver}.error_evidence`);
    stringArray(evidence.applicable_api_versions, `${driver}.error_evidence.applicable_api_versions`);
    const evidenceSources = stringArray(evidence.official_sources, `${driver}.error_evidence.official_sources`);
    if (evidenceSources.some((source) => !/^https:\/\//.test(source))) {
      throw new Error(`${driver}.error_evidence.official_sources must use HTTPS`);
    }
    nonEmptyString(evidence.evidence_summary, `${driver}.error_evidence.evidence_summary`);
    const fixtures = errors[driver];
    if (!Array.isArray(fixtures) || fixtures.length < 3) {
      throw new Error(`${driver} must define at least three official error fixtures`);
    }
    for (const rawFixture of fixtures) {
      const fixture = object(rawFixture, `${driver}.error_fixture`);
      nonEmptyString(fixture.scenario, `${driver}.error_fixture.scenario`);
      const body = object(fixture.body, `${driver}.error_fixture.body`);
      const baseResponse = body.base_resp && typeof body.base_resp === "object"
        ? body.base_resp as Record<string, unknown>
        : undefined;
      const nativeApplicationError = fixture.status === 200 && Number(baseResponse?.status_code) > 0;
      if (!Number.isInteger(fixture.status) || (Number(fixture.status) < 400 && !nativeApplicationError)) {
        throw new Error(`${driver}.error_fixture.status must be HTTP error or documented Provider application error`);
      }
    }
  }
  return value as ProviderProtocolCatalog;
}

export async function loadProviderProtocolCatalog(): Promise<ProviderProtocolCatalog> {
  return validateProviderProtocolCatalog(JSON.parse(
    await readFile(join(here, "provider_protocol_contracts.json"), "utf8"),
  ));
}

export function protocolContracts(catalog: ProviderProtocolCatalog): Array<ProviderProtocolContract & { provider_driver: string }> {
  return catalog.providers.flatMap((provider) => provider.contracts.map((contract) => ({
    ...contract,
    provider_driver: provider.provider_driver,
  })));
}

export function protocolContract(
  catalog: ProviderProtocolCatalog,
  providerDriver: string,
  contractId: string,
): ProviderProtocolContract {
  const contract = catalog.providers.find((provider) => provider.provider_driver === providerDriver)
    ?.contracts.find((candidate) => candidate.id === contractId);
  if (!contract) throw new Error(`unknown protocol contract ${providerDriver}/${contractId}`);
  return contract;
}

function pathPattern(template: string): RegExp {
  const escaped = template.replace(/[.+?^${}()|[\]\\]/g, "\\$&")
    .replace("\\{model\\}", "[^/]+")
    .replace("\\{endpoint\\}", ".+");
  return new RegExp(`^${escaped}$`);
}

export function validateProviderRequest(
  contract: ProviderProtocolContract,
  request: CapturedProviderRequest,
): string[] {
  const errors: string[] = [];
  if (request.method.toUpperCase() !== contract.http_method.toUpperCase()) {
    errors.push(`method=${request.method}; expected=${contract.http_method}`);
  }
  if (!pathPattern(contract.path).test(request.pathname)) {
    errors.push(`path=${request.pathname}; expected=${contract.path}`);
  }
  const authValue = contract.auth.kind === "header"
    ? request.headers.get(contract.auth.name)
    : request.query.get(contract.auth.name);
  if (!authValue || !authValue.startsWith(contract.auth.prefix) || authValue.length <= contract.auth.prefix.length) {
    errors.push(`missing or invalid ${contract.auth.kind} authentication ${contract.auth.name}`);
  }
  for (const [name, expected] of Object.entries(contract.required_headers ?? {})) {
    if (request.headers.get(name) !== expected) errors.push(`header ${name} must equal ${expected}`);
  }
  const actualContentType = request.headers.get("content-type")?.split(";", 1)[0].trim().toLowerCase();
  if (actualContentType !== contract.content_type.toLowerCase()) {
    errors.push(`content-type=${actualContentType ?? "<missing>"}; expected=${contract.content_type}`);
  }
  if (!request.body || typeof request.body !== "object" || Array.isArray(request.body)) {
    errors.push("request body must be a JSON object");
    return errors;
  }
  const body = request.body as Record<string, unknown>;
  for (const field of contract.required_body_fields) {
    if (body[field] === undefined || body[field] === null) errors.push(`missing body field ${field}`);
  }
  const allowed = new Set(contract.allowed_body_fields);
  for (const field of Object.keys(body)) {
    if (!allowed.has(field)) errors.push(`unknown body field ${field}`);
  }
  return errors;
}

function caseId(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-");
}

type VariantCell = {
  provider_driver: string;
  contract_id: string;
  api_type: string;
  model: ProviderModel;
};

export function buildT15Manifest(
  catalog: ProviderProtocolCatalog,
  variants: VariantCell[] = [],
): AcceptanceCase[] {
  const cases: AcceptanceCase[] = [];
  for (const provider of catalog.providers) {
    for (const contract of provider.contracts) {
      for (const apiType of contract.api_types) {
        const common: Partial<AcceptanceCase> = {
          layer: "T1.5",
          priority: "P0",
          tags: ["provider_protocol", provider.provider_driver, contract.protocol_adapter_id, apiType],
          input_entry: "zone_gateway",
          user: "acceptance-user-a",
          session: "isolated-per-case",
          provider_driver: provider.provider_driver,
          provider_instance: `t15-${provider.provider_driver}`,
          model_selector: null,
          api_type: apiType,
          method: methodsForApiType(apiType)[0] ?? apiType,
          required_capabilities: [],
          disabled_capabilities: [],
          fixtures: [],
          expected_exact_model: null,
          expected_provider_instance: `t15-${provider.provider_driver}`,
          expected_task_status: "succeeded",
          expected_error_class: null,
          expected_output: { kinds: [], attachment_count: { min: 0, max: 0 }, mime_types: [] },
          semantic_rubric: [],
          timeout_ms: 30_000,
          max_attempts: 1,
          estimated_cost_usd: 0,
          cleanup: ["reset_provider_mock", "remove_t15_provider_instance"],
          protocol_contract_id: contract.id,
          protocol_evidence_revision: catalog.revision,
          protocol_adapter_id: contract.protocol_adapter_id,
          provider_api_version: contract.api_version,
          expected_wire_fixture: `${contract.id}.request`,
          response_fixture: `${contract.id}.success`,
        };
        cases.push({
          ...common,
          case_id: caseId(`t1.5.${provider.provider_driver}.${contract.id}.${apiType}.success`),
          mock_scenario: "success",
        } as AcceptanceCase);
        const primaryApiType = contract.api_types[0];
        if (apiType === primaryApiType && contract.stream_protocol) {
          cases.push({
            ...common,
            case_id: caseId(`t1.5.${provider.provider_driver}.${contract.id}.${apiType}.stream`),
            mock_scenario: "stream_success",
            expected_wire_fixture: `${contract.id}.request.stream`,
            response_fixture: `${contract.id}.stream`,
          } as AcceptanceCase);
        }
        if (apiType === primaryApiType && contract.async_protocol) {
          cases.push({
            ...common,
            case_id: caseId(`t1.5.${provider.provider_driver}.${contract.id}.${apiType}.async`),
            mock_scenario: "async_success",
            expected_wire_fixture: `${contract.id}.request.async`,
            response_fixture: `${contract.id}.async`,
          } as AcceptanceCase);
        }
        for (const error of apiType === primaryApiType ? catalog.error_fixtures[provider.provider_driver] : []) {
          cases.push({
            ...common,
            case_id: caseId(`t1.5.${provider.provider_driver}.${contract.id}.${apiType}.error.${error.scenario}`),
            priority: "P1",
            mock_scenario: error.scenario,
            expected_task_status: "failed",
            expected_error_class: "provider_protocol_failed",
            response_fixture: `${provider.provider_driver}.error.${error.scenario}`,
          } as AcceptanceCase);
        }
      }
    }
  }
  for (const variant of variants) {
    const contract = protocolContract(catalog, variant.provider_driver, variant.contract_id);
    if (!contract.api_types.includes(variant.api_type)) {
      throw new Error(`${variant.contract_id} does not support ${variant.api_type}`);
    }
    cases.push({
      case_id: caseId(`t1.5.${variant.provider_driver}.${contract.id}.${variant.api_type}.variant.${variant.model.provider_model_id}`),
      layer: "T1.5",
      priority: "P0",
      tags: ["provider_protocol", "variant", variant.provider_driver, variant.api_type],
      input_entry: "zone_gateway",
      user: "acceptance-user-a",
      session: "isolated-per-case",
      provider_driver: variant.provider_driver,
      provider_instance: `t15-${variant.provider_driver}`,
      model_selector: { kind: "exact", value: variant.model.exact_model },
      api_type: variant.api_type,
      method: methodsForApiType(variant.api_type)[0] ?? variant.api_type,
      required_capabilities: [],
      disabled_capabilities: [],
      fixtures: [],
      mock_scenario: "success",
      expected_exact_model: variant.model.exact_model,
      expected_provider_instance: `t15-${variant.provider_driver}`,
      expected_task_status: "succeeded",
      expected_error_class: null,
      expected_output: { kinds: [], attachment_count: { min: 0, max: 0 }, mime_types: [] },
      semantic_rubric: [],
      timeout_ms: 30_000,
      max_attempts: 1,
      estimated_cost_usd: 0,
      cleanup: ["reset_provider_mock", "remove_t15_provider_instance"],
      protocol_contract_id: contract.id,
      protocol_evidence_revision: catalog.revision,
      protocol_adapter_id: contract.protocol_adapter_id,
      provider_api_version: contract.api_version,
      expected_wire_fixture: `${contract.id}.request.variant.${variant.model.provider_model_id}`,
      response_fixture: `${contract.id}.success`,
    });
  }
  return cases;
}
