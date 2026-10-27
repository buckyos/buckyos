---
name: aigc_image
description: Generate and edit images through the zone's AI models — text to image, edit or restyle from a reference image, masked inpaint, upscale, background removal — check the files and hand them over.
requires: the buckyos-agent commands on PATH (gen_image, edit_image, …: aios agent container, or a node's agent-tools) and an AICC provider that serves the image method; results are kept in the Agent's `artifacts` bucket, which only read_write Agents may write
baseline: standard
---
#### When to use
- Make a new picture from a description (`gen_image`), change an existing one (`edit_image`), repaint a marked region (`inpaint_image`), enlarge it (`upscale_image`), cut out the subject (`remove_bg`).
- A dedicated AI command gives a much better picture than drawing with ImageMagick, SVG or Python; keep those for layout, cropping and format work on the results.
- Reading or describing an image is `llm_understand_media` (or `ocr_image` / `caption_image`); speech and music: aigc_audio; clips and films: aigc_video.

#### Before you start
- The commands below are standard-environment commands: in the AIOS `container`, and on a node through its `$BUCKYOS_ROOT/bin/agent-tools` (they run there as your agent). If absent here, check `agent-session runtime list --available` (each runtime's `baseline_check` names what it lacks) and explicitly delegate generation to a runtime that has them with `aigc_image` and no workspace, exchanging inputs and outputs by `agent-session copy` or object ids. Preserve a user-selected runtime/workspace for its own work. Report a missing compatible runtime instead of treating missing commands as absent models. `identify`/`convert` (ImageMagick) for checks are not standard: use them where they exist.
- `command -v gen_image resume_aicc_task` — the second one is newer; when it is missing, re-running a command with the same `--idempotency-key` is the way back to an interrupted task (see When it fails).
- No provider check is needed before a call: run the command and read its result. To see which models serve a method: `ai_provider models --api-type image.txt2img` (exact model, logical names, list price). `ai_provider list` shows providers and their health and lists only the models your owner may use. `ai_quota` lists the token quota rules that limit you; image, audio and video methods are not under token quota.
- Work in the session's working directory (the shell starts there) and write results under it, e.g. `out/…`.

#### Find the real commands
- `<command> --help` lists the options. Each command is one readable TS file: `/opt/buckyos/bin/opendan/buckyos-agent/commands/<command>.ts`.
- Default routes in this zone today: txt2img, img2img, inpaint → `gpt-image-2@codex-proxy`; upscale → `fal-ai/esrgan`; bg_remove → `fal-ai/imageutils/rembg`; Gemini image models are reachable as `--model image.txt2img.gemini` (0.034–0.134 USD per image).

#### Shortest path
```bash
mkdir -p out
gen_image 'A product photo of a matte black desk lamp on a white table' out/lamp.png --quality medium
# detail.files[0].url = obj://bucket/user/<agent>/<owner>/artifacts/<hash>/lamp.png; out/lamp.png is a local copy
identify out/lamp.png
edit_image 'obj://bucket/…/lamp.png' 'Make the lamp brass, keep everything else' out/lamp-brass.png
```
Give the edit instruction as one quoted argument. Then list the result's `detail.files[].url` in the final report's artifacts (see Inputs and outputs).

#### Inputs and outputs
- Input image forms: a local path (sent inline as base64), `http(s)://…` or `data:` URL, an earlier result's `obj://bucket/…`, or an object id. For a message attachment, copy its id exactly with the type prefix — `named_object:cyfile:…`, or bare `cyfile:…` / `chunk:…`; the hex part alone does not resolve.
- A local file bigger than about 700 KB fails with an opaque `RPC call error: 500` (request body limit). Pass an earlier result instead (`detail.files[].url` or `obj_id`), or shrink first: `convert big.png -resize "1536x1536>" -quality 90 small.jpg`.
- Every result goes into this Agent's `artifacts` bucket (the owner's documents: kept, backed up, shown in Files under the Agent's app): `detail.files[].url` is its `obj://bucket/…`. The output path is optional — give it when a local tool (identify, convert) needs the file. To get a local file later, or of an attachment: `materialize_resource 'obj://bucket/…' in/photo.png` or `materialize_resource 'named_object:cyfile:…' in/photo.png` (refuses to overwrite). A similar-looking file found on disk is a different input; keep the user's object.
- `gen_image <prompt> [out] [--size WxH] [--aspect 1:1|16:9|9:16|4:3|3:4] [--quality low|medium|high] [--n N] [--seed N] [--negative TEXT] [--format png|jpg|webp]`. Which options take effect depends on the routed model: gpt-image-2 uses size, quality and format and ignores aspect, seed and negative (AICC logs a warning); Gemini image models use aspect and seed and accept `--format jpg` (png/webp are rejected). Without `--size` gpt-image-2 gave 1374x1145. The command keeps one image even with `--n 2`; `resume_aicc_task <task_id> [out/set.png]` keeps all of them (local copies `out/set-1.png`, `out/set-2.png`, …).
- `edit_image <input> <prompt> [out] [--strength 0-1] [--format …]` takes one reference image; gpt-image-2 ignores `--strength`. For several reference images write a derived script (below) — `image.img2img` accepts an `images` list.
- `inpaint_image <input> <mask> <prompt> [out]`: gpt-image-2 expects a PNG mask of the same size whose transparent pixels are the area to repaint (`alpha_zero_is_edit_area`, used when `--mask-semantics` is not given); `white_area_is_edit_area` / `black_area_is_edit_area` are rejected by this model. Make one: `convert in.png -alpha set -region 300x200+120+80 -alpha transparent mask.png`.
- `upscale_image <in> [out] [--scale 2|4] [--target-width N] [--target-height N] [--preserve-faces]`, `remove_bg <in> [out] [--mode rgba_image|mask]` run on fal models.
- Result: stdout is one `AgentToolResult` JSON line (no `--json` needed); stderr carries progress. On success read `detail.files[]`: `url` (`obj://bucket/…` in the `artifacts` bucket), `obj_id`, `path` (the local copy, `null` without an output path; an extension is added when the name has none), `bytes`, `mime`, `source_kind`, and `detail.task_id` (the AICC task). A result AICC stored is copied from its 7-day `outputs` bucket, not uploaded again. `mime` may be missing for stored objects; trust `identify`.
- Hand-over from a Work session: list the result's `detail.files[].url` unchanged in the final report — `report({report: "…", artifacts: ["obj://bucket/…"], is_end: true})`, or `<artifacts>["obj://bucket/…"]</artifacts>` next to `<report end="true">`; a typed `obj_id` (`cyfile:…`) is accepted too. If you changed the file afterward, list its relative path under this session's working directory (e.g. `out/lamp.png`) so the host stores those bytes in the same bucket. Cross-session hand-over uses the reported url or object id, never another session's path or `host:/…` reference. A path written in prose is not a delivery.

#### When it fails
- Exit codes: 0 done, 1 bad arguments (nothing sent), 2 AICC/provider failure, 3 no route or provider (`no_candidate_model`, e.g. a misspelled exact model `x@provider`), 4 timeout (inspect the result), 5 local file I/O.
- A misspelled logical name (`--model image.txt2img.foo`) does not fail: AICC falls back to the parent route and you get the default model. Use a name from `ai_provider models` or the exact `<model>@<provider>` form when the model matters.
- A provider rejection arrives as exit 2 `=> failed` with `{"code":"provider_error"|"invalid_request"|"unsupported_operation",…}` and the task id; change the request (option, model, input), do not just repeat it.
- Two task layers: `shell` may return early with "still running … continues as task <id>" — a shell task; keep waiting as its hint says (`wait_task`, or `agent-session task wait <id> --wait-ms …`) rather than starting the command again. The AICC task id (`t-…`) is in `detail.task_id`, in timeout errors and in the `__BUCKYOS_AGENT_PROGRESS__` stderr lines.
- `=> wait_timeout` with `detail.still_running: true` and `detail.task_id` means the AICC task keeps running. `resume_aicc_task <task_id> [out/x.png]` waits and keeps the results in the bucket (and local copies when a path is given) without paying again. If you passed `--idempotency-key K`, re-running the same command with the same K also re-attaches to the same task within 24 h (it also returns a failed task again — use a new key to really retry).
- HTTP 408/504 and provider timeouts may also exit 4; a timeout or dropped connection alone does not establish the task's state. With a known task id, use `resume_aicc_task <task_id> --no-wait` to check it. Without one, keep the error/trace and consult `buckyos_admin`; do not invent a task id or automatically submit again.
- A retry after a real failure is a new billable call; change something first and say so in the report.
- `=> resource_not_permitted` (exit 2, `detail.error_code`): the model is not open to your owner — pick one `ai_provider models` lists or tell the user; `=> quota_exhausted` / `quota_budget_exceeded`: the owner's AI quota does not cover the call (`detail.details` names the rule and `reset_at_ms`, `null` = only an admin recharge helps) — stop and report it, do not retry at once.

#### Check the result
- `identify out/x.png` (format, WxH, non-zero size); `compare -metric RMSE in.png out.png null:` shows whether an edit changed anything.
- Look at it: `llm_understand_media out/x.png --goal "does it show <what was asked>?"` before claiming the content matches.
- Success means `status: "success"` plus every expected file present and valid — not exit code 0 alone.

#### Source and extending
- Recipe template: `/opt/buckyos/bin/opendan/buckyos-agent/commands/gen_image.ts` — six steps: parse argv → build the request → `initRuntime()` → AICC call (`callAicc` / `textToImage`, logical model routed first) → deliver the artifacts into the `artifacts` bucket → `emit()` the result. Other commands are the same skeleton.
- Library: `lib/runtime.ts` (login), `lib/aicc.ts` (`callAicc`, `resumeAiccTask`, `describeFailure`), `lib/io.ts` (`resolveInputResource`, `pickArtifact`, `deliverArtifact`, `saveResourceToPath`), `lib/bucket.ts` (`uploadNamed` — bytes into `artifacts` or the 7-day `scratch` bucket, `copyToArtifacts`, `readBucketObject`), `lib/cli.ts`, `lib/result.ts`, `lib/types.ts`. SDK types: the `buckyos` entry of `/opt/buckyos/bin/opendan/buckyos-agent/deno.json` (`/opt/buckyos/libexec/buckyos-tool/dist/*.d.ts`), e.g. `aicc_client.d.ts`.
- Derive instead of editing the read-only copy: a one-off script goes in the session directory; a keeper goes in `$OPENDAN_AGENT_ROOT/tools/` (on PATH; give it the first line `#!/usr/bin/env -S deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors` and `chmod +x` to call it by name). Import the library by absolute path and run with the installed config:
```bash
cat > edit_multi.ts <<'EOF'
import { ndm_proxy } from "buckyos";
import { initRuntime } from "/opt/buckyos/bin/opendan/buckyos-agent/lib/runtime.ts";
import { callAicc, describeFailure } from "/opt/buckyos/bin/opendan/buckyos-agent/lib/aicc.ts";
import { deliverArtifact, pickArtifact, resolveInputResource } from "/opt/buckyos/bin/opendan/buckyos-agent/lib/io.ts";
const [prompt, out, ...inputs] = Deno.args;
const images = await Promise.all(inputs.map((i) => resolveInputResource(i, "image/*")));
const runtime = await initRuntime();
const call = await callAicc(runtime, { method: "image.img2img", model: "image.img2img", request: { images, prompt } });
if (call.status !== "succeeded" || !call.summary) throw new Error(`${call.taskId}: ${describeFailure(call)}`);
const saved = await deliverArtifact(runtime, call.summary, pickArtifact(call.summary, "image")!, (ndm_proxy as any).createNdmProxyClient(), { tool: "edit_multi", path: out });
console.log(JSON.stringify({ task_id: call.taskId, status: call.status, saved }));
EOF
deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors edit_multi.ts 'Combine both' out/combo.png in/a.png in/b.png
```

Checked against: BuckyOS 0.7.0 (buckyos-agent commands and AICC source of this tree; DV zone test.buckyos.io: gen_image and edit_image via gpt-image-2@codex-proxy, error cases; inpaint/upscale/remove_bg not run; the multi-image script type-checked in the container, not run), 2026-10-17. Bucket delivery (`detail.files[].url`, copy from AICC `outputs`, `obj://bucket/` inputs and report artifacts) 2026-10-26: unit tests and a local nfs-server only, not yet run on DV.
