// inpaint_image — image.inpaint
// Doc: aicc_agent_cli_tools.md §3.3
// 详细的 6 步配方注释见 gen_image.ts。

import { ndm_proxy } from "buckyos";

import { ArgError, bailArgError, COMMON_OPTIONS_HELP, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { initRuntime } from "../lib/runtime.ts";
import { callAicc, commonPolicyOptions, describeFailure, requestNamedObjectOutput } from "../lib/aicc.ts";
import { pickArtifact, resolveInputResource, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
import {
  bailAiccError, bailAiccFailed, bailIoError, bailNoArtifact, bailRuntimeError,
  emitAndExit, errorResult, EXIT_ARG_ERROR, EXIT_SUCCESS, successResult,
} from "../lib/result.ts";

const TOOL = "inpaint_image";
const METHOD = "image.inpaint";

export const HELP = `Usage: inpaint_image <input_image> <mask_image> <prompt> <output_image> [options]

Options:
  --mask-semantics <white_area_is_edit_area|black_area_is_edit_area|alpha_zero_is_edit_area>
  --format <png|jpg|webp>
${COMMON_OPTIONS_HELP}`;

const MASK_SEMANTICS = new Set([
  "white_area_is_edit_area",
  "black_area_is_edit_area",
  "alpha_zero_is_edit_area",
]);

function formatToMime(f: string | undefined): string | undefined {
  if (!f) return undefined;
  if (f === "png") return "image/png";
  if (f === "jpg" || f === "jpeg") return "image/jpeg";
  if (f === "webp") return "image/webp";
  throw new ArgError(`--format invalid: ${f}`);
}

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 4) {
    emitAndExit(errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional" }), EXIT_ARG_ERROR);
  }
  const [srcImage, maskImage, prompt, outputPath] = parsed.positional;

  const input: Record<string, unknown> = { prompt };
  let srcRes; let maskRes;
  try {
    const sem = requireString(parsed.flags, "mask-semantics");
    if (sem !== undefined) {
      if (!MASK_SEMANTICS.has(sem)) throw new ArgError(`--mask-semantics invalid: ${sem}`);
      input.mask_semantics = sem;
    }
    const mime = formatToMime(requireString(parsed.flags, "format"));
    if (mime) input.output = { media_type: mime };
    srcRes = await resolveInputResource(srcImage, "image/*");
    maskRes = await resolveInputResource(maskImage, "image/png");
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
      resources: [srcRes, maskRes],
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
