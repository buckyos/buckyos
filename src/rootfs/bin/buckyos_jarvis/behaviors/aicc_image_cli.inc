## AICC Image CLI

The AIOS runtime provides image commands on `PATH`. Use them through `exec_bash` for image tasks.

### Tool selection

- To modify, restyle, or add something to an existing image, you MUST use `edit_image` as the first choice.
- To create a new image from text, you MUST use `gen_image` as the first choice.
- Use `inpaint_image` when the user supplies a mask, `upscale_image` for resolution enhancement, and `remove_bg` for background removal.
- Do not replace an applicable AICC image command with hand-written SVG, ImageMagick drawing, or Python compositing merely because those utilities are available.

### Inputs and invocation

- Run `command -v <command>` once if availability is uncertain. Do not repeatedly probe an available command.
- For a message attachment whose source type is `named_object`, pass `named_object:<obj_id>` as the input image.
- For a URL attachment, pass its URL. A local file path and a `data:` URL are also accepted.
- Write results to the current workspace when one is available; otherwise use a session-local output path.
- Quote the edit instruction as one argument. Typical invocations are:

```bash
edit_image 'named_object:<obj_id>' 'Add a large bumblebee to the center of the flower.' './flower-with-bumblebee.png'
gen_image 'A precise product photo of a matte black desk lamp.' './desk-lamp.png'
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

- Treat the command as successful only when its `AgentToolResult` reports success and the output file exists.
- Return every requested image with `<attachment path="..." title="..." />`; mentioning a path in prose is not delivery.
- If the selected AICC command is missing or fails, inspect and report its structured error. Use a fallback only when the AICC capability is unavailable or the command cannot perform the requested operation, and state that fallback explicitly.
