---
name: aigc_audio
description: Produce audio through the zone's AI models — read text aloud (text to speech), generate music, clean up a recording — then check that the file plays and hand it over; transcription is a side tool.
requires: the standard buckyos-agent commands on PATH (text_to_speech, gen_music, enhance_audio, speech_to_text, resume_aicc_task) and an AICC provider that serves the audio method; results are kept in the Agent's `artifacts` bucket, which only read_write Agents may write; local conversion, assembly and media inspection need the tools named in those steps
baseline: standard
commands: ffmpeg, ffprobe
---
#### Generate audio

Use `text_to_speech` for narration, `gen_music` for a new clip, `enhance_audio` to clean a recording, and `speech_to_text` for its words or subtitles. Use `llm_understand_media` for general content questions and `aigc_video` to put voice under video.

```bash
mkdir -p out
text_to_speech 'Welcome home. Your files are ready.' out/welcome --lang en-US --idempotency-key welcome-v1
```

Run in the selected workspace. The AICC commands work on the private runtime and nodes with `$BUCKYOS_ROOT/bin/agent-tools`. If the needed command is missing, check `agent-session runtime list --available` or `agent-session runtime get <id>`. Delegate generation to a private Work session with `aigc_audio` and `--workspace none`. Use `agent-session copy` or an object id to transfer files. Only local conversion and probing need `ffmpeg`/`ffprobe`. Delegate those steps separately if necessary.

- For a script use `text_to_speech "$(cat script.txt)" out/voice --lang en-US`. Gemini TTS wraps raw 24 kHz mono PCM as WAV even if you ask for an `.mp3` filename. Read the delivered path. Voice, pace and format support vary by model.
- For music use `gen_music 'Quiet instrumental piano intro' out/intro --duration 15 --instrumental`. Verify actual length. Music generation creates a new track.
- For cleanup use `enhance_audio in.wav out/clean --task denoise`. For a transcript use `speech_to_text in.wav out/transcript.txt`. Without an output path ASR returns text in `output`. For subtitles request `--format srt --timestamps segment` and inspect `detail.data` for the provider's structured result.
- Use the original local path, URL, `obj://bucket/…` or full typed object id (`named_object:cyfile:…`). Local inputs are base64-encoded and may exceed the RPC limit around 700 KB. Prefer the stored object in that case.

#### Recover, verify and deliver

Read the one-line JSON result on stdout. Check `status`, `detail.task_id`, and `detail.files[]` for the delivered `url`, `obj_id`, `bytes`, `mime` and optional local `path`. For TTS these identify the wrapped WAV. Include `detail.files[].url` in the Work report's `artifacts`. For audio you converted or joined, list the new file's path relative to the session directory so the host stores the finished bytes. Pass bucket URLs or typed object ids between sessions.

A background shell task finishes via `<<task_result>>` (xllm uses `wait_task`). The separate AICC id appears in `__BUCKYOS_AGENT_PROGRESS__` or `detail.task_id`. On `wait_timeout` with `still_running`, use `resume_aicc_task <task_id> [out]`. After an HTTP 408/504 or disconnect, check a known id with `--no-wait`. For an unknown outcome keep the trace for `buckyos_admin`. The same command and `--idempotency-key` reattach within 24 h, including a saved failure. Resubmission with a new key can incur another charge.

Check the requested local file exists and has bytes. Verify content with `speech_to_text` or `llm_understand_media`. Use `ffprobe` for duration/streams and `ffmpeg` for decoding when required. Report the task id and only the checks performed. For long scripts, conversion, model options, stems, pricing or errors, read `$OPENDAN_AGENT_ROOT/cmd_skills/aigc_audio/references/audio.md`.
