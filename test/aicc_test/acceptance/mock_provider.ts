import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

type Scenario =
  | "success"
  | "stream_success"
  | "async_success"
  | "async_failed"
  | "async_pending"
  | "bad_request"
  | "unauthorized"
  | "forbidden"
  | "not_found"
  | "idempotency_conflict"
  | "rate_limit"
  | "provider_5xx"
  | "connection_failed"
  | "timeout_short"
  | "timeout_long"
  | "malformed_response"
  | "wrong_mime"
  | "missing_usage"
  | "safety_blocked"
  | "quota_exhausted"
  | "invalid_resource"
  | "embedding_dimension_mismatch"
  | "embedding_row_count_mismatch"
  | "embedding_order_mismatch"
  | "embedding_nonfinite"
  | "rerank_missing_score"
  | "rerank_document_id_mismatch"
  | "rerank_result_count_mismatch";

type RecordedRequest = {
  id: number;
  at: string;
  method: string;
  path: string;
  headers: Record<string, string>;
  body: Json | null;
  scenario: Scenario;
};

type State = {
  defaultScenario: Scenario;
  scenarios: Map<string, Scenario>;
  provider: {
    health: string;
    quota: string;
    latency_ms: number;
    capabilities: string[];
  };
  requests: RecordedRequest[];
  calls: number;
  errors: number;
  streamChunks: number;
  operations: Map<string, { polls: number; scenario: Scenario }>;
};

const state: State = {
  defaultScenario: "success",
  scenarios: new Map(),
  provider: {
    health: "available",
    quota: "normal",
    latency_ms: 0,
    capabilities: [],
  },
  requests: [],
  calls: 0,
  errors: 0,
  streamChunks: 0,
  operations: new Map(),
};

const VALID_SCENARIOS = new Set<Scenario>([
  "success",
  "stream_success",
  "async_success",
  "async_failed",
  "async_pending",
  "bad_request",
  "unauthorized",
  "forbidden",
  "not_found",
  "idempotency_conflict",
  "rate_limit",
  "provider_5xx",
  "connection_failed",
  "timeout_short",
  "timeout_long",
  "malformed_response",
  "wrong_mime",
  "missing_usage",
  "safety_blocked",
  "quota_exhausted",
  "invalid_resource",
  "embedding_dimension_mismatch",
  "embedding_row_count_mismatch",
  "embedding_order_mismatch",
  "embedding_nonfinite",
  "rerank_missing_score",
  "rerank_document_id_mismatch",
  "rerank_result_count_mismatch",
]);

function parsePort(args: string[]): number {
  const index = args.indexOf("--port");
  const value = index >= 0 ? Number(args[index + 1]) : 18080;
  if (!Number.isInteger(value) || value < 0 || value > 65535) {
    throw new Error("--port must be between 0 and 65535");
  }
  return value;
}

function parseHost(args: string[]): string {
  const index = args.indexOf("--host");
  const value = index >= 0 ? args[index + 1]?.trim() : "127.0.0.1";
  if (!value || !/^(?:localhost|\d{1,3}(?:\.\d{1,3}){3})$/.test(value)) {
    throw new Error("--host must be localhost or an IPv4 address");
  }
  return value;
}

function json(response: ServerResponse, status: number, value: Json): void {
  const body = JSON.stringify(value);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  response.end(body);
}

function text(response: ServerResponse, status: number, value: string, mime = "text/plain"): void {
  response.writeHead(status, {
    "content-type": mime,
    "content-length": Buffer.byteLength(value),
  });
  response.end(value);
}

async function readJson(request: IncomingMessage): Promise<Json | null> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(Buffer.from(chunk));
  if (chunks.length === 0) return null;
  const raw = Buffer.concat(chunks).toString("utf8");
  try {
    return JSON.parse(raw) as Json;
  } catch {
    return { malformed_request_body: raw.slice(0, 256) };
  }
}

function object(value: Json | null): { [key: string]: Json } | null {
  return value && typeof value === "object" && !Array.isArray(value) ? value : null;
}

