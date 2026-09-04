import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  loadProviderProtocolCatalog,
  protocolContract,
  type CapturedProviderRequest,
  type ProviderProtocolCatalog,
  type ProviderProtocolContract,
  selectOfficialModels,
  validateProviderAuxiliaryRequest,
  validateProviderRequest,
} from "./provider_protocol_contracts.ts";

type Selection = { provider_driver: string; contract_id: string; api_type?: string; scenario: string; selection_seed?: string };
type DiscoveryContract = {
  mode: "machine_api" | "catalog_only";
  path?: string;
  required_query?: Record<string, string>;
  required_headers?: Record<string, string>;
  response_shape?: "openai" | "anthropic" | "gemini" | "sn";
};

export const T15_PROVIDER_DISCOVERY_CONTRACTS: Record<string, DiscoveryContract> = {
  openai: { mode: "machine_api", path: "/v1/models", response_shape: "openai" },
  claude: {
    mode: "machine_api",
    path: "/v1/models",
    required_query: { limit: "1000" },
    required_headers: { "anthropic-version": "2023-06-01" },
    response_shape: "anthropic",
  },
  "google-gemini": {
    mode: "machine_api",
    path: "/v1beta/models",
    required_query: { pageSize: "1000" },
    response_shape: "gemini",
  },
  fal: { mode: "catalog_only" },
  minimax: {
    mode: "machine_api",
    path: "/anthropic/v1/models",
    required_query: { limit: "1000" },
    response_shape: "anthropic",
  },
  openrouter: { mode: "machine_api", path: "/api/v1/models", response_shape: "openai" },
  kimi: { mode: "machine_api", path: "/v1/models", response_shape: "openai" },
  glm: { mode: "catalog_only" },
  deepseek: { mode: "machine_api", path: "/models", response_shape: "openai" },
  doubao: { mode: "catalog_only" },
  qwen: { mode: "catalog_only" },
  "sn-ai-provider": { mode: "machine_api", path: "/api/v1/ai/models", response_shape: "sn" },
};
type AuditRecord = {
  received_at: string;
  selection: Selection;
  method: string;
  pathname: string;
  query: Record<string, string>;
  headers: Record<string, string>;
  body: unknown;
  async_step?: "poll" | "result" | "cancel";
  validation_errors: string[];
};

function json(response: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}): void {
  response.writeHead(status, { "content-type": "application/json", ...headers });
  response.end(JSON.stringify(body));
}

async function bodyBytes(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  return Buffer.concat(chunks);
}

function parseRequestBody(request: IncomingMessage, bytes: Buffer): unknown {
  const contentType = request.headers["content-type"] ?? "";
  if (contentType.startsWith("application/json")) {
    try {
      return bytes.length > 0 ? JSON.parse(bytes.toString("utf8")) : {};
    } catch {
      return undefined;
    }
  }
  if (contentType.startsWith("multipart/form-data")) {
    const fields: Record<string, unknown> = {};
    for (const match of bytes.toString("latin1").matchAll(/content-disposition:\s*form-data;\s*name="([^"]+)"(?:;\s*filename="([^"]*)")?[\s\S]*?\r\n\r\n([\s\S]*?)(?=\r\n--)/gi)) {
      fields[match[1]] = match[2] !== undefined ? { filename: match[2], bytes: Buffer.byteLength(match[3], "latin1") } : match[3];
    }
    return fields;
  }
  return undefined;
}

function safeHeaders(headers: IncomingMessage["headers"]): Record<string, string> {
  return Object.fromEntries(Object.entries(headers).map(([name, value]) => [
    name,
    /authorization|api-key|token|secret/i.test(name) ? "[REDACTED]" : Array.isArray(value) ? value.join(",") : value ?? "",
  ]));
}

