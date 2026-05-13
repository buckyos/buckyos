// upscale_image — image.upscale
// Doc: aicc_agent_cli_tools.md §3.4
// 详细配方见 gen_image.ts。

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, flagBool, flagInt, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, resolveInputResource, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "upscale_image";
const METHOD = "image.upscale";

export const HELP = `Usage: upscale_image <input_image> <output_image> [options]

Options:
  --scale <2|4>
  --target-width <px>
  --target-height <px>
  --preserve-faces
  --format <png|jpg|webp>
${COMMON_OPTIONS_HELP}`;

function formatToMime(f: string | undefined): string | undefined {
  if (!f) return undefined;
  if (f === "png") return "image/png";
  if (f === "jpg" || f === "jpeg") return "image/jpeg";
  if (f === "webp") return "image/webp";
  throw new ArgError(`--format invalid: ${f}`);
}

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcImage, outputPath] = parsed.positional;

  const input: Record<string, unknown> = {};
  let inputResource;
  try {
    const scale = flagInt(parsed.flags, "scale");
    if (scale !== undefined) {
      if (scale !== 2 && scale !== 4) throw new ArgError(`--scale must be 2 or 4`);
      input.scale = scale;
    }
    const w = flagInt(parsed.flags, "target-width");
    if (w !== undefined) input.target_width = w;
    const h = flagInt(parsed.flags, "target-height");
    if (h !== undefined) input.target_height = h;
    if (flagBool(parsed.flags, "preserve-faces")) input.preserve_faces = true;
    const mime = formatToMime(requireString(parsed.flags, "format"));
    if (mime) input.output = { media_type: mime };
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

  const artifact = pickArtifact(call.summary, "image");
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try { saved = await saveArtifactToPath(artifact, suffixPathByMime(outputPath, "image/png"), ndmProxy); }
  catch (err) { bailIoError(TOOL, call.taskId, err); }

  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${saved.path}`, {
      method: METHOD, capability: "image", task_id: call.taskId,
      files: [{ path: saved.path, mime: saved.mime ?? null, bytes: saved.bytes, source_kind: saved.source_kind }],
    }),
    EXIT_SUCCESS,
  );
}

if (import.meta.main) {
  await run(Deno.args);
}
