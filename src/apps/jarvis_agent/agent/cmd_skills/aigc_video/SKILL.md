---
name: aigc_video
description: Make videos with the zone's AI models — one video.generate request takes a prompt plus start/end keyframes or reference images, clips and audio; video edit/extend/upscale as far as the providers support it; and multi-shot films from a storyboard (images, narration, clips, ffmpeg assembly with subtitles), with task ids recorded so long jobs resume instead of being paid twice.
requires: the standard buckyos-agent commands on PATH (gen_video, edit_video, resume_aicc_task, …) and an AICC provider that serves video.generate; results are kept in the Agent's `artifacts` bucket, which only read_write Agents may write; local editing, assembly and media inspection need the tools named in those steps
baseline: standard
commands: ffmpeg, ffprobe, magick
---
#### Generate a clip

```bash
mkdir -p out
gen_video 'A wave rolls onto a beach at sunset, static camera' out/wave.mp4 \
  --duration 5 --aspect 16:9 --idempotency-key wave-v1 --timeout 900
```

Use `gen_video` for generation. Get agreement and label any slideshow or still-animation substitute. Use `llm_understand_media` for description. The default `video.generate` routes by capability. Check models and prices with `ai_provider models --api-type video.generate`. Estimate cost before a multi-shot job.

Run in the selected workspace. The AICC commands work on `private` and nodes with `$BUCKYOS_ROOT/bin/agent-tools`. If the needed command is missing, check `agent-session runtime list --available` or `agent-session runtime get <id>`. Delegate generation to a private Work session with `aigc_video` and `--workspace none`. Transfer inputs by `agent-session copy` or object id. Generation needs no local media tools. For assembly, probes or image preparation, verify `ffmpeg`, `ffprobe` or `magick` on the runtime that will perform that step.

For conditions, edit/extend/upscale, provider limits and errors, read `$OPENDAN_AGENT_ROOT/cmd_skills/aigc_video/references/video-options.md`. For multi-shot films, narration, subtitles, first-frame continuation and ffmpeg checks, read `$OPENDAN_AGENT_ROOT/cmd_skills/aigc_video/references/assembly.md` before starting. Keep each shot's key, AICC id, status and delivered URL in `storyboard.json` so a failed shot can be retried independently.

#### Recover without paying twice

Keep the AICC `t-…` from `__BUCKYOS_AGENT_PROGRESS__` on stderr or `detail.task_id`. A background shell task has its own id and returns `<<task_result>>` (xllm uses `wait_task`).

- `wait_timeout` with `still_running` means the AICC task continues. `resume_aicc_task <task_id> [out/shot.mp4]` waits for it and returns the result and recorded cost. `--no-wait` only checks status.
- For HTTP 408/504, a dropped connection or killed shell, inspect the known task id. With no id, keep the trace for `buckyos_admin`. The identical request and `--idempotency-key` reattach within 24 h, including to a failed task. A changed request needs a new key and may incur another charge.

#### Verify and deliver

Check `status` and `detail.files[]` in the JSON result. The delivered `url` and `obj_id` are durable. `path` gives the optional local copy. Check its existence and size if requested, or use `materialize_resource 'obj://bucket/…' out/shot.mp4` to fetch it. Compare `detail.video.measured_metadata` and `deviations` to the request. Distinguish measured values from `provider_metadata`. Verify the scene/motion with `llm_understand_media out/wave.mp4 --goal "Does the wave move as requested?"` (system frame sampling needs no local ffmpeg).

Require a successful AICC task and a playable output. Report the generation method, task ids, measured duration/size, shots, cost and checks performed. Name the actual model only if the task identifies it. Include `detail.files[].url` in the Work report's `artifacts`. For an assembled film, include its path relative to the session directory to store the finished bytes; include the storyboard and reusable assets when appropriate. Use reported bucket URLs or typed ids across sessions.
