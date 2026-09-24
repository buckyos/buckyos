// AgentToolResult wire types plus projections derived from the canonical
// AICC client exported by the BuckyOS WebSDK.

import type { buckyos } from "buckyos";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | {
  [k: string]: JsonValue;
};

type SdkAiccClient = ReturnType<typeof buckyos.getAiccClient>;

export type ResourceRef = Parameters<
  SdkAiccClient["imageToImage"]
>[0]["images"][number];
export type AiccInferenceResponse =
  | Awaited<ReturnType<SdkAiccClient["helperLlmChat"]>>
  | Awaited<ReturnType<SdkAiccClient["helperTextToImage"]>>
  | Awaited<ReturnType<SdkAiccClient["imageToImage"]>>
  | Awaited<ReturnType<SdkAiccClient["imageInpaint"]>>
  | Awaited<ReturnType<SdkAiccClient["imageUpscale"]>>
  | Awaited<ReturnType<SdkAiccClient["imageBackgroundRemove"]>>
  | Awaited<ReturnType<SdkAiccClient["visionOcr"]>>
  | Awaited<ReturnType<SdkAiccClient["visionCaption"]>>
  | Awaited<ReturnType<SdkAiccClient["visionDetect"]>>
  | Awaited<ReturnType<SdkAiccClient["visionSegment"]>>
  | Awaited<ReturnType<SdkAiccClient["audioTextToSpeech"]>>
  | Awaited<ReturnType<SdkAiccClient["audioSpeechRecognition"]>>
  | Awaited<ReturnType<SdkAiccClient["audioMusic"]>>
  | Awaited<ReturnType<SdkAiccClient["audioEnhance"]>>
  | Awaited<ReturnType<SdkAiccClient["videoTextToVideo"]>>
  | Awaited<ReturnType<SdkAiccClient["videoImageToVideo"]>>
  | Awaited<ReturnType<SdkAiccClient["videoToVideo"]>>
  | Awaited<ReturnType<SdkAiccClient["videoExtend"]>>
  | Awaited<ReturnType<SdkAiccClient["videoUpscale"]>>;

export type AiArtifact = {
  name: string;
  resource: ResourceRef;
  mime?: string | null;
  metadata?: JsonValue | null;
};

function resourceMime(resource: ResourceRef): string | undefined {
  if (resource.kind === "base64") return resource.mime;
  if (resource.kind === "url") return resource.mime_hint;
  return undefined;
}

function isResourceRef(value: unknown): value is ResourceRef {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const kind = (value as Record<string, unknown>).kind;
  return kind === "url" || kind === "base64" || kind === "named_object";
}

function responseRecord(
  response: AiccInferenceResponse,
): Record<string, unknown> {
  return response as unknown as Record<string, unknown>;
}

export function aiResponseText(response: AiccInferenceResponse): string {
  const record = responseRecord(response);
  if (typeof record.value === "string") return record.value;
  if (
    record.value && typeof record.value === "object" &&
    !Array.isArray(record.value)
  ) {
    const nested = aiResponseText(record.value as AiccInferenceResponse);
    if (nested) return nested;
  }
  if (typeof record.text === "string") return record.text;
  if (Array.isArray(record.captions)) {
    return record.captions.flatMap((caption) =>
      caption && typeof caption === "object" &&
        typeof (caption as Record<string, unknown>).text === "string"
        ? [(caption as Record<string, unknown>).text as string]
        : []
    ).join("\n");
  }
  const message = record.message;
  if (!message || typeof message !== "object" || Array.isArray(message)) {
    return "";
  }
  const content = (message as Record<string, unknown>).content;
  if (!Array.isArray(content)) return "";
  return content.flatMap((block) =>
    block && typeof block === "object" &&
      (block as Record<string, unknown>).type === "text" &&
      typeof (block as Record<string, unknown>).text === "string"
      ? [(block as Record<string, unknown>).text as string]
      : []
  ).join("\n");
}