function scenarioFrom(request: IncomingMessage, body: Json | null): Scenario {
  const requestId = request.headers["x-aicc-request-id"];
  if (typeof requestId === "string" && state.scenarios.has(requestId)) {
    return state.scenarios.get(requestId)!;
  }
  const header = request.headers["x-aicc-mock-scenario"];
  if (typeof header === "string" && VALID_SCENARIOS.has(header as Scenario)) {
    return header as Scenario;
  }
  const bodyObject = object(body);
  const options = object(bodyObject?.options ?? null);
  const behavior = object(options?.mock_behavior ?? bodyObject?.mock_behavior ?? null);
  const value = behavior?.scenario;
  return typeof value === "string" && VALID_SCENARIOS.has(value as Scenario)
    ? value as Scenario
    : state.defaultScenario;
}

function sanitizedHeaders(request: IncomingMessage): Record<string, string> {
  return Object.fromEntries(
    Object.entries(request.headers).map(([key, value]) => [
      key,
      /authorization|api-key|cookie/i.test(key) ? "[REDACTED]" : String(value ?? ""),
    ]),
  );
}

function model(body: Json | null): string {
  return String(object(body)?.model ?? "mock-model");
}

function usage(): { input_tokens: number; output_tokens: number; total_tokens: number } {
  return { input_tokens: 10, output_tokens: 5, total_tokens: 15 };
}

function deterministicText(body: Json | null, scenario: Scenario = "success"): string {
  if (JSON.stringify(body).includes("Rerank the documents")) {
    if (scenario === "rerank_missing_score") {
      return JSON.stringify({
        results: [
          { index: 1, id: "right" },
          { index: 0, id: "wrong", score: 0.01 },
        ],
      });
    }
    if (scenario === "rerank_document_id_mismatch") {
      return JSON.stringify({
        results: [
          { index: 1, id: "not-a-request-document", score: 0.99 },
          { index: 0, id: "wrong", score: 0.01 },
        ],
      });
    }
    if (scenario === "rerank_result_count_mismatch") {
      return JSON.stringify({ results: [{ index: 1, id: "right", score: 0.99 }] });
    }
    return JSON.stringify({
      results: [
        { index: 1, id: "right", score: 0.99 },
        { index: 0, id: "wrong", score: 0.01 },
      ],
    });
  }
  if (JSON.stringify(body).includes("aicc_acceptance_marker")) {
    return JSON.stringify({ marker: "BUCKYOS-AICC-4827" });
  }
  return "BUCKYOS-AICC-4827";
}

function requestsToolCall(body: Json | null): boolean {
  const bodyObject = object(body);
  return Array.isArray(bodyObject?.tools) && bodyObject.tools.some((tool) => {
    const value = object(tool);
    return value.type === "function" || value.name === "echo_marker" ||
      object(value.function)?.name === "echo_marker";
  });
}

function errorResponse(response: ServerResponse, scenario: Scenario): boolean {
  const map: Partial<Record<Scenario, [number, string]>> = {
    bad_request: [400, "invalid_request"],
    unauthorized: [401, "invalid_api_key"],
    forbidden: [403, "content_policy"],
    not_found: [404, "model_not_found"],
    idempotency_conflict: [409, "idempotency_conflict"],
    rate_limit: [429, "rate_limit"],
    quota_exhausted: [429, "quota_exhausted"],
    provider_5xx: [503, "provider_unavailable"],
    safety_blocked: [403, "safety_blocked"],
    invalid_resource: [422, "invalid_resource"],
  };
  const mapped = map[scenario];
  if (!mapped) return false;
  state.errors += 1;
  json(response, mapped[0], {
    error: {
      type: mapped[1],
      code: `mock/${mapped[1]}`,
      message: `deterministic ${mapped[1]} from AICC mock provider`,
    },
  });
  return true;
}

function openAiResponse(body: Json | null, includeUsage: boolean, scenario: Scenario = "success"): Json {
  const output: Json[] = requestsToolCall(body)
    ? [{
      type: "function_call",
      id: `fc_mock_${state.calls}`,
      call_id: `call_mock_${state.calls}`,
      name: "echo_marker",
      arguments: JSON.stringify({ marker: "BUCKYOS-AICC-4827" }),
      status: "completed",
    }]
    : [{
      type: "message",
      id: `msg_mock_${state.calls}`,
      role: "assistant",
      status: "completed",
      content: [{ type: "output_text", text: deterministicText(body, scenario) }],
    }];
  const result: { [key: string]: Json } = {
    id: `resp_mock_${state.calls}`,
    object: "response",
    status: "completed",
    model: model(body),
    output,
  };
  if (includeUsage) result.usage = { input_tokens: 10, output_tokens: 5, total_tokens: 15 };
  return result;
}

