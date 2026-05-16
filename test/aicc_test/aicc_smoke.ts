import { ndm_proxy } from "buckyos";

import { initTestRuntime } from "../test_helpers/buckyos_client.ts";

type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

type AiccMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: AiResponse | null;
  event_ref?: string | null;
};

type AiArtifact = {
  name: string;
  resource: {
    kind: string;
    obj_id?: string;
    url?: string;
    mime?: string;
    mime_hint?: string;
    data_base64?: string;
  };
  mime?: string | null;
  metadata?: JsonValue | null;
};

type AiContent =
  | { type: "text"; text: string }
  | { type: "image"; source: AiArtifact["resource"] }
  | { type: "document"; source: AiArtifact["resource"]; title?: string | null }
  | {
    type: "tool_use";
    call_id: string;
    name: string;
    args?: Record<string, JsonValue>;
  }
  | {
    type: "thinking";
    summary?: string | null;
    text?: string | null;
    provider_metadata?: JsonValue | null;
  }
  | { type: "provider_state"; provider: string; value: JsonValue };

type AiMessage = {
  role: "assistant";
  content: AiContent[];
};

type AiResponse = {
  message: AiMessage;
  usage?: JsonValue | null;
  cost?: JsonValue | null;
  finish_reason?: string | null;
  provider_task_ref?: string | null;
  extra?: JsonValue | null;
};

type TaskRecord = {
  id: number;
  status: string;
  message?: string | null;
  updated_at?: number;
  data?: {
    aicc?: {
      external_task_id?: string;
      output?: JsonValue;
      error?: JsonValue;
      status?: string;
    };
  };
};

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

type ModelEntry = {
  exact_model: string;
  api_types: string[];
  logical_mounts: string[];
};

type ProviderEntry = {
  models: ModelEntry[];
};

type ModelsListResponse = {
  providers?: ProviderEntry[];
};

type ResourceRef =
  | { kind: "url"; url: string; mime_hint?: string }
  | { kind: "base64"; mime: string; data_base64: string }
  | { kind: "named_object"; obj_id: string };

type SmokePayload = {
  input_json?: Record<string, unknown>;
  resources?: ResourceRef[];
  options?: Record<string, unknown>;
};

type SmokeContext = {
  generatedImage?: ResourceRef;
  generatedImageDimensions?: ImageDimensions;
  ndmProxy?: NdmProxyClient;
};

type ImageDimensions = {
  width: number;
  height: number;
};

function materializedImageDimensions(
  response: AiResponse,
): ImageDimensions | null {
  if (
    !response.extra || typeof response.extra !== "object" ||
    Array.isArray(response.extra)
  ) {
    return null;
  }
  const artifacts = (response.extra as Record<string, JsonValue>)
    .materialized_artifacts;
  if (!Array.isArray(artifacts)) {
    return null;
  }
  for (const item of artifacts) {
    if (!item || typeof item !== "object" || Array.isArray(item)) {
      continue;
    }
    const record = item as Record<string, JsonValue>;
    const mime = typeof record.mime === "string" ? record.mime : "";
    const width = typeof record.width === "number" ? record.width : 0;
    const height = typeof record.height === "number" ? record.height : 0;
    if (mime.startsWith("image/") && width > 0 && height > 0) {
      return { width, height };
    }
  }
  return null;
}

type Capability =
  | "llm"
  | "embedding"
  | "rerank"
  | "image"
  | "vision"
  | "audio"
  | "video"
  | "agent";

type SummaryCheck =
  | "text"
  | "embedding"
  | "artifact"
  | "vision"
  | "video"
  | "any";

type SmokeCase = {
  method: string;
  capability: Capability;
  defaultAlias?: string;
  buildPayload: (
    runId: string,
    context: SmokeContext,
  ) => SmokePayload | Promise<SmokePayload>;
  check: SummaryCheck;
  requirements?: Record<string, unknown>;
  capturesGeneratedImage?: boolean;
  returnsUnstructuredData?: boolean;
  requiresGeneratedImage?: boolean;
};

type SavedArtifact = {
  artifact_index: number;
  name: string;
  source_kind: string;
  source_url?: string;
  object_id?: string;
  mime?: string;
  path?: string;
  bytes?: number;
  opened: boolean;
};

type FinalOpenedResource = {
  source_kind: string;
  obj_id?: string;
  source_url?: string;
  path: string;
  bytes: number;
  mime?: string;
  resolved_obj_id?: string;
  reader_kind?: string;
};

type NdmProxyClient = {
  openReader: (
    request: { obj_id: string; inner_path?: string | null },
  ) => Promise<{
    response: Response;
    totalSize: number | null;
    resolvedObjectId?: string;
    readerKind?: string;
  }>;
};

type CaseResult =
  | {
    status: "passed";
    method: string;
    modelAlias: string;
    taskId: string;
    response: AiResponse;
    artifactFiles: SavedArtifact[];
    reportDir: string;
  }
  | {
    status: "skipped";
    method: string;
    modelAlias: string;
    reason: string;
    reportDir: string;
  }
  | {
    status: "failed";
    method: string;
    modelAlias: string;
    error: string;
    reportDir: string;
  };

const AICC_MODEL_ALIAS = getEnv("AICC_MODEL_ALIAS") ?? "llm.plan";
const AICC_TEST_INPUT = getEnv("AICC_TEST_INPUT") ??
  "今天天气如何，我在sanjose";
const AICC_WAIT_TIMEOUT_MS = Number(
  getEnv("AICC_WAIT_TIMEOUT_MS") ?? "90000",
);
const AICC_TEST_VIDEO_URL = getEnv("AICC_TEST_VIDEO_URL") ??
  "https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/360/Big_Buck_Bunny_360_10s_1MB.mp4";
const AICC_TEST_AUDIO_URL = getEnv("AICC_TEST_AUDIO_URL") ??
  "https://raw.githubusercontent.com/Jakobovski/free-spoken-digit-dataset/master/recordings/0_jackson_0.wav";
const AICC_TEST_AUDIO_BASE64 = getEnv("AICC_TEST_AUDIO_BASE64");
const AICC_SMOKE_METHODS = getEnv("AICC_SMOKE_METHODS");
const AICC_REPORT_DIR = getEnv("AICC_REPORT_DIR") ?? "reports/aicc_smoke";

const videoResource: ResourceRef = {
  kind: "url",
  url: AICC_TEST_VIDEO_URL,
  mime_hint: "video/mp4",
};
const audioResource: ResourceRef = AICC_TEST_AUDIO_BASE64
  ? {
    kind: "base64",
    mime: "audio/wav",
    data_base64: AICC_TEST_AUDIO_BASE64,
  }
  : {
    kind: "url",
    url: AICC_TEST_AUDIO_URL,
    mime_hint: "audio/wav",
  };
