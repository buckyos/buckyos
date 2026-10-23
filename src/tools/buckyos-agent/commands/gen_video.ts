// gen_video — video.generate (helper.video_generate for a logical model)
// Doc: aicc_agent_cli_tools.md §6.1, protocol doc/aicc/video_generation.md
// 单 artifact 输出配方。文本、首尾帧、参考图/视频/音频都是同一个请求的
// conditions；视频类调用通常阻塞较久——靠 callAicc 内部对 task-manager
// 轮询即可，CLI 不需要做额外处理。

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  flagInt,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import type { TypedRequestMap } from "../lib/aicc.ts";
import { saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  pickVideo,
  takeRepeatedFlags,
  VIDEO_CONDITION_FLAGS,
  VIDEO_OUTPUT_HELP,
  videoConditions,
  videoOutputSpec,
} from "../lib/video.ts";
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

const TOOL = "gen_video";
const METHOD = "video.generate";

export const HELP = `Usage: gen_video <prompt> <output_video> [options]

The prompt names a material as @{id} (write @@ for a literal @). Say what a
reference is for in the prompt; the model sees it as "image 1", "video 2"...
A prompt may be "" when the conditions say enough.

Conditions:
  --first-frame <image>           start keyframe (id "first")
  --last-frame <image>            end keyframe (id "last")
  --ref-image <id>=<image>        reference image; repeatable
  --ref-video <id>=<video>        reference video; repeatable
  --ref-audio <id>=<audio>        reference audio; repeatable
  --with-audio <id>               let the model hear a --ref-video's own audio
  --clip <id>=<start>-<end>       seconds of a reference video/audio to use
                                  (AICC does not trim yet: a range shorter than
                                  the clip is refused)
  A material is a local path, an http(s) URL or named_object:<obj_id>.
  Keyframes and references cannot be mixed on most models.

${VIDEO_OUTPUT_HELP}
  --negative <text>               negative prompt (only some models)
  --seed <int>                    (only some models)

An option the chosen model cannot honour is refused, never dropped.
${COMMON_OPTIONS_HELP}`;

export async function run(argv: string[]): Promise<never> {
  let repeated;
  try {
    repeated = takeRepeatedFlags(argv, VIDEO_CONDITION_FLAGS);
  } catch (err) {
    bailArgError(TOOL, err as Error);
  }
  const parsed = parseArgvOrExit(TOOL, HELP, repeated.rest);
  if (parsed.positional.length < 2) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, {
        error: "missing positional",
      }),
      EXIT_ARG_ERROR,
    );
  }
  const [prompt, outputPath] = parsed.positional;

  const request: TypedRequestMap[typeof METHOD] = {};
  if (prompt.trim()) request.prompt = prompt;
  try {
    const conditions = await videoConditions(repeated.values, {
      start: requireString(parsed.flags, "first-frame"),
      end: requireString(parsed.flags, "last-frame"),
    });
    if (conditions.length) request.conditions = conditions;
    const output = videoOutputSpec(parsed.flags);
    if (output) request.output = output;
    const negative = requireString(parsed.flags, "negative");
    if (negative !== undefined) request.negative_prompt = negative;
    const seed = flagInt(parsed.flags, "seed");
    if (seed !== undefined) request.seed = seed;
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
    saved = await saveArtifactToPath(
      artifact,
      suffixPathByMime(outputPath, artifact.mime ?? "video/mp4"),
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
        obj_id: saved.obj_id ?? null,
      }],
      video: artifact.metadata ?? null,
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
