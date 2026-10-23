# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T06-23-08-308Z-b2a88825`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-21.3`
- Real model calls: 86/92
- Planned maximum cost: $0.920000
- Known actual / unknown estimated exposure: $0.000000 / $0.920000
- Total exposure / budget: $0.920000 / $30.000000
- Unknown-cost calls: 86; budget exceeded: false
- Results: passed=21, failed=4, provider_restricted=0, skipped=0, not_applicable=0, review=50

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-openai.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-nano.image.img2img --case t2.sn-ai-provider.sn-ai-provider-main.gpt-6-luna.image.img2img --case t2.sn-ai-provider.sn-ai-provider-main.gpt-6-sol.image.img2img --case t2.sn-ai-provider.sn-ai-provider-main.gpt-image-2.image.img2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-main.gpt-4o-mini-transcribe.audio.asr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-4o-mini-transcribe@sn-ai-provider-main | audio.asr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-4o-mini-tts.audio.tts | T2 | sn-ai-provider/sn-ai-provider-main | gpt-4o-mini-tts@sn-ai-provider-main | audio.tts | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-4o-transcribe-diarize.audio.asr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-4o-transcribe-diarize@sn-ai-provider-main | audio.asr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.3-codex.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.3-codex@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.3-codex.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.3-codex@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.3-codex.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.3-codex@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-mini.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-mini@sn-ai-provider-main | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-mini.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-mini@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-mini.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-mini@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-mini.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-mini@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-mini.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-mini@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-nano.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-nano@sn-ai-provider-main | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-nano.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-nano@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-nano.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-nano@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-nano.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-nano@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-nano.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-nano@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-pro.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-pro@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-pro.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-pro@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-pro.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-pro@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4-pro.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4-pro@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4@sn-ai-provider-main | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.4.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.4@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5-pro.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5-pro@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5-pro.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5-pro@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5-pro.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5-pro@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5-pro.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5-pro@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5@sn-ai-provider-main | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.5.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.5@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna@sn-ai-provider-main | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-luna.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna@sn-ai-provider-main | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-luna.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-luna.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-luna.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-luna.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol@sn-ai-provider-main | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol@sn-ai-provider-main | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-sol.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-sol.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-sol.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-sol.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-terra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra@sn-ai-provider-main | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-terra.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra@sn-ai-provider-main | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-terra.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-terra.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-terra.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-5.6-terra.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-astra@sn-ai-provider-main | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-astra.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-astra@sn-ai-provider-main | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-astra.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-astra@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-astra.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-astra@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-astra.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-astra@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-astra.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-astra@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-luna@sn-ai-provider-main | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-luna.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-luna@sn-ai-provider-main | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-luna.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-luna@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-luna.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-luna@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-luna.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-luna@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-luna.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-luna@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-sol@sn-ai-provider-main | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-sol@sn-ai-provider-main | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-sol.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-sol@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-sol.llm | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-sol@sn-ai-provider-main | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-sol.vision.caption | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-sol@sn-ai-provider-main | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-6-sol.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-main | gpt-6-sol@sn-ai-provider-main | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-image-2.image.img2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-image-2@sn-ai-provider-main | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-image-2.image.inpaint | T2 | sn-ai-provider/sn-ai-provider-main | gpt-image-2@sn-ai-provider-main | image.inpaint | review |
| t2.sn-ai-provider.sn-ai-provider-main.gpt-image-2.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-main | gpt-image-2@sn-ai-provider-main | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-main.text-embedding-3-large.embedding.text | T2 | sn-ai-provider/sn-ai-provider-main | text-embedding-3-large@sn-ai-provider-main | embedding.text | passed |
| t2.sn-ai-provider.sn-ai-provider-main.text-embedding-3-small.embedding.text | T2 | sn-ai-provider/sn-ai-provider-main | text-embedding-3-small@sn-ai-provider-main | embedding.text | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-main | gpt-4o-mini-transcribe | gpt-4o-mini-transcribe | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-4o-mini-tts | gpt-4o-mini-tts | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-4o-transcribe-diarize | gpt-4o-transcribe-diarize | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.3-codex | gpt-5.3-codex | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | text-embedding-3-small | text-embedding-3-small | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-4o-mini-transcribe | gpt-4o-mini-transcribe | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-4o-mini-tts | gpt-4o-mini-tts | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-4o-transcribe-diarize | gpt-4o-transcribe-diarize | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.3-codex | gpt-5.3-codex | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-ai-provider-main | text-embedding-3-small | text-embedding-3-small | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 33 generated output object(s)
