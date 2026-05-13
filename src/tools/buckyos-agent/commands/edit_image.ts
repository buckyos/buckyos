// edit_image — image.img2img
// Doc: aicc_agent_cli_tools.md §3.2
// 详细的 6 步配方注释见 gen_image.ts。

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  flagFloat,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import {
  callAicc,
  commonPolicyOptions,
  describeFailure,
  requestNamedObjectOutput,
} from "../lib/aicc.ts";
import { pickArtifact, resolveInputResource, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
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

const TOOL = "edit_image";
const METHOD = "image.img2img";

export const HELP = `Usage: edit_image <input_image> <prompt> <output_image> [options]

Options:
  --strength <0.0-1.0>
  --format <png|jpg|webp>
${COMMON_OPTIONS_HELP}`;

function formatToMime(format: string | undefined): string | undefined {
  if (!format) return undefined;
  if (format === "png") return "image/png";
  if (format === "jpg" || format === "jpeg") return "image/jpeg";
  if (format === "webp") return "image/webp";
  throw new ArgError(`--format invalid: ${format}`);
}

export async function run(argv: string[]): Promise<never> {
  // 1. 参数
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 3) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcImage, prompt, outputPath] = parsed.positional;

  // 2. input_json + 输入图片 resource
  const input: Record<string, unknown> = { prompt };
  let inputResource;
  try {
    const strength = flagFloat(parsed.flags, "strength");
    if (strength !== undefined) {
      if (strength < 0 || strength > 1) throw new ArgError(`--strength must be in [0,1]`);
      input.strength = strength;
    }
    const mime = formatToMime(requireString(parsed.flags, "format"));
    if (mime) input.output = { media_type: mime };
    // resolveInputResource 把字符串映射成 ResourceRef:
    //   "named_object:<obj_id>" / "chunk:<obj_id>"  → named_object
    //   "http(s)://..." / "data:..."                → url
    //   其他路径                                     → 本地读 + base64 (小文件用)
    inputResource = await resolveInputResource(srcImage, "image/*");
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    bailIoError(TOOL, undefined, err);
  }

  // 3 + 4. 运行时 + AICC
  let runtime;
  try { runtime = await initRuntime(); } catch (err) { bailRuntimeError(TOOL, err); }
  let call;
  try {
    call = await callAicc(runtime, {
      capability: "image",
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

  // 5. 落盘
  const artifact = pickArtifact(call.summary, "image");
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try { saved = await saveArtifactToPath(artifact, suffixPathByMime(outputPath, "image/png"), ndmProxy); }
  catch (err) { bailIoError(TOOL, call.taskId, err); }

  // 6. emit
  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${saved.path}`, {
      method: METHOD,
      capability: "image",
      task_id: call.taskId,
      files: [{ path: saved.path, mime: saved.mime ?? null, bytes: saved.bytes, source_kind: saved.source_kind }],
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
