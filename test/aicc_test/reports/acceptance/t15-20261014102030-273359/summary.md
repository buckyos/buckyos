# AICC E2E Acceptance Report

- Run: `t15-20261014102030-273359`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `official-provider-protocols-2026-10-13.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=781, failed=5, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 785/1334 (58.85%); passed=781, failed=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation --case t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.success --case t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.ta[REDACTED] --case t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async --case t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.typesafe.typesafe.systemone.v1.decision.success | T1.5 | typesafe/t15-20261014102030-273359-typesafe | jev-1.13.0@t15-20261014102030-273359-typesafe | decision | passed |
| t1.5.typesafe.typesafe.systemone.v1.decision.error.auth | T1.5 | typesafe/t15-20261014102030-273359-typesafe | jev-1.13.0@t15-20261014102030-273359-typesafe | decision | passed |
| t1.5.typesafe.typesafe.systemone.v1.decision.error.rate_limit | T1.5 | typesafe/t15-20261014102030-273359-typesafe | jev-1.13.0@t15-20261014102030-273359-typesafe | decision | passed |
| t1.5.typesafe.typesafe.systemone.v1.decision.error.server_error | T1.5 | typesafe/t15-20261014102030-273359-typesafe | jev-1.13.0@t15-20261014102030-273359-typesafe | decision | passed |
| t1.5.typesafe.typesafe.systemone.v1.decision.response.malformed_response | T1.5 | typesafe/t15-20261014102030-273359-typesafe | jev-1.13.0@t15-20261014102030-273359-typesafe | decision | passed |
| t1.5.typesafe.typesafe.systemone.v1.decision.response.wrong_content_type | T1.5 | typesafe/t15-20261014102030-273359-typesafe | jev-1.13.0@t15-20261014102030-273359-typesafe | decision | passed |
| t1.5.typesafe.typesafe.systemone.v1.decision.response.missing_required_response_field | T1.5 | typesafe/t15-20261014102030-273359-typesafe | jev-1.13.0@t15-20261014102030-273359-typesafe | decision | passed |
| t1.5.openai.openai.responses.v1.llm.success | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.inline-artifact-provider-state | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.stream | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.stream-interrupted | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.permission | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai | gpt-5.6@t15-20261014102030-273359-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.vision.caption.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | vision.caption | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.permission | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai-isolation-1 | gpt-5.6@t15-20261014102030-273359-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | gpt-5.6@t15-20261014102030-273359-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | gpt-5.6@t15-20261014102030-273359-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | gpt-5.6@t15-20261014102030-273359-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | gpt-5.6@t15-20261014102030-273359-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | gpt-5.6@t15-20261014102030-273359-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.permission | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai-isolation-2 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | text-embedding-3-small@t15-20261014102030-273359-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | gpt-image-2@t15-20261014102030-273359-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.ta[REDACTED] | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | gpt-image-2@t15-20261014102030-273359-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | gpt-image-2@t15-20261014102030-273359-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | gpt-image-2@t15-20261014102030-273359-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.permission | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | gpt-image-2@t15-20261014102030-273359-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai-isolation-3 | gpt-image-2@t15-20261014102030-273359-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.ta[REDACTED] | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.permission | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai-isolation-4 | gpt-image-2@t15-20261014102030-273359-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-image-2@t15-20261014102030-273359-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-image-2@t15-20261014102030-273359-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-image-2@t15-20261014102030-273359-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-image-2@t15-20261014102030-273359-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-image-2@t15-20261014102030-273359-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-image-2@t15-20261014102030-273359-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.ta[REDACTED] | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-image-2@t15-20261014102030-273359-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.ta[REDACTED] | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.permission | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai-isolation-5 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-4o-mini-tts@t15-20261014102030-273359-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.permission | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai-isolation-6 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | gpt-transcribe@t15-20261014102030-273359-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.ta[REDACTED] | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.async | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.async-failed | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.async_poll_timeout | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.async_artifact_unavailable | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.error.invalid_request | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.error.authentication | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.error.permission | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.error.not_found | T1.5 | openai/t15-20261014102030-273359-openai-isolation-7 | sora-2@t15-20261014102030-273359-openai-isolation-7 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.error.rate_limit | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | sora-2@t15-20261014102030-273359-openai-isolation-8 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.error.server_error | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | sora-2@t15-20261014102030-273359-openai-isolation-8 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.response.malformed_response | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | sora-2@t15-20261014102030-273359-openai-isolation-8 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.response.wrong_content_type | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | sora-2@t15-20261014102030-273359-openai-isolation-8 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.txt2video.response.missing_required_response_field | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | sora-2@t15-20261014102030-273359-openai-isolation-8 | video.txt2video | passed |
| t1.5.openai.openai.videos.v1.video.img2video.success | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | sora-2@t15-20261014102030-273359-openai-isolation-8 | video.img2video | passed |
| t1.5.openai.openai.videos.v1.video.img2video.ta[REDACTED] | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | sora-2@t15-20261014102030-273359-openai-isolation-8 | video.img2video | passed |
| t1.5.openai.openai.responses.v1.llm.cloud-update | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.history | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.tool-history | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.native-history | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.3-codex:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-mini:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-nano:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.4:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5-pro:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.5:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-luna:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-sol:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6-terra:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-high | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-high@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-low | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-low@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-max | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-max@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-medium | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-medium@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-none | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-none@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | agent.computer_use | passed |
| t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.img2img | passed |
| t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | image.txt2img | passed |
| t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.caption | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-xhigh | T1.5 | openai/t15-20261014102030-273359-openai-isolation-8 | gpt-5.6:reasoning-xhigh@t15-20261014102030-273359-openai-isolation-8 | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.success | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.stream | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.stream-interrupted | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.invalid_request | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.authentication | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.permission | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.rate_limit | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.overloaded | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.response.malformed_response | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.response.wrong_content_type | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.response.missing_required_response_field | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.success | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.success | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.history | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.tool-history | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.structured-output | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-high@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-high@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-high@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-low@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-low@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-low@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-max@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-max@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-max@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-medium@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-medium@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-medium@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-xhigh@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-xhigh@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5-1:reasoning-xhigh@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-high@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-high@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-high@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-low@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-low@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-low@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-max@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-max@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-max@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-medium@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-medium@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-medium@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-xhigh@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-xhigh@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-fable-5:reasoning-xhigh@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-4-5-20251001-reasoning-none | T1.5 | claude/t15-20261014102030-273359-claude | claude-haiku-4-5-20251001:reasoning-none@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-4-5-20251001-reasoning-none | T1.5 | claude/t15-20261014102030-273359-claude | claude-haiku-4-5-20251001:reasoning-none@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-4-5-20251001-reasoning-none | T1.5 | claude/t15-20261014102030-273359-claude | claude-haiku-4-5-20251001:reasoning-none@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-4-5-20251001-reasoning-thinking | T1.5 | claude/t15-20261014102030-273359-claude | claude-haiku-4-5-20251001:reasoning-thinking@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-4-5-20251001-reasoning-thinking | T1.5 | claude/t15-20261014102030-273359-claude | claude-haiku-4-5-20251001:reasoning-thinking@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-4-5-20251001-reasoning-thinking | T1.5 | claude/t15-20261014102030-273359-claude | claude-haiku-4-5-20251001:reasoning-thinking@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-high@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-high@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-high@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-low@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-low@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-low@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-max@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-max@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-max@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-medium@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-medium@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-medium@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-xhigh@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-xhigh@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-opus-5:reasoning-xhigh@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-high@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-high@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-high | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-high@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-low@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-low@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-low | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-low@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-max@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-max@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-max | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-max@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-medium@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-medium@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-medium | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-medium@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-xhigh@t15-20261014102030-273359-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-xhigh@t15-20261014102030-273359-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-xhigh | T1.5 | claude/t15-20261014102030-273359-claude | claude-sonnet-5:reasoning-xhigh@t15-20261014102030-273359-claude | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.inline-artifact-provider-state | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.stream | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.stream-interrupted | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini | gemini-3.5-flash@t15-20261014102030-273359-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-1 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-1 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-1 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.5-transcribe@t15-20261014102030-273359-google-gemini-isolation-2 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-2 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-2 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.txt2img.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.txt2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.img2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.img2img | passed |
| t1.5.google-gemini.gemini.interactions.image.v1beta.image.img2img.generated-artifact-id.same-provider | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-image@t15-20261014102030-273359-google-gemini-isolation-3 | image.img2img | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-3 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-3 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.tts.v1beta.audio.tts.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | gemini-3.1-flash-tts-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.tts | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-4 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-4 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.interactions.music.v1beta.audio.music.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | lyria-3-clip-preview@t15-20261014102030-273359-google-gemini-isolation-5 | audio.music | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-5 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-5 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-5 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-5 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-5 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-5 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-6 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-6 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-6 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-6 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.text.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-6 | embedding.text | passed |
| t1.5.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-embedding-2@t15-20261014102030-273359-google-gemini-isolation-6 | embedding.multimodal | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-6 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-6 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-6 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-6 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-6 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-6 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-6 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-7 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-7 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-7 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-7 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.video2video.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-7 | video.video2video | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.extend.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-7 | video.extend | passed |
| t1.5.google-gemini.gemini.interactions.video.v1beta.video.extend.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | gemini-omni-1.1-flash@t15-20261014102030-273359-google-gemini-isolation-7 | video.extend | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.async | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.async-failed | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.async_poll_timeout | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.invalid_argument | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.authentication | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.permission | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.not_found | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-7 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-7 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.quota | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-8 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.error.unavailable | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-8 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.response.malformed_response | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-8 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.response.wrong_content_type | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-8 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.response.missing_required_response_field | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-8 | video.txt2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.img2video.success | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-8 | video.img2video | passed |
| t1.5.google-gemini.gemini.predict-long-running.v1beta.video.img2video.ta[REDACTED] | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | veo-3.1-generate-preview@t15-20261014102030-273359-google-gemini-isolation-8 | video.img2video | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.history | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.tool-history | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-2.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-2.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-2.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-2.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-2.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-2.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-2.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3-flash-preview-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3-flash-preview-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3-flash-preview-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3-flash-preview-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3-flash-preview-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3-flash-preview-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3-flash-preview:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.1-pro-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.1-pro-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.1-pro-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.1-pro-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.1-pro-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.1-pro-preview-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.1-pro-preview:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-lite-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash-lite:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.5-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.5-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.5-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.5-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.5-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.5-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.5-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.6-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.6-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.6-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.6-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.6-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.6-flash-reasoning-minimal | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.6-flash:reasoning-minimal@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.7-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.7-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.7-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.7-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.7-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.7-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.7-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.7-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.7-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.7-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.7-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.7-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.7-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.7-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.7-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.7-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.7-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.7-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.7-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.8-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.8-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.8-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.8-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.8-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.8-flash-reasoning-high | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-high@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.8-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.8-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.8-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.8-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.8-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.8-flash-reasoning-low | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-low@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.variant.gemini-3.8-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | audio.asr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.llm.variant.gemini-3.8-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.variant.gemini-3.8-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.variant.gemini-3.8-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.variant.gemini-3.8-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.variant.gemini-3.8-flash-reasoning-medium | T1.5 | google-gemini/t15-20261014102030-273359-google-gemini-isolation-8 | gemini-3.8-flash:reasoning-medium@t15-20261014102030-273359-google-gemini-isolation-8 | vision.segment | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.success | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.ta[REDACTED] | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async-failed | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async_poll_timeout | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async_artifact_unavailable | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.async-cancel | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.validation | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.authentication | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.rate_limit | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.error.unavailable | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.malformed_response | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.wrong_content_type | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.esrgan.queue-v1.image.upscale.response.missing_required_response_field | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/esrgan@t15-20261014102030-273359-fal | image.upscale | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.success | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/imageutils/rembg@t15-20261014102030-273359-fal | image.bg_remove | failed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.ta[REDACTED] | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/imageutils/rembg@t15-20261014102030-273359-fal | image.bg_remove | failed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/imageutils/rembg@t15-20261014102030-273359-fal | image.bg_remove | failed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed | T1.5 | fal/t15-20261014102030-273359-fal | fal-ai/imageutils/rembg@t15-20261014102030-273359-fal | image.bg_remove | failed |
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.artifact-id.google-gemini.gemini.interactions.image.v1beta.image.txt2img.to.openai.openai.images.edits.v1.image.img2img.cross-provider
- t1.5.custom.claude.anthropic.messages.2023-06-01.llm.success
- t1.5.custom.claude.anthropic.messages.2023-06-01.vision.caption.success
- t1.5.custom.claude.anthropic.messages.2023-06-01.vision.ocr.success
- t1.5.custom.google-gemini.gemini.embed-content.v1beta.embedding.multimodal.success
- t1.5.custom.google-gemini.gemini.embed-content.v1beta.embedding.text.success
- t1.5.custom.google-gemini.gemini.interactions.image.v1beta.image.img2img.success
- t1.5.custom.google-gemini.gemini.interactions.image.v1beta.image.txt2img.success
- t1.5.custom.google-gemini.gemini.interactions.music.v1beta.audio.music.success
- t1.5.custom.google-gemini.gemini.interactions.transcription.v1beta.audio.asr.success
- t1.5.custom.google-gemini.gemini.interactions.tts.v1beta.audio.tts.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.llm.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.caption.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.detect.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.ocr.success
- t1.5.custom.google-gemini.gemini.interactions.v1beta.vision.segment.success
- t1.5.custom.google-gemini.gemini.interactions.video.v1beta.video.extend.success
- t1.5.custom.google-gemini.gemini.interactions.video.v1beta.video.video2video.success
- t1.5.custom.google-gemini.gemini.predict-long-running.v1beta.video.img2video.success
- t1.5.custom.google-gemini.gemini.predict-long-running.v1beta.video.txt2video.success
- t1.5.custom.openai.openai.audio.speech.v1.audio.tts.success
- t1.5.custom.openai.openai.audio.transcriptions.v1.audio.asr.success
- t1.5.custom.openai.openai.embeddings.v1.embedding.text.success
- t1.5.custom.openai.openai.images.edits.v1.image.img2img.success
- t1.5.custom.openai.openai.images.edits.v1.image.inpaint.success
- t1.5.custom.openai.openai.images.generations.v1.image.txt2img.success
- t1.5.custom.openai.openai.responses.computer.v1.agent.computer_use.success
- t1.5.custom.openai.openai.responses.v1.llm.success
- t1.5.custom.openai.openai.responses.v1.vision.caption.success
- t1.5.custom.openai.openai.responses.v1.vision.ocr.success
- t1.5.custom.openai.openai.videos.v1.video.img2video.success
- t1.5.custom.openai.openai.videos.v1.video.txt2video.success
- t1.5.deepseek.deepseek.responses.v1.llm.error.authentication
- t1.5.deepseek.deepseek.responses.v1.llm.error.insufficient_balance
- t1.5.deepseek.deepseek.responses.v1.llm.error.invalid_format
- t1.5.deepseek.deepseek.responses.v1.llm.error.invalid_parameters
- t1.5.deepseek.deepseek.responses.v1.llm.error.overloaded
- t1.5.deepseek.deepseek.responses.v1.llm.error.rate_limit
- t1.5.deepseek.deepseek.responses.v1.llm.response.malformed_response
- t1.5.deepseek.deepseek.responses.v1.llm.response.missing_required_response_field
- t1.5.deepseek.deepseek.responses.v1.llm.response.wrong_content_type
- t1.5.deepseek.deepseek.responses.v1.llm.stream
- t1.5.deepseek.deepseek.responses.v1.llm.stream-interrupted
- t1.5.deepseek.deepseek.responses.v1.llm.success
- t1.5.deepseek.deepseek.responses.v1.vision.caption.success
- t1.5.deepseek.deepseek.responses.v1.vision.ocr.success
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.error.authentication
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.error.invalid_request
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.error.overloaded
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.error.rate_limit
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.error.server_error
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.response.malformed_response
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.response.missing_required_response_field
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.response.wrong_content_type
- t1.5.doubao-agent-plan.doubao-agent-plan.embedding-multimodal.plan-v3.embedding.multimodal.success
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.img2img.success
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.img2img.ta[REDACTED]
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.error.authentication
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.error.invalid_request
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.error.overloaded
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.error.rate_limit
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.error.server_error
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.response.malformed_response
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.response.missing_required_response_field
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.response.wrong_content_type
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.success
- t1.5.doubao-agent-plan.doubao-agent-plan.images.plan-v3.image.txt2img.ta[REDACTED]
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.error.authentication
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.error.invalid_request
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.error.overloaded
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.error.rate_limit
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.error.server_error
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.response.malformed_response
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.response.missing_required_response_field
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.response.wrong_content_type
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.stream
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.stream-interrupted
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.llm.success
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.vision.caption.success
- t1.5.doubao-agent-plan.doubao-agent-plan.responses.plan-v3.vision.ocr.success
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.error.authentication
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.error.invalid_request
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.error.rate_limit
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.error.server_error
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.response.malformed_response
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.response.missing_required_response_field
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.response.wrong_content_type
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.success
- t1.5.doubao-agent-plan.doubao-agent-plan.tts.plan-v3.audio.tts.ta[REDACTED]
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.extend.success
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.extend.ta[REDACTED]
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.img2video.success
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.img2video.ta[REDACTED]
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.async
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.async-cancel
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.async-failed
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.async_poll_timeout
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.error.authentication
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.error.invalid_request
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.error.overloaded
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.error.rate_limit
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.error.server_error
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.response.malformed_response
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.response.missing_required_response_field
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.response.wrong_content_type
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.success
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.txt2video.ta[REDACTED]
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.video2video.success
- t1.5.doubao-agent-plan.doubao-agent-plan.video-tasks.plan-v3.video.video2video.ta[REDACTED]
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.authentication
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.empty_audio
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.invalid_audio_format
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.invalid_request
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.rate_limit
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.server_busy
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.error.server_error
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.response.malformed_response
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.response.missing_required_response_field
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.response.wrong_content_type
- t1.5.doubao-speech.doubao-speech.asr.flash.v3.audio.asr.success
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.async
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.async-failed
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.async_poll_timeout
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.authentication
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.empty_audio
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.invalid_audio_format
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.invalid_request
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.rate_limit
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.server_busy
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.error.server_error
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.response.malformed_response
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.response.missing_required_response_field
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.response.wrong_content_type
- t1.5.doubao-speech.doubao-speech.asr.task.v3.audio.asr.success
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.error.authentication
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.error.invalid_request
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.error.rate_limit
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.error.server_error
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.response.malformed_response
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.response.missing_required_response_field
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.response.wrong_content_type
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.success
- t1.5.doubao-speech.doubao-speech.tts.v3.audio.tts.ta[REDACTED]
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.error.authentication
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.error.invalid_request
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.error.overloaded
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.error.rate_limit
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.error.server_error
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.response.malformed_response
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.response.missing_required_response_field
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.response.wrong_content_type
- t1.5.doubao.doubao.embedding-multimodal.v3.embedding.multimodal.success
- t1.5.doubao.doubao.images.v3.image.img2img.success
- t1.5.doubao.doubao.images.v3.image.img2img.ta[REDACTED]
- t1.5.doubao.doubao.images.v3.image.txt2img.error.authentication
- t1.5.doubao.doubao.images.v3.image.txt2img.error.invalid_request
- t1.5.doubao.doubao.images.v3.image.txt2img.error.overloaded
- t1.5.doubao.doubao.images.v3.image.txt2img.error.rate_limit
- t1.5.doubao.doubao.images.v3.image.txt2img.error.server_error
- t1.5.doubao.doubao.images.v3.image.txt2img.response.malformed_response
- t1.5.doubao.doubao.images.v3.image.txt2img.response.missing_required_response_field
- t1.5.doubao.doubao.images.v3.image.txt2img.response.wrong_content_type
- t1.5.doubao.doubao.images.v3.image.txt2img.success
- t1.5.doubao.doubao.images.v3.image.txt2img.ta[REDACTED]
- t1.5.doubao.doubao.responses.v3.llm.error.authentication
- t1.5.doubao.doubao.responses.v3.llm.error.invalid_request
- t1.5.doubao.doubao.responses.v3.llm.error.overloaded
- t1.5.doubao.doubao.responses.v3.llm.error.rate_limit
- t1.5.doubao.doubao.responses.v3.llm.error.server_error
- t1.5.doubao.doubao.responses.v3.llm.response.malformed_response
- t1.5.doubao.doubao.responses.v3.llm.response.missing_required_response_field
- t1.5.doubao.doubao.responses.v3.llm.response.wrong_content_type
- t1.5.doubao.doubao.responses.v3.llm.stream
- t1.5.doubao.doubao.responses.v3.llm.stream-interrupted
- t1.5.doubao.doubao.responses.v3.llm.success
- t1.5.doubao.doubao.responses.v3.vision.caption.success
- t1.5.doubao.doubao.responses.v3.vision.ocr.success
- t1.5.doubao.doubao.translation.responses.v3.llm.error.authentication
- t1.5.doubao.doubao.translation.responses.v3.llm.error.invalid_request
- t1.5.doubao.doubao.translation.responses.v3.llm.error.overloaded
- t1.5.doubao.doubao.translation.responses.v3.llm.error.rate_limit
- t1.5.doubao.doubao.translation.responses.v3.llm.error.server_error
- t1.5.doubao.doubao.translation.responses.v3.llm.response.malformed_response
- t1.5.doubao.doubao.translation.responses.v3.llm.response.missing_required_response_field
- t1.5.doubao.doubao.translation.responses.v3.llm.response.wrong_content_type
- t1.5.doubao.doubao.translation.responses.v3.llm.success
- t1.5.doubao.doubao.video-tasks.v3.video.extend.success
- t1.5.doubao.doubao.video-tasks.v3.video.extend.ta[REDACTED]
- t1.5.doubao.doubao.video-tasks.v3.video.img2video.success
- t1.5.doubao.doubao.video-tasks.v3.video.img2video.ta[REDACTED]
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.async
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.async-cancel
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.async-failed
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.async_poll_timeout
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.error.authentication
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.error.invalid_request
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.error.overloaded
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.error.rate_limit
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.error.server_error
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.response.malformed_response
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.response.missing_required_response_field
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.response.wrong_content_type
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.success
- t1.5.doubao.doubao.video-tasks.v3.video.txt2video.ta[REDACTED]
- t1.5.doubao.doubao.video-tasks.v3.video.video2video.success
- t1.5.doubao.doubao.video-tasks.v3.video.video2video.ta[REDACTED]
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async-cancel
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async-failed
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async_artifact_unavailable
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.async_poll_timeout
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.authentication
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.rate_limit
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.unavailable
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.error.validation
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.malformed_response
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.missing_required_response_field
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.response.wrong_content_type
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.success
- t1.5.fal.fal.deepfilternet3.queue-v1.audio.enhance.ta[REDACTED]
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-cancel
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async_artifact_unavailable
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async_poll_timeout
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.authentication
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.rate_limit
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.unavailable
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.error.validation
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.malformed_response
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.missing_required_response_field
- t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.response.wrong_content_type
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async-cancel
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async-failed
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async_artifact_unavailable
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.async_poll_timeout
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.authentication
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.rate_limit
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.unavailable
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.error.validation
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.malformed_response
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.missing_required_response_field
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.response.wrong_content_type
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.success
- t1.5.fal.fal.video-upscaler.queue-v1.video.upscale.ta[REDACTED]
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.authentication
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.insufficient_balance
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.invalid_parameters
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.overloaded
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.rate_limit
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.server_error
- t1.5.glm.glm.audio.speech.v4.audio.tts.response.malformed_response
- t1.5.glm.glm.audio.speech.v4.audio.tts.response.missing_required_response_field
- t1.5.glm.glm.audio.speech.v4.audio.tts.response.wrong_content_type
- t1.5.glm.glm.audio.speech.v4.audio.tts.success
- t1.5.glm.glm.audio.speech.v4.audio.tts.ta[REDACTED]
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.authentication
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.insufficient_balance
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.invalid_parameters
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.overloaded
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.rate_limit
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.server_error
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.response.malformed_response
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.response.missing_required_response_field
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.response.wrong_content_type
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.success
- t1.5.glm.glm.chat-completions.v4.llm.error.authentication
- t1.5.glm.glm.chat-completions.v4.llm.error.insufficient_balance
- t1.5.glm.glm.chat-completions.v4.llm.error.invalid_parameters
- t1.5.glm.glm.chat-completions.v4.llm.error.overloaded
- t1.5.glm.glm.chat-completions.v4.llm.error.rate_limit
- t1.5.glm.glm.chat-completions.v4.llm.error.server_error
- t1.5.glm.glm.chat-completions.v4.llm.response.malformed_response
- t1.5.glm.glm.chat-completions.v4.llm.response.missing_required_response_field
- t1.5.glm.glm.chat-completions.v4.llm.response.wrong_content_type
- t1.5.glm.glm.chat-completions.v4.llm.stream
- t1.5.glm.glm.chat-completions.v4.llm.stream-interrupted
- t1.5.glm.glm.chat-completions.v4.llm.success
- t1.5.glm.glm.chat-completions.v4.vision.caption.success
- t1.5.glm.glm.chat-completions.v4.vision.ocr.success
- t1.5.glm.glm.embeddings.v4.embedding.text.error.authentication
- t1.5.glm.glm.embeddings.v4.embedding.text.error.insufficient_balance
- t1.5.glm.glm.embeddings.v4.embedding.text.error.invalid_parameters
- t1.5.glm.glm.embeddings.v4.embedding.text.error.overloaded
- t1.5.glm.glm.embeddings.v4.embedding.text.error.rate_limit
- t1.5.glm.glm.embeddings.v4.embedding.text.error.server_error
- t1.5.glm.glm.embeddings.v4.embedding.text.response.malformed_response
- t1.5.glm.glm.embeddings.v4.embedding.text.response.missing_required_response_field
- t1.5.glm.glm.embeddings.v4.embedding.text.response.wrong_content_type
- t1.5.glm.glm.embeddings.v4.embedding.text.success
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.authentication
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.insufficient_balance
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.invalid_parameters
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.overloaded
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.rate_limit
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.server_error
- t1.5.glm.glm.images.generations.v4.image.txt2img.response.malformed_response
- t1.5.glm.glm.images.generations.v4.image.txt2img.response.missing_required_response_field
- t1.5.glm.glm.images.generations.v4.image.txt2img.response.wrong_content_type
- t1.5.glm.glm.images.generations.v4.image.txt2img.success
- t1.5.glm.glm.images.generations.v4.image.txt2img.ta[REDACTED]
- t1.5.glm.glm.videos.generations.v4.video.img2video.success
- t1.5.glm.glm.videos.generations.v4.video.img2video.ta[REDACTED]
- t1.5.glm.glm.videos.generations.v4.video.txt2video.async
- t1.5.glm.glm.videos.generations.v4.video.txt2video.async-failed
- t1.5.glm.glm.videos.generations.v4.video.txt2video.async_poll_timeout
- t1.5.glm.glm.videos.generations.v4.video.txt2video.error.authentication
- t1.5.glm.glm.videos.generations.v4.video.txt2video.error.insufficient_balance
- t1.5.glm.glm.videos.generations.v4.video.txt2video.error.invalid_parameters
- t1.5.glm.glm.videos.generations.v4.video.txt2video.error.overloaded
- t1.5.glm.glm.videos.generations.v4.video.txt2video.error.rate_limit
- t1.5.glm.glm.videos.generations.v4.video.txt2video.error.server_error
- t1.5.glm.glm.videos.generations.v4.video.txt2video.response.malformed_response
- t1.5.glm.glm.videos.generations.v4.video.txt2video.response.missing_required_response_field
- t1.5.glm.glm.videos.generations.v4.video.txt2video.response.wrong_content_type
- t1.5.glm.glm.videos.generations.v4.video.txt2video.success
- t1.5.glm.glm.videos.generations.v4.video.txt2video.ta[REDACTED]
- t1.5.kimi.kimi.chat-completions.v1.llm.error.authentication
- t1.5.kimi.kimi.chat-completions.v1.llm.error.invalid_request
- t1.5.kimi.kimi.chat-completions.v1.llm.error.not_found
- t1.5.kimi.kimi.chat-completions.v1.llm.error.permission
- t1.5.kimi.kimi.chat-completions.v1.llm.error.rate_limit
- t1.5.kimi.kimi.chat-completions.v1.llm.error.server_error
- t1.5.kimi.kimi.chat-completions.v1.llm.response.malformed_response
- t1.5.kimi.kimi.chat-completions.v1.llm.response.missing_required_response_field
- t1.5.kimi.kimi.chat-completions.v1.llm.response.wrong_content_type
- t1.5.kimi.kimi.chat-completions.v1.llm.stream
- t1.5.kimi.kimi.chat-completions.v1.llm.stream-interrupted
- t1.5.kimi.kimi.chat-completions.v1.llm.success
- t1.5.kimi.kimi.chat-completions.v1.vision.caption.success
- t1.5.kimi.kimi.chat-completions.v1.vision.ocr.success
- t1.5.minimax.minimax.anthropic.messages.v1.llm.error.authentication
- t1.5.minimax.minimax.anthropic.messages.v1.llm.error.content_policy
- t1.5.minimax.minimax.anthropic.messages.v1.llm.error.insufficient_balance
- t1.5.minimax.minimax.anthropic.messages.v1.llm.error.invalid_parameters
- t1.5.minimax.minimax.anthropic.messages.v1.llm.error.rate_limit
- t1.5.minimax.minimax.anthropic.messages.v1.llm.error.server_error
- t1.5.minimax.minimax.anthropic.messages.v1.llm.error.timeout
- t1.5.minimax.minimax.anthropic.messages.v1.llm.response.malformed_response
- t1.5.minimax.minimax.anthropic.messages.v1.llm.response.missing_required_response_field
- t1.5.minimax.minimax.anthropic.messages.v1.llm.response.wrong_content_type
- t1.5.minimax.minimax.anthropic.messages.v1.llm.stream
- t1.5.minimax.minimax.anthropic.messages.v1.llm.stream-interrupted
- t1.5.minimax.minimax.anthropic.messages.v1.llm.success
- t1.5.minimax.minimax.anthropic.messages.v1.vision.caption.success
- t1.5.minimax.minimax.anthropic.messages.v1.vision.ocr.success
- t1.5.minimax.minimax.image-generation.v1.image.img2img.success
- t1.5.minimax.minimax.image-generation.v1.image.img2img.ta[REDACTED]
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.authentication
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.content_policy
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.insufficient_balance
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.invalid_parameters
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.rate_limit
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.server_error
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.error.timeout
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.response.malformed_response
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.response.missing_required_response_field
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.response.wrong_content_type
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.success
- t1.5.minimax.minimax.image-generation.v1.image.txt2img.ta[REDACTED]
- t1.5.minimax.minimax.music-generation.v1.audio.music.error.authentication
- t1.5.minimax.minimax.music-generation.v1.audio.music.error.content_policy
- t1.5.minimax.minimax.music-generation.v1.audio.music.error.insufficient_balance
- t1.5.minimax.minimax.music-generation.v1.audio.music.error.invalid_parameters
- t1.5.minimax.minimax.music-generation.v1.audio.music.error.rate_limit
- t1.5.minimax.minimax.music-generation.v1.audio.music.error.server_error
- t1.5.minimax.minimax.music-generation.v1.audio.music.error.timeout
- t1.5.minimax.minimax.music-generation.v1.audio.music.response.malformed_response
- t1.5.minimax.minimax.music-generation.v1.audio.music.response.missing_required_response_field
- t1.5.minimax.minimax.music-generation.v1.audio.music.response.wrong_content_type
- t1.5.minimax.minimax.music-generation.v1.audio.music.success
- t1.5.minimax.minimax.music-generation.v1.audio.music.ta[REDACTED]
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.authentication
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.content_policy
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.insufficient_balance
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.invalid_parameters
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.rate_limit
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.server_error
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.error.timeout
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.response.malformed_response
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.response.missing_required_response_field
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.response.wrong_content_type
- t1.5.minimax.minimax.speech-to-text.v1.audio.asr.success
- t1.5.minimax.minimax.t2a.v2.audio.tts.error.authentication
- t1.5.minimax.minimax.t2a.v2.audio.tts.error.content_policy
- t1.5.minimax.minimax.t2a.v2.audio.tts.error.insufficient_balance
- t1.5.minimax.minimax.t2a.v2.audio.tts.error.invalid_parameters
- t1.5.minimax.minimax.t2a.v2.audio.tts.error.rate_limit
- t1.5.minimax.minimax.t2a.v2.audio.tts.error.server_error
- t1.5.minimax.minimax.t2a.v2.audio.tts.error.timeout
- t1.5.minimax.minimax.t2a.v2.audio.tts.response.malformed_response
- t1.5.minimax.minimax.t2a.v2.audio.tts.response.missing_required_response_field
- t1.5.minimax.minimax.t2a.v2.audio.tts.response.wrong_content_type
- t1.5.minimax.minimax.t2a.v2.audio.tts.success
- t1.5.minimax.minimax.t2a.v2.audio.tts.ta[REDACTED]
- t1.5.minimax.minimax.video-generation.v2.video.img2video.success
- t1.5.minimax.minimax.video-generation.v2.video.img2video.ta[REDACTED]
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.async
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.async-cancel
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.async-failed
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.async_poll_timeout
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.authentication
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.content_policy
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.insufficient_balance
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.invalid_parameters
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.rate_limit
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.server_error
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.error.timeout
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.response.malformed_response
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.response.missing_required_response_field
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.response.wrong_content_type
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.success
- t1.5.minimax.minimax.video-generation.v2.video.txt2video.ta[REDACTED]
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.authentication
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.invalid_request
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.permission
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.provider_error
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.rate_limit
- t1.5.openrouter.openrouter.decisions.alpha.decision.response.malformed_response
- t1.5.openrouter.openrouter.decisions.alpha.decision.response.missing_required_response_field
- t1.5.openrouter.openrouter.decisions.alpha.decision.response.wrong_content_type
- t1.5.openrouter.openrouter.decisions.alpha.decision.success
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.authentication
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.invalid_request
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.permission
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.provider_error
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.rate_limit
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.response.malformed_response
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.response.missing_required_response_field
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.response.wrong_content_type
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.success
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.authentication
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.invalid_request
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.permission
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.provider_error
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.rate_limit
- t1.5.openrouter.openrouter.rerank.v1.rerank.response.malformed_response
- t1.5.openrouter.openrouter.rerank.v1.rerank.response.missing_required_response_field
- t1.5.openrouter.openrouter.rerank.v1.rerank.response.wrong_content_type
- t1.5.openrouter.openrouter.rerank.v1.rerank.success
- t1.5.openrouter.openrouter.responses.v1.llm.error.authentication
- t1.5.openrouter.openrouter.responses.v1.llm.error.invalid_request
- t1.5.openrouter.openrouter.responses.v1.llm.error.permission
- t1.5.openrouter.openrouter.responses.v1.llm.error.provider_error
- t1.5.openrouter.openrouter.responses.v1.llm.error.rate_limit
- t1.5.openrouter.openrouter.responses.v1.llm.history
- t1.5.openrouter.openrouter.responses.v1.llm.native-history
- t1.5.openrouter.openrouter.responses.v1.llm.response.malformed_response
- t1.5.openrouter.openrouter.responses.v1.llm.response.missing_required_response_field
- t1.5.openrouter.openrouter.responses.v1.llm.response.wrong_content_type
- t1.5.openrouter.openrouter.responses.v1.llm.stream
- t1.5.openrouter.openrouter.responses.v1.llm.stream-interrupted
- t1.5.openrouter.openrouter.responses.v1.llm.success
- t1.5.openrouter.openrouter.responses.v1.llm.tool-history
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.authentication
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.invalid_request
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.not_found
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.permission
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.rate_limit
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.server_error
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.malformed_response
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.missing_required_response_field
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.wrong_content_type
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream-interrupted
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.success
- t1.5.qwen.qwen.chat-completions.compatible-v1.vision.caption.success
- t1.5.qwen.qwen.chat-completions.compatible-v1.vision.ocr.success
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.permission
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.success
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.ta[REDACTED]
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async-failed
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_poll_timeout
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.permission
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.success
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.ta[REDACTED]
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async-failed
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.async_poll_timeout
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.authentication
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.invalid_request
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.not_found
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.permission
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.rate_limit
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.error.server_error
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.malformed_response
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.success
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.txt2video.ta[REDACTED]
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.authentication
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.invalid_request
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.not_found
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.permission
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.rate_limit
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.server_error
- t1.5.qwen.qwen.responses.compatible-v1.llm.inline-artifact-provider-state
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.malformed_response
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.missing_required_response_field
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.wrong_content_type
- t1.5.qwen.qwen.responses.compatible-v1.llm.stream
- t1.5.qwen.qwen.responses.compatible-v1.llm.stream-interrupted
- t1.5.qwen.qwen.responses.compatible-v1.llm.success
- t1.5.qwen.qwen.responses.compatible-v1.llm.video-input
- t1.5.qwen.qwen.responses.compatible-v1.vision.caption.success
- t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.success
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.authentication
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.insufficient_balance
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.invalid_argument
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.timeout
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.unavailable
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.malformed_response
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.missing_required_response_field
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.wrong_content_type
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.stream
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.stream-interrupted
- t1.5.sn-ai-provider.sn.openai-responses.v1.llm.success
- t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.success
- t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.success
- t1.5.switch.claude.anthropic.messages.2023-06-01.llm.to.google-gemini.gemini.interactions.v1beta.llm
- t1.5.switch.claude.anthropic.messages.2023-06-01.llm.to.openai.openai.responses.v1.llm
- t1.5.switch.claude.anthropic.messages.2023-06-01.llm.to.openrouter.openrouter.responses.v1.llm
- t1.5.switch.google-gemini.gemini.interactions.v1beta.llm.to.claude.anthropic.messages.2023-06-01.llm
- t1.5.switch.google-gemini.gemini.interactions.v1beta.llm.to.openai.openai.responses.v1.llm
- t1.5.switch.google-gemini.gemini.interactions.v1beta.llm.to.openrouter.openrouter.responses.v1.llm
- t1.5.switch.openai.openai.responses.v1.llm.to.claude.anthropic.messages.2023-06-01.llm
- t1.5.switch.openai.openai.responses.v1.llm.to.google-gemini.gemini.interactions.v1beta.llm
- t1.5.switch.openai.openai.responses.v1.llm.to.openrouter.openrouter.responses.v1.llm
- t1.5.switch.openrouter.openrouter.responses.v1.llm.to.claude.anthropic.messages.2023-06-01.llm
- t1.5.switch.openrouter.openrouter.responses.v1.llm.to.google-gemini.gemini.interactions.v1beta.llm
- t1.5.switch.openrouter.openrouter.responses.v1.llm.to.openai.openai.responses.v1.llm

## Confirmed product defects

- **defect.aicc.t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.success** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=fal.imageutils-rembg.queue-v1; scenario=success; captured_requests=2; wire contract violations: ["unexpected async request GET /fal-ai/imageutils/requests/fal_mock_1/status"]; Error: task ended Failed: {"code":"invalid_request","detail":{"code":"invalid_request","details":{"request_id":"aicc-18de5ea6d423802d-6bc","retry_after_ms":null},"message":"fal Queue request failed","provider_code":"400"},"message":"fal Queue request failed"}. Evidence: cases/t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.success.json
- **defect.aicc.t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.ta[REDACTED]** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=fal.imageutils-rembg.queue-v1; scenario=success; captured_requests=2; wire contract violations: ["unexpected async request GET /fal-ai/imageutils/requests/fal_mock_1/status"]; Error: task ended Failed: {"code":"invalid_request","detail":{"code":"invalid_request","details":{"request_id":"aicc-18de5ea71e39589a-6be","retry_after_ms":null},"message":"fal Queue request failed","provider_code":"400"},"message":"fal Queue request failed"}. Evidence: cases/t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.ta[REDACTED].json
- **defect.aicc.t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=fal.imageutils-rembg.queue-v1; scenario=async_success; captured_requests=2; wire contract violations: ["unexpected async request GET /fal-ai/imageutils/requests/fal_mock_1/status"]; missing async poll wire request; missing async result wire request; missing async result wire request; Error: task ended Failed: {"code":"invalid_request","detail":{"code":"invalid_request","details":{"request_id":"aicc-18de5ea762bb74ec-6c0","retry_after_ms":null},"message":"fal Queue request failed","provider_code":"400"},"message":"fal Queue request failed"}. Evidence: cases/t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async.json
- **defect.aicc.t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=fal.imageutils-rembg.queue-v1; scenario=async_failed; captured_requests=2; wire contract violations: ["unexpected async request GET /fal-ai/imageutils/requests/fal_mock_1/status"]; missing async poll wire request before failure mapping. Evidence: cases/t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed.json
- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:fal:cases: Error: health recovery failed after t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed: wire contract violations: ["unexpected async request GET /fal-ai/imageutils/requests/fal_mock_1/status"]; Error: task ended Failed: {"code":"invalid_request","detail":{"code":"invalid_request","details":{"request_id":"aicc-18de5ea7f20caab3-6c4","retry_after_ms":null},"message":"fal Queue request failed","provider_code":"400"},"message":"fal Queue request failed"}. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset
