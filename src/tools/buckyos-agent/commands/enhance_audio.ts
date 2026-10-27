// enhance_audio — audio.enhance
// Doc: aicc_agent_cli_tools.md §5.4
// 这个 method 可以返回多个 artifact（去噪/去混响 + 各分轨 stems），
// 所以 step 5 用了一个 for 循环。

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  DELIVERY_HELP,
  flagBool,
  flagFloat,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import type { TypedRequestMap } from "../lib/aicc.ts";
import {
  deliverArtifact,
  deliveredSummary,
  pickArtifact,
  resolveInputResource,
} from "../lib/io.ts";
import type { DeliveredOutput } from "../lib/io.ts";
import { aiResponseArtifacts } from "../lib/types.ts";
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

const TOOL = "enhance_audio";
const METHOD = "audio.enhance";

export const HELP = `Usage: enhance_audio <input_audio> [output_audio] [options]

Options:
  --task <denoise|dereverb|separate_voice|normalize>
  --strength <0.0-1.0>
  --return-stems
  --stems-dir <dir>                local copies of the stems (default: stems/ beside the output)
${COMMON_OPTIONS_HELP}${DELIVERY_HELP}`;

const TASK = new Set(["denoise", "dereverb", "separate_voice", "normalize"]);

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
  const [srcAudio, outputPath] = parsed.positional;
  const stemsDir = requireString(parsed.flags, "stems-dir");

  const request = {
    audio: undefined,
    task: "",
  } as unknown as TypedRequestMap[typeof METHOD];
  let inputResource;
  try {
    const task = requireString(parsed.flags, "task");
    if (task === undefined) throw new ArgError("--task is required");
    if (!TASK.has(task)) throw new ArgError(`--task invalid: ${task}`);
    request.task = task;
    const strength = flagFloat(parsed.flags, "strength");
    if (strength !== undefined) {
      if (strength < 0 || strength > 1) {
        throw new ArgError(`--strength must be in [0,1]`);
      }
      request.strength = strength;
    }
    if (flagBool(parsed.flags, "return-stems")) request.return_stems = true;
    inputResource = await resolveInputResource(srcAudio, "audio/*");
    request.audio = inputResource;
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

  const arts = aiResponseArtifacts(call.summary);
  const primary = pickArtifact(call.summary, "audio");
  if (!primary) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();

  const files: DeliveredOutput[] = [];
  try {
    const savedPrimary = await deliverArtifact(runtime, call.summary, primary, ndmProxy, {
      tool: TOOL,
      path: outputPath,
      defaultMime: "audio/wav",
    });
    files.push(savedPrimary);
    if (arts.length > 1) {
      const dir = stemsDir ??
        (savedPrimary.path?.includes("/")
          ? `${savedPrimary.path.slice(0, savedPrimary.path.lastIndexOf("/"))}/stems`
          : "stems");
      for (let i = 0; i < arts.length; i += 1) {
        if (arts[i] === primary) continue;
        const a = arts[i];
        const name = (a.name || `stem_${i}`).replace(/[^a-z0-9._-]+/gi, "-");
        const ext = (a.mime ?? a.resource.mime ?? "").split("/").at(-1) ||
          "wav";
        files.push(
          await deliverArtifact(runtime, call.summary, a, ndmProxy, {
            tool: TOOL,
            path: outputPath || stemsDir ? `${dir}/${name}.${ext}` : undefined,
          }),
        );
      }
    }
  } catch (err) {
    bailIoError(TOOL, call.taskId, err);
  }

  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      deliveredSummary(TOOL, files),
      { method: METHOD, capability: "audio", task_id: call.taskId, files },
    ),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
