# AICC E2E Acceptance Report

- Run: `aicc-2026-10-20T08-53-40-748Z-445172e4`
- Commit: `8fb143d9ba528bf83d77009aef5a852ad0996cda`
- Capability baseline: `2026-10-16.2`
- Real model calls: 5/5
- Planned maximum cost: $0.050000
- Known actual / unknown estimated exposure: $0.000000 / $0.050000
- Total exposure / budget: $0.050000 / $0.100000
- Unknown-cost calls: 5; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=5

## Execution notes

- Tested the current dirty worktree after `uv run buckyos-build.py -s aicc`; the installed AICC binary and the build artifact both had SHA-256 `ad93d9d92ec515dbd6d9971addfa07abf42d79d8914b74cb2d187a06c6f74191`.
- The five real TTS calls completed with readable audio artifacts and passed durable usage/route attribution checks. They remain `review` because no Judge or manual listening was performed.
- These cases use the standard T2 request with default voice. They do not validate an explicitly requested voice style or the native-video status/result usage merge.
- Provider responses did not supply settled cost. USD 0.05 is estimated unknown-cost exposure, not the actual bill.
- Temporary Provider settings were restored and the five generated Named Data objects were removed. The supplied API key was also cleared from the local test configuration after execution.
- The newly built AICC remains installed. The previous executable is preserved at `/opt/buckyos/bin/aicc/aicc.t2-before-20261020-0855`.

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.openrouter-tts.local.toml --allow-credential-mutation --timeout-ms 120000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.openrouter.openrouter-t2.google-gemini-3.1-flash-tts-preview.audio.tts | T2 | openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview@openrouter-t2 | audio.tts | review |
| t2.openrouter.openrouter-t2.google-gemini-3.8-flash-lite-tts.audio.tts | T2 | openrouter/openrouter-t2 | google/gemini-3.8-flash-lite-tts@openrouter-t2 | audio.tts | review |
| t2.openrouter.openrouter-t2.google-gemini-3.8-flash-tts.audio.tts | T2 | openrouter/openrouter-t2 | google/gemini-3.8-flash-tts@openrouter-t2 | audio.tts | review |
| t2.openrouter.openrouter-t2.minimax-speech-2.8-hd.audio.tts | T2 | openrouter/openrouter-t2 | minimax/speech-2.8-hd@openrouter-t2 | audio.tts | review |
| t2.openrouter.openrouter-t2.minimax-speech-2.8-turbo.audio.tts | T2 | openrouter/openrouter-t2 | minimax/speech-2.8-turbo@openrouter-t2 | audio.tts | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview | google/gemini-3.1-flash-tts-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-lite-tts | google/gemini-3.8-flash-lite-tts | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-tts | google/gemini-3.8-flash-tts | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-hd | minimax/speech-2.8-hd | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-turbo | minimax/speech-2.8-turbo | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview | google/gemini-3.1-flash-tts-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-lite-tts | google/gemini-3.8-flash-lite-tts | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-tts | google/gemini-3.8-flash-tts | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-hd | minimax/speech-2.8-hd | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-turbo | minimax/speech-2.8-turbo | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 5 generated output object(s)
- temporary Provider credentials restored after testing (openrouter)
