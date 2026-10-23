// resume_aicc_task — pick up an AICC generation task by its task id
// 详细的 6 步配方注释见 gen_image.ts。这里没有新请求：(2) 换成"按 task id 读任务"，
// (4) 换成 resumeAiccTask()，不会再次提交、不会再次计费。
//
// 用途：媒体命令被 shell 超时/断连打断、或 --timeout 到了（exit 4，detail.task_id），
// 服务端任务仍在跑；用 AICC task id 续等并下载产物，而不是重新生成。

import { ndm_proxy } from "buckyos";

import { bailArgError, flagBool, parseArgvOrExit } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { describeFailure, resumeAiccTask } from "../lib/aicc.ts";
import { saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError,
  bailAiccFailed,
  bailIoError,
  bailRuntimeError,
  emitAndExit,
  errorResult,
  EXIT_ARG_ERROR,
  EXIT_SUCCESS,
  EXIT_TIMEOUT,
  successResult,
} from "../lib/result.ts";
import { aiResponseArtifacts, aiResponseText } from "../lib/types.ts";
import type { AiArtifact, JsonValue } from "../lib/types.ts";

const TOOL = "resume_aicc_task";
const DEFAULT_WAIT_MS = 900_000;

export const HELP =
  `Usage: resume_aicc_task <aicc_task_id> [output_path] [options]

Picks up an AICC task (the task_id in a media command's result, its wait_timeout
error, or its __BUCKYOS_AGENT_PROGRESS__ stderr lines) without submitting it
again: waits while it runs, then saves its result files.

Positional:
  <aicc_task_id>   task-manager id of the AICC task, e.g. t-0123abcd...
  [output_path]    where to save the result; several results are saved as
                   <name>-<n>.<ext>. Omit it to only report state and results.

Options:
  --no-wait            report the current state at once
  --timeout <seconds>  max wait (default AICC_DEFAULT_TIMEOUT or 900)

Exit codes: 0 done, 1 bad arguments, 2/3 the task failed, 4 still running
(not a failure: run again later), 5 saving failed.`;

function numberedPath(
  path: string,
  index: number,
  mime?: string | null,
): string {
  const withExt = suffixPathByMime(path, mime ?? undefined);
  const dot = withExt.lastIndexOf(".");
  const slash = withExt.lastIndexOf("/");
  return dot > slash
    ? `${withExt.slice(0, dot)}-${index}${withExt.slice(dot)}`
    : `${withExt}-${index}`;
}

function artifactMime(a: AiArtifact): string | null {
  return a.mime ?? a.resource.mime ?? a.resource.mime_hint ?? null;
}

export async function run(argv: string[]): Promise<never> {
  // ── 1. 解析参数 ────────────────────────────────────────────────────────
  const parsed = parseArgvOrExit(TOOL, HELP, argv, ["no-wait"]);
  const [taskId, outputPath] = parsed.positional;
  if (!taskId) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, {
        error: "missing <aicc_task_id>",
      }),
      EXIT_ARG_ERROR,
    );
  }
  if (parsed.positional.length > 2) {
    bailArgError(TOOL, "expected <aicc_task_id> [output_path]");
  }

  // ── 2. 没有新请求：等待上限 ─────────────────────────────────────────────
  const envWait = Number(Deno.env.get("AICC_DEFAULT_TIMEOUT"));
  const waitMs = flagBool(parsed.flags, "no-wait")
    ? 0
    : parsed.common.timeoutMs ??
      (Number.isFinite(envWait) && envWait > 0 ? envWait : DEFAULT_WAIT_MS);

  // ── 3. 拿到 BuckyOS 运行时 ────────────────────────────────────────────
  let runtime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }

  // ── 4. 按 task id 续等（不重新提交） ──────────────────────────────────
  let task;
  try {
    task = await resumeAiccTask(runtime, taskId, waitMs);
  } catch (err) {
    bailAiccError(TOOL, "task.get", err);
  }
  const method = task.method ?? "aicc";
  if (task.status === "running") {
    emitAndExit(
      errorResult(
        TOOL,
        `${TOOL} => still_running`,
        `AICC task ${taskId} is still running (phase ${task.phase}); it was not resubmitted. Run this command again later.`,
        { method, task_id: taskId, phase: task.phase, still_running: true },
      ),
      EXIT_TIMEOUT,
    );
  }
  if (task.status === "failed" || !task.summary) {
    bailAiccFailed(TOOL, method, taskId, describeFailure(task));
  }

  // ── 5. 把产物拉到本地 ─────────────────────────────────────────────────
  const artifacts = aiResponseArtifacts(task.summary);
  const text = aiResponseText(task.summary);
  const files: Array<
    {
      path: string;
      mime: string | null;
      bytes: number;
      source_kind: string;
      obj_id: string | null;
    }
  > = [];
  if (outputPath) {
    // deno-lint-ignore no-explicit-any
    const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
    try {
      for (let i = 0; i < artifacts.length; i += 1) {
        const a = artifacts[i];
        const dest = artifacts.length === 1
          ? suffixPathByMime(outputPath, artifactMime(a) ?? undefined)
          : numberedPath(outputPath, i + 1, artifactMime(a));
        const saved = await saveArtifactToPath(a, dest, ndmProxy);
        files.push({
          path: saved.path,
          mime: saved.mime ?? null,
          bytes: saved.bytes,
          source_kind: saved.source_kind,
          obj_id: saved.obj_id ?? null,
        });
      }
    } catch (err) {
      bailIoError(TOOL, taskId, err);
    }
  }

  // ── 6. emit AgentToolResult ───────────────────────────────────────────
  const listed = artifacts.map((a) => ({
    name: a.name,
    mime: artifactMime(a),
    kind: a.resource.kind,
    obj_id: a.resource.kind === "named_object" ? a.resource.obj_id : null,
    metadata: a.metadata ?? null,
  }));
  const summary = files.length
    ? `${TOOL} wrote ${files.map((f) => f.path).join(", ")}`
    : artifacts.length
    ? `AICC task ${taskId} succeeded with ${artifacts.length} result file(s); pass an output path to save them`
    : `AICC task ${taskId} succeeded`;
  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      summary,
      {
        method,
        task_id: taskId,
        files,
        artifacts: listed,
        cost: (task.summary as unknown as { cost?: JsonValue }).cost ?? null,
      },
      text || undefined,
    ),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
