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
  "kimi",
  "glm",
  "deepseek",
  "doubao",
  "qwen",
  "sn-ai-provider",
] as const;

const OFFICIAL_PROTOCOL_SOURCE_HOSTS: Record<string, Set<string>> = {
  openai: new Set(["platform.openai.com", "developers.openai.com"]),
  claude: new Set(["platform.claude.com", "docs.anthropic.com"]),
  "google-gemini": new Set(["ai.google.dev"]),
  fal: new Set(["fal.ai"]),
  minimax: new Set(["platform.minimax.io"]),
  openrouter: new Set(["openrouter.ai"]),
  kimi: new Set(["platform.kimi.com"]),
  glm: new Set(["docs.z.ai", "docs.bigmodel.cn"]),
  deepseek: new Set(["api-docs.deepseek.com"]),
  doubao: new Set(["www.volcengine.com", "docs.volcengine.com"]),
  qwen: new Set(["www.alibabacloud.com"]),
  "sn-ai-provider": new Set(["github.com", "developers.openai.com"]),
};

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
  body_field_types: Record<
    string,
    Array<"string" | "number" | "boolean" | "array" | "object">
  >;
  stream_protocol?:
    | "openai_responses"
    | "openai_chat"
    | "claude_messages"
    | "gemini_interactions"
    | "openrouter_chat";
  async_protocol?:
    | "fal_queue"
    | "minimax_video"
    | "google_lro"
    | "openai_video";
  async_steps?: Array<{
    name: "poll" | "result" | "cancel";
    http_method: string;
    path: string;
    required_query_fields?: string[];
  }>;
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
    provider_profile_id: string;
    endpoint_path: string;
    credential_type: "api_key" | "bearer";
    instance_fields?: { region?: string; workspace?: string; account?: string };
    official_first_party_model_ids?: Record<string, string[]>;
    official_variant_sources?: string[];
    official_variant_rules?: Array<{
      model_ids: string[];
      variants: Record<string, Record<string, unknown>>;
    }>;
    test_model_ids: Record<string, string>;
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
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return value;
}

function stringArray(value: unknown, field: string): string[] {
  if (
    !Array.isArray(value) || value.length === 0 ||
    value.some((item) => typeof item !== "string" || !item)
  ) {
    throw new Error(`${field} must be a non-empty string array`);
  }
  return value as string[];
}