const allCases: SmokeCase[] = [
  {
    method: "image.txt2img",
    capability: "image",
    check: "artifact",
    capturesGeneratedImage: true,
    returnsUnstructuredData: true,
    buildPayload: () => ({
      input_json: {
        prompt:
          "A clean product-style icon of a small cloud node on a white background",
        size: "1024x1024",
      },
    }),
  },
  {
    method: "llm.chat",
    capability: "llm",
    defaultAlias: AICC_MODEL_ALIAS,
    check: "text",
    requirements: { must_features: ["web_search"] },
    buildPayload: () => ({
      input_json: {
        messages: [{
          role: "user",
          content: [{ type: "text", text: AICC_TEST_INPUT }],
        }],
        temperature: 0.2,
        max_output_tokens: 2560,
      },
    }),
  },
  {
    method: "llm.completion",
    capability: "llm",
    defaultAlias: AICC_MODEL_ALIAS,
    check: "text",
    buildPayload: () => ({
      input_json: {
        messages: [{
          role: "user",
          content: [{
            type: "text",
            text: `${AICC_TEST_INPUT}\n请用一句话回答。`,
          }],
        }],
        temperature: 0.2,
        max_output_tokens: 512,
      },
    }),
  },
  {
    method: "embedding.text",
    capability: "embedding",
    check: "embedding",
    buildPayload: () => ({
      input_json: { items: [{ text: "BuckyOS AICC smoke embedding text" }] },
    }),
  },
  {
    method: "embedding.multimodal",
    capability: "embedding",
    check: "embedding",
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: { items: [{ text: "A small gallery image" }] },
      resources: [requireGeneratedImage(context, "embedding.multimodal")],
    }),
  },
  {
    method: "rerank",
    capability: "rerank",
    check: "any",
    buildPayload: () => ({
      input_json: {
        query: "BuckyOS scheduler",
        documents: [
          {
            id: "a",
            text: "scheduler reads system-config and writes node_config",
          },
          { id: "b", text: "image generation returns artifacts" },
        ],
      },
    }),
  },
  {
    method: "image.img2img",
    capability: "image",
    check: "artifact",
    returnsUnstructuredData: true,
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: {
        prompt: "Make the image brighter while preserving the subject",
      },
      resources: [requireGeneratedImage(context, "image.img2img")],
    }),
  },
  {
    method: "image.inpaint",
    capability: "image",
    check: "artifact",
    returnsUnstructuredData: true,
    requiresGeneratedImage: true,
    buildPayload: async (_runId, context) => {
      const generatedImage = requireGeneratedImage(context, "image.inpaint");
      return {
        input_json: { prompt: "Fill the transparent area naturally" },
        resources: [
          generatedImage,
          await maskResourceForGeneratedImage(context),
        ],
      };
    },
  },
  {
    method: "image.upscale",
    capability: "image",
    check: "artifact",
    returnsUnstructuredData: true,
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: { scale: 2 },
      resources: [requireGeneratedImage(context, "image.upscale")],
    }),
  },
  {
    method: "image.bg_remove",
    capability: "image",
    check: "artifact",
    returnsUnstructuredData: true,
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      resources: [requireGeneratedImage(context, "image.bg_remove")],
    }),
  },
  {
    method: "vision.ocr",
    capability: "vision",
    check: "vision",
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: { prompt: "Extract any visible text from this image." },
      resources: [requireGeneratedImage(context, "vision.ocr")],
    }),
  },
  {
    method: "vision.caption",
    capability: "vision",
    check: "vision",
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: { prompt: "Caption this image in one sentence." },
      resources: [requireGeneratedImage(context, "vision.caption")],
    }),
  },
  {
    method: "vision.detect",
    capability: "vision",
    check: "vision",
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: { prompt: "Detect the main objects in this image." },
      resources: [requireGeneratedImage(context, "vision.detect")],
    }),
  },
  {
    method: "vision.segment",
    capability: "vision",
    check: "vision",
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: { prompt: "Segment the primary foreground object." },
      resources: [requireGeneratedImage(context, "vision.segment")],
    }),
  },
  {
    method: "audio.tts",
    capability: "audio",
    check: "artifact",
    returnsUnstructuredData: true,
    buildPayload: () => ({
      input_json: {
        text: "BuckyOS AICC text to speech smoke test.",
        voice: "alloy",
        output: { media_type: "audio/mpeg" },
      },
    }),
  },
  {
    method: "audio.asr",
    capability: "audio",
    check: "text",
    buildPayload: () => ({
      input_json: { language: "en" },
      resources: [audioResource],
    }),
  },
  {
    method: "audio.music",
    capability: "audio",
    defaultAlias: "audio.music.lyria-002",
    check: "artifact",
    returnsUnstructuredData: true,
    buildPayload: () => ({
      input_json: {
        prompt:
          "Generate an original four-second single-note electronic sine tone with no melody, lyrics, artist style, or copyrighted reference.",
      },
    }),
  },
  {
    method: "audio.enhance",
    capability: "audio",
    check: "artifact",
    returnsUnstructuredData: true,
    buildPayload: () => ({
      resources: [audioResource],
    }),
  },
  {
    method: "video.txt2video",
    capability: "video",
    check: "video",
    returnsUnstructuredData: true,
    buildPayload: () => ({
      input_json: {
        prompt: "A four-second animation of a cloud node pulsing gently.",
      },
    }),
  },
  {
    method: "video.img2video",
    capability: "video",
    check: "video",
    returnsUnstructuredData: true,
    requiresGeneratedImage: true,
    buildPayload: (_runId, context) => ({
      input_json: {
        prompt: "Animate the source image with a subtle camera move.",
      },
      resources: [requireGeneratedImage(context, "video.img2video")],
    }),
  },
  {
    method: "video.video2video",
    capability: "video",
    check: "video",
    returnsUnstructuredData: true,
    buildPayload: () => ({
      input_json: { prompt: "Stabilize and lightly enhance this input video." },
      resources: [videoResource],
    }),
  },
  {
    method: "video.extend",
    capability: "video",
    check: "video",
    returnsUnstructuredData: true,
    buildPayload: () => ({
      input_json: { prompt: "Extend the motion for a few more seconds." },
      resources: [videoResource],
    }),
  },
  {
    method: "video.upscale",
    capability: "video",
    check: "artifact",
    returnsUnstructuredData: true,
    buildPayload: () => ({
      resources: [videoResource],
    }),
  },
  {
    method: "agent.computer_use",
    capability: "agent",
    check: "any",
    buildPayload: () => ({
      input_json: {
        task: "Open a browser and report the page title.",
        environment: "browser",
      },
    }),
  },
];

