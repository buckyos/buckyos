// AgentToolResult builders + emit helpers. Stdout always prints exactly one
// AgentToolResult JSON line; stderr is reserved for human-readable progress.

import {
  AGENT_TOOL_PROTOCOL_VERSION,
  AgentToolPendingReason,
  AgentToolResult,
  AgentToolStatus,
  EXIT_AICC_FAILED,
  EXIT_ARG_ERROR,
  EXIT_IO_FAILED,
  EXIT_ROUTE_FAILED,
  EXIT_SUCCESS,
  EXIT_TIMEOUT,
  JsonValue,
} from "./types.ts";

export function baseResult(
  tool: string,
  status: AgentToolStatus,
  title: string,
  summary: string,
  detail: JsonValue,
): AgentToolResult {
  return {
    agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION,
    tool,
    status,
    title,
    summary,
    detail,
  };
}

export function successResult(
  tool: string,
  title: string,
  summary: string,
  detail: JsonValue,
  output?: string,
): AgentToolResult {
  const r = baseResult(tool, "success", title, summary, detail);
  r.return_code = 0;
  if (typeof output === "string") r.output = output;
  return r;
}

export function errorResult(
  tool: string,
  title: string,
  summary: string,
  detail: JsonValue,
): AgentToolResult {
  return baseResult(tool, "error", title, summary, detail);
}

export function pendingResult(
  tool: string,
  reason: AgentToolPendingReason,
  task_id: string,
  title: string,
  summary: string,
  detail: JsonValue,
): AgentToolResult {
  const r = baseResult(tool, "pending", title, summary, detail);
  r.pending_reason = reason;
  r.task_id = task_id;
  return r;
}

export function emit(result: AgentToolResult): void {
  // Single-line JSON to stdout — the wire format for AgentToolResult.
  console.log(JSON.stringify(result));
}

export function emitAndExit(result: AgentToolResult, exitCode: number): never {
  emit(result);
  Deno.exit(exitCode);
}

// Classify a thrown error from the AICC RPC into the right CLI exit code.
export function classifyAiccError(message: string): number {
  const lowered = message.toLowerCase();
  if (
    lowered.includes("no_provider_available") ||
    lowered.includes("no route candidate generated") ||
    lowered.includes("provider_unavailable") ||
    lowered.includes("route_failed") ||
    lowered.includes("model_alias_not_mapped")
  ) {
    return EXIT_ROUTE_FAILED;
  }
  if (lowered.includes("timed out") || lowered.includes("timeout")) {
    return EXIT_TIMEOUT;
  }
  return EXIT_AICC_FAILED;
}

// ---------------------------------------------------------------------------
// Common "bail" helpers. Each command file uses these at the obvious failure
// points so the recipe stays linear (no try/catch wrappers around the whole
// flow). Agents writing new tools should use the same set.
// ---------------------------------------------------------------------------

export function bailRuntimeError(tool: string, err: unknown): never {
  const msg = err instanceof Error ? err.message : String(err);
  emitAndExit(
    errorResult(tool, `${tool} => runtime_init_failed`, msg, { error: msg }),
    EXIT_AICC_FAILED,
  );
}

export function bailAiccError(tool: string, method: string, err: unknown): never {
  const msg = err instanceof Error ? err.message : String(err);
  emitAndExit(
    errorResult(tool, `${tool} => aicc_call_failed`, msg, { method, error: msg }),
    classifyAiccError(msg),
  );
}

export function bailAiccFailed(
  tool: string,
  method: string,
  taskId: string,
  reason: string,
): never {
  emitAndExit(
    errorResult(tool, `${tool} => failed`, reason, { method, task_id: taskId, error: reason }),
    classifyAiccError(reason),
  );
}

export function bailIoError(tool: string, taskId: string | undefined, err: unknown): never {
  const msg = err instanceof Error ? err.message : String(err);
  emitAndExit(
    errorResult(tool, `${tool} => io_failed`, msg, {
      task_id: taskId ?? null,
      error: msg,
    }),
    EXIT_IO_FAILED,
  );
}

export function bailNoArtifact(tool: string, method: string, taskId: string): never {
  emitAndExit(
    errorResult(tool, `${tool} => no_artifact`, "AICC returned no expected artifact", {
      method,
      task_id: taskId,
    }),
    EXIT_AICC_FAILED,
  );
}

export { EXIT_AICC_FAILED, EXIT_ARG_ERROR, EXIT_IO_FAILED, EXIT_ROUTE_FAILED, EXIT_SUCCESS, EXIT_TIMEOUT };
