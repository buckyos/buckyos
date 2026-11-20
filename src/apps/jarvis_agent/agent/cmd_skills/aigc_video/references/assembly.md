#### Storyboard and assets

Record the budget and, for each shot in `storyboard.json`, id, prompt, keyframe URL/path, narration text/file, duration, idempotency key, task id, status (planned/running/done/failed), output URL/path and cost. Update as each result arrives. Retry only the failed shot with a corrected request and new key. Shots can run as separate shell tasks with distinct keys.

Generate keyframes with `gen_image`/`edit_image`, narration with `text_to_speech`. On models accepting references, reuse `--ref-image hero=…` across shots for character consistency. For start-frame shots use `--first-frame`. For a continuation substitute, extract a frame near the end, generate from it and join. Label this method in the report.

#### Assemble and check

Use a runtime with `ffmpeg`/`ffprobe`; `agent-session run --runtime <id> --cwd <dir> -- ffmpeg …` can run assembly separately. Give a TS wrapper `--allow-run` if it launches ffmpeg. For the examples use 1280×720, 24 fps, common H.264/AAC streams. Compute frames as ceil((narration seconds + 0.5) × 24), duration as frames / 24. A 3.97 s narration needs 108 frames.

```bash
F=108
D=4.5
mkdir -p out
ffmpeg -i s1.mp4 -i n1.wav -filter_complex "[0:v]scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=24,tpad=stop_mode=clone:stop_duration=30,trim=duration=$D[v];[1:a]aresample=48000,apad,atrim=duration=$D[a]" -map "[v]" -map "[a]" -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p -c:a aac -ar 48000 -ac 2 seg1.mp4
```

To retain the source sound under the narration, replace the audio part of that filter (after `[v];`) with `[0:a]aresample=48000,volume=0.3[b];[1:a]aresample=48000[n];[b][n]amix=inputs=2:duration=longest:normalize=0,apad,atrim=duration=$D[a]`. This needs audio in `s1.mp4`.

For an agreed still-image push-in, set `F`/`D` for that shot's narration. Assemble matching streams and add soft subtitles using an SRT timed to the shot lengths.

```bash
ffmpeg -i key2.png -i n2.wav -filter_complex "[0:v]scale=2560:-2,zoompan=z=min(zoom+0.0008\,1.1):x=iw/2-(iw/zoom/2):y=ih/2-(ih/zoom/2):d=$F:s=1280x720:fps=24,setsar=1[v];[1:a]aresample=48000,apad,atrim=duration=$D[a]" -map "[v]" -map "[a]" -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p -c:a aac -ar 48000 -ac 2 seg2.mp4
printf "file %s\n" seg1.mp4 seg2.mp4 > list.txt
ffmpeg -f concat -safe 0 -i list.txt -c copy joined.mp4
ffmpeg -i joined.mp4 -i subs.srt -map 0 -map 1 -c copy -c:s mov_text out/film.mp4
ffmpeg -sseof -0.2 -i s1.mp4 -frames:v 1 -update 1 s1-last.png
```

Burned subtitles use `-vf "subtitles=subs.srt:force_style=FontName=DejaVu Sans"`. For CJK, ext-core 1.3.1+ supplies Noto Sans CJK SC as a fallback. Set `font=Noto Sans CJK SC` explicitly with `drawtext`. Soft subtitles use the player's fonts.

```bash
ffprobe -v error -show_entries format=duration:stream=codec_type,width,height,r_frame_rate,duration -of compact=p=0 out/film.mp4
ffmpeg -v error -i out/film.mp4 -f null -
ffmpeg -i out/film.mp4 -vf 'fps=1,scale=320:-2,tile=4x2' -frames:v 1 sheet.png
```

Check each shot and the final file for requested dimensions, frame rate, audio, and audio/video lengths within about 0.05 s in this recipe. Review the contact sheet with `llm_understand_media` and the final narration with `speech_to_text`.
