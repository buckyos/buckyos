// caption_image — vision.caption
// Doc: aicc_agent_cli_tools.md §4.2
// Text-out 配方；详细注释见 gen_image.ts / ocr_image.ts。

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
import { resolveInputResource, writeTextFile } from "../lib/io.ts";
import { aiResponseText } from "../lib/types.ts";
import {
  bailAiccError,
  bailAiccFailed,
  bailIoError,
  bailRuntimeError,
  emitAndExit,
  errorResult,
  EXIT_ARG_ERROR,
  EXIT_SUCCESS,
  successResult,
} from "../lib/result.ts";

const TOOL = "caption_image";
const METHOD = "vision.caption";

export const HELP = `Usage: caption_image <image> [output_text] [options]

Options:
  --style <short|dense|alt_text>
  --lang <language_tag>
  --n <count>
${COMMON_OPTIONS_HELP}`;

const STYLE = new Set(["short", "dense", "alt_text"]);

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
  const [srcImage, outText] = parsed.positional;

  const request = {
    image: undefined,
  } as unknown as TypedRequestMap[typeof METHOD];
  let inputResource;
  try {
    const style = requireString(parsed.flags, "style");
    if (style !== undefined) {
      if (!STYLE.has(style)) throw new ArgError(`--style invalid: ${style}`);
      request.style = style;
    }
    const lang = requireString(parsed.flags, "lang");
    if (lang !== undefined) request.language = lang;
    const n = flagInt(parsed.flags, "n");
    if (n !== undefined) request.n = n;
    inputResource = await resolveInputResource(srcImage, "image/*");
    request.image = inputResource;
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

  const text = aiResponseText(call.summary);
  const files: Array<
    { path: string; bytes: number; mime: string; source_kind: string }
  > = [];
  if (outText) {
    try {
      await writeTextFile(outText, text);
      files.push({
        path: outText,
        bytes: new TextEncoder().encode(text).byteLength,
        mime: "text/plain",
        source_kind: "inline_text",
      });
    } catch (err) {
      bailIoError(TOOL, call.taskId, err);
    }
  }

  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      outText ? `${TOOL} wrote ${outText}` : text.slice(0, 120),
      { method: METHOD, capability: "vision", task_id: call.taskId, files },
      outText ? undefined : text,
    ),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