function getEnv(name: string): string | null {
  const value = Deno.env.get(name);
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

class MissingGeneratedImageError extends Error {
  constructor(method: string) {
    super(`${method} requires image.txt2img to return an image artifact`);
    this.name = "MissingGeneratedImageError";
  }
}

function requireGeneratedImage(
  context: SmokeContext,
  method: string,
): ResourceRef {
  if (!context.generatedImage) {
    throw new MissingGeneratedImageError(method);
  }
  return context.generatedImage;
}

function normalizeTaskList(result: unknown): TaskRecord[] {
  if (Array.isArray(result)) {
    return result as TaskRecord[];
  }
  if (
    result && typeof result === "object" &&
    Array.isArray((result as { tasks?: unknown }).tasks)
  ) {
    return (result as { tasks: TaskRecord[] }).tasks;
  }
  return [];
}

function normalizeTask(result: unknown): TaskRecord {
  if (result && typeof result === "object" && "task" in result) {
    return (result as { task: TaskRecord }).task;
  }
  return result as TaskRecord;
}

function asAiResponse(value: unknown): AiResponse | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if ("text" in record || "tool_calls" in record || "artifacts" in record) {
    throw new Error(
      `deprecated response fields found: ${JSON.stringify(value)}`,
    );
  }
  const message = record.message as AiMessage | undefined;
  if (
    !message || message.role !== "assistant" ||
    !Array.isArray(message.content)
  ) {
    return null;
  }
  return value as AiResponse;
}

function safeFileSegment(value: string): string {
  const safe = value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return safe || "case";
}

function joinPath(base: string, ...segments: string[]): string {
  let result = base.replace(/\/+$/g, "");
  for (const segment of segments) {
    result = `${result}/${segment.replace(/^\/+|\/+$/g, "")}`;
  }
  return result;
}

async function writeJsonFile(path: string, value: unknown): Promise<void> {
  await Deno.writeTextFile(path, `${JSON.stringify(value, null, 2)}\n`);
}

function mimeExtension(mime: string | null | undefined): string | null {
  const normalized = mime?.split(";")[0]?.trim().toLowerCase();
  if (!normalized) {
    return null;
  }
  const known: Record<string, string> = {
    "image/png": "png",
    "image/jpeg": "jpg",
    "image/jpg": "jpg",
    "image/webp": "webp",
    "image/gif": "gif",
    "audio/mpeg": "mp3",
    "audio/mp3": "mp3",
    "audio/wav": "wav",
    "audio/x-wav": "wav",
    "audio/ogg": "ogg",
    "video/mp4": "mp4",
    "video/webm": "webm",
  };
  return known[normalized] ??
    normalized.split("/").at(-1)?.replace(/[^a-z0-9]+/g, "") ??
    null;
}

function extensionFromUrl(url: string): string | null {
  try {
    const pathname = new URL(url).pathname;
    const filename = pathname.split("/").pop() ?? "";
    const match = filename.match(/\.([a-z0-9]{1,8})$/i);
    return match?.[1]?.toLowerCase() ?? null;
  } catch {
    return null;
  }
}

function artifactMime(artifact: AiArtifact): string | undefined {
  return artifact.mime ?? artifact.resource.mime_hint ??
    artifact.resource.mime ??
    undefined;
}

function aiResponseTextContent(response: AiResponse): string {
  return response.message.content
    .filter((block): block is Extract<AiContent, { type: "text" }> =>
      block.type === "text"
    )
    .map((block) => block.text)
    .join("\n");
}

function aiResponseArtifacts(response: AiResponse): AiArtifact[] {
  const artifacts: AiArtifact[] = [];
  response.message.content.forEach((block, index) => {
    if (block.type === "image") {
      artifacts.push({
        name: `image_${index + 1}`,
        resource: block.source,
        mime: block.source.mime_hint ?? block.source.mime ?? "image/*",
      });
    }
    if (block.type === "document") {
      artifacts.push({
        name: block.title ?? `document_${index + 1}`,
        resource: block.source,
        mime: block.source.mime_hint ?? block.source.mime ?? null,
      });
    }
  });
  return artifacts;
}

function firstImageArtifact(
  response: AiResponse | null,
): ResourceRef | null {
  const artifacts = response ? aiResponseArtifacts(response) : [];
  for (const artifact of artifacts) {
    if (!artifact.resource) {
      continue;
    }
    const mime = artifactMime(artifact);
    if (mime?.startsWith("image/")) {
      if (
        artifact.resource.kind === "named_object" && artifact.resource.obj_id
      ) {
        return { kind: "named_object", obj_id: artifact.resource.obj_id };
      }
      if (artifact.resource.kind === "url" && artifact.resource.url) {
        return {
          kind: "url",
          url: artifact.resource.url,
          mime_hint: artifact.resource.mime_hint ?? mime,
        };
      }
      if (
        artifact.resource.kind === "base64" &&
        artifact.resource.mime &&
        artifact.resource.data_base64
      ) {
        return {
          kind: "base64",
          mime: artifact.resource.mime,
          data_base64: artifact.resource.data_base64,
        };
      }
    }
  }
  return null;
}

