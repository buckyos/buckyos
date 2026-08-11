import { callAicc, describeFailure } from "./aicc.ts";
import type { AiccRuntime } from "./runtime.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

function runtimeWithTask(task: Record<string, unknown>): AiccRuntime {
  const aiccRpc = {
    call: () => Promise.resolve({ task_id: "t-aicc-1", status: "running" }),
  };
  const taskMgr = {
    call: (method: string, params: Record<string, unknown>) => {
      if (method === "get_task") {
        assertEquals(params, { task_id: "t-aicc-1" });
        return Promise.resolve(task);
      }
      throw new Error(`unexpected method: ${method}`);
    },
  };
  return {
    buckyos: {
      getServiceRpcClient: (service: string) =>
        service === "aicc" ? aiccRpc : taskMgr,
    },
    userId: "devtest",
    ownerUserId: "devtest",
    zoneHost: "test.buckyos.io",
    appId: "buckyos_jarvis",
  };
}

Deno.test("callAicc resolves a completed beta2.2 task", async () => {
  const output = {
    message: {
      role: "assistant",
      content: [{ type: "text", text: "done" }],
    },
  };
  const task = {
    task_id: "t-aicc-1",
    phase: "Terminal",
    outcome: "Succeeded",
    result: {
      request: { external_task_id: "aicc-1" },
      result: { output },
    },
  };

  const result = await callAicc(runtimeWithTask(task), {
    capability: "video",
    method: "video.img2video",
  });

  assertEquals(result.status, "succeeded");
  assertEquals(result.summary, output);
});

Deno.test("describeFailure reads a beta2.2 task error", async () => {
  const error = { code: "provider_failed", message: "video failed" };
  const task = {
    task_id: "t-aicc-1",
    phase: "Terminal",
    outcome: "Failed",
    result: {
      request: { external_task_id: "aicc-1" },
      error,
    },
  };

  const result = await callAicc(runtimeWithTask(task), {
    capability: "video",
    method: "video.img2video",
  });

  assertEquals(result.status, "failed");
  assertEquals(describeFailure(result), JSON.stringify(error));
});
