#### Conditions and output

Use `<command> --help` for installed options. A material can be a local file, URL, `obj://bucket/…`, or typed id such as `named_object:cyfile:…`. Reuse prior `detail.files[].url` or `obj_id`. Local bytes are sent inline and can exceed the RPC limit above about 700 KB. AICC measures reference clips/audio before routing.

```bash
gen_video 'Slow push in on @{first}' shots/s1.mp4 --first-frame 'obj://bucket/…' --duration 5 --idempotency-key shot1-v1
gen_video 'The cat from @{cat} jumps over @{fence}' out/jump.mp4 --ref-image cat=in/cat.jpg --ref-image fence=in/fence.jpg --idempotency-key jump-v1
```

- Keyframes use `--first-frame IMG` and/or `--last-frame IMG`. For references repeat `--ref-image ID=IMG`, `--ref-video ID=VIDEO` or `--ref-audio ID=AUDIO`. `--with-audio ID` retains a ref video's own sound. `--clip ID=START-END` currently must span the whole clip. Missing required audio is `resource_invalid`.
- The prompt names materials with `@{ID}` (`first`/`last` for keyframes and `@@` for a literal `@`). Unknown refs fail before submission. H3, H3-Max and Seedance disallow mixing keyframes with references.
- Set `--duration S`, `--aspect W:H`, `--short-edge PX` or `--size WxH`, `--fps N`, `--audio auto|generate|none`, `--format mp4|webm|mov`, `--negative TEXT`, `--seed N` only as the model supports. An unsupported exact-model request can return `unsupported_operation`, or `no_candidate_model` on a logical route.
- `edit_video INPUT PROMPT [out]` takes the same output options. `upscale_video INPUT [out]` accepts `--target-resolution 1080p|4k`, `--denoise`, `--sharpen 0-1`, `--fps`. `extend_video INPUT PROMPT [out]` accepts `--continuation-handle H`, but AICC does not return a provider handle. If native continuation fails, offer a labeled last-frame generation and join (possible visible cut).
- In the Oct 2026 DV catalog H3 provided 2K and always added sound. H3-Max supported 480p/768p with `--audio auto` and could carry an audio track without generating sound. Both took 5–15 s. Check the active Zone catalog for current support and price. Real kRPC calls produced a five-second H3-Max clip for $0.25 at 480p and H3 for $0.64 at 2K; these were not CLI runs.

#### Errors and source

Exit 1 is bad arguments before submission, 2 provider/request failure, 3 no route, 4 timeout, 5 file I/O. `resource_not_permitted` and `quota_*` require an access or budget change. Delivery rejects multiple videos. A provider URL may expire, so use the delivered bucket copy.

`resume_aicc_task` waits up to 900 s by default (`--timeout S`). AICC polls accepted jobs for up to 30 min and does not resubmit if the answer is lost. Some providers cannot cancel an accepted job and can still charge. Reusing a key for different inputs returns `idempotency_conflict`.

Source in the container is `/opt/buckyos/bin/opendan/buckyos-agent/commands/gen_video.ts` with `lib/video.ts` and `lib/aicc.ts`. Nodes use `$BUCKYOS_ROOT/libexec/buckyos-agent`. See the installed `deno.json`/`aicc_client.d.ts` and `doc/aicc/video_generation.md` in the repository. `gen_video`/`edit_video` and bucket delivery had unit coverage; the Oct 2026 DV runs did not exercise CLI editing/upscaling.
