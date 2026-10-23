import {
  AiccWaitTimeout,
  callAicc,
  describeFailure,
  resumeAiccTask,
  textToImage,
} from "./aicc.ts";
import { aiResponseArtifacts, aiResponseData } from "./types.ts";
import {
  pickVideo,
  takeRepeatedFlags,
  VIDEO_CONDITION_FLAGS,
  videoConditions,
} from "./video.ts";
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
    videoEdit: (request: unknown) => {
      calls.push(["video.edit", request]);
      return Promise.resolve({
        task_id: "task-1",
        status: "succeeded",
        videos: [{
          resource: { kind: "url", url: "https://example.invalid/video.mp4" },
          media_type: "video/mp4",
          provider_metadata: { duration_seconds: 4 },
        }],
      });
    },
  });

  const result = await callAicc(runtime, {
    method: "video.edit",
    model: "video.edit",
    request: {
      video: { kind: "named_object", obj_id: "chunk:video" },
      prompt: "make it snow",
      output: { duration_seconds: 4 },
    },
    maxCostUsd: 1.25,
    traceId: "trace-1",
  });

  assertEquals(calls[0], ["route.resolve", {
    trace_id: "trace-1",
    api_type: "video.edit",
    logical_model: "video.edit",
    requirements: {},
    disable: {},
    policy: {
      profile: "balanced",
      allow_fallback: true,
      runtime_failover: true,
      max_cost: { amount: 1.25, currency: "USD" },
    },
  }]);
  assertEquals(calls[1], ["video.edit", {
    video: { kind: "named_object", obj_id: "chunk:video" },
    prompt: "make it snow",
    output: { duration_seconds: 4 },
    exact_model: "vendor-model@provider-main",
    trace_id: "trace-1",
  }]);
  assertEquals(result.status, "succeeded");
  const [video] = aiResponseArtifacts(result.summary!);
  assertEquals([video.name, video.mime], ["video_1", "video/mp4"]);
  assertEquals(video.metadata, {
    media_type: "video/mp4",
    provider_metadata: { duration_seconds: 4 },
  });
});

Deno.test("a logical video.generate is routed by the AICC helper with its conditions", async () => {
  const calls: Array<[string, unknown]> = [];
  const runtime = runtimeWithClients({
    routeResolve: () => {
      throw new Error("conditions must reach routing, not be left behind");
    },
    helperVideoGenerate: (request: unknown) => {
      calls.push(["helper.video_generate", request]);
      return Promise.resolve({
        task_id: "task-video",
        status: "succeeded",
        videos: [{
          resource: { kind: "named_object", obj_id: "chunk:video" },
          media_type: "video/mp4",
          provider_metadata: {},
          measured_metadata: { duration_seconds: 6.04, has_audio: true },
          last_frame: { kind: "named_object", obj_id: "chunk:frame" },
        }],
      });
    },
  });
  const conditions = [{
    type: "keyframe" as const,
    id: "first",
    position: "start" as const,
    resource: { kind: "named_object" as const, obj_id: "chunk:image" },
  }];
  const result = await callAicc(runtime, {
    method: "video.generate",
    model: "video.generate",
    request: { prompt: "@{first} comes alive", conditions },
    idempotencyKey: "shot-3",
  });
  assertEquals(calls, [["helper.video_generate", {
    prompt: "@{first} comes alive",
    conditions,
    logical_model: "video.generate",
    requirements: {},
    disable: {},
    policy: { profile: "balanced", allow_fallback: true, runtime_failover: true },
    idempotency_key: "shot-3",
  }]]);
  assertEquals(
    aiResponseArtifacts(result.summary!).map((artifact) => artifact.name),
    ["video_1", "last_frame_1"],
  );
  assertEquals(aiResponseData(result.summary!), {
    videos: [{
      media_type: "video/mp4",
      provider_metadata: {},
      measured_metadata: { duration_seconds: 6.04, has_audio: true },
    }],
  });
});

Deno.test("pickVideo refuses a result with more videos than requested", () => {
  const video = (id: string) => ({
    resource: { kind: "named_object" as const, obj_id: id },
    provider_metadata: {},
  });
  const response = {
    task_id: "t",
    status: "succeeded" as const,
    videos: [video("chunk:a"), video("chunk:b")],
  };
  let message = "";
  try {
    pickVideo(response);
  } catch (err) {
    message = (err as Error).message;
  }
  assertEquals(message, "the result holds 2 videos but one was requested");
  assertEquals(
    pickVideo({ ...response, videos: [video("chunk:a")] })?.name,
    "video_1",
  );
});