export function aiResponseArtifacts(
  response: AiccInferenceResponse,
): AiArtifact[] {
  const record = responseRecord(response);
  const artifacts: AiArtifact[] = [];
  const append = (name: string, value: unknown) => {
    if (isResourceRef(value)) {
      artifacts.push({
        name,
        resource: value,
        mime: resourceMime(value),
        metadata: null,
      });
    }
  };
  if (Array.isArray(record.images)) {
    record.images.forEach((value, index) =>
      append(`image_${index + 1}`, value)
    );
  }
  append("image", record.image);
  append("audio", record.audio);
  append("video", record.video);
  if (Array.isArray(record.stems)) {
    record.stems.forEach((value, index) => append(`stem_${index + 1}`, value));
  }
  if (Array.isArray(record.artifacts)) {
    record.artifacts.forEach((artifact, index) => {
      if (
        !artifact || typeof artifact !== "object" || Array.isArray(artifact)
      ) return;
      const item = artifact as Record<string, unknown>;
      append(
        typeof item.name === "string" ? item.name : `artifact_${index + 1}`,
        item.resource,
      );
      const appended = artifacts.at(-1);
      if (appended && typeof item.mime === "string") appended.mime = item.mime;
    });
  } else if (record.artifacts && typeof record.artifacts === "object") {
    for (
      const [name, value] of Object.entries(
        record.artifacts as Record<string, unknown>,
      )
    ) {
      append(name, value);
    }
  }
  const message = record.message;
  if (message && typeof message === "object" && !Array.isArray(message)) {
    const content = (message as Record<string, unknown>).content;
    if (Array.isArray(content)) {
      content.forEach((block, index) => {
        if (!block || typeof block !== "object" || Array.isArray(block)) return;
        const item = block as Record<string, unknown>;
        if (item.type === "image") append(`image_${index + 1}`, item.source);
        if (item.type === "document") {
          append(
            typeof item.title === "string"
              ? item.title
              : `document_${index + 1}`,
            item.source,
          );
        }
      });
    }
  }
  return artifacts;
}

export function aiResponseData(
  response: AiccInferenceResponse,
): JsonValue | null {
  const record = responseRecord(response);
  if (record.value !== undefined) return record.value as JsonValue;
  const omitted = new Set([
    "task_id",
    "status",
    "usage",
    "cost",
    "finish_reason",
    "provider_task_ref",
    "route_trace",
    "event_ref",
    "error",
    "message",
    "images",
    "image",
    "audio",
    "video",
  ]);
  const data = Object.fromEntries(
    Object.entries(record).filter(([key]) => !omitted.has(key)),
  );
  return Object.keys(data).length ? data as JsonValue : null;
}

export type Profile = "balanced" | "cheap" | "fast" | "quality";

export type AgentToolStatus = "success" | "error" | "pending";

export type AgentToolPendingReason =
  | "long_running"
  | "user_approval"
  | "wait_for_install";

export const AGENT_TOOL_PROTOCOL_VERSION = "1";

export interface AgentToolResult {
  agent_tool_protocol: string;
  tool?: string;
  cmd_name?: string;
  status: AgentToolStatus;
  task_id?: string;
  pending_reason?: AgentToolPendingReason;
  check_after?: number;
  estimated_wait?: string;
  title: string;
  summary: string;
  // Serialized as `detail` on the wire to match Rust `#[serde(rename = "detail")]`.
  detail: JsonValue;
  cmd_args?: string;
  return_code?: number;
  partial_output?: string;
  output?: string;
}

// CLI process exit codes — doc §2.5
export const EXIT_SUCCESS = 0;
export const EXIT_ARG_ERROR = 1;
export const EXIT_AICC_FAILED = 2;
export const EXIT_ROUTE_FAILED = 3;
export const EXIT_TIMEOUT = 4;
export const EXIT_IO_FAILED = 5;