export function validateProviderProtocolCatalog(
  value: unknown,
): ProviderProtocolCatalog {
  const root = object(value, "protocol catalog");
  if (root.schema_version !== 1) {
    throw new Error("unsupported provider protocol contract schema");
  }
  nonEmptyString(root.revision, "revision");
  nonEmptyString(root.checked_at, "checked_at");
  if (!Array.isArray(root.providers)) {
    throw new Error("providers must be an array");
  }
  const providers = new Set<string>();
  const ids = new Set<string>();
  for (const rawProvider of root.providers) {
    const provider = object(rawProvider, "provider");
    const driver = nonEmptyString(provider.provider_driver, "provider_driver");
    if (providers.has(driver)) {
      throw new Error(`duplicate protocol provider ${driver}`);
    }
    providers.add(driver);
    nonEmptyString(
      provider.provider_profile_id,
      `${driver}.provider_profile_id`,
    );
    if (
      typeof provider.endpoint_path !== "string" ||
      (provider.endpoint_path !== "" && !provider.endpoint_path.startsWith("/"))
    ) {
      throw new Error(`${driver}.endpoint_path must be empty or start with /`);
    }
    if (!["api_key", "bearer"].includes(String(provider.credential_type))) {
      throw new Error(`${driver}.credential_type is invalid`);
    }
    if (provider.instance_fields !== undefined) {
      const fields = object(
        provider.instance_fields,
        `${driver}.instance_fields`,
      );
      for (const [name, fieldValue] of Object.entries(fields)) {
        if (!["region", "workspace", "account"].includes(name)) {
          throw new Error(`${driver}.instance_fields.${name} is invalid`);
        }
        nonEmptyString(fieldValue, `${driver}.instance_fields.${name}`);
      }
    }
    const testModelIds = object(
      provider.test_model_ids,
      `${driver}.test_model_ids`,
    );
    for (const [apiType, modelId] of Object.entries(testModelIds)) {
      nonEmptyString(apiType, `${driver}.test_model_ids key`);
      nonEmptyString(modelId, `${driver}.test_model_ids.${apiType}`);
    }
    if (["openai", "claude", "google-gemini", "fal"].includes(driver)) {
      const officialModelIds = object(
        provider.official_first_party_model_ids,
        `${driver}.official_first_party_model_ids`,
      );
      for (const [apiType, modelId] of Object.entries(testModelIds)) {
        const pool = stringArray(
          officialModelIds[apiType],
          `${driver}.official_first_party_model_ids.${apiType}`,
        );
        if (!pool.includes(String(modelId))) {
          throw new Error(
            `${driver}.test_model_ids.${apiType} is absent from its official model pool`,
          );
        }
      }
    }
    if (provider.official_variant_rules !== undefined) {
      const sources = stringArray(
        provider.official_variant_sources,
        `${driver}.official_variant_sources`,
      );
      if (
        sources.some((source) =>
          !/^https:\/\//.test(source) ||
          !OFFICIAL_PROTOCOL_SOURCE_HOSTS[driver]?.has(new URL(source).hostname)
        )
      ) {
        throw new Error(
          `${driver}.official_variant_sources must use the Provider official domain`,
        );
      }
      if (
        !Array.isArray(provider.official_variant_rules) ||
        provider.official_variant_rules.length === 0
      ) {
        throw new Error(
          `${driver}.official_variant_rules must be a non-empty array`,
        );
      }
      for (
        const [index, rawRule] of (provider.official_variant_rules as unknown[])
          .entries()
      ) {
        const rule = object(
          rawRule,
          `${driver}.official_variant_rules[${index}]`,
        );
        stringArray(
          rule.model_ids,
          `${driver}.official_variant_rules[${index}].model_ids`,
        );
        const variants = object(
          rule.variants,
          `${driver}.official_variant_rules[${index}].variants`,
        );
        if (Object.keys(variants).length === 0) {
          throw new Error(
            `${driver}.official_variant_rules[${index}].variants must not be empty`,
          );
        }
        for (const [variant, options] of Object.entries(variants)) {
          nonEmptyString(
            variant,
            `${driver}.official_variant_rules[${index}].variants key`,
          );
          object(
            options,
            `${driver}.official_variant_rules[${index}].variants.${variant}`,
          );
        }
      }
    }
    if (!Array.isArray(provider.contracts) || provider.contracts.length === 0) {
      throw new Error(`${driver}.contracts must not be empty`);
    }
    for (const rawContract of provider.contracts) {
      const contract = object(rawContract, `${driver}.contract`);
      const id = nonEmptyString(contract.id, `${driver}.contract.id`);
      if (ids.has(id)) throw new Error(`duplicate protocol contract ${id}`);
      ids.add(id);
      for (
        const field of [
          "protocol_adapter_id",
          "api_version",
          "operation",
          "http_method",
          "path",
          "content_type",
          "evidence_summary",
        ]
      ) {
        nonEmptyString(contract[field], `${id}.${field}`);
      }
      stringArray(contract.api_types, `${id}.api_types`);
      for (const apiType of contract.api_types as string[]) {
        if (!testModelIds[apiType]) {
          throw new Error(`${id} has no ${driver}.test_model_ids.${apiType}`);
        }
      }
      stringArray(contract.allowed_body_fields, `${id}.allowed_body_fields`);
      if (
        !Array.isArray(contract.required_body_fields) ||
        contract.required_body_fields.some((field) => typeof field !== "string")
      ) {
        throw new Error(`${id}.required_body_fields must be a string array`);
      }
      const allowed = new Set(contract.allowed_body_fields as string[]);
      const bodyFieldTypes = object(
        contract.body_field_types,
        `${id}.body_field_types`,
      );
      for (const required of contract.required_body_fields as string[]) {
        if (!allowed.has(required)) {
          throw new Error(`${id} required field ${required} is not allowed`);
        }
        if (!bodyFieldTypes[required]) {
          throw new Error(
            `${id} required field ${required} has no type schema`,
          );
        }
      }
      for (const [field, rawTypes] of Object.entries(bodyFieldTypes)) {
        if (!allowed.has(field)) {
          throw new Error(`${id} type schema field ${field} is not allowed`);
        }
        const types = stringArray(rawTypes, `${id}.body_field_types.${field}`);
        if (
          types.some((type) =>
            !["string", "number", "boolean", "array", "object"].includes(type)
          )
        ) {
          throw new Error(
            `${id}.body_field_types.${field} contains invalid type`,
          );
        }
      }
      const sources = stringArray(
        contract.official_sources,
        `${id}.official_sources`,
      );
      if (sources.some((source) => !/^https:\/\//.test(source))) {
        throw new Error(`${id}.official_sources must use HTTPS`);
      }
      if (
        sources.some((source) =>
          !OFFICIAL_PROTOCOL_SOURCE_HOSTS[driver]?.has(new URL(source).hostname)
        )
      ) {
        throw new Error(
          `${id}.official_sources must use the Provider official domain`,
        );
      }
      const auth = object(contract.auth, `${id}.auth`);
      if (!["header", "query"].includes(String(auth.kind))) {
        throw new Error(`${id}.auth.kind is invalid`);
      }
      nonEmptyString(auth.name, `${id}.auth.name`);
      if (typeof auth.prefix !== "string") {
        throw new Error(`${id}.auth.prefix must be a string`);
      }
      if (contract.async_protocol) {
        if (
          !Array.isArray(contract.async_steps) ||
          !contract.async_steps.some((step) =>
            object(step, `${id}.async_step`).name === "poll"
          )
        ) {
          throw new Error(
            `${id}.async_steps must include poll for async protocols`,
          );
        }
        for (
          const [index, rawStep] of (contract.async_steps as unknown[])
            .entries()
        ) {
          const step = object(rawStep, `${id}.async_steps[${index}]`);
          if (!["poll", "result", "cancel"].includes(String(step.name))) {
            throw new Error(`${id}.async_steps[${index}].name is invalid`);
          }
          nonEmptyString(
            step.http_method,
            `${id}.async_steps[${index}].http_method`,
          );
          nonEmptyString(step.path, `${id}.async_steps[${index}].path`);
          if (step.required_query_fields !== undefined) {
            stringArray(
              step.required_query_fields,
              `${id}.async_steps[${index}].required_query_fields`,
            );
          }
        }
      } else if (contract.async_steps !== undefined) {
        throw new Error(`${id}.async_steps requires async_protocol`);
      }
      const fixtureErrors = validateProviderSuccessFixture(
        contract as unknown as ProviderProtocolContract,
      );
      if (fixtureErrors.length > 0) {
        throw new Error(
          `${id}.success_fixture is invalid: ${fixtureErrors.join("; ")}`,
        );
      }
    }
  }
  for (const required of REQUIRED_T15_PROVIDER_DRIVERS) {
    if (!providers.has(required)) {
      throw new Error(`protocol catalog missing Provider ${required}`);
    }
  }
  const errors = object(root.error_fixtures, "error_fixtures");
  const errorEvidence = object(root.error_evidence, "error_evidence");
  for (const driver of providers) {
    const evidence = object(errorEvidence[driver], `${driver}.error_evidence`);
    stringArray(
      evidence.applicable_api_versions,
      `${driver}.error_evidence.applicable_api_versions`,
    );
    const evidenceSources = stringArray(
      evidence.official_sources,
      `${driver}.error_evidence.official_sources`,
    );
    if (evidenceSources.some((source) => !/^https:\/\//.test(source))) {
      throw new Error(
        `${driver}.error_evidence.official_sources must use HTTPS`,
      );
    }
    if (
      evidenceSources.some((source) =>
        !OFFICIAL_PROTOCOL_SOURCE_HOSTS[driver]?.has(new URL(source).hostname)
      )
    ) {
      throw new Error(
        `${driver}.error_evidence.official_sources must use the Provider official domain`,
      );
    }
    nonEmptyString(
      evidence.evidence_summary,
      `${driver}.error_evidence.evidence_summary`,
    );
    const fixtures = errors[driver];
    if (!Array.isArray(fixtures) || fixtures.length < 3) {
      throw new Error(
        `${driver} must define at least three official error fixtures`,
      );
    }
    for (const rawFixture of fixtures) {
      const fixture = object(rawFixture, `${driver}.error_fixture`);
      nonEmptyString(fixture.scenario, `${driver}.error_fixture.scenario`);
      const body = object(fixture.body, `${driver}.error_fixture.body`);
      const baseResponse = body.base_resp && typeof body.base_resp === "object"
        ? body.base_resp as Record<string, unknown>
        : undefined;
      const nativeApplicationError = fixture.status === 200 &&
        Number(baseResponse?.status_code) > 0;
      if (
        !Number.isInteger(fixture.status) ||
        (Number(fixture.status) < 400 && !nativeApplicationError)
      ) {
        throw new Error(
          `${driver}.error_fixture.status must be HTTP error or documented Provider application error`,
        );
      }
    }
  }
  return value as ProviderProtocolCatalog;
}

export async function loadProviderProtocolCatalog(): Promise<
  ProviderProtocolCatalog
> {
  return validateProviderProtocolCatalog(JSON.parse(
    await readFile(join(here, "provider_protocol_contracts.json"), "utf8"),
  ));
}

function seededIndex(seed: string, length: number): number {
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(seed)) {
    hash ^= byte;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash % length;
}

export function selectOfficialModels(
  catalog: ProviderProtocolCatalog,
  providerDriver: string,
  seed: string,
): Record<string, string> {
  const provider = catalog.providers.find((candidate) =>
    candidate.provider_driver === providerDriver
  );
  if (!provider?.official_first_party_model_ids) {
    throw new Error(`${providerDriver} has no official first-party model pool`);
  }
  return Object.fromEntries(
    Object.entries(provider.official_first_party_model_ids).map((
      [apiType, pool],
    ) => [
      apiType,
      pool[seededIndex(`${seed}\0${providerDriver}\0${apiType}`, pool.length)],
    ]),
  );
}

export function protocolContracts(
  catalog: ProviderProtocolCatalog,
): Array<ProviderProtocolContract & { provider_driver: string }> {
  return catalog.providers.flatMap((provider) =>
    provider.contracts.map((contract) => ({
      ...contract,
      provider_driver: provider.provider_driver,
    }))
  );
}

export function protocolContract(
  catalog: ProviderProtocolCatalog,
  providerDriver: string,
  contractId: string,
): ProviderProtocolContract {
  const contract = catalog.providers.find((provider) =>
    provider.provider_driver === providerDriver
  )
    ?.contracts.find((candidate) => candidate.id === contractId);
  if (!contract) {
    throw new Error(
      `unknown protocol contract ${providerDriver}/${contractId}`,
    );
  }
  return contract;
}

function pathPattern(template: string): RegExp {
  const escaped = template.replace(/[.+?^${}()|[\]\\]/g, "\\$&")
    .replace(/\\\{[^}]+\\\}/g, "[^/]+");
  return new RegExp(`^${escaped}$`);
}

export function validateProviderAuxiliaryRequest(
  contract: ProviderProtocolContract,
  request: Pick<
    CapturedProviderRequest,
    "method" | "pathname" | "query" | "headers"
  >,
): {
  step?: NonNullable<ProviderProtocolContract["async_steps"]>[number];
  errors: string[];
} {
  const step = contract.async_steps?.find((candidate) =>
    candidate.http_method.toUpperCase() === request.method.toUpperCase() &&
    pathPattern(candidate.path).test(request.pathname)
  );
  const errors: string[] = [];
  if (!step) {
    return {
      errors: [
        `unexpected async request ${request.method} ${request.pathname}`,
      ],
    };
  }
  const authValue = contract.auth.kind === "header"
    ? request.headers.get(contract.auth.name)
    : request.query.get(contract.auth.name);
  if (
    !authValue || !authValue.startsWith(contract.auth.prefix) ||
    authValue.length <= contract.auth.prefix.length
  ) {
    errors.push(
      `missing or invalid ${contract.auth.kind} authentication ${contract.auth.name}`,
    );
  }
  for (const field of step.required_query_fields ?? []) {
    if (!request.query.get(field)) errors.push(`missing query field ${field}`);
  }
  return { step, errors };
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
  if (
    !authValue || !authValue.startsWith(contract.auth.prefix) ||
    authValue.length <= contract.auth.prefix.length
  ) {
    errors.push(
      `missing or invalid ${contract.auth.kind} authentication ${contract.auth.name}`,
    );
  }
  for (
    const [name, expected] of Object.entries(contract.required_headers ?? {})
  ) {
    if (request.headers.get(name) !== expected) {
      errors.push(`header ${name} must equal ${expected}`);
    }
  }
  const actualContentType = request.headers.get("content-type")?.split(
    ";",
    1,
  )[0].trim().toLowerCase();
  if (actualContentType !== contract.content_type.toLowerCase()) {
    errors.push(
      `content-type=${
        actualContentType ?? "<missing>"
      }; expected=${contract.content_type}`,
    );
  }
  if (
    !request.body || typeof request.body !== "object" ||
    Array.isArray(request.body)
  ) {
    errors.push("request body must be a JSON object");
    return errors;
  }
  const body = request.body as Record<string, unknown>;
  for (const field of contract.required_body_fields) {
    if (body[field] === undefined || body[field] === null) {
      errors.push(`missing body field ${field}`);
    }
  }
  const allowed = new Set(contract.allowed_body_fields);
  for (const field of Object.keys(body)) {
    if (!allowed.has(field)) errors.push(`unknown body field ${field}`);
    const expectedTypes = contract.body_field_types[field];
    if (
      expectedTypes && !expectedTypes.includes(
        Array.isArray(body[field])
          ? "array"
          : body[field] !== null && typeof body[field] === "object"
          ? "object"
          : typeof body[field] as never,
      )
    ) {
      errors.push(`body field ${field} has invalid type`);
    }
  }
  validateNestedProviderBody(contract, body, errors);
  return errors;
}

function recordValue(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function validateNestedProviderBody(
  contract: ProviderProtocolContract,
  body: Record<string, unknown>,
  errors: string[],
): void {
  for (const field of contract.required_body_fields) {
    if (typeof body[field] === "string" && !(body[field] as string).trim()) {
      errors.push(`body field ${field} must be a non-empty string`);
    }
    if (Array.isArray(body[field]) && body[field].length === 0) {
      errors.push(`body field ${field} must be a non-empty array`);
    }
  }
  for (const [field, value] of Object.entries(body)) {
    if (typeof value === "number" && !Number.isFinite(value)) {
      errors.push(`body field ${field} must be a finite number`);
    }
  }

  if (body.messages !== undefined) {
    if (Array.isArray(body.messages) && body.messages.length > 0) {
      body.messages.forEach((value, index) => {
        const message = recordValue(value);
        if (!message) {
          errors.push(`body field messages[${index}] must be an object`);
          return;
        }
        if (typeof message.role !== "string" || !message.role) {
          errors.push(
            `body field messages[${index}].role must be a non-empty string`,
          );
        }
        if (
          message.content === undefined && message.tool_calls === undefined &&
          message.tool_call_id === undefined
        ) {
          errors.push(
            `body field messages[${index}] must include content or tool call data`,
          );
        }
        if (Array.isArray(message.content)) {
          if (
            message.content.length === 0 ||
            message.content.some((part) => !recordValue(part))
          ) {
            errors.push(
              `body field messages[${index}].content must contain content block objects`,
            );
          }
        } else if (
          message.content !== undefined && message.content !== null &&
          typeof message.content !== "string"
        ) {
          errors.push(`body field messages[${index}].content has invalid type`);
        }
      });
    }
  }

  if (
    body.tools !== undefined &&
    (!Array.isArray(body.tools) ||
      body.tools.some((tool) => !recordValue(tool)))
  ) {
    errors.push("body field tools must be an array of objects");
  }
  if (body.documents !== undefined) {
    if (
      !Array.isArray(body.documents) || body.documents.length === 0 ||
      body.documents.some((document) =>
        typeof document !== "string" && !recordValue(document)
      )
    ) {
      errors.push(
        "body field documents must be a non-empty array of strings or objects",
      );
    }
  }
  if (
    body.instances !== undefined &&
    (!Array.isArray(body.instances) || body.instances.length === 0 ||
      body.instances.some((instance) => !recordValue(instance)))
  ) {
    errors.push("body field instances must be a non-empty array of objects");
  }
  if (
    contract.operation === "models.embedContent" && body.content !== undefined
  ) {
    const content = recordValue(body.content);
    if (
      !content || !Array.isArray(content.parts) || content.parts.length === 0 ||
      content.parts.some((part) => !recordValue(part))
    ) {
      errors.push(
        "body field content.parts must be a non-empty array of objects",
      );
    }
  }
  if (
    contract.operation === "interactions.create" && Array.isArray(body.input) &&
    (body.input.length === 0 ||
      body.input.some((item) => typeof item !== "string" && !recordValue(item)))
  ) {
    errors.push(
      "body field input must be a non-empty array of strings or objects",
    );
  }
  if (contract.operation === "responses.create" && Array.isArray(body.input)) {
    validateOpenAiResponsesInput(body.input, errors);
  }
  if (
    contract.operation === "chat.completions.create" &&
    Array.isArray(body.messages)
  ) {
    validateOpenAiChatMessages(body.messages, errors);
  }
  if (
    contract.operation === "messages.create" && Array.isArray(body.messages)
  ) {
    validateClaudeMessages(body, errors);
  }
  if (contract.operation === "interactions.create") {
    if (Array.isArray(body.input)) {
      validateGeminiInteractionInput(body.input, errors);
    }
    validateGeminiInteractionConfig(body, errors);
  }

  for (
    const field of [
      "max_tokens",
      "max_completion_tokens",
      "max_output_tokens",
      "dimensions",
      "n",
      "top_n",
    ]
  ) {
    if (
      typeof body[field] === "number" &&
      (!Number.isInteger(body[field]) || body[field] <= 0)
    ) {
      errors.push(`body field ${field} must be a positive integer`);
    }
  }
  if (contract.id === "minimax.music-generation.v1") {
    const model = String(body.model ?? "");
    const isCover = model === "music-cover" || model === "music-cover-free";
    if (isCover) {
      const references = [
        body.audio_url,
        body.audio_base64,
        body.cover_feature_id,
      ]
        .filter((value) => typeof value === "string" && value.length > 0);
      if (references.length !== 1) {
        errors.push("MiniMax cover music requires exactly one audio reference");
      }
    } else if (
      body.is_instrumental !== true && body.lyrics_optimizer !== true &&
      (typeof body.lyrics !== "string" || !body.lyrics.trim())
    ) {
      errors.push(
        "MiniMax non-instrumental music requires lyrics or lyrics_optimizer=true",
      );
    }
  }
}

function validateOpenAiResponsesInput(
  input: unknown[],
  errors: string[],
): void {
  input.forEach((item, index) => {
    const message = recordValue(item);
    if (!message) return;
    if (message.type === "function_call") {
      for (const field of ["call_id", "name", "arguments"]) {
        if (
          typeof message[field] !== "string" ||
          !(message[field] as string).trim()
        ) {
          errors.push(
            `body field input[${index}].${field} must be a non-empty string`,
          );
        }
      }
      return;
    }
    if (message.type === "function_call_output") {
      if (typeof message.call_id !== "string" || !message.call_id.trim()) {
        errors.push(
          `body field input[${index}].call_id must be a non-empty string`,
        );
      }
      if (
        typeof message.output !== "string" && !Array.isArray(message.output)
      ) {
        errors.push(
          `body field input[${index}].output must be a string or content array`,
        );
      }
      return;
    }
    if (message.type !== "message") return;
    const role = typeof message.role === "string" ? message.role : "";
    if (!["user", "assistant", "system", "developer"].includes(role)) {
      errors.push(
        `body field input[${index}].role is invalid for OpenAI Responses`,
      );
      return;
    }
    const content = Array.isArray(message.content) ? message.content : [];
    if (typeof message.content === "string" && role !== "assistant") return;
    if (content.length === 0) {
      errors.push(
        `body field input[${index}].content must be a non-empty array`,
      );
    }
    content.forEach((part, partIndex) => {
      const block = recordValue(part);
      if (!block || typeof block.type !== "string") {
        errors.push(
          `body field input[${index}].content[${partIndex}] must be a typed object`,
        );
        return;
      }
      const type = block.type;
      if (role === "assistant" && !["output_text", "refusal"].includes(type)) {
        errors.push(
          `body field input[${index}].content[${partIndex}].type=${type}; assistant messages require output_text or refusal`,
        );
      }
      if (role !== "assistant" && type === "output_text") {
        errors.push(
          `body field input[${index}].content[${partIndex}].type=output_text; ${role} messages require input content`,
        );
      }
      if (
        role !== "assistant" &&
        !["input_text", "input_image", "input_file"].includes(type)
      ) {
        errors.push(
          `body field input[${index}].content[${partIndex}].type=${type}; ${role} message content is invalid`,
        );
      }
    });
  });
}

function validateOpenAiChatMessages(
  messages: unknown[],
  errors: string[],
): void {
  messages.forEach((value, index) => {
    const message = recordValue(value);
    if (!message) return;
    const role = String(message.role ?? "");
    if (
      !["developer", "system", "user", "assistant", "tool", "function"]
        .includes(role)
    ) {
      errors.push(
        `body field messages[${index}].role is invalid for Chat Completions`,
      );
      return;
    }
    if (
      role === "tool" &&
      (typeof message.tool_call_id !== "string" || !message.tool_call_id.trim())
    ) {
      errors.push(
        `body field messages[${index}].tool_call_id must be a non-empty string`,
      );
    }
    if (role !== "tool" && message.tool_call_id !== undefined) {
      errors.push(
        `body field messages[${index}].tool_call_id is only valid for tool messages`,
      );
    }
    if (message.tool_calls !== undefined) {
      if (
        role !== "assistant" || !Array.isArray(message.tool_calls) ||
        message.tool_calls.length === 0
      ) {
        errors.push(
          `body field messages[${index}].tool_calls must be a non-empty assistant array`,
        );
        return;
      }
      message.tool_calls.forEach((value, toolIndex) => {
        const toolCall = recordValue(value);
        const fn = recordValue(toolCall?.function);
        if (
          !toolCall || typeof toolCall.id !== "string" ||
          toolCall.type !== "function" || !fn || typeof fn.name !== "string" ||
          typeof fn.arguments !== "string"
        ) {
          errors.push(
            `body field messages[${index}].tool_calls[${toolIndex}] is invalid`,
          );
        }
      });
    }
    if (message.reasoning_details !== undefined) {
      if (
        role !== "assistant" || !Array.isArray(message.reasoning_details) ||
        message.reasoning_details.length === 0 ||
        message.reasoning_details.some((detail) => !recordValue(detail))
      ) {
        errors.push(
          `body field messages[${index}].reasoning_details must be a non-empty assistant array`,
        );
      }
    }
  });
}

function validateClaudeMessages(
  body: Record<string, unknown>,
  errors: string[],
): void {
  if (Array.isArray(body.system)) {
    body.system.forEach((value, index) => {
      const block = recordValue(value);
      if (
        !block || block.type !== "text" || typeof block.text !== "string"
      ) errors.push(`body field system[${index}] must be a text block`);
    });
  }
  const messages = body.messages as unknown[];
  let pendingToolUses = new Set<string>();
  messages.forEach((value, index) => {
    const message = recordValue(value);
    if (!message) return;
    const role = String(message.role ?? "");
    if (!["user", "assistant"].includes(role)) {
      errors.push(
        `body field messages[${index}].role is invalid for Claude Messages`,
      );
      return;
    }
    const blocks = Array.isArray(message.content) ? message.content : [];
    if (pendingToolUses.size > 0) {
      const resultIds = blocks.map(recordValue).filter((
        block,
      ): block is Record<string, unknown> =>
        Boolean(block) && block!.type === "tool_result"
      ).map((block) => String(block.tool_use_id ?? ""));
      const leadingResults = blocks.slice(0, resultIds.length).every((block) =>
        recordValue(block)?.type === "tool_result"
      );
      if (
        role !== "user" || !leadingResults ||
        new Set(resultIds).size !== resultIds.length ||
        resultIds.length !== pendingToolUses.size || resultIds.some((id) =>
          !pendingToolUses.has(id)
        )
      ) {
        errors.push(
          `body field messages[${index}] must immediately return every preceding Claude tool_use`,
        );
      }
      pendingToolUses = new Set();
    }
    blocks.forEach((value, blockIndex) => {
      const block = recordValue(value);
      if (!block) return;
      if (block.type === "tool_use") {
        if (
          role !== "assistant" || typeof block.id !== "string" ||
          typeof block.name !== "string" || !recordValue(block.input)
        ) {
          errors.push(
            `body field messages[${index}].content[${blockIndex}] is an invalid Claude tool_use`,
          );
        } else if (pendingToolUses.has(block.id)) {
          errors.push(
            `body field messages[${index}].content[${blockIndex}] has a duplicate Claude tool_use id`,
          );
        } else pendingToolUses.add(block.id);
      }
      if (
        block.type === "tool_result" &&
        (role !== "user" || typeof block.tool_use_id !== "string")
      ) {
        errors.push(
          `body field messages[${index}].content[${blockIndex}] is an invalid Claude tool_result`,
        );
      }
    });
  });
  if (pendingToolUses.size > 0) {
    errors.push(
      "Claude tool_use blocks are missing an immediate tool_result message",
    );
  }
  const outputConfig = recordValue(body.output_config);
  const format = recordValue(outputConfig?.format);
  if (
    format &&
    (format.type !== "json_schema" || !recordValue(format.schema))
  ) {
    errors.push(
      "body field output_config.format must contain type=json_schema and an object schema",
    );
  }
  const thinking = recordValue(body.thinking);
  if (
    String(body.model ?? "").startsWith("claude-") &&
    String(body.model ?? "").split("-").includes("5") &&
    thinking?.type === "enabled"
  ) {
    errors.push(
      "body field thinking.type=enabled is invalid for Claude 5; use adaptive thinking",
    );
  }
}

function validateGeminiInteractionInput(
  input: unknown[],
  errors: string[],
): void {
  const pendingCalls = new Map<string, string>();
  input.forEach((value, index) => {
    const step = recordValue(value);
    if (!step || typeof step.type !== "string") return;
    const path = `body field input[${index}]`;
    if (step.type === "user_input" || step.type === "model_output") {
      if (
        !Array.isArray(step.content) || step.content.length === 0 ||
        step.content.some((part) => !recordValue(part))
      ) errors.push(`${path}.content must be a non-empty content array`);
      if (
        step.type === "model_output" && Array.isArray(step.content) &&
        step.content.some((part) =>
          ["function_call", "function_result", "thought"].includes(
            String(recordValue(part)?.type ?? ""),
          )
        )
      ) errors.push(`${path}.content cannot contain Gemini interaction steps`);
      return;
    }
    if (step.type === "function_call") {
      if (
        typeof step.id !== "string" || typeof step.name !== "string" ||
        !recordValue(step.arguments)
      ) errors.push(`${path} is an invalid Gemini function_call step`);
      else if (pendingCalls.has(step.id)) {
        errors.push(`${path}.id duplicates a preceding Gemini function_call`);
      } else pendingCalls.set(step.id, step.name);
      return;
    }
    if (step.type === "function_result") {
      if (
        typeof step.call_id !== "string" || !step.call_id ||
        typeof step.name !== "string" || !step.name ||
        step.result === undefined
      ) errors.push(`${path} is an invalid Gemini function_result step`);
      if (step.id !== undefined) {
        errors.push(`${path}.id is invalid; function_result requires call_id`);
      }
      if (
        typeof step.call_id === "string" && !pendingCalls.has(step.call_id)
      ) {
        errors.push(
          `${path}.call_id does not match a preceding Gemini function_call`,
        );
      }
      if (
        typeof step.call_id === "string" && pendingCalls.has(step.call_id) &&
        pendingCalls.get(step.call_id) !== step.name
      ) {
        errors.push(
          `${path}.name does not match the preceding Gemini function_call`,
        );
      }
      if (typeof step.call_id === "string") pendingCalls.delete(step.call_id);
      return;
    }
    if (
      step.type === "thought" && step.summary !== undefined &&
      (!Array.isArray(step.summary) ||
        step.summary.some((part) => recordValue(part)?.type !== "text"))
    ) errors.push(`${path}.summary must be an array of text content`);
  });
  if (pendingCalls.size > 0) {
    errors.push(
      "Gemini function_call steps are missing matching function_result steps",
    );
  }
}

function validateGeminiInteractionConfig(
  body: Record<string, unknown>,
  errors: string[],
): void {
  const generation = recordValue(body.generation_config);
  if (body.generation_config !== undefined && !generation) {
    errors.push("body field generation_config must be an object");
  }
  const generationKeys = new Set([
    "image_config",
    "max_output_tokens",
    "seed",
    "speech_config",
    "stop_sequences",
    "thinking_budget",
    "thinking_level",
    "thinking_summaries",
    "tool_choice",
    "transcription_config",
    "video_config",
  ]);
  for (const key of Object.keys(generation ?? {})) {
    if (!generationKeys.has(key)) {
      errors.push(
        `body field generation_config.${key} is not defined by Gemini v1beta`,
      );
    }
  }
  const model = String(body.model ?? "");
  if (
    model.startsWith("gemini-3") && generation?.thinking_budget !== undefined
  ) {
    errors.push(
      "body field generation_config.thinking_budget is invalid for Gemini 3; use thinking_level",
    );
  }
  if (
    model.startsWith("gemini-3") && generation?.thinking_level !== undefined
  ) {
    const allowed = gemini3ThinkingLevels(model);
    if (!allowed.has(String(generation.thinking_level))) {
      errors.push(
        `body field generation_config.thinking_level=${generation.thinking_level}; ${model} supports ${
          [...allowed].join("|")
        }`,
      );
    }
  }
  if (
    model.startsWith("gemini-2.5") && generation?.thinking_level !== undefined
  ) {
    errors.push(
      "body field generation_config.thinking_level is invalid for Gemini 2.5; use thinking_budget",
    );
  }
  const formats = Array.isArray(body.response_format)
    ? body.response_format
    : [body.response_format];
  formats.filter((format) => format !== undefined).forEach((value, index) => {
    const format = recordValue(value);
    const path = Array.isArray(body.response_format)
      ? `body field response_format[${index}]`
      : "body field response_format";
    if (
      !format ||
      !["text", "image", "audio", "video"].includes(String(format.type ?? ""))
    ) {
      errors.push(`${path} must be a typed Gemini response format object`);
      return;
    }
    const keys: Record<string, Set<string>> = {
      text: new Set(["type", "mime_type", "schema"]),
      image: new Set([
        "type",
        "aspect_ratio",
        "delivery",
        "image_size",
        "mime_type",
      ]),
      audio: new Set([
        "type",
        "bit_rate",
        "delivery",
        "mime_type",
        "sample_rate",
      ]),
      video: new Set([
        "type",
        "aspect_ratio",
        "delivery",
        "duration",
        "resolution",
      ]),
    };
    for (const key of Object.keys(format)) {
      if (!keys[String(format.type)].has(key)) {
        errors.push(
          `${path}.${key} is not defined for Gemini ${format.type} output`,
        );
      }
    }
    if (
      format.type === "text" && format.mime_type !== undefined &&
      !["text/plain", "application/json"].includes(String(format.mime_type))
    ) errors.push(`${path}.mime_type is invalid for Gemini text output`);
    if (
      format.type === "image" && format.mime_type !== undefined &&
      format.mime_type !== "image/jpeg"
    ) errors.push(`${path}.mime_type is invalid for Gemini image output`);
    if (
      format.type === "audio" && format.mime_type !== undefined &&
      ![
        "audio/mp3",
        "audio/ogg_opus",
        "audio/l16",
        "audio/wav",
        "audio/alaw",
        "audio/mulaw",
      ].includes(String(format.mime_type))
    ) errors.push(`${path}.mime_type is invalid for Gemini audio output`);
  });
}

function gemini3ThinkingLevels(model: string): Set<string> {
  if (model === "gemini-3-pro-preview") return new Set(["low", "high"]);
  if (model === "gemini-3.1-flash-lite-image") {
    return new Set(["minimal", "high"]);
  }
  if (
    ["gemini-3.8-flash", "gemini-3.7-flash", "gemini-3.1-pro-preview"].includes(
      model,
    )
  ) {
    return new Set(["low", "medium", "high"]);
  }
  return new Set(["minimal", "low", "medium", "high"]);
}

export function validateProviderSuccessFixture(
  contract: ProviderProtocolContract,
): string[] {
  if (contract.success_fixture_base64) return [];
  const fixture = recordValue(contract.success_fixture);
  if (!fixture) return ["fixture must be an object"];
  const errors: string[] = [];
  const requireArray = (field: string) => {
    if (
      !Array.isArray(fixture[field]) ||
      (fixture[field] as unknown[]).length === 0
    ) {
      errors.push(`${field} must be a non-empty array`);
    }
  };
  switch (contract.operation) {
    case "messages.create":
      requireArray("content");
      break;
    case "chat.completions.create":
      requireArray("choices");
      break;
    case "responses.create":
      requireArray("output");
      break;
    case "embeddings.create":
      requireArray("data");
      break;
    case "rerank.create":
      requireArray("results");
      break;
    case "models.embedContent": {
      const embedding = recordValue(fixture.embedding);
      if (
        !embedding || !Array.isArray(embedding.values) ||
        embedding.values.length === 0
      ) {
        errors.push("embedding.values must be a non-empty array");
      }
      break;
    }
    case "interactions.create": {
      if (fixture.outputs !== undefined) {
        errors.push("outputs is not an Interactions response field");
      }
      if (!Array.isArray(fixture.steps) || fixture.steps.length === 0) {
        errors.push("steps must be a non-empty array");
        break;
      }
      fixture.steps.forEach((value, index) => {
        const step = recordValue(value);
        if (
          !step || step.type !== "model_output" ||
          !Array.isArray(step.content) || step.content.length === 0
        ) {
          errors.push(`steps[${index}] must be a model_output with content`);
        }
      });
      break;
    }
    case "queue.submit":
      if (typeof fixture.request_id !== "string" || !fixture.request_id) {
        errors.push("request_id is required");
      }
      if (
        typeof fixture.response_url !== "string" ||
        !fixture.response_url.endsWith("/response")
      ) {
        errors.push("response_url must end with /response");
      }
      break;
  }
  if (contract.id === "fal.deepfilternet3.queue-v1") {
    const timings = recordValue(contract.async_result_fixture?.timings);
    for (const field of ["preprocess", "inference", "postprocess"]) {
      if (typeof timings?.[field] !== "number") {
        errors.push(`async_result_fixture.timings.${field} is required`);
      }
    }
  }
  return errors;
}

function caseId(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-");
}

function providerErrorCode(fixture: ProtocolErrorFixture): string {
  const body = fixture.body;
  const error = body.error && typeof body.error === "object"
    ? body.error as Record<string, unknown>
    : undefined;
  const baseResponse = body.base_resp && typeof body.base_resp === "object"
    ? body.base_resp as Record<string, unknown>
    : undefined;
  for (
    const value of [
      error?.code,
      error?.status,
      error?.type,
      body.code,
      body.error_type,
      baseResponse?.status_code,
    ]
  ) {
    if (value !== undefined && value !== null && String(value)) {
      return String(value);
    }
  }
  return String(fixture.status);
}

function providerErrorRetriable(fixture: ProtocolErrorFixture): boolean {
  const explicit = Object.entries(fixture.headers ?? {}).find(([name]) =>
    name.toLowerCase() === "x-fal-retryable"
  )?.[1];
  if (explicit) return explicit.toLowerCase() === "true";
  return fixture.status === 429 || fixture.status >= 500 ||
    [
      "rate_limit",
      "quota",
      "timeout",
      "unavailable",
      "overloaded",
      "server_error",
      "provider_error",
    ]
      .includes(fixture.scenario);
}

const TASK_RESULT_ARTIFACT_API_TYPES = new Set([
  "image.txt2img",
  "image.img2img",
  "image.inpaint",
  "image.upscale",
  "image.bg_remove",
  "audio.tts",
  "audio.music",
  "audio.enhance",
  "video.txt2video",
  "video.img2video",
  "video.video2video",
  "video.extend",
  "video.upscale",
]);

type VariantCell = {
  provider_driver: string;
  contract_id: string;
  api_type: string;
  model: ProviderModel;
  expected_provider_options?: Record<string, unknown>;
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
          tags: [
            "provider_protocol",
            provider.provider_driver,
            contract.protocol_adapter_id,
            apiType,
          ],
          input_entry: "zone_gateway",
          user: "acceptance-user-a",
          session: "isolated-per-case",
          provider_driver: provider.provider_driver,
          provider_instance: `t15-${provider.provider_driver}`,
          model_selector: null,
          api_type: apiType,
          method: methodsForApiType(apiType)[0] ?? apiType,
          execution_mode: "immediate",
          required_capabilities: [],
          disabled_capabilities: [],
          fixtures: [],
          expected_exact_model: null,
          expected_provider_instance: `t15-${provider.provider_driver}`,
          expected_task_status: "succeeded",
          expected_error_class: null,
          expected_output: {
            kinds: [],
            attachment_count: { min: 0, max: 0 },
            mime_types: [],
          },
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
          case_id: caseId(
            `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.success`,
          ),
          mock_scenario: "success",
        } as AcceptanceCase);
        if (TASK_RESULT_ARTIFACT_API_TYPES.has(apiType)) {
          cases.push({
            ...common,
            case_id: caseId(
              `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.task-result-artifact`,
            ),
            tags: [...common.tags!, "task_result_artifact"],
            mock_scenario: "success",
            expected_wire_fixture:
              `${contract.id}.request.task-result-artifact`,
            response_fixture: `${contract.id}.success`,
          } as AcceptanceCase);
        }
        const primaryApiType = contract.api_types[0];
        if (apiType === primaryApiType && contract.stream_protocol) {
          cases.push({
            ...common,
            case_id: caseId(
              `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.stream`,
            ),
            mock_scenario: "stream_success",
            execution_mode: "stream",
            expected_wire_fixture: `${contract.id}.request.stream`,
            response_fixture: `${contract.id}.stream`,
          } as AcceptanceCase);
          cases.push({
            ...common,
            case_id: caseId(
              `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.stream-interrupted`,
            ),
            priority: "P1",
            mock_scenario: "stream_interrupted",
            execution_mode: "stream",
            expected_task_status: "failed",
            expected_error_class: "provider_protocol_failed",
            expected_wire_fixture: `${contract.id}.request.stream`,
            response_fixture: `${contract.id}.stream.interrupted`,
          } as AcceptanceCase);
        }
        if (apiType === primaryApiType && contract.async_protocol) {
          cases.push({
            ...common,
            case_id: caseId(
              `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.async`,
            ),
            mock_scenario: "async_success",
            expected_wire_fixture: `${contract.id}.request.async`,
            response_fixture: `${contract.id}.async`,
          } as AcceptanceCase);
          cases.push({
            ...common,
            case_id: caseId(
              `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.async-failed`,
            ),
            priority: "P1",
            mock_scenario: "async_failed",
            expected_task_status: "failed",
            expected_error_class: "provider_protocol_failed",
            expected_wire_fixture: `${contract.id}.request.async`,
            response_fixture: `${contract.id}.async.failed`,
          } as AcceptanceCase);
          const terminalFailureScenarios =
            contract.async_protocol === "google_lro" ||
              contract.async_protocol === "minimax_video"
              ? ["async_poll_timeout"] as const
              : ["async_poll_timeout", "async_artifact_unavailable"] as const;
          for (const scenario of terminalFailureScenarios) {
            cases.push({
              ...common,
              case_id: caseId(
                `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.${scenario}`,
              ),
              priority: "P1",
              mock_scenario: scenario,
              expected_task_status: "failed",
              expected_error_class: "provider_protocol_failed",
              expected_wire_fixture: `${contract.id}.request.async`,
              response_fixture: `${contract.id}.${scenario}`,
              timeout_ms: scenario === "async_poll_timeout"
                ? 1_500
                : common.timeout_ms,
            } as AcceptanceCase);
          }
          if (contract.async_steps?.some((step) => step.name === "cancel")) {
            cases.push({
              ...common,
              case_id: caseId(
                `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.async-cancel`,
              ),
              priority: "P1",
              mock_scenario: "async_cancel",
              expected_task_status: "failed",
              expected_wire_fixture: `${contract.id}.request.async.cancel`,
              response_fixture: `${contract.id}.async.cancelled`,
            } as AcceptanceCase);
          }
        }
        for (
          const error of apiType === primaryApiType
            ? catalog.error_fixtures[provider.provider_driver]
            : []
        ) {
          cases.push({
            ...common,
            case_id: caseId(
              `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.error.${error.scenario}`,
            ),
            priority: "P1",
            tags: [...common.tags!, "official_error"],
            mock_scenario: error.scenario,
            expected_task_status: "failed",
            expected_error_class: "provider_protocol_failed",
            response_fixture:
              `${provider.provider_driver}.error.${error.scenario}`,
            expected_aicc_error_code: "provider_error",
            expected_provider_error_code: providerErrorCode(error),
            expected_retriable: providerErrorRetriable(error),
          } as AcceptanceCase);
        }
        if (apiType === primaryApiType) {
          for (
            const scenario of [
              "malformed_response",
              "wrong_content_type",
              "missing_required_response_field",
            ] as const
          ) {
            cases.push({
              ...common,
              case_id: caseId(
                `t1.5.${provider.provider_driver}.${contract.id}.${apiType}.response.${scenario}`,
              ),
              priority: "P1",
              mock_scenario: scenario,
              expected_task_status: "failed",
              expected_error_class: "provider_protocol_failed",
              response_fixture: `${contract.id}.response.${scenario}`,
            } as AcceptanceCase);
          }
        }
      }
    }
  }
  const cloudUpdateBase = cases.find((testCase) =>
    testCase.provider_driver === "openai" &&
    testCase.protocol_contract_id === "openai.responses.v1" &&
    testCase.api_type === "llm" &&
    testCase.mock_scenario === "success"
  );
  if (!cloudUpdateBase) {
    throw new Error(
      "OpenAI Responses LLM contract is required for the cloud update case",
    );
  }
  cases.push({
    ...cloudUpdateBase,
    case_id: "t1.5.openai.openai.responses.v1.llm.cloud-update",
    tags: [...cloudUpdateBase.tags, "cloud_update"],
    cleanup: [...cloudUpdateBase.cleanup, "restore_cloud_provider_rules"],
  });
  const historyProviders = new Set([
    "openai",
    "openrouter",
    "claude",
    "google-gemini",
  ]);
  const historyBases = cases.filter((testCase) =>
    historyProviders.has(testCase.provider_driver ?? "") &&
    testCase.api_type === "llm" &&
    testCase.mock_scenario === "success" &&
    !testCase.tags.includes("cloud_update") &&
    !testCase.tags.includes("history") &&
    !testCase.tags.includes("tool_history")
  );
  for (const base of historyBases) {
    cases.push({
      ...base,
      case_id: caseId(
        `t1.5.${base.provider_driver}.${base.protocol_contract_id}.llm.history`,
      ),
      tags: [...base.tags, "history"],
      expected_wire_fixture: `${base.protocol_contract_id}.request.history`,
    });
    cases.push({
      ...base,
      case_id: caseId(
        `t1.5.${base.provider_driver}.${base.protocol_contract_id}.llm.tool-history`,
      ),
      tags: [...base.tags, "tool_history"],
      expected_wire_fixture:
        `${base.protocol_contract_id}.request.tool-history`,
    });
    if (base.provider_driver === "openai") {
      cases.push({
        ...base,
        case_id: caseId(
          `t1.5.${base.provider_driver}.${base.protocol_contract_id}.llm.native-history`,
        ),
        tags: [...base.tags, "history", "native_history"],
        expected_wire_fixture:
          `${base.protocol_contract_id}.request.native-history`,
      });
    }
    if (base.provider_driver === "openrouter") {
      cases.push({
        ...base,
        case_id: caseId(
          `t1.5.${base.provider_driver}.${base.protocol_contract_id}.llm.reasoning-history`,
        ),
        tags: [...base.tags, "history", "reasoning_history"],
        expected_wire_fixture:
          `${base.protocol_contract_id}.request.reasoning-history`,
      });
    }
    if (base.provider_driver === "claude") {
      cases.push({
        ...base,
        case_id: caseId(
          `t1.5.${base.provider_driver}.${base.protocol_contract_id}.llm.structured-output`,
        ),
        tags: [...base.tags, "structured_output"],
        expected_wire_fixture:
          `${base.protocol_contract_id}.request.structured-output`,
      });
    }
  }
  for (const source of historyBases) {
    for (const target of historyBases) {
      if (source.provider_driver === target.provider_driver) continue;
      cases.push({
        ...target,
        case_id: caseId(
          `t1.5.switch.${source.provider_driver}.${source.protocol_contract_id}.llm.to.${target.provider_driver}.${target.protocol_contract_id}.llm`,
        ),
        priority: "P0",
        tags: [...target.tags, "history", "provider_switch_matrix"],
        session: "shared-provider-switch",
        expected_wire_fixture:
          `${target.protocol_contract_id}.request.provider-switch`,
        switch_source_provider_driver: source.provider_driver ?? undefined,
        switch_source_contract_id: source.protocol_contract_id,
        switch_source_model_id: catalog.providers.find((provider) =>
          provider.provider_driver === source.provider_driver
        )
          ?.test_model_ids.llm,
        switch_target_provider_driver: target.provider_driver ?? undefined,
        switch_target_contract_id: target.protocol_contract_id,
      } as AcceptanceCase);
    }
  }
  const customDrivers = new Set(["openai", "claude", "google-gemini", "fal"]);
  cases.push(
    ...cases.filter((testCase) =>
      customDrivers.has(testCase.provider_driver ?? "") &&
      testCase.mock_scenario === "success" &&
      !testCase.tags.includes("cloud_update") &&
      !testCase.tags.includes("history") &&
      !testCase.tags.includes("tool_history") &&
      !testCase.tags.includes("structured_output") &&
      !testCase.tags.includes("task_result_artifact") &&
      !testCase.tags.includes("custom_provider")
    ).map((testCase) => ({
      ...testCase,
      case_id: caseId(
        `t1.5.custom.${testCase.provider_driver}.${testCase.protocol_contract_id}.${testCase.api_type}.success`,
      ),
      tags: [...testCase.tags, "custom_provider"],
      provider_instance: `t15-custom-${testCase.provider_driver}`,
      expected_provider_instance: `t15-custom-${testCase.provider_driver}`,
    })),
  );
  for (const variant of variants) {
    const contract = protocolContract(
      catalog,
      variant.provider_driver,
      variant.contract_id,
    );
    const openAiResponsesImage = variant.provider_driver === "openai" &&
      variant.model.provider_model_id.startsWith("gpt-5") &&
      ["image.txt2img", "image.img2img"].includes(variant.api_type) &&
      contract.operation === "responses.create";
    if (
      !contract.api_types.includes(variant.api_type) && !openAiResponsesImage
    ) {
      throw new Error(
        `${variant.contract_id} does not support ${variant.api_type}`,
      );
    }
    cases.push({
      case_id: caseId(
        `t1.5.${variant.provider_driver}.${contract.id}.${variant.api_type}.variant.${variant.model.provider_model_id}`,
      ),
      layer: "T1.5",
      priority: "P0",
      tags: [
        "provider_protocol",
        "variant",
        variant.provider_driver,
        variant.api_type,
      ],
      input_entry: "zone_gateway",
      user: "acceptance-user-a",
      session: "isolated-per-case",
      provider_driver: variant.provider_driver,
      provider_instance: `t15-${variant.provider_driver}`,
      model_selector: { kind: "exact", value: variant.model.exact_model },
      api_type: variant.api_type,
      method: methodsForApiType(variant.api_type)[0] ?? variant.api_type,
      execution_mode: "immediate",
      required_capabilities: [],
      disabled_capabilities: [],
      fixtures: [],
      mock_scenario: "success",
      expected_exact_model: variant.model.exact_model,
      expected_provider_instance: `t15-${variant.provider_driver}`,
      expected_task_status: "succeeded",
      expected_error_class: null,
      expected_output: {
        kinds: [],
        attachment_count: { min: 0, max: 0 },
        mime_types: [],
      },
      semantic_rubric: [],
      timeout_ms: 30_000,
      max_attempts: 1,
      estimated_cost_usd: 0,
      cleanup: ["reset_provider_mock", "remove_t15_provider_instance"],
      protocol_contract_id: contract.id,
      protocol_evidence_revision: catalog.revision,
      protocol_adapter_id: contract.protocol_adapter_id,
      provider_api_version: contract.api_version,
      expected_wire_fixture:
        `${contract.id}.request.variant.${variant.model.provider_model_id}`,
      response_fixture: `${contract.id}.success`,
      expected_provider_options: variant.expected_provider_options,
    });
  }
  return cases;
}
