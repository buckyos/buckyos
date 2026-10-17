---
name: aigc_audio
description: Produce audio through the zone's AI models — read text aloud (text to speech), generate music, clean up a recording — then check that the file plays and hand it over; transcription is a side tool.
requires: aios agent container with the buckyos-agent commands on PATH (text_to_speech, gen_music, enhance_audio, speech_to_text) plus ffmpeg/ffprobe, and an AICC provider that serves the audio method
---
#### When to use
- Typical requests: "read this aloud", "make a 30-second voice-over", "a short jingle for the intro", "remove the hum from this recording".
- Speech from text: narration, a spoken reply, a voice-over (`text_to_speech`). New music from a description (`gen_music`). Denoise or normalize an existing recording (`enhance_audio`).
- `speech_to_text` gives the exact words of a recording (transcripts, subtitles) and is a good way to check generated speech; a general "what is in this audio" question is `llm_understand_media`.
- Pictures: aigc_image; video and putting voice under video: aigc_video.

#### Before you start
- `command -v text_to_speech resume_aicc_task ffprobe` — `resume_aicc_task` is newer; without it, re-run with the same `--idempotency-key` to get back to an interrupted task.
- Which models serve a method: `ai_provider models --api-type audio.tts` (exact model, logical names, price). No health check is needed before a call.
- Defaults in this zone today: audio.tts → `gemini-2.5-flash-preview-tts`; audio.music → `lyria-3-clip-preview` (0.04 USD per request); audio.enhance → `fal-ai/deepfilternet3`; audio.asr → `gemini-3.5-transcribe`.

#### Find the real commands
- `<command> --help`; source `/opt/buckyos/bin/opendan/buckyos-agent/commands/<command>.ts` shows exactly which request fields each option sets.

#### Shortest path
```bash
mkdir -p out
text_to_speech 'Welcome home. Your files are ready.' out/welcome --lang en-US
# raw PCM from the model is saved as WAV: use the path in detail.files[0].path (out/welcome.wav)
ffprobe -v error -show_entries format=duration:stream=codec_name,sample_rate,channels -of default=nw=1 out/welcome.wav
```
For Chinese text pass `--lang zh-CN` (not tested with the current model).
List `out/welcome.wav` in the final report's artifacts.

#### Inputs and outputs
- `text_to_speech <text> <out> [--lang en-US|zh-CN|…] [--gender male|female|neutral] [--style warm|clear|…] [--instructions TEXT] [--speed F] [--format mp3|wav|ogg] [--sample-rate HZ]`. Put the text in one quoted argument; for a file use `"$(cat script.txt)"`.
- The default Gemini TTS model returns raw 16-bit PCM, 24 kHz, mono (`audio/l16`); the command wraps it in a WAV header and saves it with a `.wav` extension, whatever extension you gave — read the real path from `detail.files[0].path`. For mp3: `ffmpeg -i out/x.wav -c:a libmp3lame out/x.mp3`. `--format wav` is rejected by this model (`Audio MIME type AUDIO_WAV is not supported`), `--format ogg` is not accepted by the Gemini adapter, `--format mp3` is accepted by the adapter but untested here.
- Voice options are best effort: a voice spec the model cannot honour falls back to its default voice without an error (Gemini: style picks a voice, language goes to the speech config, gender/instructions are dropped; `--speed` is ignored with a warning). Listen or transcribe before promising a particular voice.
- No length limit or chunking exists in AICC; long text is one long request. For long scripts generate per paragraph (each with its own `--idempotency-key`), convert each to wav, then join: `printf "file %s\n" p1.wav p2.wav > list.txt && ffmpeg -f concat -safe 0 -i list.txt -c copy all.wav`.
- `gen_music <prompt> <out> [--duration S] [--instrumental] [--lyrics TEXT|FILE] [--seed N] [--format …]` makes a new clip; Lyria takes duration/instrumental/lyrics as hints added to the prompt, so check the real length. Continuing an existing track is not supported — describe the result as a new clip.
- `enhance_audio <in> <out> --task denoise|dereverb|separate_voice|normalize [--strength 0-1] [--return-stems] [--stems-dir D]`; the routed fal model is a denoiser, other tasks are untested.
- `speech_to_text <in> [out.txt] [--lang L] [--timestamps none|segment|word] [--diarization] [--format txt|json|vtt|srt]`; without an output path the text is in the result's `output`.
- Inputs: local path (inline base64; above ~700 KB it fails with `RPC call error: 500`), URL, or an object id copied with its type prefix (`named_object:cyfile:…`). `materialize_resource 'named_object:cyfile:…' in/rec.m4a` fetches an attachment for ffmpeg.
- Result: one `AgentToolResult` JSON line on stdout (no `--json` needed). Read `status`, `detail.task_id` and `detail.files[]` (`path`, `bytes`, `obj_id`; `mime` may say `application/octet-stream`). `resume_aicc_task <task_id>` without an output path lists the true result mime (e.g. `audio/l16`) and the recorded cost; with a path it saves PCM as WAV too.
- Hand-over from a Work session: file under the working directory, relative path in the final report — `report({report: "…", artifacts: ["out/welcome.wav"], is_end: true})` or `<artifacts>["out/welcome.wav"]</artifacts>` beside `<report end="true">`. An object id (`cyfile:…`) is accepted as well. Naming a path in prose does not attach it.

