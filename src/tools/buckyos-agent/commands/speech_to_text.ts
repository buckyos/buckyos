// speech_to_text — audio.asr
// Doc: aicc_agent_cli_tools.md §5.2
// Text-out 配方；详细见 gen_image.ts / ocr_image.ts。

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  flagBool,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import type { TypedRequestMap } from "../lib/aicc.ts";
import { resolveInputResource, writeTextFile } from "../lib/io.ts";
import { aiResponseData, aiResponseText } from "../lib/types.ts";
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

const TOOL = "speech_to_text";
const METHOD = "audio.asr";

export const HELP = `Usage: speech_to_text <input_audio> [output_text] [options]

Options:
  --lang <language_tag>
  --timestamps <none|segment|word>
  --diarization
  --format <txt|json|vtt|srt>
  --artifact-dir <dir>
${COMMON_OPTIONS_HELP}`;

const TS = new Set(["none", "segment", "word"]);
const FORMAT = new Set(["txt", "json", "vtt", "srt"]);

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
  const [srcAudio, outText] = parsed.positional;

  const request = {
    audio: undefined,
  } as unknown as TypedRequestMap[typeof METHOD];
  let inputResource;
  try {
    const lang = requireString(parsed.flags, "lang");
    if (lang !== undefined) request.language = lang;
    const ts = requireString(parsed.flags, "timestamps");
    if (ts !== undefined) {
      if (!TS.has(ts)) throw new ArgError(`--timestamps invalid: ${ts}`);
      request.timestamps = ts;
    }
    if (flagBool(parsed.flags, "diarization")) request.diarization = true;
    const fmt = requireString(parsed.flags, "format");
    if (fmt !== undefined) {
      if (!FORMAT.has(fmt)) throw new ArgError(`--format invalid: ${fmt}`);
      // "txt" 是默认行为，不需要额外的 output_formats；其它格式让 AICC 把
      // 结构化产物由 canonical typed response 的 artifacts 字段返回。
      if (fmt !== "txt") request.output_formats = [fmt];
    }
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
      {
        method: METHOD,
        capability: "audio",
        task_id: call.taskId,
        files,
        data: aiResponseData(call.summary),
      },
      outText ? undefined : text,
    ),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
