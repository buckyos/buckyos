#### Model and format options

`<command> --help` gives the installed flags. `ai_provider models --api-type audio.tts` lists current routes and prices (also `audio.music`, `audio.enhance`, `audio.asr`). `resume_aicc_task <task_id>` gives the recorded cost.

- TTS accepts `--lang`, `--gender male|female|neutral`, `--style`, `--instructions`, `--speed`, `--format mp3|wav|ogg` and `--sample-rate`. The tested Gemini model uses style and language, drops gender/instructions and warns on speed. It rejected explicit WAV and the adapter rejected OGG. Omit the format for wrapped WAV, transcode for MP3. Use `--lang zh-CN` for Chinese and check the result. `detail.artifacts[]` on resume can list original `audio/l16` while `detail.files[]` contains delivered `audio/wav`.
- Music accepts `--duration`, `--instrumental`, `--lyrics TEXT|FILE`, `--seed` and `--format`. Lyria takes length, lyrics and instrumentation as prompt hints.
- Enhancement accepts `--task denoise|dereverb|separate_voice|normalize`, `--strength 0-1`, `--return-stems` and `--stems-dir D`. The routed DeepFilterNet model is a denoiser. Stems are kept in the bucket, with optional local copies in `D` or `stems/` beside the output.
- ASR accepts `--lang`, `--timestamps none|segment|word`, `--diarization` and `--format txt|json|vtt|srt`. Transcribing generated speech adds a paid call. Compare with the requested words.

#### Local processing

For long scripts generate each paragraph with its own stable key. At roughly 2.5–3 spoken words/s, measure the actual duration before assembly. Materialize stored audio with `materialize_resource 'obj://bucket/…' in.wav`. Use matching stream parameters before concatenating.

```bash
printf "file %s\n" p1.wav p2.wav > list.txt
ffmpeg -f concat -safe 0 -i list.txt -c copy all.wav
ffmpeg -i all.wav -c:a libmp3lame all.mp3
ffprobe -v error -show_entries format=duration:stream=codec_name,sample_rate,channels -of default=nw=1 all.wav
ffmpeg -v error -i all.wav -f null -
ffmpeg -i all.wav -af silencedetect=n=-40dB:d=0.5 -f null -
```

For older headerless Gemini PCM use `ffmpeg -f s16le -ar 24000 -ac 1 -i in.pcm out.wav`.

#### Errors and implementation

Exit 1 means bad arguments before submission, 2 AICC/provider failure, 3 no route, 4 timeout, 5 file I/O. Change the rejected option on provider 400. For `resource_not_permitted`, `quota_exhausted` or `quota_budget_exceeded`, report the owner's access or quota issue (`ai_quota` shows rules).

The container ships `commands/text_to_speech.ts` and `lib/{aicc,io,bucket}.ts` under `/opt/buckyos/bin/opendan/buckyos-agent`. Nodes ship them under `$BUCKYOS_ROOT/libexec/buckyos-agent`. Use the installed `deno.json` and SDK `aicc_client.d.ts` for derived scripts in the session directory or `$OPENDAN_AGENT_ROOT/tools/`. Grant `--allow-run` if invoking ffmpeg. `uploadNamed` stores bytes in `artifacts` or the seven-day `scratch` bucket.

BuckyOS 0.7.0 DV (2026-10-17) exercised Gemini TTS, WAV wrapping and concat. Music, enhancement and ASR were not run there. Bucket delivery was unit-tested on 2026-10-26.
