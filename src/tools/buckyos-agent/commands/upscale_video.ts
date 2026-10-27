// upscale_video — video.upscale
// Doc: aicc_agent_cli_tools.md §6.5

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  DELIVERY_HELP,
  flagBool,
  flagFloat,
  flagInt,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { pickVideo } from "../lib/video.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import type { TypedRequestMap } from "../lib/aicc.ts";
import {
  resolveInputResource,
  deliverArtifact,
  deliveredSummary,
} from "../lib/io.ts";
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

const TOOL = "upscale_video";
const METHOD = "video.upscale";

export const HELP = `Usage: upscale_video <input_video> [output_video] [options]

Options:
  --target-resolution <1080p|4k>
  --denoise
  --sharpen <0.0-1.0>
  --fps <int>
${COMMON_OPTIONS_HELP}${DELIVERY_HELP}`;

const TARGET = new Set(["1080p", "4k"]);

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 1) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, {
        error: "missing positional",
      }),
      EXIT_ARG_ERROR,
    );
  }
  const [srcVideo, outputPath] = parsed.positional;

  const request = {
    video: undefined,
    target_resolution: "1080p",
  } as unknown as TypedRequestMap[typeof METHOD];
  let inputResource;
  try {
    const target = requireString(parsed.flags, "target-resolution");
    if (target !== undefined) {
      if (!TARGET.has(target)) {
        throw new ArgError(`--target-resolution invalid: ${target}`);
      }
      request.target_resolution = target;
    }
    if (flagBool(parsed.flags, "denoise")) request.denoise = true;
    const sharpen = flagFloat(parsed.flags, "sharpen");
    if (sharpen !== undefined) {
      if (sharpen < 0 || sharpen > 1) {
        throw new ArgError(`--sharpen must be in [0,1]`);
      }
      request.sharpen = sharpen;
    }
    const fps = flagInt(parsed.flags, "fps");
    if (fps !== undefined) request.output = { fps };
    inputResource = await resolveInputResource(srcVideo, "video/*");
    request.video = inputResource;
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
      defaultMime: "video/mp4",
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
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