function rewriteMockUrls(value: unknown, authority: string, endpoint: string): unknown {
  if (typeof value === "string") {
    return value.replaceAll("http://mock/{endpoint}", `http://${authority}/${endpoint}`)
      .replaceAll("http://mock", `http://${authority}`);
  }
  if (Array.isArray(value)) return value.map((item) => rewriteMockUrls(item, authority, endpoint));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, rewriteMockUrls(item, authority, endpoint)]));
  }
  return value;
}

function discoveryFixture(
  provider: ProviderProtocolCatalog["providers"][number],
  shape: NonNullable<DiscoveryContract["response_shape"]>,
  selectionSeed?: string,
): unknown {
  const randomModels = provider.official_first_party_model_ids && selectionSeed
    ? Object.values(selectOfficialModels(
      { schema_version: 1, revision: "selection", checked_at: "selection", providers: [provider], error_evidence: {}, error_fixtures: {} },
      provider.provider_driver,
      selectionSeed,
    ))
    : [];
  const modelIds = [...new Set([...Object.values(provider.test_model_ids), ...randomModels])];
  if (shape === "sn") {
    return {
      revision: "t15-mock-1",
      items: modelIds.map((model) => ({ model, provider: "t15-mock", display_name: model })),
      default_model: modelIds[0] ?? null,
    };
  }
  if (shape === "gemini") {
    return {
      models: modelIds.map((id) => ({ name: `models/${id}`, baseModelId: `models/${id}` })),
      nextPageToken: "",
    };
  }
  if (shape === "anthropic") {
    return {
      data: modelIds.map((id) => ({ id, type: "model", display_name: id, created_at: "2026-01-01T00:00:00Z" })),
      has_more: false,
      first_id: modelIds[0] ?? null,
      last_id: modelIds.at(-1) ?? null,
    };
  }
  return {
    object: "list",
    data: modelIds.map((id) => ({ id, object: "model", owned_by: provider.provider_driver })),
  };
}

function streamFixture(contract: ProviderProtocolContract): string {
  switch (contract.stream_protocol) {
    case "openai_responses":
      return [
        "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_mock_1\",\"object\":\"response\",\"status\":\"in_progress\",\"output\":[]}}",
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_mock_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"BUCKYOS-AICC-4827\"}",
        `event: response.completed\ndata: ${JSON.stringify({ type: "response.completed", response: contract.success_fixture })}`,
      ].join("\n\n") + "\n\n";
    case "claude_messages":
      return [
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_mock_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"mock-model\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":4,\"output_tokens\":0}}}",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"BUCKYOS-AICC-4827\"}}",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":3}}",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}",
      ].join("\n\n") + "\n\n";
    case "gemini_interactions":
      return [
        "event: interaction.created\ndata: {\"event_type\":\"interaction.created\",\"interaction\":{\"id\":\"interaction_mock_1\",\"status\":\"in_progress\"}}",
        "event: content.delta\ndata: {\"event_type\":\"content.delta\",\"delta\":{\"type\":\"text\",\"text\":\"BUCKYOS-AICC-4827\"}}",
        `event: interaction.completed\ndata: ${JSON.stringify({ event_type: "interaction.completed", interaction: contract.success_fixture })}`,
      ].join("\n\n") + "\n\n";
    case "openai_chat":
    case "openrouter_chat":
      return [
        "data: {\"id\":\"gen_mock_1\",\"object\":\"chat.completion.chunk\",\"created\":1770000000,\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"BUCKYOS-AICC-4827\"},\"finish_reason\":null}]}",
        "data: {\"id\":\"gen_mock_1\",\"object\":\"chat.completion.chunk\",\"created\":1770000000,\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":3,\"total_tokens\":7}}",
        "data: [DONE]",
      ].join("\n\n") + "\n\n";
    default:
      throw new Error(`contract ${contract.id} has no streaming protocol`);
  }
}

