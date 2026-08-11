// gen_music — audio.music
// Doc: aicc_agent_cli_tools.md §5.3
// 单 artifact 输出配方。

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagBool, flagInt, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "gen_music";
const METHOD = "audio.music";

export const HELP = `Usage: gen_music <prompt> <output_audio> [options]

Options:
  --duration <seconds>
  --instrumental
  --lyrics <text_or_file>
  --seed <int>
  --format <mp3|wav|ogg>
${COMMON_OPTIONS_HELP}`;

function formatToMime(f: string | undefined): string | undefined {
  if (!f) return undefined;
  if (f === "mp3") return "audio/mpeg";
  if (f === "wav") return "audio/wav";
  if (f === "ogg") return "audio/ogg";
  throw new ArgError(`--format invalid: ${f}`);
}

// --lyrics 接受字面量字符串或一个本地文本文件路径。
async function resolveLyrics(value: string): Promise<string> {
  try {
    const stat = await Deno.stat(value);
    if (stat.isFile) return await Deno.readTextFile(value);
  } catch {
    // not a path → treat as literal
  }
  return value;
}

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [prompt, outputPath] = parsed.positional;

  const input: Record<string, unknown> = { prompt };
  try {
    const duration = flagInt(parsed.flags, "duration");
    if (duration !== undefined) input.duration = duration;
    if (flagBool(parsed.flags, "instrumental")) input.instrumental = true;
    const lyrics = requireString(parsed.flags, "lyrics");
    if (lyrics !== undefined) input.lyrics = await resolveLyrics(lyrics);
    const seed = flagInt(parsed.flags, "seed");
    if (seed !== undefined) input.seed = seed;
    const mime = formatToMime(requireString(parsed.flags, "format"));
    if (mime) input.output = { media_type: mime };
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
      ...commonPolicyOptions(parsed.common),
    });
  } catch (err) { bailAiccError(TOOL, METHOD, err); }
  if (call.status === "failed" || !call.summary) {
    bailAiccFailed(TOOL, METHOD, call.taskId, describeFailure(call));
  }

  const artifact = pickArtifact(call.summary, "audio");
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try { saved = await saveArtifactToPath(artifact, suffixPathByMime(outputPath, "audio/mpeg"), ndmProxy); }
  catch (err) { bailIoError(TOOL, call.taskId, err); }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${saved.path}`, {
      method: METHOD, capability: "audio", task_id: call.taskId,
      files: [{ path: saved.path, mime: saved.mime ?? null, bytes: saved.bytes, source_kind: saved.source_kind }],
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
