// extend_video — video.extend
// Doc: aicc_agent_cli_tools.md §6.4

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagInt, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, resolveInputResource, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "extend_video";
const METHOD = "video.extend";

export const HELP = `Usage: extend_video <previous_video> <prompt> <output_video> [options]

Options:
  --continuation-handle <provider_handle>
  --duration <seconds>
  --resolution <720p|1080p|4k>
${COMMON_OPTIONS_HELP}`;

const RESOLUTION = new Set(["720p", "1080p", "4k"]);

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 3) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcVideo, prompt, outputPath] = parsed.positional;

  const input: Record<string, unknown> = { prompt };
  let inputResource;
  try {
    const handle = requireString(parsed.flags, "continuation-handle");
    if (handle !== undefined) input.continuation_handle = handle;
    const duration = flagInt(parsed.flags, "duration");
    if (duration !== undefined) input.duration = duration;
    const resolution = requireString(parsed.flags, "resolution");
    if (resolution !== undefined) {
      if (!RESOLUTION.has(resolution)) throw new ArgError(`--resolution invalid: ${resolution}`);
      input.resolution = resolution;
    }
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