function decodeBase64(dataBase64: string): Uint8Array {
  const binary = atob(dataBase64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function encodeBase64(bytes: Uint8Array): string {
  const chunkSize = 0x8000;
  let binary = "";
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(
      ...bytes.subarray(index, index + chunkSize),
    );
  }
  return btoa(binary);
}

function readUint32BE(bytes: Uint8Array, offset: number): number {
  return (
    ((bytes[offset] ?? 0) << 24) |
    ((bytes[offset + 1] ?? 0) << 16) |
    ((bytes[offset + 2] ?? 0) << 8) |
    (bytes[offset + 3] ?? 0)
  ) >>> 0;
}

function parsePngDimensions(bytes: Uint8Array): ImageDimensions | null {
  if (
    bytes.length >= 24 &&
    bytes[0] === 0x89 &&
    bytes[1] === 0x50 &&
    bytes[2] === 0x4e &&
    bytes[3] === 0x47
  ) {
    return {
      width: readUint32BE(bytes, 16),
      height: readUint32BE(bytes, 20),
    };
  }
  return null;
}

function parseJpegDimensions(bytes: Uint8Array): ImageDimensions | null {
  if (bytes.length < 4 || bytes[0] !== 0xff || bytes[1] !== 0xd8) {
    return null;
  }
  let offset = 2;
  while (offset + 9 < bytes.length) {
    while (bytes[offset] === 0xff) {
      offset += 1;
    }
    const marker = bytes[offset];
    offset += 1;
    if (marker === 0xd9 || marker === 0xda) {
      break;
    }
    const length = ((bytes[offset] ?? 0) << 8) | (bytes[offset + 1] ?? 0);
    if (length < 2 || offset + length > bytes.length) {
      break;
    }
    const isSof = (marker >= 0xc0 && marker <= 0xc3) ||
      (marker >= 0xc5 && marker <= 0xc7) ||
      (marker >= 0xc9 && marker <= 0xcb) ||
      (marker >= 0xcd && marker <= 0xcf);
    if (isSof) {
      return {
        height: ((bytes[offset + 3] ?? 0) << 8) | (bytes[offset + 4] ?? 0),
        width: ((bytes[offset + 5] ?? 0) << 8) | (bytes[offset + 6] ?? 0),
      };
    }
    offset += length;
  }
  return null;
}

async function resourceToBytes(
  resource: ResourceRef,
  ndmProxy?: NdmProxyClient,
): Promise<{ bytes: Uint8Array; mime?: string }> {
  if (resource.kind === "base64") {
    return { bytes: decodeBase64(resource.data_base64), mime: resource.mime };
  }
  if (resource.kind === "url") {
    const response = await fetch(resource.url);
    if (!response.ok) {
      throw new Error(
        `failed to download image resource: ${response.status} ${response.statusText}`,
      );
    }
    return {
      bytes: new Uint8Array(await response.arrayBuffer()),
      mime: response.headers.get("content-type") ?? resource.mime_hint,
    };
  }
  if (resource.kind === "named_object" && ndmProxy) {
    const opened = await ndmProxy.openReader({ obj_id: resource.obj_id });
    return {
      bytes: new Uint8Array(await opened.response.arrayBuffer()),
      mime: opened.response.headers.get("content-type") ?? undefined,
    };
  }
  throw new Error(
    `cannot inspect named_object image resource ${resource.obj_id}`,
  );
}

async function imageDimensionsFromResource(
  resource: ResourceRef,
  ndmProxy?: NdmProxyClient,
): Promise<ImageDimensions> {
  const { bytes, mime } = await resourceToBytes(resource, ndmProxy);
  const dimensions = parsePngDimensions(bytes) ?? parseJpegDimensions(bytes);
  if (!dimensions || dimensions.width <= 0 || dimensions.height <= 0) {
    throw new Error(
      `unsupported generated image dimensions: ${mime ?? "unknown"}`,
    );
  }
  return dimensions;
}

function makeCrc32Table(): Uint32Array {
  const table = new Uint32Array(256);
  for (let index = 0; index < 256; index += 1) {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = (value & 1) ? (0xedb88320 ^ (value >>> 1)) : (value >>> 1);
    }
    table[index] = value >>> 0;
  }
  return table;
}

const crc32Table = makeCrc32Table();

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc = crc32Table[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function adler32(bytes: Uint8Array): number {
  let a = 1;
  let b = 0;
  for (const byte of bytes) {
    a = (a + byte) % 65521;
    b = (b + a) % 65521;
  }
  return ((b << 16) | a) >>> 0;
}

function concatBytes(chunks: Uint8Array<ArrayBufferLike>[]): Uint8Array {
  const length = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const result = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.length;
  }
  return result;
}

function uint32BE(value: number): Uint8Array {
  return new Uint8Array([
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ]);
}

function pngChunk(type: string, data: Uint8Array): Uint8Array {
  const typeBytes = new TextEncoder().encode(type);
  return concatBytes([
    uint32BE(data.length),
    typeBytes,
    data,
    uint32BE(crc32(concatBytes([typeBytes, data]))),
  ]);
}

function zlibStored(bytes: Uint8Array): Uint8Array {
  const chunks: Uint8Array<ArrayBufferLike>[] = [new Uint8Array([0x78, 0x01])];
  for (let offset = 0; offset < bytes.length; offset += 65535) {
    const block = bytes.subarray(offset, offset + 65535);
    const finalBlock = offset + block.length >= bytes.length ? 1 : 0;
    const header = new Uint8Array(5);
    header[0] = finalBlock;
    header[1] = block.length & 0xff;
    header[2] = (block.length >>> 8) & 0xff;
    const nlen = (~block.length) & 0xffff;
    header[3] = nlen & 0xff;
    header[4] = (nlen >>> 8) & 0xff;
    chunks.push(header, block);
  }
  chunks.push(uint32BE(adler32(bytes)));
  return concatBytes(chunks);
}

async function deflateBytes(bytes: Uint8Array): Promise<Uint8Array> {
  try {
    const copy = new Uint8Array(bytes.length);
    copy.set(bytes);
    const stream = new Blob([copy.buffer]).stream().pipeThrough(
      new CompressionStream("deflate"),
    );
    return new Uint8Array(await new Response(stream).arrayBuffer());
  } catch {
    return zlibStored(bytes);
  }
}

