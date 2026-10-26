---
name: aigc_audio
description: Produce audio through the zone's AI models — read text aloud (text to speech), generate music, clean up a recording — then check that the file plays and hand it over; transcription is a side tool.
requires: the standard buckyos-agent commands on PATH (text_to_speech, gen_music, enhance_audio, speech_to_text, resume_aicc_task) and an AICC provider that serves the audio method; stored results are read through the Zone NDM proxy, which only read_write Agents may read; local conversion, assembly and media inspection need the tools named in those steps
baseline: standard
---
#### When to use
- Typical requests: "read this aloud", "make a 30-second voice-over", "a short jingle for the intro", "remove the hum from this recording".
- Speech from text: narration, a spoken reply, a voice-over (`text_to_speech`). New music from a description (`gen_music`). Denoise or normalize an existing recording (`enhance_audio`).
- `speech_to_text` gives the exact words of a recording (transcripts, subtitles) and is a good way to check generated speech; a general "what is in this audio" question is `llm_understand_media`.
- Pictures: aigc_image; video and putting voice under video: aigc_video.

#### Before you start
- Audio generation uses standard AICC commands, available on a node through `$BUCKYOS_ROOT/bin/agent-tools` and in the AIOS `container`. An available `standard` host can generate audio without a container or local ffmpeg. Preserve a user-selected runtime/workspace. If a required AICC command is missing, inspect `agent-session runtime list --available` and its `baseline_check`, then delegate generation to a runtime with that command using `aigc_audio`; use `--workspace none` for a separate one-off task and exchange inputs and outputs by `agent-session copy` or object ids. Missing local commands do not establish that the zone has no audio models.
- Check the command for the requested method, for example `command -v text_to_speech resume_aicc_task`. If task recovery is unavailable, re-run with the same `--idempotency-key` to reattach to an interrupted task.
- Local transcoding, concatenation and decoding checks require `ffmpeg`; duration/stream inspection requires `ffprobe`. Check only the tools needed by those steps. If absent here, delegate those steps to an available runtime that has them, copying files or passing object ids. A runtime's `extended` label alone does not prove it has the required tool. A missing conversion or inspection tool does not block the AICC generation step; report the specific remaining step if no runtime can perform it.
- Which models serve a method: `ai_provider models --api-type audio.tts` (exact model, logical names, price). No health check is needed before a call.
- Defaults in this zone today: audio.tts → `gemini-2.5-flash-preview-tts`; audio.music → `lyria-3-clip-preview` (0.04 USD per request); audio.enhance → `fal-ai/deepfilternet3`; audio.asr → `gemini-3.5-transcribe`.

#### Find the real commands
- `<command> --help`; source `/opt/buckyos/bin/opendan/buckyos-agent/commands/<command>.ts` shows exactly which request fields each option sets.
- On a node, substitute `$BUCKYOS_ROOT/libexec/buckyos-agent` (default `/opt/buckyos/libexec/buckyos-agent`) for the container source root `/opt/buckyos/bin/opendan/buckyos-agent` in the examples below.

#### Shortest path
```bash
mkdir -p out
text_to_speech 'Welcome home. Your files are ready.' out/welcome --lang en-US
```
The command wraps raw PCM from the model as WAV without ffmpeg; use the path in `detail.files[0].path` (`out/welcome.wav`).
For Chinese text pass `--lang zh-CN` (not tested with the current model).
List `out/welcome.wav` in the final report's artifacts.

#### Inputs and outputs
- `text_to_speech <text> <out> [--lang en-US|zh-CN|…] [--gender male|female|neutral] [--style warm|clear|…] [--instructions TEXT] [--speed F] [--format mp3|wav|ogg] [--sample-rate HZ]`. Put the text in one quoted argument; for a file use `"$(cat script.txt)"`.
- The default Gemini TTS model returns raw 16-bit PCM, 24 kHz, mono (`audio/l16`); the command wraps it in a WAV header and saves it with a `.wav` extension, whatever extension you gave — read the real path from `detail.files[0].path`. For mp3: `ffmpeg -i out/x.wav -c:a libmp3lame out/x.mp3`. `--format wav` is rejected by this model (`Audio MIME type AUDIO_WAV is not supported`), `--format ogg` is not accepted by the Gemini adapter, `--format mp3` is accepted by the adapter but untested here.
- Voice options are best effort: a voice spec the model cannot honour falls back to its default voice without an error (Gemini: style picks a voice, language goes to the speech config, gender/instructions are dropped; `--speed` is ignored with a warning). Listen or transcribe before promising a particular voice.
- No length limit or chunking exists in AICC; long text is one long request. For long scripts generate per paragraph (each with its own `--idempotency-key`), then, on a runtime with ffmpeg, convert to a common format if needed and join: `printf "file %s\n" p1.wav p2.wav > list.txt && ffmpeg -f concat -safe 0 -i list.txt -c copy all.wav`.
- `gen_music <prompt> <out> [--duration S] [--instrumental] [--lyrics TEXT|FILE] [--seed N] [--format …]` makes a new clip; Lyria takes duration/instrumental/lyrics as hints added to the prompt, so check the real length. Continuing an existing track is not supported — describe the result as a new clip.
- `enhance_audio <in> <out> --task denoise|dereverb|separate_voice|normalize [--strength 0-1] [--return-stems] [--stems-dir D]`; the routed fal model is a denoiser, other tasks are untested.
- `speech_to_text <in> [out.txt] [--lang L] [--timestamps none|segment|word] [--diarization] [--format txt|json|vtt|srt]`; without an output path the text is in the result's `output`.
- Inputs: local path (inline base64; above ~700 KB it fails with `RPC call error: 500`), URL, or an object id copied with its type prefix (`named_object:cyfile:…`). `materialize_resource 'named_object:cyfile:…' in/rec.m4a` fetches an attachment for ffmpeg.
- Result: one `AgentToolResult` JSON line on stdout (no `--json` needed). Read `status`, `detail.task_id` and `detail.files[]` (`path`, `bytes`, `obj_id`; `mime` may say `application/octet-stream`). `resume_aicc_task <task_id>` without an output path lists the true result mime (e.g. `audio/l16`) and the recorded cost; with a path it saves PCM as WAV too.
- Hand-over from a Work session: list the final playable file's relative path under this session's working directory — `report({report: "…", artifacts: ["out/welcome.wav"], is_end: true})` or `<artifacts>["out/welcome.wav"]</artifacts>` beside `<report end="true">` — so the host uploads those bytes. An unmodified result's typed `detail.files[].obj_id` is also accepted. Use the saved path after any conversion, including the command's PCM-to-WAV wrapping; the source object id still names the raw PCM. Cross-session hand-over uses the report's resulting object id, never another session's path or `host:/…` reference. Naming a path in prose does not attach it.

