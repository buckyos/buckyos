// AICC + AgentToolResult shared types. Mirrors Rust definitions in
// src/frame/agent_tool/src/lib.rs (AgentToolResult / AgentToolStatus /
// AgentToolPendingReason) and aicc_client.rs (AiResponse / AiArtifact /
// ResourceRef / AiccMethodResponse).

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [k: string]: JsonValue };

export type ResourceRef =
  | { kind: "url"; url: string; mime_hint?: string }
  | { kind: "base64"; mime: string; data_base64: string }
  | { kind: "named_object"; obj_id: string };

export type AiArtifact = {
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

export type AiRole = "system" | "user" | "assistant" | "tool" | "developer";

export type AiToolCall = {
  name: string;
  args: { [k: string]: JsonValue };
  call_id: string;
};

export type AiToolResultContent =
  | { type: "text"; text: string }
  | { type: "image"; source: ResourceRef }
  | { type: "document"; source: ResourceRef; title?: string | null };

export type AiContent =
  | { type: "text"; text: string }
  | { type: "image"; source: ResourceRef }
  | { type: "document"; source: ResourceRef; title?: string | null }
  | { type: "tool_use"; call_id: string; name: string; args?: { [k: string]: JsonValue } }
  | { type: "tool_result"; call_id: string; content: AiToolResultContent[]; is_error?: boolean }
  | {
    type: "thinking";
    summary?: string | null;
    text?: string | null;
    provider_metadata?: JsonValue | null;
  }
  | { type: "provider_state"; provider: string; value: JsonValue };

export type AiMessage = {
  role: AiRole;
  content: AiContent[];
};

export type AiResponse = {
  message: AiMessage;
  usage?: JsonValue | null;
  cost?: JsonValue | null;
  finish_reason?: string | null;
  provider_task_ref?: string | null;
  extra?: JsonValue | null;
};

function resourceMime(resource: ResourceRef): string | undefined {
  if (resource.kind === "base64") return resource.mime;
  if (resource.kind === "url") return resource.mime_hint;
  return undefined;
}

export function aiResponseText(response: AiResponse): string {
  return response.message.content
    .filter((block): block is Extract<AiContent, { type: "text" }> => block.type === "text")
    .map((block) => block.text)
    .join("\n");
}

export function aiResponseToolCalls(response: AiResponse): AiToolCall[] {
  return response.message.content
    .filter((block): block is Extract<AiContent, { type: "tool_use" }> => block.type === "tool_use")
    .map((block) => ({
      name: block.name,
      args: block.args ?? {},
      call_id: block.call_id,
    }));
}

export function aiResponseArtifacts(response: AiResponse): AiArtifact[] {
  return response.message.content.flatMap((block, index) => {
    if (block.type === "image") {
      return [{
        name: `image_${index + 1}`,
        resource: block.source,
        mime: resourceMime(block.source),
        metadata: null,
      }];
    }
    if (block.type === "document") {
      return [{
        name: block.title ?? `document_${index + 1}`,
        resource: block.source,
        mime: resourceMime(block.source),
        metadata: null,
      }];
    }
    return [];
  });
}

export type AiccMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: AiResponse | null;
  event_ref?: string | null;
};

export type Profile = "balanced" | "cheap" | "fast" | "quality";

export type Capability =
  | "llm"
  | "embedding"
  | "rerank"
  | "image"
  | "vision"
  | "audio"
  | "video"
  | "agent";

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