function chatResponse(body: Json | null, includeUsage: boolean, scenario: Scenario = "success"): Json {
  const message: { [key: string]: Json } = requestsToolCall(body)
    ? {
      role: "assistant",
      content: null,
      tool_calls: [{
        id: `call_mock_${state.calls}`,
        type: "function",
        function: {
          name: "echo_marker",
          arguments: JSON.stringify({ marker: "BUCKYOS-AICC-4827" }),
        },
      }],
    }
    : { role: "assistant", content: deterministicText(body, scenario) };
  const result: { [key: string]: Json } = {
    id: `chatcmpl_mock_${state.calls}`,
    object: "chat.completion",
    model: model(body),
    choices: [{
      index: 0,
      message,
      finish_reason: "stop",
    }],
  };
  if (includeUsage) result.usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 };
  return result;
}

function geminiResponse(includeUsage: boolean): Json {
  const result: { [key: string]: Json } = {
    candidates: [{
      content: { role: "model", parts: [{ text: "BUCKYOS-AICC-4827" }] },
      finishReason: "STOP",
      index: 0,
    }],
  };
  if (includeUsage) {
    result.usageMetadata = {
      promptTokenCount: 10,
      candidatesTokenCount: 5,
      totalTokenCount: 15,
    };
  }
  return result;
}

function claudeResponse(body: Json | null, includeUsage: boolean): Json {
  const result: { [key: string]: Json } = {
    id: `msg_mock_${state.calls}`,
    type: "message",
    role: "assistant",
    model: model(body),
    content: [{ type: "text", text: "BUCKYOS-AICC-4827" }],
    stop_reason: "end_turn",
    stop_sequence: null,
  };
  if (includeUsage) result.usage = { input_tokens: 10, output_tokens: 5 };
  return result;
}

function streamOpenAi(response: ServerResponse): void {
  response.writeHead(200, { "content-type": "text/event-stream" });
  const events = [
    { type: "response.output_text.delta", delta: "BUCKYOS-" },
    { type: "response.output_text.delta", delta: "AICC-4827" },
    { type: "response.completed", response: openAiResponse(null, true) },
  ];
  for (const event of events) {
    response.write(`event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`);
    state.streamChunks += 1;
  }
  response.end();
}

function streamChat(response: ServerResponse): void {
  response.writeHead(200, { "content-type": "text/event-stream" });
  for (const content of ["BUCKYOS-", "AICC-4827"]) {
    response.write(`data: ${JSON.stringify({
      id: `chatcmpl_mock_${state.calls}`,
      object: "chat.completion.chunk",
      choices: [{ index: 0, delta: { content }, finish_reason: null }],
    })}\n\n`);
    state.streamChunks += 1;
  }
  response.write("data: [DONE]\n\n");
  response.end();
}

