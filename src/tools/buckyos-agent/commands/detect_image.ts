// detect_image — vision.detect
// Doc: aicc_agent_cli_tools.md §4.3
// "JSON-out" 配方：结果主要在 summary.extra 里，整包序列化落盘。

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagFloat, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import { resolveInputResource, writeJsonFile } from "../lib/io.ts";
import { aiResponseArtifacts, aiResponseText } from "../lib/types.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "detect_image";
const METHOD = "vision.detect";

export const HELP = `Usage: detect_image <image> <output_json> [options]

Options:
  --classes <class1,class2,...>
  --threshold <float>
  --bbox-format <xywh>
  --bbox-unit <px|ratio>
${COMMON_OPTIONS_HELP}`;

const BBOX_FORMAT = new Set(["xywh"]);
const BBOX_UNIT = new Set(["px", "ratio"]);

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcImage, outputJson] = parsed.positional;

  const input: Record<string, unknown> = {};
  let inputResource;
  try {
    const classes = requireString(parsed.flags, "classes");
    if (classes !== undefined) input.classes = classes.split(",").map((s) => s.trim()).filter(Boolean);
    const th = flagFloat(parsed.flags, "threshold");
    if (th !== undefined) input.threshold = th;
    const bf = requireString(parsed.flags, "bbox-format");
    if (bf !== undefined) {
      if (!BBOX_FORMAT.has(bf)) throw new ArgError(`--bbox-format invalid: ${bf}`);
      input.bbox_format = bf;
    }
    const bu = requireString(parsed.flags, "bbox-unit");
    if (bu !== undefined) {
      if (!BBOX_UNIT.has(bu)) throw new ArgError(`--bbox-unit invalid: ${bu}`);
      input.bbox_unit = bu;
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
      capability: "vision",
      method: METHOD,
      modelAlias: parsed.common.model ?? METHOD,
      inputJson: input,
      resources: [inputResource],
      ...commonPolicyOptions(parsed.common),
    });
  } catch (err) { bailAiccError(TOOL, METHOD, err); }
  if (call.status === "failed" || !call.summary) {
    bailAiccFailed(TOOL, METHOD, call.taskId, describeFailure(call));
  }

  const body = {
    text: aiResponseText(call.summary) || null,
    extra: call.summary.extra ?? null,
    artifacts: aiResponseArtifacts(call.summary),
  };
  try { await writeJsonFile(outputJson, body); } catch (err) { bailIoError(TOOL, call.taskId, err); }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${outputJson}`, {
      method: METHOD, capability: "vision", task_id: call.taskId,
      files: [{
        path: outputJson,
        bytes: new TextEncoder().encode(JSON.stringify(body)).byteLength,
        mime: "application/json",
        source_kind: "inline_json",
      }],
      extra: call.summary.extra ?? null,
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
