import type { MatrixCell } from "./types.ts";

export type ResourceRef =
  | { kind: "url"; url: string; mime_hint?: string }
  | { kind: "named_object"; obj_id: string }
  | { kind: "base64"; mime: string; data_base64: string };

export type ResourceRepresentation = ResourceRef["kind"];
export type ResourceFixture = ResourceRef | Partial<Record<ResourceRepresentation, ResourceRef>>;

export type FixtureRefs = {
  image?: ResourceFixture;
  mask?: ResourceFixture;
  audio?: ResourceFixture;
  video?: ResourceFixture;
  document?: ResourceFixture;
  documents?: Record<string, ResourceFixture>;
};
type SingularFixtureKind = Exclude<keyof FixtureRefs, "documents">;

export function requiredFixtureKinds(apiType: string): string[] {
  if (apiType === "image.img2img" || apiType.startsWith("vision.")) return ["image"];
  if (apiType === "image.inpaint") return ["image", "mask"];
  if (apiType === "image.upscale" || apiType === "image.bg_remove") return ["image"];
  if (apiType === "embedding.multimodal") return ["image"];
  if (apiType === "audio.asr" || apiType === "audio.enhance") return ["audio"];
  if (apiType === "video.img2video") return ["image"];
  if (apiType === "video.video2video" || apiType === "video.extend" || apiType === "video.upscale") return ["video"];
  return [];
}

function requireFixture(
  fixtures: FixtureRefs,
  kind: SingularFixtureKind,
  apiType: string,
  representation?: ResourceRepresentation,
  documentFormat?: string,
): ResourceRef {
  const value = kind === "document" && documentFormat
    ? fixtures.documents?.[documentFormat] ?? (documentFormat === "pdf" ? fixtures.document : undefined)
    : fixtures[kind];
  if (!value) throw new Error(`${apiType} requires configured ${kind} fixture`);
  if ("kind" in value) {
    if (representation && value.kind !== representation) {
      throw new Error(`${apiType} requires ${kind} fixture representation ${representation}`);
    }
    return value;
  }
  const selected = representation ? value[representation] : value.base64 ?? value.named_object ?? value.url;
  if (!selected) {
    throw new Error(`${apiType} requires ${kind} fixture representation ${representation ?? "any"}`);
  }
  return selected;
}

function io(
  apiType: string,
  fixtures: FixtureRefs,
  representation?: ResourceRepresentation,
  documentFormat?: string,
): {
  input_json: Record<string, unknown>;
  resources: ResourceRef[];
} {
  switch (apiType) {
    case "llm":
      return {
        input_json: {
          messages: [{
            role: "user",
            content: [{
              type: "text",
              text: "Reply with the exact marker BUCKYOS-AICC-4827.",
            }],
          }],
          max_output_tokens: 512,
        },
        resources: [],
      };
    case "embedding.text":
      return {
        input_json: {
          items: [
            { id: "item-1", text: "BuckyOS AICC marker 4827" },
            { id: "item-2", text: "A second deterministic embedding item" },
          ],
        },
        resources: [],
      };
    case "embedding.multimodal":
      return {
        input_json: { items: [{ id: "item-1", text: "pink flower" }] },
        resources: [],
      };
    case "rerank":
      return {
        input_json: {
          query: "Which document contains marker 4827?",
          documents: [
            { id: "wrong", text: "This record has no marker." },
            { id: "right", text: "The marker is BUCKYOS-AICC-4827." },
          ],
        },
        resources: [],
      };
    case "image.txt2img":
      return { input_json: { prompt: "A blue square containing 4827" }, resources: [] };
    case "image.img2img":
      return {
        input_json: { prompt: "Preserve composition and use warm evening light" },
        resources: [requireFixture(fixtures, "image", apiType, representation)],
      };
    case "image.inpaint":
      return {
        input_json: { prompt: "Fill the masked region with green leaves" },
        resources: [
          requireFixture(fixtures, "image", apiType, representation),
          requireFixture(fixtures, "mask", apiType, representation),
        ],
      };
    case "image.upscale":
      return {
        input_json: { scale: 2 },
        resources: [requireFixture(fixtures, "image", apiType, representation)],
      };
    case "image.bg_remove":
      return {
        input_json: {},
        resources: [requireFixture(fixtures, "image", apiType, representation)],
      };
    case "vision.ocr":
      return {
        input_json: { prompt: "Return the visible marker." },
        resources: [requireFixture(fixtures, "image", apiType, representation)],
      };
    case "vision.caption":
    case "vision.detect":
    case "vision.segment":
      return {
        input_json: { prompt: `Execute ${apiType} on the supplied image.` },
        resources: [requireFixture(fixtures, "image", apiType, representation)],
      };
    case "audio.tts":
      return { input_json: { text: "BuckyOS test number four eight two seven" }, resources: [] };
    case "audio.asr":
    case "audio.enhance":
      return {
        input_json: apiType === "audio.asr" ? { language: "zh" } : { operation: "denoise" },
        resources: [requireFixture(fixtures, "audio", apiType, representation)],
      };
    case "audio.music":
      return { input_json: { prompt: "A four-second calm instrumental test tone" }, resources: [] };
    case "video.txt2video":
      return { input_json: { prompt: "A paper plane moving across a desk", duration_seconds: 4 }, resources: [] };
    case "video.img2video":
      return {
        input_json: { prompt: "Slow camera push with subtle motion", duration_seconds: 4 },
        resources: [requireFixture(fixtures, "image", apiType, representation)],
      };
    case "video.video2video":
    case "video.extend":
    case "video.upscale":
      return {
        input_json: apiType === "video.extend" ? { duration_seconds: 4 } : {},
        resources: [requireFixture(fixtures, "video", apiType, representation)],
      };
    case "agent.computer_use":
      return {
        input_json: {
          task: "Report the title visible in the supplied test environment.",
          environment: "browser",
        },
        resources: [],
      };
    default:
      throw new Error(`no payload builder for ${apiType}`);
  }
}

