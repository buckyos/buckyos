// edit_video — video.edit
// Doc: aicc_agent_cli_tools.md §6.2, protocol doc/aicc/video_generation.md
// 改写已有视频（换风格、换元素）；结构与 gen_video 一致，只是输入是一段视频。

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  DELIVERY_HELP,
  parseArgvOrExit,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import type { TypedRequestMap } from "../lib/aicc.ts";
import {
  resolveInputResource,
  deliverArtifact,
  deliveredSummary,
} from "../lib/io.ts";
import { pickVideo, VIDEO_OUTPUT_HELP, videoOutputSpec } from "../lib/video.ts";
import {
  bailAiccError,
  bailAiccFailed,
  bailIoError,
  bailNoArtifact,
  bailRuntimeError,
  emitAndExit,
  errorResult,
  EXIT_ARG_ERROR,
  EXIT_SUCCESS,
  successResult,
} from "../lib/result.ts";

const TOOL = "edit_video";
const METHOD = "video.edit";

export const HELP =
  `Usage: edit_video <input_video> <prompt> [output_video] [options]

Rewrites an existing video as the prompt says. To make a new video that
follows a reference clip, use gen_video --ref-video instead.

${VIDEO_OUTPUT_HELP}
${COMMON_OPTIONS_HELP}${DELIVERY_HELP}`;

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, {
        error: "missing positional",
      }),
      EXIT_ARG_ERROR,
    );
  }
  const [srcVideo, prompt, outputPath] = parsed.positional;

  let request: TypedRequestMap[typeof METHOD];
  try {
    const output = videoOutputSpec(parsed.flags);
    request = {
      video: await resolveInputResource(srcVideo, "video/*"),
      prompt,
      ...(output ? { output } : {}),
    };
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    bailIoError(TOOL, undefined, err);
  }

  let runtime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }
  let call;
  try {
    call = await callAicc(runtime, {
      method: METHOD,
      model: parsed.common.model ?? METHOD,
      request,
      ...commonPolicyOptions(parsed.common),
    });
  } catch (err) {
    bailAiccError(TOOL, METHOD, err);
  }
  if (call.status === "failed" || !call.summary) {
    bailAiccFailed(TOOL, METHOD, call.taskId, describeFailure(call));
  }

  let artifact;
  try {
    artifact = pickVideo(call.summary);
  } catch (err) {
    bailAiccFailed(TOOL, METHOD, call.taskId, (err as Error).message);
  }
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try {
    saved = await deliverArtifact(runtime, call.summary, artifact, ndmProxy, {
      tool: TOOL,
      path: outputPath,
      defaultMime: artifact.mime ?? "video/mp4",
    });
  } catch (err) {
    bailIoError(TOOL, call.taskId, err);
  }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, deliveredSummary(TOOL, [saved]), {
      method: METHOD,
      capability: "video",
      task_id: call.taskId,
      files: [saved],
      video: artifact.metadata ?? null,
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