Deno.test("video condition flags keep their order and attach clips and audio", async () => {
  const { rest, values } = takeRepeatedFlags([
    "a prompt",
    "--ref-image",
    "hero=named_object:chunk:hero",
    "--duration",
    "6",
    "--ref-video",
    "motion=https://example.invalid/m.mp4",
    "--ref-image",
    "prop=named_object:chunk:prop",
    "--clip",
    "motion=0-3",
    "--with-audio",
    "motion",
  ], VIDEO_CONDITION_FLAGS);
  assertEquals(rest, ["a prompt", "--duration", "6"]);
  const conditions = await videoConditions(values, {
    start: "named_object:chunk:first",
  });
  assertEquals(conditions.map((condition) => condition.id), [
    "first",
    "hero",
    "prop",
    "motion",
  ]);
  assertEquals(conditions[3], {
    type: "reference_video",
    id: "motion",
    resource: { kind: "url", url: "https://example.invalid/m.mp4", mime_hint: "video/*" },
    time_range: { start_seconds: 0, end_seconds: 3 },
    include_audio: true,
  });
});

Deno.test("callAicc preserves exact model debug override", async () => {
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
    error: { code: "provider_error" as const, message: "video failed" },
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


Deno.test("decision calls route with actual mixed question requirements and preserve stricter caller limits", async () => {
  const calls: Array<[string, any]> = [];
  const runtime = runtimeWithClients({
    routeResolve: (request: unknown) => { calls.push(["route.resolve", request]); return Promise.resolve({selected_exact_model:"jev-1.13.0@typesafe-main"}); },
    decisionEvaluate: (request: unknown) => { calls.push(["decision.evaluate",request]); return Promise.resolve({task_id:"decision-task",status:"succeeded",answers:[]}); },
  });
  await callAicc(runtime,{method:"decision.evaluate",model:"decision",requirements:{decision:{max_options:20}},request:{state:{text:"urgent"},questions:[{id:"a",type:"choice",instructions:"Select",options:[{id:"x",description:"Example"}]},{id:"b",type:"boolean",instructions:"Urgent?"}]}});
  assertEquals(calls[0][1].api_type,"decision");
  assertEquals(calls[0][1].requirements.decision.question_types,["choice","boolean"]);
  assertEquals(calls[0][1].requirements.decision.structured_state,true);
  assertEquals(calls[0][1].requirements.decision.max_options,20);
  assertEquals(calls[1][1].exact_model,"jev-1.13.0@typesafe-main");
});


function taskSequence(records: Array<Record<string, unknown>>) {
  let index = 0;
  return {
    getTask: (taskId: string) => {
      const record = records[Math.min(index, records.length - 1)];
      index += 1;
      return Promise.resolve({
        task_id: taskId,
        name: "AICC video.generate",
        schema_id: "aicc.compute/v1",
        ...record,
      });
    },
  };
}

Deno.test("resumeAiccTask waits for a running AICC task and returns its output", async () => {
  const runtime = runtimeWithClients({}, taskSequence([
    { phase: "Running" },
    {
      phase: "Terminal",
      outcome: "Succeeded",
      result: {
        result: {
          output: {
            artifacts: [{
              name: "video",
              resource: { kind: "named_object", obj_id: "chunk:video" },
              mime: "video/mp4",
            }],
          },
        },
      },
    },
  ]));
  const result = await resumeAiccTask(runtime, "t-1", 10_000);
  assertEquals(result.status, "succeeded");
  assertEquals(result.method, "video.generate");
  assertEquals(aiResponseArtifacts(result.summary!).length, 1);
});

Deno.test("resumeAiccTask without wait reports a running task", async () => {
  const runtime = runtimeWithClients({}, taskSequence([{ phase: "Running" }]));
  const result = await resumeAiccTask(runtime, "t-2", 0);
  assertEquals([result.status, result.phase, result.summary], [
    "running",
    "Running",
    null,
  ]);
});

Deno.test("resumeAiccTask reports a failed task with its error", async () => {
  const runtime = runtimeWithClients({}, taskSequence([{
    phase: "Terminal",
    outcome: "Failed",
    error: { code: "provider_error", message: "quota exhausted" },
  }]));
  const result = await resumeAiccTask(runtime, "t-3", 0);
  assertEquals(result.status, "failed");
  assertEquals(
    describeFailure(result),
    JSON.stringify({ code: "provider_error", message: "quota exhausted" }),
  );
});

Deno.test("resumeAiccTask refuses a task that is not an AICC task", async () => {
  const runtime = runtimeWithClients({}, taskSequence([{
    phase: "Running",
    schema_id: "tool.shell/v1",
  }]));
  let message = "";
  try {
    await resumeAiccTask(runtime, "t-4", 0);
  } catch (err) {
    message = (err as Error).message;
  }
  assertEquals(message.includes("not an AICC task"), true);
});

Deno.test("callAicc wait timeout names the task that keeps running", async () => {
  const runtime = runtimeWithClients({
    videoGenerate: () =>
      Promise.resolve({ task_id: "t-5", status: "running" }),
  }, taskSequence([{ phase: "Running" }]));
  let error: unknown;
  try {
    await callAicc(runtime, {
      method: "video.generate",
      model: "veo@provider-main",
      request: { prompt: "a wave" },
      waitTimeoutMs: 1,
    });
  } catch (err) {
    error = err;
  }
  assertEquals(error instanceof AiccWaitTimeout, true);
  assertEquals((error as AiccWaitTimeout).taskId, "t-5");
});
