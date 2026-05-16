// enhance_audio — audio.enhance
// Doc: aicc_agent_cli_tools.md §5.4
// 这个 method 可以返回多个 artifact（去噪/去混响 + 各分轨 stems），
// 所以 step 5 用了一个 for 循环。

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagBool, flagFloat, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, resolveInputResource, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import { aiResponseArtifacts } from "../lib/types.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "enhance_audio";
const METHOD = "audio.enhance";

export const HELP = `Usage: enhance_audio <input_audio> <output_audio> [options]

Options:
  --task <denoise|dereverb|separate_voice|normalize>
  --strength <0.0-1.0>
  --return-stems
  --stems-dir <dir>
${COMMON_OPTIONS_HELP}`;

const TASK = new Set(["denoise", "dereverb", "separate_voice", "normalize"]);

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcAudio, outputPath] = parsed.positional;
  const stemsDir = requireString(parsed.flags, "stems-dir");

  const input: Record<string, unknown> = {};
  let inputResource;
  try {
    const task = requireString(parsed.flags, "task");
    if (task !== undefined) {
      if (!TASK.has(task)) throw new ArgError(`--task invalid: ${task}`);
      input.task = task;
    }
    const strength = flagFloat(parsed.flags, "strength");
    if (strength !== undefined) {
      if (strength < 0 || strength > 1) throw new ArgError(`--strength must be in [0,1]`);
      input.strength = strength;
    }
    if (flagBool(parsed.flags, "return-stems")) input.return_stems = true;
    inputResource = await resolveInputResource(srcAudio, "audio/*");
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    bailIoError(TOOL, undefined, err);
  }

  let runtime;
  try { runtime = await initRuntime(); } catch (err) { bailRuntimeError(TOOL, err); }
  let call;
  try {
    call = await callAicc(runtime, {
      capability: "audio",
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

  const arts = aiResponseArtifacts(call.summary);
  const primary = pickArtifact(call.summary, "audio");
  if (!primary) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();

  const dest = suffixPathByMime(outputPath, "audio/wav");
  const files: Array<{ path: string; bytes: number; mime: string | null; source_kind: string }> = [];
  try {
    const savedPrimary = await saveArtifactToPath(primary, dest, ndmProxy);
    files.push({
      path: savedPrimary.path, bytes: savedPrimary.bytes,
      mime: savedPrimary.mime ?? null, source_kind: savedPrimary.source_kind,
    });
    if (arts.length > 1) {
      const dir = stemsDir ??
        (dest.includes("/") ? `${dest.slice(0, dest.lastIndexOf("/"))}/stems` : "stems");
      for (let i = 0; i < arts.length; i += 1) {
        if (arts[i] === primary) continue;
        const a = arts[i];
        const name = (a.name || `stem_${i}`).replace(/[^a-z0-9._-]+/gi, "-");
        const ext = (a.mime ?? a.resource.mime ?? "").split("/").at(-1) || "wav";
        const stemSaved = await saveArtifactToPath(a, `${dir}/${name}.${ext}`, ndmProxy);
        files.push({
          path: stemSaved.path, bytes: stemSaved.bytes,
          mime: stemSaved.mime ?? null, source_kind: stemSaved.source_kind,
        });
      }
    }
  } catch (err) { bailIoError(TOOL, call.taskId, err); }

  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      files.length === 1 ? `${TOOL} wrote ${files[0].path}` : `${TOOL} wrote ${files.length} files`,
      { method: METHOD, capability: "audio", task_id: call.taskId, files },
    ),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