export function buildExactRequest(args: {
  cell: MatrixCell;
  runId: string;
  fixtures: FixtureRefs;
}): Record<string, unknown> {
  const payload = io(
    args.cell.api_type,
    args.fixtures,
    args.cell.resource_representation,
    args.cell.document_format,
  );
  const inputJson = { ...payload.input_json };
  const resources = [...payload.resources];
  const fixture = (kind: SingularFixtureKind): ResourceRef => requireFixture(
    args.fixtures,
    kind,
    args.cell.api_type,
    args.cell.resource_representation,
    kind === "document" ? args.cell.document_format : undefined,
  );
  let requirements: Record<string, unknown> = {};
  let toolSpecs: Record<string, unknown>[] = [];
  if (args.cell.api_type === "llm") {
    if (args.cell.input_kinds.includes("code")) {
      inputJson.messages = [{
        role: "user",
        content: [{ type: "text", text: "Complete this code with marker 4827: const marker =" }],
      }];
    }
    for (const kind of args.cell.input_kinds) {
      if (kind === "image") resources.push(fixture("image"));
      else if (kind === "document") resources.push(fixture("document"));
      else if (kind === "audio") resources.push(fixture("audio"));
      else if (kind === "video") resources.push(fixture("video"));
    }
    if (args.cell.input_kinds.includes("image")) requirements = { must_features: ["vision"] };
  }
  if (args.cell.api_type === "embedding.multimodal") {
    inputJson.items = args.cell.input_kinds.includes("text")
      ? [{ id: "item-1", text: "pink flower" }]
      : [];
    for (const kind of args.cell.input_kinds) {
      if (kind === "image") resources.push(fixture("image"));
      else if (kind === "audio") resources.push(fixture("audio"));
      else if (kind === "video") resources.push(fixture("video"));
      else if (kind === "document") resources.push(fixture("document"));
    }
  }
  if (args.cell.api_type === "llm" && args.cell.output_kinds.includes("json")) {
    const messages = inputJson.messages as Array<Record<string, unknown>>;
    messages[0] = {
      role: "user",
      content: [{ type: "text", text: "Return JSON with marker exactly BUCKYOS-AICC-4827." }],
    };
    inputJson.response_format = {
      type: "json_schema",
      name: "aicc_acceptance_marker",
      strict: true,
      schema: {
        type: "object",
        properties: { marker: { type: "string", const: "BUCKYOS-AICC-4827" } },
        required: ["marker"],
        additionalProperties: false,
      },
    };
    requirements = {
      ...requirements,
      must_features: [...new Set([...((requirements.must_features as string[] | undefined) ?? []), "json_output"])],
      resp_format: "json",
    };
  }
  if (args.cell.api_type === "llm" && args.cell.output_kinds.includes("tool_call")) {
    const messages = inputJson.messages as Array<Record<string, unknown>>;
    messages[0] = {
      role: "user",
      content: [{ type: "text", text: "Call echo_marker once with marker BUCKYOS-AICC-4827." }],
    };
    inputJson.tool_choice = "required";
    toolSpecs = [{
      name: "echo_marker",
      description: "Returns the supplied deterministic marker",
      args_schema: {
        type: "object",
        properties: { marker: { type: "string" } },
        required: ["marker"],
        additionalProperties: false,
      },
      output_schema: { type: "object" },
    }];
    requirements = {
      ...requirements,
      must_features: [...new Set([...((requirements.must_features as string[] | undefined) ?? []), "tool_calling"])],
    };
  }
  if (args.cell.variant === "embedding_large_artifact") {
    inputJson.items = Array.from({ length: 101 }, (_, index) => ({
      id: `item-${index + 1}`,
      text: `BuckyOS deterministic embedding row ${index + 1}`,
    }));
    inputJson.response_format = "object_id";
    inputJson.output = { resource_format: "named_object" };
  }
  return {
    capability: args.cell.api_type.split(".")[0],
    model: { alias: args.cell.exact_model },
    requirements,
    ...(args.cell.method === "llm.chat" ? { disable: { web_search: true } } : {}),
    payload: {
      input_json: inputJson,
      resources,
      tool_specs: toolSpecs,
      options: {
        session_id: `${args.runId}:${args.cell.case_id}`,
        rootid: args.runId,
      },
    },
    idempotency_key: `${args.runId}:${args.cell.case_id}`,
  };
}