async function management(
  request: IncomingMessage,
  response: ServerResponse,
  path: string,
  body: Json | null,
): Promise<boolean> {
  if (path.startsWith("/__mock/fixtures/") && request.method === "GET") {
    const name = path.slice("/__mock/fixtures/".length);
    if (name.endsWith(".png")) {
      const bytes = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=", "base64");
      response.writeHead(200, { "content-type": "image/png", "content-length": bytes.length });
      response.end(bytes);
      return true;
    }
    if (name.endsWith(".wav")) {
      const bytes = Buffer.from("RIFFmock-audio-WAVE");
      response.writeHead(200, { "content-type": "audio/wav", "content-length": bytes.length });
      response.end(bytes);
      return true;
    }
    if (name.endsWith(".mp4")) {
      const bytes = Buffer.from("00000018667479706d703432", "hex");
      response.writeHead(200, { "content-type": "video/mp4", "content-length": bytes.length });
      response.end(bytes);
      return true;
    }
  }
  if (path === "/__mock/health" && request.method === "GET") {
    json(response, 200, { ok: true, ...state.provider });
    return true;
  }
  if (path === "/__mock/reset" && request.method === "POST") {
    state.defaultScenario = "success";
    state.scenarios.clear();
    state.requests = [];
    state.calls = 0;
    state.errors = 0;
    state.streamChunks = 0;
    state.operations.clear();
    json(response, 200, { ok: true });
    return true;
  }
  if (path === "/__mock/scenario" && request.method === "POST") {
    const input = object(body);
    const scenario = input?.scenario;
    if (typeof scenario !== "string" || !VALID_SCENARIOS.has(scenario as Scenario)) {
      json(response, 400, { error: "invalid scenario" });
      return true;
    }
    const requestId = input?.request_id;
    if (typeof requestId === "string" && requestId) {
      state.scenarios.set(requestId, scenario as Scenario);
    } else {
      state.defaultScenario = scenario as Scenario;
    }
    json(response, 200, { ok: true });
    return true;
  }
  if (path === "/__mock/provider_state" && request.method === "POST") {
    const input = object(body);
    if (typeof input?.health === "string") state.provider.health = input.health;
    if (typeof input?.quota === "string") state.provider.quota = input.quota;
    if (typeof input?.latency_ms === "number") state.provider.latency_ms = input.latency_ms;
    if (Array.isArray(input?.capabilities)) {
      state.provider.capabilities = input.capabilities.filter((item): item is string =>
        typeof item === "string"
      );
    }
    json(response, 200, { ok: true, ...state.provider });
    return true;
  }
  if (path === "/__mock/requests" && request.method === "GET") {
    json(response, 200, { requests: state.requests as unknown as Json });
    return true;
  }
  if (path === "/__mock/metrics" && request.method === "GET") {
    json(response, 200, {
      calls: state.calls,
      errors: state.errors,
      stream_chunks: state.streamChunks,
    });
    return true;
  }
  return false;
}