#### When it fails
- Exit codes: 1 bad arguments (nothing sent), 2 AICC/provider failure, 3 no route (`no_candidate_model`), 4 wait timeout, 5 file I/O.
- Provider rejection: exit 2 `=> failed` with `{"code":"provider_error",…,"provider_code":"400"}` — drop or change the offending option rather than repeating the call.
- Two task layers: the `shell` call may return early with "still running … continues as task <id>" — that id is a shell task; keep waiting the way the hint says (`wait_task` with that id, or `agent-session task wait <id> --wait-ms …`) instead of starting the command again. The AICC generation has its own id (`t-…`), printed in the command's `__BUCKYOS_AGENT_PROGRESS__` stderr lines, in `detail.task_id` of the result and in a timeout error.
- Exit 4 or a shell that returned early means still running, not failed: `resume_aicc_task <task_id> out/x` finishes it without a second charge. With `--idempotency-key K`, the same command and K re-attach within 24 h (and return an earlier failure again — use a new key for a real retry).
- Costs: `ai_provider models --api-type audio.tts` shows list prices; `resume_aicc_task <task_id>` shows what a finished task cost (the TTS sentence above: 0.001 USD).

#### Check the result
- `ffprobe` shows a real duration and sample rate; `ffmpeg -v error -i out/x.wav -f null -` decodes without errors; `ffmpeg -i out/x.wav -af silencedetect=n=-40dB:d=0.5 -f null -` finds long silences. A rough speech length is 2.5–3 words per second.
- Deliver the `.wav` (or an `-c:a libmp3lame` mp3 for a smaller file). A file from an older command that is raw PCM (`ffprobe`: "Invalid data found") converts with `ffmpeg -f s16le -ar 24000 -ac 1 -i in.pcm out.wav`.
- One-line length check: `ffprobe -v error -show_entries format=duration -of csv=p=0 out/x.wav`.
- Content: `speech_to_text out/x.wav` (one more paid call) or `llm_understand_media out/x.wav --goal "what is said?"`; compare with the requested text before claiming it.

#### Source and extending
- Template: `/opt/buckyos/bin/opendan/buckyos-agent/commands/gen_image.ts` (parse argv → build request → `initRuntime()` → `callAicc` → save artifacts → `emit()`); `text_to_speech.ts` is the audio variant. Library in `lib/` (`aicc.ts`, `io.ts`, `cli.ts`, `result.ts`, `types.ts`); SDK types via the `buckyos` entry of the installed `deno.json` (`aicc_client.d.ts` lists every request field, e.g. `AudioTextToSpeechRequest`).
- A derived tool (e.g. chunk a long script, speak each part, convert and join) goes in the session directory or `$OPENDAN_AGENT_ROOT/tools/`; import `/opt/buckyos/bin/opendan/buckyos-agent/lib/*.ts` by absolute path and run with `deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors <script>.ts` (add `--allow-run` when it calls ffmpeg). The installed copy is read-only.

Checked against: BuckyOS 0.7.0 (buckyos-agent commands and AICC source of this tree; DV zone test.buckyos.io: text_to_speech via gemini-2.5-flash-preview-tts incl. the wav rejection, PCM → WAV and concat; gen_music, enhance_audio, speech_to_text not run), 2026-10-17.
