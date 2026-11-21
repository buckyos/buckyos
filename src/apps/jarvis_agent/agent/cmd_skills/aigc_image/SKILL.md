---
name: aigc_image
description: Generate and edit images through the zone's AI models — text to image, edit or restyle from a reference image, masked inpaint, upscale, background removal — check the files and hand them over.
requires: the buckyos-agent commands on PATH (gen_image, edit_image, …: aios agent container, or a node's agent-tools) and an AICC provider that serves the image method; results are kept in the Agent's `artifacts` bucket, which only read_write Agents may write
baseline: standard
commands: magick, identify, convert, deno
---
#### Generate or edit

Use `gen_image` for a new picture, `edit_image` for one reference, `inpaint_image` for a mask, `upscale_image` for enlargement and `remove_bg` for cutouts. Use `llm_understand_media`, `ocr_image` or `caption_image` to read an image.

```bash
mkdir -p out
gen_image 'A matte black desk lamp on a white table, product photo' out/lamp.png --quality medium
edit_image 'obj://bucket/…/lamp.png' 'Make the lamp brass, keep the setting' out/brass.png
```

Replace the example URL with `detail.files[0].url` from generation. Quote the whole prompt. Work in the selected session directory. The AICC commands run on `private` and on nodes with `$BUCKYOS_ROOT/bin/agent-tools`. If the required command is missing, check `agent-session runtime list --available` or `agent-session runtime get <id>`. Delegate generation to a private Work session with `aigc_image`, `--workspace none`, and exchange files by `agent-session copy` or typed object id.

- Inputs may be local paths, HTTP(S)/data URLs, bucket URLs or typed ids (`named_object:cyfile:…`, `cyfile:…`, `chunk:…`). Keep the attachment's exact id. A first cross-Zone fetch can take minutes, and `attachment-awaiting-acceptance` needs owner action. Local inputs above about 700 KB can exceed the inline RPC limit. Use the original object or resize a local copy.
- Inspect `<command> --help` for flags and `ai_provider models --api-type image.txt2img` for accessible models/prices. Select an exact `<model>@<provider>` when it matters. A misspelled logical subroute can fall back to its parent. For mask creation, model-specific flags, getting all `--n` results, and a runnable multi-reference edit, read `$OPENDAN_AGENT_ROOT/cmd_skills/aigc_image/references/editing.md`.

#### Verify and deliver

Stdout is one JSON result. Check `status: "success"`, `detail.task_id` and every expected `detail.files[]` entry (`url`, `obj_id`, `bytes`, `mime`, optional local `path`). `materialize_resource 'obj://bucket/…' in/photo.png` fetches the file later and refuses overwrites. Use `identify out/brass.png` or `magick identify` for type and dimensions when available. Inspect the picture with `llm_understand_media out/brass.png --goal "Is the lamp brass, with the original setting?"`.

Include `detail.files[].url` in the Work report's `artifacts`; for a further edited file include its path relative to the session work directory so the host stores those bytes. Cross-session delivery takes a reported `obj://bucket/…` or typed id.

#### Recover a pending task

A background shell id finishes via `<<task_result>>` (xllm uses `wait_task`). Save the separate AICC id from `__BUCKYOS_AGENT_PROGRESS__` or `detail.task_id`. On `wait_timeout` with `still_running`, use `resume_aicc_task <task_id> [out]`. After an HTTP 408/504 or disconnect, check a known id with `--no-wait`. Without one, retain the trace for `buckyos_admin`. The same command and `--idempotency-key` reattach within 24 h, including an earlier failure.

Exit 1 means bad arguments, 2 provider/request failure, 3 no route, 4 timeout, 5 file I/O. Correct rejected inputs/options before any billable retry with a new key. `resource_not_permitted` needs an accessible model. Report `quota_exhausted` or `quota_budget_exceeded` with `detail.details` and `reset_at_ms`; `ai_quota` lists the owner's rules.

## 用量与成本核对

AICenter 的 Usage 消费卡统计当前筛选条件下的全部记录，不限当前分页；金额按币种分开显示。点击 token 数值可以查看已上报的缓存、模态 token、图片张数、音视频秒数、字符和请求单位等明细，不把这些不同单位相加。不存在的维度不会显示；部分上报只代表已知小计。

调用结果的 `usage.cost_source` 区分 Provider 实报和静态价格估算；`usage.cost_unknown_reason` 说明缺失用量、费率或币种。费用未知不表示免费。媒体任务已成功但费用未知时，先核对结果与计费用量，不为获取费用重复生成。历史记录不会随新价格自动改写。
