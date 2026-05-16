// gen_image — image.txt2img
// Doc: aicc_agent_cli_tools.md §3.1
//
// 这个文件是 Agent 写自己的 AICC 工具的范本。结构上从上到下就是一份配方:
//
//   1) 解析 argv → 位置参数 + 选项
//   2) 构造 input_json（AICC payload.input_json 里要送的字段）
//   3) initRuntime() 拿到登录后的 BuckyOS 会话
//   4) callAicc() 同步调用 + 等待 task 完成
//   5) 从 AiResponse.message 的 artifact 派生视图把产物拉回本地
//   6) emit() 一行 AgentToolResult JSON 给上游
//
// 改用途时，主要变化点只有 (1) (2) (5)；(3)(4)(6) 是机械的。

import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  COMMON_OPTIONS_HELP,
  flagInt,
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
import { pickArtifact, saveArtifactToPath, suffixPathByMime } from "../lib/io.ts";
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

const TOOL = "gen_image";
const METHOD = "image.txt2img";

export const HELP = `Usage: gen_image <prompt> <output_image> [options]

Positional:
  <prompt>            Text prompt
  <output_image>      Local path for the generated image

Options:
  --negative <text>
  --n <count>
  --aspect <1:1|16:9|9:16|4:3|3:4>
  --size <WxH>
  --quality <low|medium|high>
  --seed <int>
  --format <png|jpg|webp>
${COMMON_OPTIONS_HELP}`;

const ASPECT = new Set(["1:1", "16:9", "9:16", "4:3", "3:4"]);
const QUALITY = new Set(["low", "medium", "high"]);

function formatToMime(format: string | undefined): string | undefined {
  if (!format) return undefined;
  if (format === "png") return "image/png";
  if (format === "jpg" || format === "jpeg") return "image/jpeg";
  if (format === "webp") return "image/webp";
  throw new ArgError(`--format invalid: ${format}`);
}

export async function run(argv: string[]): Promise<never> {
  // ── 1. 解析参数 ────────────────────────────────────────────────────────
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length < 2) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, { error: "missing positional arguments" }),
      EXIT_ARG_ERROR,
    );
  }
  const [prompt, outputPath] = parsed.positional;

  // ── 2. 构造 input_json ─────────────────────────────────────────────────
  // 字段名 (prompt / negative_prompt / aspect_ratio / quality / seed / n /
  // output.media_type / output.size) 来自 aicc_api设计.md image capability
  // 那一节。每个 method 的可选字段需要查 doc。
  const input: Record<string, unknown> = { prompt };
  try {
    const negative = requireString(parsed.flags, "negative");
    if (negative !== undefined) input.negative_prompt = negative;
    const n = flagInt(parsed.flags, "n");
    if (n !== undefined) input.n = n;
    const aspect = requireString(parsed.flags, "aspect");
    if (aspect !== undefined) {
      if (!ASPECT.has(aspect)) throw new ArgError(`--aspect invalid: ${aspect}`);
      input.aspect_ratio = aspect;
    }
    const size = requireString(parsed.flags, "size");
    if (size !== undefined) {
      if (!/^\d+x\d+$/i.test(size)) throw new ArgError(`--size must be WxH, got ${size}`);
      input.size = size;
    }
    const quality = requireString(parsed.flags, "quality");
    if (quality !== undefined) {
      if (!QUALITY.has(quality)) throw new ArgError(`--quality invalid: ${quality}`);
      input.quality = quality;
    }
    const seed = flagInt(parsed.flags, "seed");
    if (seed !== undefined) input.seed = seed;
    const mime = formatToMime(requireString(parsed.flags, "format"));
    if (mime) input.output = { media_type: mime };
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    throw err;
  }

  // requestNamedObjectOutput 给 input_json 加 response_format/object_id 提示，
  // 让 provider 把产物存成 BuckyOS named_object 而不是塞 base64 回响应里。
  // 大产物（image/audio/video）都应该这么调。
  const inputJson = requestNamedObjectOutput(input);

  // ── 3. 拿到 BuckyOS 运行时 ────────────────────────────────────────────
  let runtime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }

  // ── 4. 调用 AICC ─────────────────────────────────────────────────────
  // callAicc 会:
  //   • 用 modelAlias / capability / policy 包成完整 AiMethodRequest
  //   • 一次性走 aicc.<method> RPC，拿到 succeeded / running / failed
  //   • 如果是 running，按 external_task_id 在 task-manager 里轮询到终态
  let call;
  try {
    call = await callAicc(runtime, {
      capability: "image",
      method: METHOD,
      modelAlias: parsed.common.model ?? METHOD,
      inputJson,
      ...commonPolicyOptions(parsed.common),
    });
  } catch (err) {
    bailAiccError(TOOL, METHOD, err);
  }
  if (call.status === "failed" || !call.summary) {
    bailAiccFailed(TOOL, METHOD, call.taskId, describeFailure(call));
  }

  // ── 5. 把产物拉到本地 ─────────────────────────────────────────────────
  // pickArtifact: 按 mime 前缀（"image/" / "audio/" / "video/"）筛，没命中
  // 就返回第一个 artifact。
  // saveArtifactToPath: 自动处理三种 resource:
  //   - named_object → 通过 ndm_proxy.openReader 流式拉
  //   - url          → fetch
  //   - base64       → 解码
  const artifact = pickArtifact(call.summary, "image");
  if (!artifact) bailNoArtifact(TOOL, METHOD, call.taskId);
  const dest = suffixPathByMime(outputPath, "image/png");
  // deno-lint-ignore no-explicit-any
  const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
  let saved;
  try {
    saved = await saveArtifactToPath(artifact, dest, ndmProxy);
  } catch (err) {
    bailIoError(TOOL, call.taskId, err);
  }

  // ── 6. emit AgentToolResult ───────────────────────────────────────────
  emitAndExit(
    successResult(TOOL, `${TOOL} => done`, `${TOOL} wrote ${saved.path}`, {
      method: METHOD,
      capability: "image",
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