export function assertResponseShape(
  cell: MatrixCell,
  value: unknown,
): void {
  if (!value || typeof value !== "object") throw new Error("response must be an object");
  const response = value as Record<string, unknown>;
  if (typeof response.task_id !== "string" || typeof response.status !== "string") {
    throw new Error("response must include task_id and status");
  }
  if (!["succeeded", "running"].includes(response.status)) {
    throw new Error(`unexpected task status ${String(response.status)}`);
  }
  if (response.status === "running") return;
  const result = response.result;
  if (!result || typeof result !== "object") throw new Error("succeeded response must include result");
  const resultRecord = result as Record<string, unknown>;
  for (const legacy of ["text", "tool_calls", "artifacts"]) {
    if (legacy in resultRecord) throw new Error(`deprecated top-level response field ${legacy}`);
  }
  if (!resultRecord.usage || typeof resultRecord.usage !== "object") {
    throw new Error("successful response must include usage");
  }
  if (!resultRecord.cost || typeof resultRecord.cost !== "object") {
    throw new Error("successful response must include cost");
  }
  const message = resultRecord.message;
  if (!message || typeof message !== "object") throw new Error("result.message is required");
  const messageRecord = message as Record<string, unknown>;
  if (messageRecord.role !== "assistant") throw new Error("result.message.role must be assistant");
  const content = messageRecord.content;
  if (!Array.isArray(content)) throw new Error("result.message.content must be an array");
  const text = content.filter((item) => item && typeof item === "object" &&
    (item as Record<string, unknown>).type === "text")
    .map((item) => (item as Record<string, unknown>).text)
    .filter((item): item is string => typeof item === "string")
    .join("\n");
  const extra = resultRecord.extra && typeof resultRecord.extra === "object"
    ? resultRecord.extra as Record<string, unknown>
    : {};

  if (cell.api_type === "llm" && cell.output_kinds.includes("json")) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      throw new Error("JSON output is not valid JSON");
    }
    if (!parsed || typeof parsed !== "object" ||
      (parsed as Record<string, unknown>).marker !== "BUCKYOS-AICC-4827") {
      throw new Error("JSON output does not satisfy the marker schema");
    }
    return;
  }
  if (cell.api_type === "llm" && cell.output_kinds.includes("tool_call")) {
    const calls = content.filter((item) => item && typeof item === "object" &&
      (item as Record<string, unknown>).type === "tool_use") as Array<Record<string, unknown>>;
    if (calls.length !== 1 || calls[0].name !== "echo_marker" ||
      typeof calls[0].call_id !== "string" || !calls[0].call_id ||
      !calls[0].args || typeof calls[0].args !== "object" ||
      (calls[0].args as Record<string, unknown>).marker !== "BUCKYOS-AICC-4827") {
      throw new Error("tool-call output does not match echo_marker contract");
    }
    return;
  }
  if (cell.api_type === "llm" && !text.includes("BUCKYOS-AICC-4827")) {
    throw new Error("LLM output omitted the requested acceptance marker");
  }

  if (cell.api_type.startsWith("embedding.")) {
    const embedding = extra.embedding;
    if (!embedding || typeof embedding !== "object") throw new Error("expected extra.embedding");
    const record = embedding as Record<string, unknown>;
    if (typeof record.embedding_space_id !== "string" || !record.embedding_space_id) {
      throw new Error("embedding_space_id is required");
    }
    if (cell.variant === "embedding_large_artifact") {
      const artifact = record.artifact;
      if (!artifact || typeof artifact !== "object") {
        throw new Error("large embedding output must use an artifact");
      }
      const artifactRecord = artifact as Record<string, unknown>;
      if (artifactRecord.rows !== 101 || typeof artifactRecord.dimensions !== "number" ||
        artifactRecord.dimensions <= 0 || artifactRecord.embedding_space_id !== record.embedding_space_id) {
        throw new Error("embedding artifact rows, dimensions, and space metadata are invalid");
      }
      return;
    }
    if (!Array.isArray(record.data)) throw new Error("embedding.data must be an array");
    const expectedItems = cell.api_type === "embedding.text" ? 2 : 1;
    if (record.data.length !== expectedItems) {
      throw new Error(`embedding item count ${record.data.length} != ${expectedItems}`);
    }
    let dimensions: number | undefined;
    for (const [index, item] of record.data.entries()) {
      if (!item || typeof item !== "object") throw new Error(`embedding item ${index} is invalid`);
      const itemRecord = item as Record<string, unknown>;
      if (itemRecord.index !== index) {
        throw new Error(`embedding item ${index} has provider index ${String(itemRecord.index)}`);
      }
      const vector = itemRecord.embedding ?? itemRecord.values;
      if (!Array.isArray(vector) || vector.length === 0 ||
        vector.some((number) => typeof number !== "number" || !Number.isFinite(number))) {
        throw new Error(`embedding item ${index} must contain a finite vector`);
      }
      dimensions ??= vector.length;
      if (dimensions !== vector.length) throw new Error("embedding dimensions are inconsistent");
    }
    return;
  }
  if (cell.api_type === "rerank") {
    const rerank = extra.rerank;
    if (!rerank || typeof rerank !== "object") throw new Error("expected extra.rerank");
    const results = (rerank as Record<string, unknown>).results;
    if (!Array.isArray(results) || results.length !== 2) throw new Error("rerank must return two results");
    const first = results[0] as Record<string, unknown>;
    if (first.id !== "right" || typeof first.score !== "number" || !Number.isFinite(first.score)) {
      throw new Error("rerank must rank the marker document first with a finite score");
    }
    return;
  }
  const artifactKinds = new Set(["image", "audio", "video"]);
  if (cell.output_kinds.some((kind) => artifactKinds.has(kind))) {
    const expectedPrefix = `${cell.output_kinds.find((kind) => artifactKinds.has(kind))}/`;
    const artifacts = content.filter((item) => item && typeof item === "object" &&
      ["image", "document"].includes(String((item as Record<string, unknown>).type)));
    if (artifacts.length === 0) throw new Error(`expected ${expectedPrefix} artifact output`);
    const materialized = Array.isArray(extra.materialized_artifacts)
      ? extra.materialized_artifacts.filter((item): item is Record<string, unknown> =>
        Boolean(item) && typeof item === "object" && !Array.isArray(item))
      : [];
    const hasMime = artifacts.some((item, index) => {
      const source = (item as Record<string, unknown>).source;
      if (!source || typeof source !== "object") return false;
      const resource = source as Record<string, unknown>;
      const materializedMime = materialized.find((entry) => entry.content_index === index)?.mime;
      const mime = resource.mime_hint ?? resource.mime ?? materializedMime;
      const addressable = typeof resource.url === "string" || typeof resource.obj_id === "string" ||
        typeof resource.data_base64 === "string";
      return addressable && typeof mime === "string" && mime.startsWith(expectedPrefix);
    });
    if (!hasMime) throw new Error(`artifact must be addressable and use MIME ${expectedPrefix}*`);
    return;
  }
  if (cell.api_type === "vision.detect" || cell.api_type === "vision.segment") {
    if (!text.trim() && Object.keys(extra).length === 0) {
      throw new Error("expected structured vision output");
    }
    return;
  }
  if (cell.output_kinds.includes("text") && !text.trim()) {
    throw new Error("expected non-empty text output block");
  }
}