export function createT15MockHandler(catalog: ProviderProtocolCatalog) {
  const catalogDrivers = new Set(catalog.providers.map((provider) => provider.provider_driver));
  const discoveryDrivers = new Set(Object.keys(T15_PROVIDER_DISCOVERY_CONTRACTS));
  const missingDiscovery = [...catalogDrivers].filter((driver) => !discoveryDrivers.has(driver));
  const unknownDiscovery = [...discoveryDrivers].filter((driver) => !catalogDrivers.has(driver));
  if (missingDiscovery.length > 0 || unknownDiscovery.length > 0) {
    throw new Error(`T1.5 discovery contracts differ from Provider catalog: missing=${missingDiscovery.join(",")} unknown=${unknownDiscovery.join(",")}`);
  }
  let selection: Selection | undefined;
  let requests: AuditRecord[] = [];
  return async (request: IncomingMessage, response: ServerResponse): Promise<void> => {
    try {
      const url = new URL(request.url ?? "/", "http://mock.invalid");
      if (url.pathname === "/__mock/health") return json(response, 200, { ok: true, revision: catalog.revision });
      if (url.pathname === "/__mock/reset" && request.method === "POST") {
        selection = undefined;
        requests = [];
        return json(response, 200, { ok: true });
      }
      if (url.pathname === "/__mock/select" && request.method === "POST") {
        const parsed = JSON.parse((await bodyBytes(request)).toString("utf8")) as Selection;
        const contract = protocolContract(catalog, parsed.provider_driver, parsed.contract_id);
        const scenarios = new Set([
          "success",
          "malformed_response",
          "wrong_content_type",
          "missing_required_response_field",
          ...catalog.error_fixtures[parsed.provider_driver].map((fixture) => fixture.scenario),
        ]);
        if (contract.stream_protocol) {
          scenarios.add("stream_success");
          scenarios.add("stream_interrupted");
        }
        if (contract.async_protocol) {
          scenarios.add("async_success");
          scenarios.add("async_failed");
          scenarios.add("async_poll_timeout");
          scenarios.add("async_artifact_unavailable");
          if (contract.async_steps?.some((step) => step.name === "cancel")) scenarios.add("async_cancel");
        }
        if (!scenarios.has(parsed.scenario)) return json(response, 400, { error: "unknown scenario" });
        selection = parsed;
        requests = [];
        return json(response, 200, { ok: true, selection });
      }
      if (url.pathname === "/__mock/requests" && request.method === "GET") {
        return json(response, 200, { selection, requests });
      }
      const selectedProvider = selection
        ? catalog.providers.find((provider) => provider.provider_driver === selection?.provider_driver)
        : undefined;
      const discoveryProvider = selectedProvider && T15_PROVIDER_DISCOVERY_CONTRACTS[selectedProvider.provider_driver]?.mode === "machine_api"
        ? selectedProvider
        : undefined;
      const discovery = discoveryProvider
        ? T15_PROVIDER_DISCOVERY_CONTRACTS[discoveryProvider.provider_driver]
        : undefined;
      if (discoveryProvider && discovery?.path === url.pathname && request.method === "GET") {
        const auth = discoveryProvider.contracts[0].auth;
        const authValue = auth.kind === "header"
          ? request.headers[auth.name.toLowerCase()]
          : url.searchParams.get(auth.name);
        const value = Array.isArray(authValue) ? authValue[0] : authValue;
        if (!value || !value.startsWith(auth.prefix) || value.length <= auth.prefix.length) {
          return json(response, 401, { error: { code: "authentication_error", message: "invalid discovery credential" } });
        }
        const discoveryErrors = [
          ...Object.entries(discovery.required_query ?? {}).flatMap(([name, expected]) =>
            url.searchParams.get(name) === expected ? [] : [`query ${name} must equal ${expected}`]
          ),
          ...Object.entries(discovery.required_headers ?? {}).flatMap(([name, expected]) =>
            request.headers[name] === expected ? [] : [`header ${name} must equal ${expected}`]
          ),
        ];
        requests.push({
          received_at: new Date().toISOString(),
          selection: selection!,
          method: request.method,
          pathname: url.pathname,
          query: Object.fromEntries(url.searchParams),
          headers: safeHeaders(request.headers),
          body: null,
          validation_errors: discoveryErrors,
        });
        if (discoveryErrors.length > 0) {
          return json(response, 400, { type: "t15_mock_contract_violation", errors: discoveryErrors });
        }
          return json(response, 200, discoveryFixture(
            discoveryProvider,
            discovery.response_shape!,
            selection?.selection_seed,
          ));
      }
      if (selectedProvider && request.method === "GET" && url.pathname.endsWith("/models")) {
        return json(response, 404, {
          error: T15_PROVIDER_DISCOVERY_CONTRACTS[selectedProvider.provider_driver].mode === "catalog_only"
            ? "Provider uses catalog-only discovery"
            : "unexpected Provider discovery path",
        });
      }
      if (!selection) return json(response, 409, { error: "select a Provider contract before calling the mock" });
      const contract = protocolContract(catalog, selection.provider_driver, selection.contract_id);
      const captureAuxiliary = () => {
        const captured = {
          method: request.method ?? "",
          pathname: url.pathname,
          query: url.searchParams,
          headers: new Headers(request.headers as Record<string, string>),
        };
        const validation = validateProviderAuxiliaryRequest(contract, captured);
        requests.push({
          received_at: new Date().toISOString(),
          selection: selection!,
          method: captured.method,
          pathname: captured.pathname,
          query: Object.fromEntries(captured.query),
          headers: safeHeaders(request.headers),
          body: null,
          async_step: validation.step?.name,
          validation_errors: validation.errors,
        });
        return validation.errors;
      };

      if (contract.async_protocol === "fal_queue") {
        if (/\/requests\/fal_mock_1\/status$/.test(url.pathname) && request.method === "GET") {
          const errors = captureAuxiliary();
          if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
          if (selection.scenario === "async_cancel") {
            return json(response, 200, { status: "IN_QUEUE", request_id: "fal_mock_1", queue_position: 0 });
          }
          if (selection.scenario === "async_poll_timeout") {
            return json(response, 200, { status: "IN_PROGRESS", request_id: "fal_mock_1", logs: [] });
          }
          if (selection.scenario === "async_failed") {
            return json(response, 200, { status: "FAILED", request_id: "fal_mock_1", error: "mock inference failed" });
          }
          return json(response, 200, { status: "COMPLETED", request_id: "fal_mock_1", response_url: url.href.replace(/\/status$/, ""), metrics: { inference_time: 0.01 } });
        }
        if (/\/requests\/fal_mock_1(?:\/response)?$/.test(url.pathname) && request.method === "GET") {
          const errors = captureAuxiliary();
          if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
          if (selection.scenario === "async_artifact_unavailable") {
            return json(response, 404, { detail: "Result artifact is unavailable", error_type: "not_found" });
          }
          return json(response, 200, rewriteMockUrls(
            contract.async_result_fixture ?? {},
            request.headers.host ?? "127.0.0.1",
            url.pathname.replace(/^\//, ""),
          ));
        }
        if (/\/requests\/fal_mock_1\/cancel$/.test(url.pathname) && request.method === "PUT") {
          const errors = captureAuxiliary();
          if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
          return json(response, 202, { status: "CANCELLATION_REQUESTED" });
        }
      }
      if (contract.async_protocol === "minimax_video" && url.pathname === "/v1/query/video_generation" && request.method === "GET") {
        const errors = captureAuxiliary();
        if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
        return json(response, 200, selection.scenario === "async_failed"
          ? { task_id: "minimax_video_mock_1", status: "Fail", base_resp: { status_code: 1024, status_msg: "internal error" } }
          : selection.scenario === "async_poll_timeout"
          ? { task_id: "minimax_video_mock_1", status: "Processing", base_resp: { status_code: 0, status_msg: "success" } }
          : { task_id: "minimax_video_mock_1", status: "Success", file_id: "minimax_file_mock_1", base_resp: { status_code: 0, status_msg: "success" } });
      }
      if (contract.async_protocol === "google_lro" && url.pathname === "/v1beta/operations/gemini_mock_1" && request.method === "GET") {
        const errors = captureAuxiliary();
        if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
        if (selection.scenario === "async_failed") {
          return json(response, 200, { name: "operations/gemini_mock_1", done: true, error: { code: 13, message: "Internal error", status: "INTERNAL" } });
        }
        if (selection.scenario === "async_poll_timeout") {
          return json(response, 200, { name: "operations/gemini_mock_1", done: false });
        }
        return json(response, 200, {
          name: "operations/gemini_mock_1",
          done: true,
          response: {
            generateVideoResponse: { generatedSamples: [{ video: { uri: selection.scenario === "async_artifact_unavailable" ? `http://${request.headers.host}/artifacts/unavailable.mp4` : `http://${request.headers.host}/artifacts/result.mp4` } }] },
          },
        });
      }
      if (contract.async_protocol === "openai_video" && url.pathname === "/v1/videos/video_mock_1" && request.method === "GET") {
        const errors = captureAuxiliary();
        if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
        return json(response, 200, {
          id: "video_mock_1",
          object: "video",
          model: "mock-model",
          status: selection.scenario === "async_failed" ? "failed" : selection.scenario === "async_poll_timeout" ? "in_progress" : "completed",
          progress: 100,
          created_at: 1770000000,
          completed_at: 1770000001,
        });
      }
      if (contract.async_protocol === "openai_video" && url.pathname === "/v1/videos/video_mock_1/content" && request.method === "GET") {
        const errors = captureAuxiliary();
        if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
        if (selection.scenario === "async_artifact_unavailable") return json(response, 404, { error: { type: "not_found", message: "Video content unavailable" } });
        response.writeHead(200, { "content-type": "video/mp4" });
        response.end(Buffer.from("mock-video"));
        return;
      }
      if (url.pathname === "/v1/files/retrieve" && request.method === "GET") {
        if (contract.async_protocol === "minimax_video") {
          const errors = captureAuxiliary();
          if (errors.length > 0) return json(response, 400, { type: "t15_mock_contract_violation", errors });
        }
        return json(response, 200, { file: { download_url: selection.scenario === "async_artifact_unavailable" ? `http://${request.headers.host}/artifacts/unavailable.mp4` : `http://${request.headers.host}/artifacts/result.mp4` }, base_resp: { status_code: 0, status_msg: "success" } });
      }
      if (url.pathname.startsWith("/artifacts/") && request.method === "GET") {
        if (url.pathname.includes("unavailable")) return json(response, 404, { error: "artifact unavailable" });
        const mime = url.pathname.endsWith(".png") ? "image/png" : url.pathname.endsWith(".wav") ? "audio/wav" : "video/mp4";
        response.writeHead(200, { "content-type": mime });
        response.end(Buffer.from("mock-artifact"));
        return;
      }

      const bytes = await bodyBytes(request);
      const parsedBody = parseRequestBody(request, bytes);
      const captured: CapturedProviderRequest = {
        method: request.method ?? "",
        pathname: url.pathname,
        query: url.searchParams,
        headers: new Headers(request.headers as Record<string, string>),
        body: parsedBody,
      };
      const validationErrors = validateProviderRequest(contract, captured);
      requests.push({
        received_at: new Date().toISOString(),
        selection,
        method: captured.method,
        pathname: captured.pathname,
        query: Object.fromEntries(captured.query),
        headers: safeHeaders(request.headers),
        body: parsedBody,
        validation_errors: validationErrors,
      });
      if (validationErrors.length > 0) {
        return json(response, 400, { type: "t15_mock_contract_violation", errors: validationErrors });
      }
      const errorFixture = catalog.error_fixtures[selection.provider_driver]
        .find((fixture) => fixture.scenario === selection?.scenario);
      if (errorFixture) return json(response, errorFixture.status, errorFixture.body, errorFixture.headers);
      if (["stream_success", "stream_interrupted"].includes(selection.scenario)) {
        if ((parsedBody as Record<string, unknown>)?.stream !== true) {
          return json(response, 400, { type: "t15_mock_contract_violation", errors: ["stream must equal true"] });
        }
        response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
        if (selection.scenario === "stream_interrupted") {
          response.end(streamFixture(contract).split("\n\n").slice(0, 2).join("\n\n"));
          return;
        }
        response.end(streamFixture(contract));
        return;
      }
      if (selection.scenario === "malformed_response") {
        response.writeHead(200, { "content-type": "application/json" });
        response.end("{\"malformed\":");
        return;
      }
      if (selection.scenario === "wrong_content_type") {
        response.writeHead(200, { "content-type": "text/plain" });
        response.end(JSON.stringify(contract.success_fixture ?? {}));
        return;
      }
      if (selection.scenario === "missing_required_response_field") {
        return json(response, 200, {});
      }
      if (contract.success_fixture_base64) {
        response.writeHead(200, { "content-type": contract.success_content_type ?? "application/octet-stream" });
        response.end(Buffer.from(contract.success_fixture_base64, "base64"));
        return;
      }
      const rawFixture = structuredClone(contract.success_fixture ?? {}) as Record<string, unknown>;
      if (selection.provider_driver === "openai" &&
          contract.id === "openai.responses.v1" &&
          ["image.txt2img", "image.img2img"].includes(selection.api_type ?? "")) {
        rawFixture.output = [{
          type: "image_generation_call",
          id: "ig_mock_1",
          status: "completed",
          result: "aW1hZ2U=",
          output_format: "png",
        }];
      }
      if (selection.provider_driver === "google-gemini" &&
          contract.id === "gemini.interactions.v1beta" &&
          Array.isArray(rawFixture.outputs)) {
        const structuredText: Record<string, string> = {
          "vision.ocr": JSON.stringify({ text: "BUCKYOS-AICC-4827", pages: [{ page_index: 0, width: 1, height: 1, blocks: [] }] }),
          "vision.detect": JSON.stringify({ detections: [{ label: "marker", score: 1, bbox: { format: "xywh", unit: "relative", x: 0, y: 0, width: 1, height: 1 } }] }),
          "vision.segment": JSON.stringify({ masks: [{ id: "mask-1", score: 1, mask: { format: "polygon", points: [[0, 0], [1, 0], [1, 1]] } }] }),
        };
        const text = selection.api_type ? structuredText[selection.api_type] : undefined;
        if (text) rawFixture.outputs = [{ type: "text", text }];
      }
      const fixture = rewriteMockUrls(
        rawFixture,
        request.headers.host ?? "127.0.0.1",
        url.pathname.replace(/^\//, ""),
      );
      return json(response, 200, fixture);
    } catch (error) {
      return json(response, 500, { error: String(error) });
    }
  };
}

function port(args: string[]): number {
  const index = args.indexOf("--port");
  const value = index >= 0 ? Number(args[index + 1]) : 18081;
  if (!Number.isInteger(value) || value < 1 || value > 65535) throw new Error("--port must be 1..65535");
  return value;
}

if (process.argv[1] && resolve(fileURLToPath(import.meta.url)) === resolve(process.argv[1])) {
  const catalog = await loadProviderProtocolCatalog();
  const listenPort = port(process.argv.slice(2));
  const handler = createT15MockHandler(catalog);
  const server = createServer((request, response) => void handler(request, response));
  server.listen(listenPort, "127.0.0.1", () => {
    process.stdout.write(`T1.5 Provider protocol mock listening on http://127.0.0.1:${listenPort}\n`);
  });
}
