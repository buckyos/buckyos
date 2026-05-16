// speech_to_text — audio.asr
// Doc: aicc_agent_cli_tools.md §5.2
// Text-out 配方；详细见 gen_image.ts / ocr_image.ts。

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagBool, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import { resolveInputResource, writeTextFile } from "../lib/io.ts";
import { aiResponseText } from "../lib/types.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
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
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcAudio, outText] = parsed.positional;

  const input: Record<string, unknown> = {};
  let inputResource;
  try {
    const lang = requireString(parsed.flags, "lang");
    if (lang !== undefined) input.language = lang;
    const ts = requireString(parsed.flags, "timestamps");
    if (ts !== undefined) {
      if (!TS.has(ts)) throw new ArgError(`--timestamps invalid: ${ts}`);
      input.timestamps = ts;
    }
    if (flagBool(parsed.flags, "diarization")) input.diarization = true;
    const fmt = requireString(parsed.flags, "format");
    if (fmt !== undefined) {
      if (!FORMAT.has(fmt)) throw new ArgError(`--format invalid: ${fmt}`);
      // "txt" 是默认行为，不需要额外的 output_formats；其它格式让 AICC 把
      // 结构化产物会进入 AiResponse.message，并通过 artifact 派生视图读取。
      if (fmt !== "txt") input.output_formats = [fmt];
    }
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
      inputJson: input,
      resources: [inputResource],
      ...commonPolicyOptions(parsed.common),
    });
  } catch (err) { bailAiccError(TOOL, METHOD, err); }
  if (call.status === "failed" || !call.summary) {
    bailAiccFailed(TOOL, METHOD, call.taskId, describeFailure(call));
  }

  const text = aiResponseText(call.summary);
  const files: Array<{ path: string; bytes: number; mime: string; source_kind: string }> = [];
  if (outText) {
    try {
      await writeTextFile(outText, text);
      files.push({
        path: outText, bytes: new TextEncoder().encode(text).byteLength,
        mime: "text/plain", source_kind: "inline_text",
      });
    } catch (err) { bailIoError(TOOL, call.taskId, err); }
  }

  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      outText ? `${TOOL} wrote ${outText}` : text.slice(0, 120),
      { method: METHOD, capability: "audio", task_id: call.taskId, files, extra: call.summary.extra ?? null },
      outText ? undefined : text,
    ),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
