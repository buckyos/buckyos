// segment_image — vision.segment
// Doc: aicc_agent_cli_tools.md §4.4
// JSON-out 配方，detect_image.ts 同结构。

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import { resolveInputResource, writeJsonFile } from "../lib/io.ts";
import { aiResponseArtifacts, aiResponseText } from "../lib/types.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "segment_image";
const METHOD = "vision.segment";

export const HELP = `Usage: segment_image <image> <output_json> [options]

Options:
  --box <x,y,width,height>
  --point <x,y>
  --text <prompt>
  --mask-format <rle|polygon>
  --bitmap-dir <dir>
${COMMON_OPTIONS_HELP}`;

const MASK_FORMAT = new Set(["rle", "polygon"]);

function parseNumberList(value: string, count: number, flag: string): number[] {
  const parts = value.split(",").map((s) => Number(s.trim()));
  if (parts.length !== count || parts.some((n) => !Number.isFinite(n))) {
    throw new ArgError(`--${flag} expects ${count} comma-separated numbers, got ${value}`);
  }
  return parts;
}

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcImage, outputJson] = parsed.positional;

  const input: Record<string, unknown> = {};
  let inputResource;
  try {
    const box = requireString(parsed.flags, "box");
    if (box !== undefined) {
      const [x, y, w, h] = parseNumberList(box, 4, "box");
      input.box = { x, y, width: w, height: h };
    }
    const point = requireString(parsed.flags, "point");
    if (point !== undefined) {
      const [x, y] = parseNumberList(point, 2, "point");
      input.point = { x, y };
    }
    const text = requireString(parsed.flags, "text");
    if (text !== undefined) input.prompt = text;
    const mf = requireString(parsed.flags, "mask-format");
    if (mf !== undefined) {
      if (!MASK_FORMAT.has(mf)) throw new ArgError(`--mask-format invalid: ${mf}`);
      input.mask_format = mf;
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
