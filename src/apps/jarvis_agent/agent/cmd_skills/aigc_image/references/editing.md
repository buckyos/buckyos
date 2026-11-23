#### Generation and single-image operations

- `gen_image PROMPT [out]` accepts `--size WxH`, `--aspect 1:1|16:9|9:16|4:3|3:4`, `--quality low|medium|high`, `--n`, `--seed`, `--negative` and `--format png|jpg|webp`. In the tested Zone, gpt-image-2 used size, quality and format, ignoring aspect, seed and negative. Gemini used aspect and seed, accepting JPEG and rejecting PNG/WebP. Query current support and prices with `ai_provider models --api-type image.txt2img`.
- With `--n 2`, `gen_image` delivers one image. `resume_aicc_task <task_id> out/set.png` delivers all, with local copies such as `out/set-1.png`.
- `edit_image INPUT PROMPT [out]` accepts `--strength 0-1` (ignored by gpt-image-2) and `--format`. For masked edits this model expects a same-sized PNG with transparent pixels where it should repaint (`alpha_zero_is_edit_area`). It rejects white/black-area semantics.
- `upscale_image INPUT [out]` takes `--scale 2|4`, `--target-width`, `--target-height`, `--preserve-faces`. `remove_bg INPUT [out]` takes `--mode rgba_image|mask`.

```bash
magick in.png -alpha opaque -region 300x200+120+80 -channel A -evaluate set 0 +channel +region mask.png
inpaint_image in.png mask.png 'Replace the marked area with blue glass' out/fixed.png
convert big.png -resize '1536x1536>' -quality 90 small.jpg
```

For an edit, `magick compare -metric RMSE in.png out/fixed.png null:` measures the difference. Use a content check to see whether the requested change was made.

#### Several reference images

The installed `commands/gen_image.ts` demonstrates login, AICC calls and delivery. In the agent container put this script in the session directory as `edit_multi.ts` (or `$OPENDAN_AGENT_ROOT/tools/` for reuse). On a node, use the equivalent library/config under `$BUCKYOS_ROOT/libexec/buckyos-agent`.

```typescript
import { ndm_proxy } from "buckyos";
import { initRuntime } from "/opt/buckyos/bin/opendan/buckyos-agent/lib/runtime.ts";
import { callAicc, describeFailure } from "/opt/buckyos/bin/opendan/buckyos-agent/lib/aicc.ts";
import { deliverArtifact, pickArtifact, resolveInputResource } from "/opt/buckyos/bin/opendan/buckyos-agent/lib/io.ts";

const [prompt, path, ...inputs] = Deno.args;
const images = await Promise.all(inputs.map(input => resolveInputResource(input, "image/*")));
const runtime = await initRuntime();
const call = await callAicc(runtime, {
  method: "image.img2img",
  model: "image.img2img",
  request: { images, prompt },
});
if (call.status !== "succeeded" || !call.summary) {
  throw new Error(`${call.taskId}: ${describeFailure(call)}`);
}
const artifact = pickArtifact(call.summary, "image");
if (!artifact) throw new Error(`${call.taskId}: no image`);
const client = (ndm_proxy as any).createNdmProxyClient();
const saved = await deliverArtifact(runtime, call.summary, artifact, client, { tool: "edit_multi", path });
console.log(JSON.stringify({ task_id: call.taskId, saved }));
```

```bash
deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors edit_multi.ts 'Combine both' out/combo.png in/a.png in/b.png
```

BuckyOS 0.7.0 DV (2026-10-17) exercised generation and one-image edits via gpt-image-2, and error cases. Inpainting, upscale and cutout were not run there. The original multi-image recipe was type-checked only. Bucket delivery was checked with unit tests and a local nfs-server on 2026-10-26.