#### When it fails
- Exit codes: 1 bad arguments (nothing sent), 2 AICC/provider failure, 3 no route (`no_candidate_model`), 4 wait timeout, 5 file I/O.
- Provider rejection: exit 2 `=> failed` with `{"code":"provider_error",…,"provider_code":"400"}` — drop or change the offending option rather than repeating the call.
- Two task layers: the `shell` call may return early with "still running … continues as task <id>" — that id is a shell task; keep waiting the way the hint says (`wait_task` with that id, or `agent-session task wait <id> --wait-ms …`) instead of starting the command again. The AICC generation has its own id (`t-…`), printed in the command's `__BUCKYOS_AGENT_PROGRESS__` stderr lines, in `detail.task_id` of the result and in a timeout error.
- Exit 4 or a shell that returned early means still running, not failed: `resume_aicc_task <task_id> out/x` finishes it without a second charge. With `--idempotency-key K`, the same command and K re-attach within 24 h (and return an earlier failure again — use a new key for a real retry).
- `=> resource_not_permitted` / `quota_exhausted` / `quota_budget_exceeded` (`detail.error_code`): the model is not open to your owner or the owner's AI quota does not cover the call — report it, do not retry at once; `ai_quota` shows the rules.
- Costs: `ai_provider models --api-type audio.tts` shows list prices; `resume_aicc_task <task_id>` shows what a finished task cost (the TTS sentence above: 0.001 USD).

#### Check the result
- In a standard runtime, check the successful AICC task, the saved file's existence and non-zero size, and the actual path/mime in the result. Use `speech_to_text` or `llm_understand_media` below for content checks. Preserve the task id and report which checks were performed; do not claim duration, decoding or playback was verified without evidence.
- Where `ffprobe` is available, inspect duration and streams: `ffprobe -v error -show_entries format=duration:stream=codec_name,sample_rate,channels -of default=nw=1 out/x.wav`. Where `ffmpeg` is available, `ffmpeg -v error -i out/x.wav -f null -` checks decoding and `ffmpeg -i out/x.wav -af silencedetect=n=-40dB:d=0.5 -f null -` finds long silences. Delegate these checks when the task requires them and the current runtime lacks the tools. A rough speech length is 2.5–3 words per second.
- Deliver the `.wav` (or an `-c:a libmp3lame` mp3 for a smaller file). A file from an older command that is raw PCM (`ffprobe`: "Invalid data found") converts with `ffmpeg -f s16le -ar 24000 -ac 1 -i in.pcm out.wav`.
- One-line length check with `ffprobe`: `ffprobe -v error -show_entries format=duration -of csv=p=0 out/x.wav`.
- Content: `speech_to_text out/x.wav` (one more paid call) or `llm_understand_media out/x.wav --goal "what is said?"`; compare with the requested text before claiming it.

#### Source and extending
- Template: `/opt/buckyos/bin/opendan/buckyos-agent/commands/gen_image.ts` (parse argv → build request → `initRuntime()` → `callAicc` → save artifacts → `emit()`); `text_to_speech.ts` is the audio variant. Library in `lib/` (`aicc.ts`, `io.ts`, `cli.ts`, `result.ts`, `types.ts`); SDK types via the `buckyos` entry of the installed `deno.json` (`aicc_client.d.ts` lists every request field, e.g. `AudioTextToSpeechRequest`).
- A derived tool (e.g. chunk a long script, speak each part, convert and join) goes in the session directory or `$OPENDAN_AGENT_ROOT/tools/`; import `/opt/buckyos/bin/opendan/buckyos-agent/lib/*.ts` by absolute path and run with `deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors <script>.ts` (add `--allow-run` when it calls ffmpeg). The installed copy is read-only.

Checked against: BuckyOS 0.7.0 (buckyos-agent commands and AICC source of this tree; DV zone test.buckyos.io: text_to_speech via gemini-2.5-flash-preview-tts incl. the wav rejection, PCM → WAV and concat; gen_music, enhance_audio, speech_to_text not run), 2026-10-17.
