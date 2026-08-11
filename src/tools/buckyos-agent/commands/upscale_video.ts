// upscale_video — video.upscale
// Doc: aicc_agent_cli_tools.md §6.5

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagBool, flagFloat, flagInt, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, resolveInputResource, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "upscale_video";
const METHOD = "video.upscale";

export const HELP = `Usage: upscale_video <input_video> <output_video> [options]

Options:
  --target-resolution <1080p|4k>
  --denoise
  --sharpen <0.0-1.0>
  --fps <int>
${COMMON_OPTIONS_HELP}`;

const TARGET = new Set(["1080p", "4k"]);

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcVideo, outputPath] = parsed.positional;

  const input: Record<string, unknown> = {};
  let inputResource;
  try {
    const target = requireString(parsed.flags, "target-resolution");
    if (target !== undefined) {
      if (!TARGET.has(target)) throw new ArgError(`--target-resolution invalid: ${target}`);
      input.target_resolution = target;
    }
    if (flagBool(parsed.flags, "denoise")) input.denoise = true;
    const sharpen = flagFloat(parsed.flags, "sharpen");
    if (sharpen !== undefined) {
      if (sharpen < 0 || sharpen > 1) throw new ArgError(`--sharpen must be in [0,1]`);
      input.sharpen = sharpen;
    }
    const fps = flagInt(parsed.flags, "fps");
    if (fps !== undefined) input.fps = fps;
    inputResource = await resolveInputResource(srcVideo, "video/*");
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
