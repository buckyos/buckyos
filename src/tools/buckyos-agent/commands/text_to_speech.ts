// text_to_speech — audio.tts
// Doc: aicc_agent_cli_tools.md §5.1
// 单 artifact 输出配方；详细注释见 gen_image.ts。

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  flagFloat,
  flagInt,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import type { TypedRequestMap } from "../lib/aicc.ts";
import {
  pickArtifact,
  saveArtifactToPath,
  suffixPathByMime,
} from "../lib/io.ts";
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

const TOOL = "text_to_speech";
const METHOD = "audio.tts";

export const HELP = `Usage: text_to_speech <text> <output_audio> [options]

Options:
  --lang <language_tag>
  --gender <male|female|neutral>
  --style <bright|upbeat|informative|firm|excitable|youthful|breezy|easy_going|breathy|clear|smooth|gravelly|soft|even|mature|forward|friendly|casual|gentle|lively|knowledgeable|warm>
  --instructions <text>
  --speed <float>
  --format <mp3|wav|ogg>
  --sample-rate <hz>
${COMMON_OPTIONS_HELP}`;

const GENDER = new Set(["male", "female", "neutral"]);
const STYLE = new Set([
  "bright",
  "upbeat",
  "informative",
  "firm",
  "excitable",
  "youthful",
  "breezy",
  "easy_going",
  "breathy",
  "clear",
  "smooth",
  "gravelly",
  "soft",
  "even",
  "mature",
  "forward",
  "friendly",
  "casual",
  "gentle",
  "lively",
  "knowledgeable",
  "warm",
]);

function formatToMime(f: string | undefined): string | undefined {
  if (!f) return undefined;
  if (f === "mp3") return "audio/mpeg";
  if (f === "wav") return "audio/wav";
  if (f === "ogg") return "audio/ogg";
  throw new ArgError(`--format invalid: ${f}`);
}

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, {
        error: "missing positional",
      }),
      EXIT_ARG_ERROR,
    );
  }
  const [text, outputPath] = parsed.positional;

  const request = { text, voice: {} } as TypedRequestMap[typeof METHOD];
  try {
    const lang = requireString(parsed.flags, "lang");
    if (lang !== undefined) request.voice.language = lang;
    const gender = requireString(parsed.flags, "gender");
    if (gender !== undefined) {
      if (!GENDER.has(gender)) {
        throw new ArgError(`--gender invalid: ${gender}`);
      }
      request.voice.gender = gender;
    }
    const style = requireString(parsed.flags, "style");
    if (style !== undefined) {
      if (!STYLE.has(style)) throw new ArgError(`--style invalid: ${style}`);
      request.voice.style = style;
    }
    const instructions = requireString(parsed.flags, "instructions");
    if (instructions !== undefined) request.voice.instructions = instructions;
    const speed = flagFloat(parsed.flags, "speed");
    if (speed !== undefined) request.speed = speed;
    const mime = formatToMime(requireString(parsed.flags, "format"));
    const sr = flagInt(parsed.flags, "sample-rate");
    const out: Record<string, unknown> = {};
    if (mime) out.media_type = mime;
    if (sr !== undefined) out.sample_rate = sr;
    if (Object.keys(out).length > 0) request.output = out;
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    throw err;
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
      ...commonPolicyOptions(parsed.common),
      model: parsed.common.model ?? METHOD,
      request,
    });
  } catch (err) {
    bailAiccError(TOOL, METHOD, err);
  }
  if (call.status === "failed" || !call.summary) {
    bailAiccFailed(TOOL, METHOD, call.taskId, describeFailure(call));
  }

  const artifact = pickArtifact(call.summary, "audio");
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try {
    saved = await saveArtifactToPath(
      artifact,
      suffixPathByMime(outputPath, "audio/mpeg"),
      ndmProxy,
    );
  } catch (err) {
    bailIoError(TOOL, call.taskId, err);
  }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${saved.path}`, {
      method: METHOD,
      capability: "audio",
      task_id: call.taskId,
      files: [{
        path: saved.path,
        mime: saved.mime ?? null,
        bytes: saved.bytes,
        source_kind: saved.source_kind,
      }],
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
