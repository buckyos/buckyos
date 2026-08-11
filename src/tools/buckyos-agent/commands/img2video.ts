// img2video — video.img2video
// Doc: aicc_agent_cli_tools.md §6.2

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagInt, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, resolveInputResource, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "img2video";
const METHOD = "video.img2video";

export const HELP = `Usage: img2video <input_image> <prompt> <output_video> [options]

Options:
  --duration <seconds>
  --aspect <16:9|9:16|1:1>
  --resolution <720p|1080p|4k>
${COMMON_OPTIONS_HELP}`;

const ASPECT = new Set(["16:9", "9:16", "1:1"]);
const RESOLUTION = new Set(["720p", "1080p", "4k"]);

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 3) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcImage, prompt, outputPath] = parsed.positional;

  const input: Record<string, unknown> = { prompt };
  let inputResource;
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
    inputResource = await resolveInputResource(srcImage, "image/*");
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    bailIoError(TOOL, undefined, err);
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
      resources: [inputResource],
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