async function makeInpaintMaskPngBase64(
  dimensions: ImageDimensions,
): Promise<string> {
  const { width, height } = dimensions;
  const stride = width * 4 + 1;
  const raw = new Uint8Array(stride * height);
  const clearLeft = Math.floor(width * 0.35);
  const clearRight = Math.ceil(width * 0.65);
  const clearTop = Math.floor(height * 0.35);
  const clearBottom = Math.ceil(height * 0.65);

  for (let y = 0; y < height; y += 1) {
    const row = y * stride;
    raw[row] = 0;
    for (let x = 0; x < width; x += 1) {
      const offset = row + 1 + x * 4;
      const transparent = x >= clearLeft && x < clearRight &&
        y >= clearTop && y < clearBottom;
      raw[offset] = 255;
      raw[offset + 1] = 255;
      raw[offset + 2] = 255;
      raw[offset + 3] = transparent ? 0 : 255;
    }
  }

  const ihdr = new Uint8Array(13);
  ihdr.set(uint32BE(width), 0);
  ihdr.set(uint32BE(height), 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  const png = concatBytes([
    new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", await deflateBytes(raw)),
    pngChunk("IEND", new Uint8Array()),
  ]);
  return encodeBase64(png);
}

async function maskResourceForGeneratedImage(
  context: SmokeContext,
): Promise<ResourceRef> {
  let dimensions = context.generatedImageDimensions;
  if (!dimensions && context.generatedImage) {
    dimensions = await imageDimensionsFromResource(
      context.generatedImage,
      context.ndmProxy,
    );
    context.generatedImageDimensions = dimensions;
  }
  if (!dimensions) {
    throw new MissingGeneratedImageError("image.inpaint");
  }
  return {
    kind: "base64",
    mime: "image/png",
    data_base64: await makeInpaintMaskPngBase64(dimensions),
  };
}

async function saveArtifacts(
  response: AiResponse | null,
  reportDir: string,
): Promise<SavedArtifact[]> {
  const artifacts = response ? aiResponseArtifacts(response) : [];
  if (artifacts.length === 0) {
    return [];
  }

  const artifactsDir = joinPath(reportDir, "artifacts");
  await Deno.mkdir(artifactsDir, { recursive: true });
  const saved: SavedArtifact[] = [];

  for (const [index, artifact] of artifacts.entries()) {
    const sourceKind = artifact.resource?.kind ?? "unknown";
    const name = artifact.name || `artifact_${index + 1}`;
    let bytes: Uint8Array;
    let mime = artifactMime(artifact);
    let sourceUrl: string | undefined;
    let ext = mimeExtension(mime);

    if (sourceKind === "named_object" && artifact.resource.obj_id) {
      saved.push({
        artifact_index: index,
        name,
        source_kind: sourceKind,
        object_id: artifact.resource.obj_id,
        mime,
        opened: false,
      });
      continue;
    }

    if (sourceKind === "url" && artifact.resource.url) {
      sourceUrl = artifact.resource.url;
      const response = await fetch(sourceUrl);
      if (!response.ok) {
        throw new Error(
          `failed to download artifact ${
            index + 1
          } from ${sourceUrl}: ${response.status} ${response.statusText}`,
        );
      }
      mime = response.headers.get("content-type") ?? mime;
      ext = mimeExtension(mime) ?? extensionFromUrl(sourceUrl) ?? ext;
      bytes = new Uint8Array(await response.arrayBuffer());
    } else if (sourceKind === "base64" && artifact.resource.data_base64) {
      bytes = decodeBase64(artifact.resource.data_base64);
      ext = ext ?? "bin";
    } else {
      throw new Error(
        `unsupported artifact ${index + 1} resource: ${
          JSON.stringify(artifact.resource)
        }`,
      );
    }

    const filename = `${String(index + 1).padStart(2, "0")}-${
      safeFileSegment(name)
    }.${ext ?? "bin"}`;
    const path = joinPath(artifactsDir, filename);
    await Deno.writeFile(path, bytes);
    saved.push({
      artifact_index: index,
      name,
      source_kind: sourceKind,
      source_url: sourceUrl,
      mime,
      path,
      bytes: bytes.byteLength,
      opened: true,
    });
  }

  await writeJsonFile(joinPath(artifactsDir, "artifacts.json"), saved);
  return saved;
}

function extractTaskResponse(task: TaskRecord): AiResponse | null {
  return asAiResponse(task.data?.aicc?.output ?? null);
}

function extractTaskError(task: TaskRecord): unknown {
  return task.data?.aicc?.error ?? task.message ?? null;
}

function isSkippableError(message: string): boolean {
  const lowered = message.toLowerCase();
  return (
    lowered.includes("no_provider_available") ||
    lowered.includes("model_alias_not_mapped") ||
    lowered.includes("provider_unavailable") ||
    lowered.includes("no route candidate generated")
  );
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function findTaskByExternalId(
  taskManagerRpc: RpcClient,
  externalTaskId: string,
  deadlineMs: number,
  appId: string,
  userId: string,
): Promise<TaskRecord | null> {
  while (Date.now() < deadlineMs) {
    const result = await taskManagerRpc.call("list_tasks", {
      app_id: appId,
      task_type: "aicc.compute",
      source_user_id: userId,
      source_app_id: appId,
    });

    const tasks = normalizeTaskList(result).sort(
      (left, right) => (right.updated_at ?? 0) - (left.updated_at ?? 0),
    );
    const matched = tasks.find(
      (task) => task.data?.aicc?.external_task_id === externalTaskId,
    );
    if (matched) {
      return matched;
    }

    await sleep(1000);
  }

  return null;
}

async function waitForFinalTaskResult(
  taskManagerRpc: RpcClient,
  externalTaskId: string,
  appId: string,
  userId: string,
): Promise<TaskRecord> {
  const deadlineMs = Date.now() + AICC_WAIT_TIMEOUT_MS;
  const task = await findTaskByExternalId(
    taskManagerRpc,
    externalTaskId,
    deadlineMs,
    appId,
    userId,
  );

  if (!task) {
    throw new Error(
      `Timed out while locating AICC task for external_task_id=${externalTaskId}`,
    );
  }

  while (Date.now() < deadlineMs) {
    const result = await taskManagerRpc.call("get_task", { id: task.id });
    const latest = normalizeTask(result);
    if (["Completed", "Failed", "Canceled"].includes(latest.status)) {
      return latest;
    }
    await sleep(1000);
  }

  throw new Error(`Timed out while waiting for AICC task ${task.id} to finish`);
}

function randomHex(byteLength: number): string {
  const bytes = new Uint8Array(byteLength);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

function envAliasForMethod(method: string): string | null {
  const key = `AICC_${method.toUpperCase().replace(/[^A-Z0-9]+/g, "_")}_ALIAS`;
  return getEnv(key);
}

async function loadModelEntries(
  aiccRpc: RpcClient,
): Promise<ModelEntry[] | null> {
  try {
    const raw = await aiccRpc.call("models.list", {});
    if (!raw || typeof raw !== "object") {
      return null;
    }
    const result = raw as ModelsListResponse;
    const providers = Array.isArray(result.providers) ? result.providers : [];
    return providers.flatMap((provider) =>
      Array.isArray(provider.models) ? provider.models : []
    );
  } catch (err) {
    console.warn(
      `[warn] models.list failed; falling back to static aliases: ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
    return null;
  }
}

function supportsApiType(model: ModelEntry, method: string): boolean {
  return Array.isArray(model.api_types) && model.api_types.includes(method);
}

function selectModelAlias(
  testCase: SmokeCase,
  modelEntries: ModelEntry[] | null,
): { modelAlias: string; supported: boolean | null } {
  const override = envAliasForMethod(testCase.method);
  if (override) {
    return { modelAlias: override, supported: null };
  }
  if (testCase.defaultAlias) {
    return { modelAlias: testCase.defaultAlias, supported: null };
  }
  if (!modelEntries) {
    return { modelAlias: testCase.method, supported: null };
  }

  const supportedModels = modelEntries.filter((model) =>
    supportsApiType(model, testCase.method)
  );
  if (supportedModels.length === 0) {
    return { modelAlias: testCase.method, supported: false };
  }

  const mounts = supportedModels.flatMap((model) => model.logical_mounts ?? []);
  const exactMount = mounts.find((mount) => mount === testCase.method);
  if (exactMount) {
    return { modelAlias: exactMount, supported: true };
  }
  const defaultMount = mounts.find((mount) =>
    mount === `${testCase.method}.default`
  );
  if (defaultMount) {
    return { modelAlias: defaultMount, supported: true };
  }
  const prefixedMount = mounts.find((mount) =>
    mount.startsWith(`${testCase.method}.`)
  );
  if (prefixedMount) {
    return { modelAlias: prefixedMount, supported: true };
  }
  return { modelAlias: supportedModels[0].exact_model, supported: true };
}

function requestNamedObjectOutput(payload: SmokePayload): SmokePayload {
  const inputJson = { ...(payload.input_json ?? {}) };
  const output = inputJson.output && typeof inputJson.output === "object" &&
      !Array.isArray(inputJson.output)
    ? inputJson.output as Record<string, unknown>
    : {};
  return {
    ...payload,
    input_json: {
      ...inputJson,
      response_format: inputJson.response_format ?? "object_id",
      output: {
        ...output,
        resource_format: output.resource_format ?? "named_object",
      },
    },
  };
}

async function buildMethodPayload(
  testCase: SmokeCase,
  runId: string,
  modelAlias: string,
  context: SmokeContext,
): Promise<Record<string, unknown>> {
  const basePayload = await testCase.buildPayload(runId, context);
  const payload = testCase.returnsUnstructuredData
    ? requestNamedObjectOutput(basePayload)
    : basePayload;
  return {
    capability: testCase.capability,
    model: { alias: modelAlias },
    requirements: testCase.requirements ?? {},
    payload: {
      input_json: payload.input_json ?? {},
      resources: payload.resources ?? [],
      options: {
        session_id: runId,
        rootid: runId,
        ...(payload.options ?? {}),
      },
    },
    idempotency_key: runId,
  };
}

function assertAiResponse(
  testCase: SmokeCase,
  response: AiResponse | null,
): asserts response is AiResponse {
  if (!response) {
    throw new Error(`${testCase.method}: no AiResponse returned`);
  }
  const text = aiResponseTextContent(response);
  const artifacts = aiResponseArtifacts(response);
  if (testCase.check === "text") {
    if (!text.trim()) {
      throw new Error(`${testCase.method}: expected non-empty response text`);
    }
    return;
  }
  if (testCase.check === "embedding") {
    const extra = response.extra;
    if (!extra || typeof extra !== "object" || !("embedding" in extra)) {
      throw new Error(`${testCase.method}: expected extra.embedding`);
    }
    return;
  }
  if (testCase.check === "artifact") {
    if (artifacts.length === 0) {
      throw new Error(`${testCase.method}: expected at least one artifact`);
    }
    return;
  }
  if (testCase.check === "vision") {
    if (text.trim()) {
      return;
    }
    if (response.extra && typeof response.extra === "object") {
      return;
    }
    throw new Error(
      `${testCase.method}: expected text or structured vision extra`,
    );
  }
  if (testCase.check === "video") {
    if (response.provider_task_ref || artifacts.length > 0) {
      return;
    }
    if (response.extra && typeof response.extra === "object") {
      return;
    }
    throw new Error(
      `${testCase.method}: expected video operation ref, artifact, or extra`,
    );
  }
}

async function runCase(args: {
  testCase: SmokeCase;
  caseIndex: number;
  aiccRpc: RpcClient;
  taskManagerRpc: RpcClient;
  appId: string;
  userId: string;
  runId: string;
  reportRoot: string;
  modelEntries: ModelEntry[] | null;
  context: SmokeContext;
}): Promise<CaseResult> {
  const {
    testCase,
    caseIndex,
    aiccRpc,
    taskManagerRpc,
    appId,
    userId,
    runId,
    reportRoot,
    modelEntries,
    context,
  } = args;
  const selected = selectModelAlias(testCase, modelEntries);
  const modelAlias = selected.modelAlias;
  const caseRunId = `${runId}-${testCase.method.replace(/[^a-z0-9]+/gi, "-")}`;
  const reportDir = joinPath(
    reportRoot,
    `${String(caseIndex + 1).padStart(2, "0")}-${
      safeFileSegment(testCase.method)
    }`,
  );
  await Deno.mkdir(reportDir, { recursive: true });

  let requestPayload: Record<string, unknown>;
  try {
    requestPayload = await buildMethodPayload(
      testCase,
      caseRunId,
      modelAlias,
      context,
    );
  } catch (err) {
    if (err instanceof MissingGeneratedImageError) {
      const reason = err.message;
      await writeJsonFile(joinPath(reportDir, "output.json"), {
        status: "skipped",
        reason,
      });
      return {
        status: "skipped",
        method: testCase.method,
        modelAlias,
        reason,
        reportDir,
      };
    }
    throw err;
  }

  await writeJsonFile(joinPath(reportDir, "input.json"), {
    method: testCase.method,
    capability: testCase.capability,
    model_alias: modelAlias,
    run_id: caseRunId,
    selected_supported: selected.supported,
    request: requestPayload,
  });

  if (selected.supported === false) {
    const reason = `models.list has no model supporting ${testCase.method}`;
    await writeJsonFile(joinPath(reportDir, "output.json"), {
      status: "skipped",
      reason,
    });
    return {
      status: "skipped",
      method: testCase.method,
      modelAlias,
      reason,
      reportDir,
    };
  }

  let response: AiccMethodResponse;
  try {
    response = await aiccRpc.call(
      testCase.method,
      requestPayload,
    ) as AiccMethodResponse;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (isSkippableError(message)) {
      await writeJsonFile(joinPath(reportDir, "output.json"), {
        status: "skipped",
        reason: message,
      });
      return {
        status: "skipped",
        method: testCase.method,
        modelAlias,
        reason: message,
        reportDir,
      };
    }
    await writeJsonFile(joinPath(reportDir, "output.json"), {
      status: "failed",
      error: message,
    });
    return {
      status: "failed",
      method: testCase.method,
      modelAlias,
      error: message,
      reportDir,
    };
  }
  await writeJsonFile(joinPath(reportDir, "response.json"), response);

  if (!response?.task_id || !response?.status) {
    const error = `invalid AICC response: ${JSON.stringify(response, null, 2)}`;
    await writeJsonFile(joinPath(reportDir, "output.json"), {
      status: "failed",
      error,
    });
    return {
      status: "failed",
      method: testCase.method,
      modelAlias,
      error,
      reportDir,
    };
  }

  if (response.status === "failed") {
    const errorText = JSON.stringify(response.result ?? response);
    if (isSkippableError(errorText)) {
      await writeJsonFile(joinPath(reportDir, "output.json"), {
        status: "skipped",
        task_id: response.task_id,
        reason: errorText,
        response: response.result ?? null,
      });
      return {
        status: "skipped",
        method: testCase.method,
        modelAlias,
        reason: errorText,
        reportDir,
      };
    }
    await writeJsonFile(joinPath(reportDir, "output.json"), {
      status: "failed",
      task_id: response.task_id,
      error: errorText,
      response: response.result ?? null,
    });
    return {
      status: "failed",
      method: testCase.method,
      modelAlias,
      error: errorText,
      reportDir,
    };
  }

  let aiResponse: AiResponse | null;
  try {
    aiResponse = asAiResponse(response.result ?? null);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    await writeJsonFile(joinPath(reportDir, "output.json"), {
      status: "failed",
      task_id: response.task_id,
      error: message,
      response: response.result ?? null,
    });
    return {
      status: "failed",
      method: testCase.method,
      modelAlias,
      error: message,
      reportDir,
    };
  }

  if (response.status === "running" && !aiResponse) {
    let finalTask: TaskRecord;
    try {
      finalTask = await waitForFinalTaskResult(
        taskManagerRpc,
        response.task_id,
        appId,
        userId,
      );
      await writeJsonFile(joinPath(reportDir, "task.json"), finalTask);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      await writeJsonFile(joinPath(reportDir, "output.json"), {
        status: "failed",
        task_id: response.task_id,
        error: message,
      });
      return {
        status: "failed",
        method: testCase.method,
        modelAlias,
        error: message,
        reportDir,
      };
    }

    if (finalTask.status !== "Completed") {
      const errorText = JSON.stringify(extractTaskError(finalTask), null, 2);
      if (isSkippableError(errorText)) {
        await writeJsonFile(joinPath(reportDir, "output.json"), {
          status: "skipped",
          task_id: response.task_id,
          task_status: finalTask.status,
          reason: errorText,
          task_error: extractTaskError(finalTask),
        });
        return {
          status: "skipped",
          method: testCase.method,
          modelAlias,
          reason: errorText,
          reportDir,
        };
      }
      const error =
        `task ${finalTask.id} ended with ${finalTask.status}: ${errorText}`;
      await writeJsonFile(joinPath(reportDir, "output.json"), {
        status: "failed",
        task_id: response.task_id,
        task_status: finalTask.status,
        error,
        task_error: extractTaskError(finalTask),
      });
      return {
        status: "failed",
        method: testCase.method,
        modelAlias,
        error,
        reportDir,
      };
    }

    try {
      aiResponse = extractTaskResponse(finalTask);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      await writeJsonFile(joinPath(reportDir, "output.json"), {
        status: "failed",
        task_id: response.task_id,
        error: message,
        task_output: finalTask.data?.aicc?.output ?? null,
      });
      return {
        status: "failed",
        method: testCase.method,
        modelAlias,
        error: message,
        reportDir,
      };
    }
  }

  try {
    assertAiResponse(testCase, aiResponse);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    await writeJsonFile(joinPath(reportDir, "output.json"), {
      status: "failed",
      task_id: response.task_id,
      error: message,
      response: aiResponse,
    });
    return {
      status: "failed",
      method: testCase.method,
      modelAlias,
      error: message,
      reportDir,
    };
  }

  if (testCase.capturesGeneratedImage) {
    const generatedImage = firstImageArtifact(aiResponse);
    if (!generatedImage) {
      const error = `${testCase.method}: expected generated image artifact`;
      await writeJsonFile(joinPath(reportDir, "output.json"), {
        status: "failed",
        task_id: response.task_id,
        error,
        response: aiResponse,
      });
      return {
        status: "failed",
        method: testCase.method,
        modelAlias,
        error,
        reportDir,
      };
    }
    context.generatedImage = generatedImage;
    context.generatedImageDimensions =
      materializedImageDimensions(aiResponse) ??
        context.generatedImageDimensions;
    try {
      if (!context.generatedImageDimensions) {
        context.generatedImageDimensions = await imageDimensionsFromResource(
          generatedImage,
          context.ndmProxy,
        );
      }
    } catch (err) {
      console.warn(
        `[warn] failed to inspect generated image dimensions: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }
  }

  let artifactFiles: SavedArtifact[];
  try {
    artifactFiles = await saveArtifacts(aiResponse, reportDir);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    await writeJsonFile(joinPath(reportDir, "output.json"), {
      status: "failed",
      task_id: response.task_id,
      error: message,
      response: aiResponse,
    });
    return {
      status: "failed",
      method: testCase.method,
      modelAlias,
      error: message,
      reportDir,
    };
  }

  await writeJsonFile(joinPath(reportDir, "output.json"), {
    status: "passed",
    task_id: response.task_id,
    response: aiResponse,
    artifact_files: artifactFiles,
  });
  return {
    status: "passed",
    method: testCase.method,
    modelAlias,
    taskId: response.task_id,
    response: aiResponse,
    artifactFiles,
    reportDir,
  };
}

function selectedCases(): SmokeCase[] {
  if (!AICC_SMOKE_METHODS) {
    return allCases;
  }
  const allowed = new Set(
    AICC_SMOKE_METHODS.split(",").map((item) => item.trim()).filter(Boolean),
  );
  const cases = allCases.filter((testCase) => allowed.has(testCase.method));
  const needsGeneratedImage = cases.some((testCase) =>
    testCase.requiresGeneratedImage
  );
  if (
    needsGeneratedImage &&
    !cases.some((testCase) => testCase.method === "image.txt2img")
  ) {
    const generator = allCases.find((testCase) =>
      testCase.method === "image.txt2img"
    );
    return generator ? [generator, ...cases] : cases;
  }
  return cases;
}

async function openGeneratedImageAtEnd(
  ndmProxy: NdmProxyClient,
  context: SmokeContext,
  reportRoot: string,
): Promise<FinalOpenedResource | null> {
  const generatedImage = context.generatedImage;
  if (!generatedImage) {
    return null;
  }

  let bytes: Uint8Array;
  let mime: string | undefined;
  let objId: string | undefined;
  let sourceUrl: string | undefined;
  let resolvedObjectId: string | undefined;
  let readerKind: string | undefined;

  if (generatedImage.kind === "named_object") {
    objId = generatedImage.obj_id;
    const opened = await ndmProxy.openReader({ obj_id: generatedImage.obj_id });
    bytes = new Uint8Array(await opened.response.arrayBuffer());
    mime = opened.response.headers.get("content-type") ?? undefined;
    resolvedObjectId = opened.resolvedObjectId;
    readerKind = opened.readerKind;
  } else if (generatedImage.kind === "url") {
    sourceUrl = generatedImage.url;
    const response = await fetch(generatedImage.url);
    if (!response.ok) {
      throw new Error(
        `failed to download generated image: ${response.status} ${response.statusText}`,
      );
    }
    bytes = new Uint8Array(await response.arrayBuffer());
    mime = response.headers.get("content-type") ?? generatedImage.mime_hint;
  } else {
    bytes = decodeBase64(generatedImage.data_base64);
    mime = generatedImage.mime;
  }

  const ext = mimeExtension(mime) ?? "bin";
  const artifactDir = joinPath(reportRoot, "generated-image");
  await Deno.mkdir(artifactDir, { recursive: true });
  const path = joinPath(artifactDir, `generated-image.${ext}`);
  await Deno.writeFile(path, bytes);

  const result: FinalOpenedResource = {
    source_kind: generatedImage.kind,
    obj_id: objId,
    source_url: sourceUrl,
    path,
    bytes: bytes.byteLength,
    mime,
    resolved_obj_id: resolvedObjectId,
    reader_kind: readerKind,
  };
  await writeJsonFile(joinPath(artifactDir, "opened.json"), result);
  return result;
}

async function main(): Promise<void> {
  const runId = `aicc-smoke-${Date.now().toString(36)}-${randomHex(3)}`;
  const reportRoot = joinPath(AICC_REPORT_DIR, runId);
  await Deno.mkdir(reportRoot, { recursive: true });

  const { buckyos, userId, ownerUserId, zoneHost } = await initTestRuntime();
  const appId = buckyos.getAppId?.() ?? getEnv("BUCKYOS_TEST_APP_ID") ??
    "buckycli";

  const aiccRpc = buckyos.getServiceRpcClient("aicc") as RpcClient;
  const taskManagerRpc = buckyos.getServiceRpcClient(
    "task-manager",
  ) as RpcClient;
  const ndmProxy = ndm_proxy.createNdmProxyClient() as NdmProxyClient;
  const context: SmokeContext = { ndmProxy };
  const modelEntries = await loadModelEntries(aiccRpc);
  const cases = selectedCases();

  console.log("=== AICC Smoke Test ===");
  console.log(`Zone: ${zoneHost}`);
  console.log(`App ID: ${appId}`);
  console.log(`User ID: ${userId}`);
  console.log(`Owner User ID: ${ownerUserId}`);
  console.log(`Run ID: ${runId}`);
  console.log(`Cases: ${cases.length}`);
  console.log(`Report Dir: ${reportRoot}`);
  console.log(`Video URL: ${AICC_TEST_VIDEO_URL}`);
  console.log(
    `Audio URL: ${AICC_TEST_AUDIO_BASE64 ? "<base64>" : AICC_TEST_AUDIO_URL}`,
  );

  let passed = 0;
  let skipped = 0;
  let failed = 0;
  const results: CaseResult[] = [];

  for (const [caseIndex, testCase] of cases.entries()) {
    const result = await runCase({
      testCase,
      caseIndex,
      aiccRpc,
      taskManagerRpc,
      appId,
      userId,
      runId,
      reportRoot,
      modelEntries,
      context,
    });
    results.push(result);

    if (result.status === "passed") {
      passed += 1;
      console.log(
        `[PASS] ${result.method} alias=${result.modelAlias} task=${result.taskId} report=${result.reportDir}`,
      );
    } else if (result.status === "skipped") {
      skipped += 1;
      console.log(
        `[SKIP] ${result.method} alias=${result.modelAlias} report=${result.reportDir}: ${result.reason}`,
      );
    } else {
      failed += 1;
      console.log(
        `[FAIL] ${result.method} alias=${result.modelAlias} report=${result.reportDir}: ${result.error}`,
      );
    }
  }

  let finalGeneratedImage: FinalOpenedResource | null = null;
  let finalGeneratedImageError: string | null = null;
  if (context.generatedImage) {
    try {
      const openedGeneratedImage = await openGeneratedImageAtEnd(
        ndmProxy,
        context,
        reportRoot,
      );
      if (!openedGeneratedImage) {
        throw new Error("generated image object id is missing");
      }
      finalGeneratedImage = openedGeneratedImage;
      console.log(
        `[NDM] opened generated image obj_id=${openedGeneratedImage.obj_id} path=${openedGeneratedImage.path}`,
      );
    } catch (err) {
      finalGeneratedImageError = err instanceof Error
        ? err.message
        : String(err);
      console.warn(
        `[warn] generated image NDM open failed: ${finalGeneratedImageError}`,
      );
    }
  }

  await writeJsonFile(joinPath(reportRoot, "summary.json"), {
    run_id: runId,
    zone: zoneHost,
    app_id: appId,
    user_id: userId,
    owner_user_id: ownerUserId,
    report_dir: reportRoot,
    cases: cases.length,
    passed,
    skipped,
    failed,
    generated_image: context.generatedImage ?? null,
    final_generated_image: finalGeneratedImage,
    final_generated_image_error: finalGeneratedImageError,
    results: results.map((result) => {
      if (result.status === "passed") {
        return {
          status: result.status,
          method: result.method,
          model_alias: result.modelAlias,
          task_id: result.taskId,
          report_dir: result.reportDir,
          artifact_files: result.artifactFiles,
        };
      }
      return {
        status: result.status,
        method: result.method,
        model_alias: result.modelAlias,
        reason: result.status === "skipped" ? result.reason : result.error,
        report_dir: result.reportDir,
      };
    }),
  });

  console.log("\n--- summary ---");
  console.log(`passed=${passed} skipped=${skipped} failed=${failed}`);
  console.log(`report_dir=${reportRoot}`);

  buckyos.logout(false);

  if (failed > 0) {
    Deno.exit(1);
  }
  if (passed === 0 && skipped > 0) {
    Deno.exit(2);
  }
}

main().catch((error) => {
  console.error("AICC smoke test failed");
  console.error(error);
  Deno.exit(1);
});
