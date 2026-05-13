// AICC + AgentToolResult shared types. Mirrors Rust definitions in
// src/frame/agent_tool/src/lib.rs (AgentToolResult / AgentToolStatus /
// AgentToolPendingReason) and aicc_client.rs (AiResponseSummary / AiArtifact /
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

export type AiResponseSummary = {
  text?: string | null;
  tool_calls?: JsonValue[];
  artifacts?: AiArtifact[];
  usage?: JsonValue | null;
  cost?: JsonValue | null;
  finish_reason?: string | null;
  provider_task_ref?: string | null;
  extra?: JsonValue | null;
};

export type AiccMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: AiResponseSummary | null;
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
