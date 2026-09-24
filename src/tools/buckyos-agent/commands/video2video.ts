// video2video — video.video2video
// Doc: aicc_agent_cli_tools.md §6.3

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  flagBool,
  flagFloat,
  parseArgvOrExit,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import type { TypedRequestMap } from "../lib/aicc.ts";
import {
  pickArtifact,
  resolveInputResource,
  saveArtifactToPath,
  suffixPathByMime,
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

const TOOL = "video2video";
const METHOD = "video.video2video";

export const HELP =
  `Usage: video2video <input_video> <prompt> <output_video> [options]

Options:
  --preserve-motion
  --start <seconds>
  --end <seconds>
${COMMON_OPTIONS_HELP}`;

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 3) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, {
        error: "missing positional",
      }),
      EXIT_ARG_ERROR,
    );
  }
  const [srcVideo, prompt, outputPath] = parsed.positional;

  const request = {
    video: undefined,
    prompt,
  } as unknown as TypedRequestMap[typeof METHOD];
  let inputResource;
  try {
    if (flagBool(parsed.flags, "preserve-motion")) {
      request.preserve_motion = true;
    }
    const start = flagFloat(parsed.flags, "start");
    const end = flagFloat(parsed.flags, "end");
    if ((start === undefined) !== (end === undefined)) {
      throw new ArgError("--start and --end must be supplied together");
    }
    if (start !== undefined && end !== undefined && end < start) {
      throw new ArgError("--end must be greater than or equal to --start");
    }
    if (start !== undefined && end !== undefined) {
      request.time_range = { start_seconds: start, end_seconds: end };
    }
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

  const artifact = pickArtifact(call.summary, "video");
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try {
    saved = await saveArtifactToPath(
      artifact,
      suffixPathByMime(outputPath, "video/mp4"),
      ndmProxy,
    );
  } catch (err) {
    bailIoError(TOOL, call.taskId, err);
  }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${saved.path}`, {
      method: METHOD,
      capability: "video",
      task_id: call.taskId,
      files: [{
        path: saved.path,
        mime: saved.mime ?? null,
        bytes: saved.bytes,
        source_kind: saved.source_kind,
      }],
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
