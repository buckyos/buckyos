## AICC Media CLI

The AIOS runtime provides image, vision, audio, video, and AICC diagnostic commands on `PATH`. Use the matching command through `exec_bash` instead of merely describing how the task could be done.

### Tool selection

- Image generation and editing: `gen_image` creates an image from text; `edit_image` modifies or restyles an existing image; `inpaint_image` edits a masked region; `upscale_image` increases resolution; `remove_bg` removes the background.
- Image understanding: `ocr_image` extracts document text; `caption_image` describes an image; `detect_image` returns detected objects as JSON; `segment_image` returns segmentation data as JSON.
- Audio: `text_to_speech` synthesizes speech; `speech_to_text` transcribes audio; `gen_music` creates music from a prompt; `enhance_audio` cleans or enhances existing audio.
- Video: `gen_video` creates video from text; `img2video` animates an image; `video2video` transforms an existing video; `extend_video` continues a video; `upscale_video` increases video resolution.
- AICC diagnostics: `ai_provider list` lists configured providers; `ai_provider health` checks provider health; `ai_quota` queries quota, optionally filtered by capability or method. Use these for explicit status/quota requests or to diagnose an AICC availability failure, not as a substitute for the requested media command.
- For an applicable media request, the matching AICC command MUST be the first choice. Do not replace it with hand-written SVG, ImageMagick drawing, Python compositing, or a prose-only instruction merely because those alternatives are available.

### Inputs and invocation

- Run `command -v <command>` once if availability is uncertain. Do not repeatedly probe an available command.
- For a message attachment whose source type is `named_object`, pass `named_object:<obj_id>` as the input image.
- For a URL attachment, pass its URL. A local file path and a `data:` URL are also accepted.
- Write results to the current workspace when one is available; otherwise use a session-local output path.
- Quote the edit instruction as one argument. Typical invocations are:

```bash
edit_image 'named_object:<obj_id>' 'Add a large bumblebee to the center of the flower.' './flower-with-bumblebee.png'
gen_image 'A precise product photo of a matte black desk lamp.' './desk-lamp.png'
ocr_image 'named_object:<obj_id>' './recognized-text.txt'
text_to_speech 'Welcome to BuckyOS.' './welcome.wav'
img2video 'named_object:<obj_id>' 'Slow camera push-in with natural motion.' './animated.mp4'
```

Use `<command> --help` when non-default options are needed.

### Reading and extending the tools

- The read-only installed TypeScript sources are under `/opt/buckyos/bin/opendan/buckyos-agent/`.
- Inspect `commands/*.ts` for complete AICC call examples and reuse helpers from `lib/*.ts` when a built-in command does not cover the task.
- Write derived scripts only to the Agent or current Session `tools/` directory. Never modify the installed system copy.
- Run a derived script with the installed configuration so the vendored SDK resolves correctly:

```bash
deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors './tools/my-aicc-tool.ts'
```

### Completion and failure

- Treat the command as successful only when its `AgentToolResult` reports success and every expected output file exists.
- Return every requested image, audio, video, text file, or JSON artifact with `<attachment path="..." title="..." />`; mentioning a path in prose is not delivery. Inline a short text/JSON result as well when that is more useful to the user.
- If the selected AICC command is missing or fails, inspect and report its structured error. Use a fallback only when the AICC capability is unavailable or the command cannot perform the requested operation, and state that fallback explicitly.
