// ocr_image — vision.ocr
// Doc: aicc_agent_cli_tools.md §4.1
// 这是 "text out" 类型的配方：不下载二进制 artifact，从 AiResponse.message
// 的 text 派生视图直接拿
// 文本。详细 6 步注释见 gen_image.ts。

import {
  ArgError, bailArgError, COMMON_OPTIONS_HELP, flagBool, parseArgvOrExit, requireString,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure } from "../lib/aicc.ts";
import { resolveInputResource, writeJsonFile, writeTextFile } from "../lib/io.ts";
import { aiResponseArtifacts, aiResponseText } from "../lib/types.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "ocr_image";
const METHOD = "vision.ocr";

export const HELP = `Usage: ocr_image <document> [output_text] [options]

Options:
  --level <page|block|line|word>
  --lang <zh,en,...>
  --layout
  --artifact <plain_text|alto_json>
  --json-output <output_json>
${COMMON_OPTIONS_HELP}`;

const LEVEL = new Set(["page", "block", "line", "word"]);
const ARTIFACT = new Set(["plain_text", "alto_json"]);

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 1) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [doc, outText] = parsed.positional;
  const jsonOut = requireString(parsed.flags, "json-output");

  const input: Record<string, unknown> = {};
  let inputResource;
  try {
    const level = requireString(parsed.flags, "level");
    if (level !== undefined) {
      if (!LEVEL.has(level)) throw new ArgError(`--level invalid: ${level}`);
      input.level = level;
    }
    const lang = requireString(parsed.flags, "lang");
    if (lang !== undefined) input.languages = lang.split(",").map((s) => s.trim()).filter(Boolean);
    if (flagBool(parsed.flags, "layout")) input.include_layout = true;
    const artifact = requireString(parsed.flags, "artifact");
    if (artifact !== undefined) {
      if (!ARTIFACT.has(artifact)) throw new ArgError(`--artifact invalid: ${artifact}`);
      input.artifact = artifact;
    }
    inputResource = await resolveInputResource(doc);
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

  // 文本类 method 主要从 message 的 text 派生视图拿结果；如果有结构化 layout，
  // 会出现在 response.extra 里。
  const text = aiResponseText(call.summary);
  const artifacts = aiResponseArtifacts(call.summary);
  const files: Array<{ path: string; bytes: number; mime: string; source_kind: string }> = [];
  try {
    if (outText) {
      await writeTextFile(outText, text);
      files.push({
        path: outText,
        bytes: new TextEncoder().encode(text).byteLength,
        mime: "text/plain",
        source_kind: "inline_text",
      });
    }
    if (jsonOut) {
      const body = { text, extra: call.summary.extra ?? null, artifacts };
      await writeJsonFile(jsonOut, body);
      files.push({
        path: jsonOut,
        bytes: new TextEncoder().encode(JSON.stringify(body)).byteLength,
        mime: "application/json",
        source_kind: "inline_json",
      });
    }
  } catch (err) { bailIoError(TOOL, call.taskId, err); }

  emitAndExit(
    successResult(
      TOOL,
      `${TOOL} => done`,
      outText ? `${TOOL} wrote ${outText}` : truncate(text, 120),
      {
        method: METHOD, capability: "vision", task_id: call.taskId,
        files,
        extra: call.summary.extra ?? null,
      },
      outText ? undefined : text,
    ),
    EXIT_SUCCESS,
  );
}

function truncate(text: string, max: number): string {
  const t = text.trim();
  return t.length <= max ? t : `${t.slice(0, max - 3)}...`;
}

if (import.meta.main) {
  await run(Deno.args);
}
