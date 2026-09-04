import { callAicc, describeFailure, textToImage } from "./aicc.ts";
import { aiResponseArtifacts } from "./types.ts";
import type { AiccRuntime } from "./runtime.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  const stable = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(stable);
    if (value !== null && typeof value === "object") {
      return Object.fromEntries(
        Object.entries(value).sort(([left], [right]) =>
          left.localeCompare(right)
        )
          .map(([key, entry]) => [key, stable(entry)]),
      );
    }
    return value;
  };
  if (JSON.stringify(stable(actual)) !== JSON.stringify(stable(expected))) {
    throw new Error(
      `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

function runtimeWithClients(
  aicc: Record<string, unknown>,
  taskMgr?: Record<string, unknown>,
): AiccRuntime {
  return {
    buckyos: {
      getAiccClient: () => aicc,
      getTaskManagerClient: () => taskMgr ?? {},
    },
    userId: "devtest",
    ownerUserId: "devtest",
    zoneHost: "test.buckyos.io",
    appId: "buckyos_jarvis",
  } as unknown as AiccRuntime;
}

Deno.test("callAicc resolves a logical model then sends a canonical typed request", async () => {
  const calls: Array<[string, unknown]> = [];
  const runtime = runtimeWithClients({
    routeResolve: (request: unknown) => {
      calls.push(["route.resolve", request]);
      return Promise.resolve({
        selected_exact_model: "vendor-model@provider-main",
      });
    },
    videoImageToVideo: (request: unknown) => {
      calls.push(["video.img2video", request]);
      return Promise.resolve({
        task_id: "task-1",
        status: "succeeded",
        video: {
          kind: "url",
          url: "https://example.invalid/video.mp4",
          mime_hint: "video/mp4",
        },
      });
    },
  });

  const result = await callAicc(runtime, {
    method: "video.img2video",
    model: "video.img2video",
    request: {
      image: { kind: "named_object", obj_id: "chunk:image" },
      prompt: "animate",
      duration_seconds: 4,
    },
    maxCostUsd: 1.25,
    traceId: "trace-1",
  });

  assertEquals(calls[0], ["route.resolve", {
    trace_id: "trace-1",
    api_type: "video.img2video",
    logical_model: "video.img2video",
    requirements: {},
    disable: {},
    policy: {
      profile: "balanced",
      allow_fallback: true,
      runtime_failover: true,
      max_cost: { amount: 1.25, currency: "USD" },
    },
  }]);
  assertEquals(calls[1], ["video.img2video", {
    image: { kind: "named_object", obj_id: "chunk:image" },
    prompt: "animate",
    duration_seconds: 4,
    exact_model: "vendor-model@provider-main",
    trace_id: "trace-1",
  }]);
  assertEquals(result.status, "succeeded");
  assertEquals(aiResponseArtifacts(result.summary!).length, 1);
});

Deno.test("callAicc preserves TaskMgr 2.0 async completion", async () => {
  let routeCalled = false;
  const runtime = runtimeWithClients({
    routeResolve: () => {
      routeCalled = true;
      throw new Error("exact models must bypass route.resolve");
    },
    videoUpscale: () =>
      Promise.resolve({ task_id: "task-async", status: "running" }),
  }, {
    getTask: (taskId: string) =>
      Promise.resolve({
        task_id: taskId,
        phase: "Terminal",
        outcome: "Succeeded",
        result: {
          result: {
            output: {
              value: {},
              artifacts: [{
                name: "video",
                resource: { kind: "named_object", obj_id: "chunk:video" },
                mime: "video/mp4",
              }],
              usage: { request_units: 1 },
            },
          },
        },
      }),
  });

  const result = await callAicc(runtime, {
    method: "video.upscale",
    model: "provider-video@provider-main",
    request: {
      video: { kind: "named_object", obj_id: "chunk:source" },
      target_resolution: "1080p",
    },
  });

  assertEquals(routeCalled, false);
  assertEquals(result.status, "succeeded");
  assertEquals(aiResponseArtifacts(result.summary!).length, 1);
});

Deno.test("describeFailure reads canonical immediate errors", () => {
  const response = {
    task_id: "task-failed",
    status: "failed" as const,
    error: { code: "provider_error", message: "video failed" },
  };
  assertEquals(
    describeFailure({
      taskId: response.task_id,
      status: "failed",
      summary: response,
      rawResponse: response,
    }),
    JSON.stringify(response.error),
  );
});

Deno.test("textToImage uses the canonical helper request", async () => {
  let params: unknown;
  const runtime = runtimeWithClients({
    helperTextToImage: (request: unknown) => {
      params = request;
      return Promise.resolve({
        task_id: "image-task",
        status: "succeeded",
        images: [{
          kind: "url",
          url: "https://example.invalid/image.png",
          mime_hint: "image/png",
        }],
      });
    },
  });

  const result = await textToImage(runtime, {
    model: "image.txt2img",
    request: { prompt: "a fox", quality: "high" },
    traceId: "trace-image",
  });

  assertEquals(params, {
    logical_model: "image.txt2img",
    requirements: {},
    disable: {},
    trace_id: "trace-image",
    policy: {
      profile: "balanced",
      allow_fallback: true,
      runtime_failover: true,
    },
    prompt: "a fox",
    quality: "high",
  });
  assertEquals(result.status, "succeeded");
  assertEquals(aiResponseArtifacts(result.summary!).length, 1);
});
