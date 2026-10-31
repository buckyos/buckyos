# AICC E2E Acceptance Report

- Run: `aicc-2026-10-30T09-01-10-151Z-69abbe3b`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 124/147
- Planned maximum cost: $2.940000
- Known actual / unknown estimated exposure: $0.000000 / $2.640000
- Total exposure / budget: $2.640000 / $20.000000
- Unknown-cost calls: 124; budget exceeded: false
- Results: passed=34, failed=43, provider_restricted=0, skipped=0, not_applicable=0, review=48

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-us-full-20261030.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-mini-tts.audio.tts --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.caption --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.ocr --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.vision.ocr --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.txt2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-haiku-5.5.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-haiku-5.5@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-haiku-5.5.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-haiku-5.5@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-haiku-5.5.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-haiku-5.5@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-opus-5.5.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-opus-5.5@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-opus-5.5.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-opus-5.5@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-opus-5.5.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-opus-5.5@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-sonnet-5.5.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-sonnet-5.5@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-sonnet-5.5.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-sonnet-5.5@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-sonnet-5.5.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | claude-sonnet-5.5@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-3.8-flash.audio.asr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-3.8-flash@sn-us-full-20261030-t2 | audio.asr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-3.8-flash.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-3.8-flash@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-3.8-flash.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-3.8-flash@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-3.8-flash.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-3.8-flash@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-4-argon.audio.asr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-4-argon@sn-us-full-20261030-t2 | audio.asr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-4-argon.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-4-argon@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-4-argon.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-4-argon@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-4-argon.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gemini-4-argon@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.glm-5.3-flash.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-flash@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.glm-5.3-flash.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-flash@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.glm-5.3-flash.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-flash@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.glm-5.3.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.glm-5.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | glm-5@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-mini-transcribe.audio.asr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-mini-transcribe@sn-us-full-20261030-t2 | audio.asr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-mini-tts.audio.tts | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-mini-tts@sn-us-full-20261030-t2 | audio.tts | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-transcribe-diarize.audio.asr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-transcribe-diarize@sn-us-full-20261030-t2 | audio.asr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.3-codex.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.3-codex@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.3-codex.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.3-codex@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.3-codex.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.3-codex@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-mini@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-mini@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-mini@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-mini@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-mini@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-nano@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-nano@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-nano@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-nano@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-nano@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro@sn-us-full-20261030-t2 | vision.caption | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro@sn-us-full-20261030-t2 | vision.ocr | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro@sn-us-full-20261030-t2 | vision.ocr | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna@sn-us-full-20261030-t2 | agent.computer_use | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol@sn-us-full-20261030-t2 | agent.computer_use | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra@sn-us-full-20261030-t2 | agent.computer_use | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra@sn-us-full-20261030-t2 | agent.computer_use | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna@sn-us-full-20261030-t2 | agent.computer_use | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol@sn-us-full-20261030-t2 | agent.computer_use | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol@sn-us-full-20261030-t2 | agent.computer_use | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | agent.computer_use | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | vision.ocr | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.inpaint | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare@sn-us-full-20261030-t2 | image.inpaint | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.inpaint | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst@sn-us-full-20261030-t2 | image.inpaint | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.inpaint | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2@sn-us-full-20261030-t2 | image.inpaint | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | grok-4.7@sn-us-full-20261030-t2 | llm | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | grok-4.7@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | grok-4.7@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.1-pro.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3.1-pro@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.1-pro.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3.1-pro@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.1-pro.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3.1-pro@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.llm | T2 | sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3@sn-us-full-20261030-t2 | llm | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.vision.caption | T2 | sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3@sn-us-full-20261030-t2 | vision.caption | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3@sn-us-full-20261030-t2 | vision.ocr | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.text-embedding-3-large.embedding.text | T2 | sn-ai-provider/sn-us-full-20261030-t2 | text-embedding-3-large@sn-us-full-20261030-t2 | embedding.text | passed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.text-embedding-3-small.embedding.text | T2 | sn-ai-provider/sn-us-full-20261030-t2 | text-embedding-3-small@sn-us-full-20261030-t2 | embedding.text | passed |
| t2.cleanup.named_data | T2 | -/- | - | ndm.remove | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-us-full-20261030-t2 | claude-haiku-5.5 | claude-haiku-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | claude-opus-5.5 | claude-opus-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | claude-sonnet-5.5 | claude-sonnet-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-flash | glm-5.3-flash | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-flashx | glm-5.3-flash | filtered | duplicate_physical_model | glm-5.3-flash@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-prime | glm-5.3 | filtered | duplicate_physical_model | glm-5.3@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-mini-transcribe | gpt-4o-mini-transcribe | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-mini-tts | gpt-4o-mini-tts | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-transcribe-diarize | gpt-4o-transcribe-diarize | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.3-codex | gpt-5.3-codex | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol-pro | gpt-6.1-sol | filtered | duplicate_physical_model | gpt-6.1-sol@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol | gpt-6.1-sol | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra-pro | gpt-6.1-terra | filtered | duplicate_physical_model | gpt-6.1-terra@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra | gpt-6.1-terra | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare | gpt-image-2.5-flare | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst | gpt-image-2.5-sunburst | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-realtime-2.1-mini | gpt-realtime-2.1-mini | filtered | unsupported_canonical_protocol | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-realtime-2.1 | gpt-realtime-2.1 | filtered | unsupported_canonical_protocol | - |
| sn-ai-provider/sn-us-full-20261030-t2 | grok-4.7 | grok-4.7 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3.1-pro | kimi-k3.1-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | text-embedding-3-small | text-embedding-3-small | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | claude-haiku-5.5 | claude-haiku-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | claude-opus-5.5 | claude-opus-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | claude-sonnet-5.5 | claude-sonnet-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-flash | glm-5.3-flash | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-flashx | glm-5.3-flash | filtered | duplicate_physical_model | glm-5.3-flash@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3-prime | glm-5.3 | filtered | duplicate_physical_model | glm-5.3@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-mini-transcribe | gpt-4o-mini-transcribe | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-mini-tts | gpt-4o-mini-tts | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-4o-transcribe-diarize | gpt-4o-transcribe-diarize | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.3-codex | gpt-5.3-codex | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol-pro | gpt-6.1-sol | filtered | duplicate_physical_model | gpt-6.1-sol@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-sol | gpt-6.1-sol | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra-pro | gpt-6.1-terra | filtered | duplicate_physical_model | gpt-6.1-terra@sn-us-full-20261030-t2 |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra | gpt-6.1-terra | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare | gpt-image-2.5-flare | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst | gpt-image-2.5-sunburst | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | grok-4.7 | grok-4.7 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3.1-pro | kimi-k3.1-pro | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | text-embedding-3-small | text-embedding-3-small | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-mini-tts.audio.tts** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-mini-tts.audio.tts.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.caption.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.img2img** (AICC, resource_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed TaskFailureError: provider task t-3976c99f87174e63b798515109a2f044 ended Failed: {"code":"internal_error","message":"inline artifact ownership write failed"}. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.inpaint** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.inpaint.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.inpaint** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.inpaint.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.inpaint** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.inpaint.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed NdmProxyApiError: Method not allowed. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: LLM output omitted the requested acceptance marker. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.llm.json

## Cleanup

Status: failed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)