async function providerResponse(
  request: IncomingMessage,
  response: ServerResponse,
  path: string,
  body: Json | null,
  scenario: Scenario,
): Promise<void> {
  if (scenario === "connection_failed") {
    state.errors += 1;
    request.socket.destroy();
    return;
  }
  if (scenario === "timeout_short" || scenario === "timeout_long") {
    const timeout = scenario === "timeout_short" ? 2_000 : 60_000;
    await new Promise((resolve) => setTimeout(resolve, timeout));
  }
  if (errorResponse(response, scenario)) return;
  if (scenario === "malformed_response") {
    text(response, 200, "{not-json", "application/json");
    return;
  }
  if (scenario === "wrong_mime") {
    text(response, 200, "not-an-artifact", "text/plain");
    return;
  }
  const includeUsage = scenario !== "missing_usage";
  if (path === "/v1/models") {
    json(response, 200, {
      object: "list",
      data: [
        { id: "gpt-4o-mini", object: "model", owned_by: "mock" },
        { id: "gpt-5.4", object: "model", owned_by: "mock" },
        { id: "text-embedding-3-small", object: "model", owned_by: "mock" },
        { id: "gpt-image-1", object: "model", owned_by: "mock" },
        { id: "whisper-1", object: "model", owned_by: "mock" },
        { id: "tts-1", object: "model", owned_by: "mock" },
        { id: "sora-2", object: "model", owned_by: "mock" },
        { id: "sora-mock-pattern", object: "model", owned_by: "mock" },
      ],
      has_more: false,
    });
    return;
  }
  if (path === "/v1beta/models" || path === "/models") {
    json(response, 200, {
      models: [
        {
          name: "models/gemini-2.5-flash",
          displayName: "Gemini 2.5 Flash Mock",
          supportedGenerationMethods: ["generateContent", "streamGenerateContent"],
        },
        {
          name: "models/gemini-embedding-2-preview",
          displayName: "Gemini Multimodal Embedding Mock",
          supportedGenerationMethods: ["embedContent", "batchEmbedContents"],
        },
        {
          name: "models/gemini-2.5-flash-preview-tts",
          displayName: "Gemini TTS Mock",
          supportedGenerationMethods: ["generateContent"],
        },
        {
          name: "models/lyria-3-clip-preview",
          displayName: "Lyria Mock",
          supportedGenerationMethods: ["predictLongRunning"],
        },
        {
          name: "models/veo-3.1-generate-preview",
          displayName: "Veo Mock",
          supportedGenerationMethods: ["predictLongRunning"],
        },
        {
          name: "models/gemini-omni-flash-preview",
          displayName: "Gemini Omni Mock",
          supportedGenerationMethods: ["generateContent", "predictLongRunning"],
        },
        {
          name: "models/gemini-2.5-computer-use-preview-10-2025",
          displayName: "Gemini Computer Use Mock",
          supportedGenerationMethods: ["generateContent"],
        },
      ],
      nextPageToken: "",
    });
    return;
  }
  if (path === "/v1/responses") {
    if (scenario === "stream_success" || object(body)?.stream === true) streamOpenAi(response);
    else json(response, 200, openAiResponse(body, includeUsage, scenario));
    return;
  }
  if (path === "/v1/chat/completions") {
    if (scenario === "stream_success" || object(body)?.stream === true) streamChat(response);
    else json(response, 200, chatResponse(body, includeUsage, scenario));
    return;
  }
  if (path === "/v1/embeddings") {
    const input = object(body)?.input;
    const count = Array.isArray(input) ? input.length : 1;
    let data: Json[] = Array.from({ length: count }, (_, index) => ({
      object: "embedding",
      index,
      embedding: [0.1 + index, 0.2, 0.3, 0.4],
    }));
    if (scenario === "embedding_dimension_mismatch" && data.length > 1) {
      (data[1] as { [key: string]: Json }).embedding = [0.1, 0.2, 0.3];
    } else if (scenario === "embedding_row_count_mismatch") {
      data = data.slice(0, Math.max(0, count - 1));
    } else if (scenario === "embedding_order_mismatch") {
      data = data.map((item, index) => ({ ...(item as { [key: string]: Json }), index: count - index - 1 }));
    } else if (scenario === "embedding_nonfinite" && data.length > 0) {
      (data[0] as { [key: string]: Json }).embedding = [0.1, null, 0.3, 0.4];
    }
    const result: { [key: string]: Json } = { object: "list", data, model: model(body) };
    if (includeUsage) result.usage = { prompt_tokens: count, total_tokens: count };
    json(response, 200, result);
    return;
  }
  if (path === "/v1/images/generations" || path === "/v1/images/edits") {
    json(response, 200, {
      created: 1,
      data: [{
        url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
      }],
      ...(includeUsage ? { usage: usage() } : {}),
    });
    return;
  }
  if (path === "/v1/audio/transcriptions") {
    json(response, 200, { text: "今天的测试编号是四八二七", ...(includeUsage ? { usage: usage() } : {}) });
    return;
  }
  if (path === "/v1/audio/speech") {
    text(response, 200, "RIFFmock-audio-WAVE", "audio/wav");
    return;
  }
  if (path === "/v1/messages") {
    json(response, 200, claudeResponse(body, includeUsage));
    return;
  }
  if (/:(?:generateContent|streamGenerateContent)$/.test(path)) {
    json(response, 200, geminiResponse(includeUsage));
    return;
  }
  if (/:embedContent$/.test(path)) {
    json(response, 200, { embedding: { values: [0.1, 0.2, 0.3, 0.4] } });
    return;
  }
  if (/:batchEmbedContents$/.test(path)) {
    const requests = object(body)?.requests;
    const count = Array.isArray(requests) ? requests.length : 1;
    json(response, 200, {
      embeddings: Array.from({ length: count }, () => ({ values: [0.1, 0.2, 0.3, 0.4] })),
    });
    return;
  }
  if (/:predictLongRunning$/.test(path)) {
    const operationName = `operations/gemini-mock-${state.calls}`;
    state.operations.set(`/${operationName}`, { polls: 0, scenario });
    json(response, 200, { name: operationName, done: false });
    return;
  }
  if (path.startsWith("/v1beta/operations/") || path.startsWith("/operations/")) {
    const operationKey = path.replace(/^\/v1beta(?=\/operations\/)/, "");
    const operation = state.operations.get(operationKey) ?? { polls: 0, scenario };
    operation.polls += 1;
    state.operations.set(operationKey, operation);
    if (operation.scenario === "async_pending") {
      json(response, 200, { name: path.slice(1), done: false });
      return;
    }
    if (operation.scenario === "async_failed" && operation.polls >= 2) {
      json(response, 200, {
        name: path.slice(1),
        done: true,
        error: { code: 13, status: "INTERNAL", message: "deterministic async Provider failure" },
      });
      return;
    }
    json(response, 200, operation.polls < 2
      ? { name: path.slice(1), done: false }
      : {
        name: path.slice(1),
        done: true,
        response: {
          generatedVideos: [{
            video: { uri: "http://127.0.0.1:18080/__mock/fixtures/video.mp4" },
          }],
        },
      });
    return;
  }
  if (path.startsWith("/fal-ai/")) {
    const requestId = `fal-mock-${state.calls}`;
    state.operations.set(`/queue/requests/${requestId}/status`, { polls: 0, scenario });
    json(response, 200, { request_id: requestId, status: "IN_QUEUE" });
    return;
  }
  if (path === "/v1/videos" && request.method === "POST") {
    const videoId = `video_mock_${state.calls}`;
    state.operations.set(`/v1/videos/${videoId}`, { polls: 0, scenario });
    json(response, 200, { id: videoId, status: "queued" });
    return;
  }
  if (/^\/v1\/videos\/[^/]+$/.test(path) && request.method === "GET") {
    const operation = state.operations.get(path) ?? { polls: 0, scenario };
    operation.polls += 1;
    state.operations.set(path, operation);
    if (operation.scenario === "async_pending") {
      json(response, 200, { id: path.split("/").at(-1)!, status: "in_progress" });
      return;
    }
    if (operation.scenario === "async_failed" && operation.polls >= 2) {
      json(response, 200, { id: path.split("/").at(-1)!, status: "failed", error: { message: "deterministic async Provider failure" } });
      return;
    }
    json(response, 200, {
      id: path.split("/").at(-1)!,
      status: operation.polls < 2 ? "in_progress" : "completed",
    });
    return;
  }
  if (/^\/v1\/videos\/[^/]+\/content$/.test(path) && request.method === "GET") {
    text(response, 200, "mock-video", "video/mp4");
    return;
  }
  if (/^\/queue\/requests\/[^/]+\/status$/.test(path)) {
    const operation = state.operations.get(path) ?? { polls: 0, scenario };
    operation.polls += 1;
    state.operations.set(path, operation);
    json(response, 200, { status: operation.polls < 2 ? "IN_PROGRESS" : "COMPLETED" });
    return;
  }
  if (/^\/queue\/requests\/[^/]+$/.test(path)) {
    const mime = path.includes("video") ? "video/mp4" : "image/png";
    json(response, 200, { output: { url: `https://mock.invalid/output.${mime.split("/")[1]}`, content_type: mime } });
    return;
  }
  json(response, 404, { error: { type: "unknown_endpoint", message: path } });
}

const args = process.argv.slice(2);
const port = parsePort(args);
const host = parseHost(args);
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    const providerPath = url.pathname.replace(/^\/instance-[ab](?=\/)/, "");
    const body = request.method === "POST" ? await readJson(request) : null;
    if (await management(request, response, url.pathname, body)) return;
    const scenario = scenarioFrom(request, body);
    state.calls += 1;
    state.requests.push({
      id: state.calls,
      at: new Date().toISOString(),
      method: request.method ?? "GET",
      path: url.pathname,
      headers: sanitizedHeaders(request),
      body,
      scenario,
    });
    await providerResponse(request, response, providerPath, body, scenario);
  } catch (error) {
    state.errors += 1;
    json(response, 500, { error: { type: "mock_internal", message: String(error) } });
  }
});

server.listen(port, host, () => {
  const address = server.address();
  const actualPort = typeof address === "object" && address ? address.port : port;
  process.stdout.write(`${JSON.stringify({ ready: true, host, port: actualPort })}\n`);
});

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
