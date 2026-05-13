// gen_video — video.txt2video
// Doc: aicc_agent_cli_tools.md §6.1
// 单 artifact 输出配方。视频类调用通常阻塞较久——靠 callAicc 内部对
// task-manager 轮询即可，CLI 不需要做额外处理。

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagBool, flagInt, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "gen_video";
const METHOD = "video.txt2video";

export const HELP = `Usage: gen_video <prompt> <output_video> [options]

Options:
  --duration <seconds>
  --aspect <16:9|9:16|1:1>
  --resolution <720p|1080p|4k>
  --audio
  --seed <int>
  --fps <int>
  --format <mp4|webm>
${COMMON_OPTIONS_HELP}`;

const ASPECT = new Set(["16:9", "9:16", "1:1"]);
const RESOLUTION = new Set(["720p", "1080p", "4k"]);

function formatToMime(f: string | undefined): string | undefined {
  if (!f) return undefined;
  if (f === "mp4") return "video/mp4";
  if (f === "webm") return "video/webm";
  throw new ArgError(`--format invalid: ${f}`);
}

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [prompt, outputPath] = parsed.positional;

  const input: Record<string, unknown> = { prompt };
  try {
    const duration = flagInt(parsed.flags, "duration");
    if (duration !== undefined) input.duration = duration;
    const aspect = requireString(parsed.flags, "aspect");
    if (aspect !== undefined) {
      if (!ASPECT.has(aspect)) throw new ArgError(`--aspect invalid: ${aspect}`);
      input.aspect_ratio = aspect;
    }
    const resolution = requireString(parsed.flags, "resolution");
    if (resolution !== undefined) {
      if (!RESOLUTION.has(resolution)) throw new ArgError(`--resolution invalid: ${resolution}`);
      input.resolution = resolution;
    }
    if (flagBool(parsed.flags, "audio")) input.audio = true;
    const seed = flagInt(parsed.flags, "seed");
    if (seed !== undefined) input.seed = seed;
    const fps = flagInt(parsed.flags, "fps");
    if (fps !== undefined) input.fps = fps;
    const mime = formatToMime(requireString(parsed.flags, "format"));
    if (mime) input.output = { media_type: mime };
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    throw err;
  }

  let runtime;
  try { runtime = await initRuntime(); } catch (err) { bailRuntimeError(TOOL, err); }
  let call;
  try {
    call = await callAicc(runtime, {
      capability: "video",
      method: METHOD,
      modelAlias: parsed.common.model ?? METHOD,
      inputJson: requestNamedObjectOutput(input),
      ...commonPolicyOptions(parsed.common),
    });
  } catch (err) { bailAiccError(TOOL, METHOD, err); }
  if (call.status === "failed" || !call.summary) {
    bailAiccFailed(TOOL, METHOD, call.taskId, describeFailure(call));
  }

  const artifact = pickArtifact(call.summary, "video");
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try { saved = await saveArtifactToPath(artifact, suffixPathByMime(outputPath, "video/mp4"), ndmProxy); }
  catch (err) { bailIoError(TOOL, call.taskId, err); }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${saved.path}`, {
      method: METHOD, capability: "video", task_id: call.taskId,
      files: [{ path: saved.path, mime: saved.mime ?? null, bytes: saved.bytes, source_kind: saved.source_kind }],
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
