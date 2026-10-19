# AICC E2E Acceptance Report

- Run: `aicc-2026-10-19T03-13-07-483Z-f7c325aa`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.2`
- Real model calls: 0/98
- Planned maximum cost: $0.980000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=127, provider_restricted=0, skipped=98, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.preflight.baseline_mismatch.1 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.2 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.3 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.4 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.5 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.6 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.7 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.8 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.9 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.10 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.11 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.12 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.13 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.14 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.15 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.16 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.17 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.18 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.19 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.20 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.21 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.22 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.23 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.24 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.25 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.26 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.27 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.28 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.29 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.30 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.31 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.32 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.33 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.34 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.35 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.36 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.37 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.38 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.39 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.40 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.41 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.42 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.43 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.44 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.45 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.46 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.47 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.48 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.49 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.50 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.51 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.52 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.53 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.54 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.55 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.56 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.57 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.58 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.59 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.60 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.61 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.62 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.63 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.64 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.65 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.66 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.67 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.68 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.69 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.70 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.71 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.72 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.73 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.74 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.75 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.76 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.77 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.78 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.79 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.80 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.81 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.82 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.83 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.84 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.85 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.86 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.87 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.88 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.89 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.90 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.91 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.92 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.93 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.94 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.95 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.96 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.97 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.98 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.99 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.100 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.101 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.102 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.103 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.104 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.105 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.106 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.107 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.108 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.109 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.110 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.111 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.112 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.113 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.114 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.115 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.116 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.117 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.118 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.119 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.120 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.121 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.122 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.123 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.124 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.125 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.126 | T2 | -/- | - | models.list | failed |
| t2.preflight.baseline_mismatch.127 | T2 | -/- | - | models.list | failed |
| t2.qwen.qwen-t2.cmaas-qwen3.8-flash.llm | T2 | qwen/qwen-t2 | cmaas-qwen3.8-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.deepseek-v4-flash-0731.llm | T2 | qwen/qwen-t2 | deepseek-v4-flash-0731@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.deepseek-v4-pro-0813.llm | T2 | qwen/qwen-t2 | deepseek-v4-pro-0813@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.deepseek-v4-pro.llm | T2 | qwen/qwen-t2 | deepseek-v4-pro@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.deepseek-v4.1-flash-1017.llm | T2 | qwen/qwen-t2 | deepseek-v4.1-flash-1017@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.deepseek-v4.1-flash.llm | T2 | qwen/qwen-t2 | deepseek-v4.1-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.deepseek-v4.1-flash.vision.caption | T2 | qwen/qwen-t2 | deepseek-v4.1-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.deepseek-v4.1-flash.vision.ocr | T2 | qwen/qwen-t2 | deepseek-v4.1-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.glm-5.1.llm | T2 | qwen/qwen-t2 | glm-5.1@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.glm-5.2.llm | T2 | qwen/qwen-t2 | glm-5.2@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.glm-5.3.llm | T2 | qwen/qwen-t2 | glm-5.3@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.glm-5.llm | T2 | qwen/qwen-t2 | glm-5@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-k2.6.llm | T2 | qwen/qwen-t2 | kimi-k2.6@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-k2.6.vision.caption | T2 | qwen/qwen-t2 | kimi-k2.6@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-k2.6.vision.ocr | T2 | qwen/qwen-t2 | kimi-k2.6@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-k2.7-code.llm | T2 | qwen/qwen-t2 | kimi-k2.7-code@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-k2.7-code.vision.caption | T2 | qwen/qwen-t2 | kimi-k2.7-code@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-k2.7-code.vision.ocr | T2 | qwen/qwen-t2 | kimi-k2.7-code@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-k3.llm | T2 | qwen/qwen-t2 | kimi-k3@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-k3.vision.caption | T2 | qwen/qwen-t2 | kimi-k3@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-k3.vision.ocr | T2 | qwen/qwen-t2 | kimi-k3@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.6.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.6@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.6.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k2.6@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.6.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k2.6@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code-highspeed.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code-highspeed.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code-highspeed.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.7-code.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k2.7-code@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k2.8-preview.llm | T2 | qwen/qwen-t2 | kimi/kimi-k2.8-preview@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k3.llm | T2 | qwen/qwen-t2 | kimi/kimi-k3@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k3.vision.caption | T2 | qwen/qwen-t2 | kimi/kimi-k3@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-kimi-k3.vision.ocr | T2 | qwen/qwen-t2 | kimi/kimi-k3@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.minimax-m2.5.llm | T2 | qwen/qwen-t2 | MiniMax-M2.5@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.minimax-minimax-m2.1.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M2.1@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.minimax-minimax-m2.5.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M2.5@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.minimax-minimax-m2.7.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M2.7@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.minimax-minimax-m3.llm | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M3@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.minimax-minimax-m3.vision.caption | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M3@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.minimax-minimax-m3.vision.ocr | T2 | qwen/qwen-t2 | MiniMax/MiniMax-M3@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.img2img | skipped |
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.txt2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.txt2img | skipped |
| t2.qwen.qwen-t2.qwen-image-edit.image.img2img | T2 | qwen/qwen-t2 | qwen-image-edit@qwen-t2 | image.img2img | skipped |
| t2.qwen.qwen-t2.qwen3.5-122b-a10b.llm | T2 | qwen/qwen-t2 | qwen3.5-122b-a10b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-27b.llm | T2 | qwen/qwen-t2 | qwen3.5-27b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-35b-a3b.llm | T2 | qwen/qwen-t2 | qwen3.5-35b-a3b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-397b-a17b.llm | T2 | qwen/qwen-t2 | qwen3.5-397b-a17b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash-2026-02-23.llm | T2 | qwen/qwen-t2 | qwen3.5-flash-2026-02-23@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash.llm | T2 | qwen/qwen-t2 | qwen3.5-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-02-15.llm | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-02-15@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-04-20.llm | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-04-20@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus.llm | T2 | qwen/qwen-t2 | qwen3.5-plus@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-35b-a3b.llm | T2 | qwen/qwen-t2 | qwen3.6-35b-a3b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash-2026-04-16.llm | T2 | qwen/qwen-t2 | qwen3.6-flash-2026-04-16@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash.llm | T2 | qwen/qwen-t2 | qwen3.6-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus-2026-04-02.llm | T2 | qwen/qwen-t2 | qwen3.6-plus-2026-04-02@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus.llm | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus.vision.caption | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus.vision.ocr | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash-2026-07-15.llm | T2 | qwen/qwen-t2 | qwen3.7-flash-2026-07-15@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash.llm | T2 | qwen/qwen-t2 | qwen3.7-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-17.llm | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-17@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-20.llm | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-20@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-06-08.llm | T2 | qwen/qwen-t2 | qwen3.7-max-2026-06-08@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max.llm | T2 | qwen/qwen-t2 | qwen3.7-max@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-plus-2026-05-26.llm | T2 | qwen/qwen-t2 | qwen3.7-plus-2026-05-26@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-plus.llm | T2 | qwen/qwen-t2 | qwen3.7-plus@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-plus.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-plus@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-plus.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-plus@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.8-2.4t-a95b.llm | T2 | qwen/qwen-t2 | qwen3.8-2.4t-a95b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-27b.llm | T2 | qwen/qwen-t2 | qwen3.8-27b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-flash.llm | T2 | qwen/qwen-t2 | qwen3.8-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-flash.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.8-flash.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.8-max-0902.llm | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-max-0902.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.8-max-0902.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.8-max.llm | T2 | qwen/qwen-t2 | qwen3.8-max@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-max.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-max@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.8-max.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-max@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.8-omni-flash.llm | T2 | qwen/qwen-t2 | qwen3.8-omni-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-omni-flash.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-omni-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.8-omni-flash.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-omni-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.vanchin-deepseek-v4-pro-0813.llm | T2 | qwen/qwen-t2 | vanchin/deepseek-v4-pro-0813@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.vanchin-deepseek-v4-pro.llm | T2 | qwen/qwen-t2 | vanchin/deepseek-v4-pro@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.vanchin-deepseek-v4.1-flash.llm | T2 | qwen/qwen-t2 | vanchin/deepseek-v4.1-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.vanchin-deepseek-v4.1-flash.vision.caption | T2 | qwen/qwen-t2 | vanchin/deepseek-v4.1-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.vanchin-deepseek-v4.1-flash.vision.ocr | T2 | qwen/qwen-t2 | vanchin/deepseek-v4.1-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.wan2.6-t2v.video.txt2video | T2 | qwen/qwen-t2 | wan2.6-t2v@qwen-t2 | video.txt2video | skipped |
| t2.qwen.qwen-t2.wanx2.1-t2i-turbo.image.txt2img | T2 | qwen/qwen-t2 | wanx2.1-t2i-turbo@qwen-t2 | image.txt2img | skipped |
| t2.qwen.qwen-t2.zhipu-glm-5.1.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.1@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.zhipu-glm-5.2.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.2@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.zhipu-glm-5.3-flash.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.zhipu-glm-5.3-flash.vision.caption | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.zhipu-glm-5.3-flash.vision.ocr | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.zhipu-glm-5.3.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5.3@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.zhipu-glm-5.llm | T2 | qwen/qwen-t2 | ZHIPU/GLM-5@qwen-t2 | llm | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | cmaas-qwen3.8-flash | cmaas-qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | deepseek-r1-distill-llama-70b | deepseek-r1-distill-llama-70b | included | physical model | - |
| qwen/qwen-t2 | deepseek-r1-distill-qwen-1.5b | deepseek-r1-distill-qwen-1.5b | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-flash-0731 | deepseek-v4-flash-0731 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-pro-0813 | deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-pro | deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4.1-flash-1017 | deepseek-v4.1-flash-1017 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4.1-flash | deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | fun-asr-flash-2026-06-15 | fun-asr-flash-2026-06-15 | included | physical model | - |
| qwen/qwen-t2 | glm-5.1 | glm-5.1 | included | physical model | - |
| qwen/qwen-t2 | glm-5.2-fast-preview | glm-5.2-fast-preview | included | physical model | - |
| qwen/qwen-t2 | glm-5.2 | glm-5.2 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3-prime | glm-5.3-prime | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | gui-plus | gui-plus | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.5 | kimi-k2.5 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.6 | kimi/kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed | kimi/kimi-k2.7-code-highspeed | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code | kimi/kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.8-preview | kimi/kimi-k2.8-preview | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k3 | kimi/kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.1 | MiniMax/MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.5 | MiniMax/MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.7 | MiniMax/MiniMax-M2.7 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M3 | MiniMax/MiniMax-M3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/speech-02-hd | MiniMax/speech-02-hd | included | physical model | - |
| qwen/qwen-t2 | MiniMax/speech-02-turbo | MiniMax/speech-02-turbo | included | physical model | - |
| qwen/qwen-t2 | MiniMax/speech-2.8-hd | MiniMax/speech-2.8-hd | included | physical model | - |
| qwen/qwen-t2 | MiniMax/speech-2.8-turbo | MiniMax/speech-2.8-turbo | included | physical model | - |
| qwen/qwen-t2 | qwen-1.8b-chat | qwen-1.8b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen-1.8b-longcontext-chat | qwen-1.8b-longcontext-chat | included | physical model | - |
| qwen/qwen-t2 | qwen-14b-chat | qwen-14b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen-72b-chat | qwen-72b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen-7b-chat | qwen-7b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen-audio-3.0-asr-flash | qwen-audio-3.0-asr-flash | included | physical model | - |
| qwen/qwen-t2 | qwen-audio-3.0-realtime-flash | qwen-audio-3.0-realtime-flash | included | physical model | - |
| qwen/qwen-t2 | qwen-audio-3.0-realtime-plus | qwen-audio-3.0-realtime-plus | included | physical model | - |
| qwen/qwen-t2 | qwen-audio-3.1-realtime-plus | qwen-audio-3.1-realtime-plus | included | physical model | - |
| qwen/qwen-t2 | qwen-coder-plus-1106 | qwen-coder-plus-1106 | included | physical model | - |
| qwen/qwen-t2 | qwen-deep-research-2025-12-15 | qwen-deep-research-2025-12-15 | included | physical model | - |
| qwen/qwen-t2 | qwen-deep-search-planning | qwen-deep-search-planning | included | physical model | - |
| qwen/qwen-t2 | qwen-flash-character-2026-02-26 | qwen-flash-character-2026-02-26 | included | physical model | - |
| qwen/qwen-t2 | qwen-flash-character | qwen-flash-character | included | physical model | - |
| qwen/qwen-t2 | qwen-flash | qwen-flash | included | physical model | - |
| qwen/qwen-t2 | qwen-image-2.0-2026-03-03 | qwen-image-2.0-2026-03-03 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-2.0-pro-2026-03-03 | qwen-image-2.0-pro-2026-03-03 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-2.0-pro-2026-04-22 | qwen-image-2.0-pro-2026-04-22 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-2.0-pro-2026-06-22 | qwen-image-2.0-pro-2026-06-22 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-2.0-pro | qwen-image-2.0-pro | included | physical model | - |
| qwen/qwen-t2 | qwen-image-2.0 | qwen-image-2.0 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-2.1-pro | qwen-image-2.1-pro | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0 | qwen-image-3.0 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/qwen-t2 | qwen-long | qwen-long | included | physical model | - |
| qwen/qwen-t2 | qwen-max-0107 | qwen-max-0107 | included | physical model | - |
| qwen/qwen-t2 | qwen-max-1201 | qwen-max-1201 | included | physical model | - |
| qwen/qwen-t2 | qwen-max-longcontext | qwen-max-longcontext | included | physical model | - |
| qwen/qwen-t2 | qwen-max | qwen-max | included | physical model | - |
| qwen/qwen-t2 | qwen-mt-flash | qwen-mt-flash | included | physical model | - |
| qwen/qwen-t2 | qwen-mt-lite | qwen-mt-lite | included | physical model | - |
| qwen/qwen-t2 | qwen-mt-plus | qwen-mt-plus | included | physical model | - |
| qwen/qwen-t2 | qwen-mt-uni | qwen-mt-uni | included | physical model | - |
| qwen/qwen-t2 | qwen-plus-2025-01-25 | qwen-plus-2025-01-25 | included | physical model | - |
| qwen/qwen-t2 | qwen-plus-2025-04-28 | qwen-plus-2025-04-28 | included | physical model | - |
| qwen/qwen-t2 | qwen-plus-2025-07-14 | qwen-plus-2025-07-14 | included | physical model | - |
| qwen/qwen-t2 | qwen-plus-2025-11-05 | qwen-plus-2025-11-05 | included | physical model | - |
| qwen/qwen-t2 | qwen-plus-2025-12-01 | qwen-plus-2025-12-01 | included | physical model | - |
| qwen/qwen-t2 | qwen-plus-latest | qwen-plus-latest | filtered | logical_alias | - |
| qwen/qwen-t2 | qwen-plus | qwen-plus | included | physical model | - |
| qwen/qwen-t2 | qwen-turbo-0919 | qwen-turbo-0919 | included | physical model | - |
| qwen/qwen-t2 | qwen-turbo-2024-11-01 | qwen-turbo-2024-11-01 | included | physical model | - |
| qwen/qwen-t2 | qwen-vl-ocr-2025-11-20 | qwen-vl-ocr-2025-11-20 | included | physical model | - |
| qwen/qwen-t2 | qwen-vl-ocr | qwen-vl-ocr | included | physical model | - |
| qwen/qwen-t2 | qwen1.5-0.5b-chat | qwen1.5-0.5b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen1.5-1.8b-chat | qwen1.5-1.8b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen1.5-110b-chat | qwen1.5-110b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen1.5-14b-chat | qwen1.5-14b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen1.5-32b-chat | qwen1.5-32b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen1.5-72b-chat | qwen1.5-72b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen1.5-7b-chat | qwen1.5-7b-chat | included | physical model | - |
| qwen/qwen-t2 | qwen2-0.5b-instruct | qwen2-0.5b-instruct | included | physical model | - |
| qwen/qwen-t2 | qwen2-1.5b-instruct | qwen2-1.5b-instruct | included | physical model | - |
| qwen/qwen-t2 | qwen2-57b-a14b-instruct | qwen2-57b-a14b-instruct | included | physical model | - |
| qwen/qwen-t2 | qwen2-7b-instruct | qwen2-7b-instruct | included | physical model | - |
| qwen/qwen-t2 | qwen2.5-0.5b-instruct | qwen2.5-0.5b-instruct | included | physical model | - |
| qwen/qwen-t2 | qwen2.5-1.5b-instruct | qwen2.5-1.5b-instruct | included | physical model | - |
| qwen/qwen-t2 | qwen2.5-math-1.5b-instruct | qwen2.5-math-1.5b-instruct | included | physical model | - |
| qwen/qwen-t2 | qwen3-asr-flash-realtime | qwen3-asr-flash-realtime | included | physical model | - |
| qwen/qwen-t2 | qwen3-coder-flash | qwen3-coder-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3-livetranslate-flash-2025-12-01 | qwen3-livetranslate-flash-2025-12-01 | included | physical model | - |
| qwen/qwen-t2 | qwen3-livetranslate-flash | qwen3-livetranslate-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3-omni-flash-realtime-2025-09-15 | qwen3-omni-flash-realtime-2025-09-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3-omni-flash | qwen3-omni-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3-s2s-flash-realtime-2025-09-22 | qwen3-s2s-flash-realtime-2025-09-22 | included | physical model | - |
| qwen/qwen-t2 | qwen3-tts-flash-realtime | qwen3-tts-flash-realtime | included | physical model | - |
| qwen/qwen-t2 | qwen3-tts-flash | qwen3-tts-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3-vl-plus | qwen3-vl-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-122b-a10b | qwen3.5-122b-a10b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-27b | qwen3.5-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-35b-a3b | qwen3.5-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-397b-a17b | qwen3.5-397b-a17b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash-2026-02-23 | qwen3.5-flash-2026-02-23 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash | qwen3.5-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-livetranslate-flash-realtime-2026-05-19 | qwen3.5-livetranslate-flash-realtime-2026-05-19 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-livetranslate-flash-realtime | qwen3.5-livetranslate-flash-realtime | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-ocr | qwen3.5-ocr | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-flash-2026-03-15 | qwen3.5-omni-flash-2026-03-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-flash-realtime-2026-03-15 | qwen3.5-omni-flash-realtime-2026-03-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-flash-realtime | qwen3.5-omni-flash-realtime | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-flash | qwen3.5-omni-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-plus-2026-03-15 | qwen3.5-omni-plus-2026-03-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-plus-realtime-2026-03-15 | qwen3.5-omni-plus-realtime-2026-03-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-plus-realtime | qwen3.5-omni-plus-realtime | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-omni-plus | qwen3.5-omni-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-02-15 | qwen3.5-plus-2026-02-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-04-20 | qwen3.5-plus-2026-04-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus | qwen3.5-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-27b | qwen3.6-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-35b-a3b | qwen3.6-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash-2026-04-16 | qwen3.6-flash-2026-04-16 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash | qwen3.6-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus-2026-04-02 | qwen3.6-plus-2026-04-02 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash-2026-07-15 | qwen3.7-flash-2026-07-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash | qwen3.7-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-17 | qwen3.7-max-2026-05-17 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-20 | qwen3.7-max-2026-05-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-06-08 | qwen3.7-max-2026-06-08 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-preview | qwen3.7-max-preview | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max | qwen3.7-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus-2026-05-26 | qwen3.7-plus-2026-05-26 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-text-embedding-flash | qwen3.7-text-embedding-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-text-embedding | qwen3.7-text-embedding | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-text-rerank | qwen3.7-text-rerank | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-2.4t-a95b | qwen3.8-2.4t-a95b | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-27b | qwen3.8-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-livetranslate-flash-realtime | qwen3.8-livetranslate-flash-realtime | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max | qwen3.8-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-omni-flash-realtime | qwen3.8-omni-flash-realtime | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-omni-flash | qwen3.8-omni-flash | included | physical model | - |
| qwen/qwen-t2 | siliconflow/deepseek-r1-0528 | siliconflow/deepseek-r1-0528 | included | physical model | - |
| qwen/qwen-t2 | siliconflow/deepseek-v3-0324 | siliconflow/deepseek-v3-0324 | included | physical model | - |
| qwen/qwen-t2 | siliconflow/deepseek-v3.1-terminus | siliconflow/deepseek-v3.1-terminus | included | physical model | - |
| qwen/qwen-t2 | siliconflow/deepseek-v3.2 | siliconflow/deepseek-v3.2 | included | physical model | - |
| qwen/qwen-t2 | sre-gpu-auto-handle | sre-gpu-auto-handle | included | physical model | - |
| qwen/qwen-t2 | stepfun/step-3.7-flash | stepfun/step-3.7-flash | included | physical model | - |
| qwen/qwen-t2 | stepfun/step-5-preview | stepfun/step-5-preview | included | physical model | - |
| qwen/qwen-t2 | test-sre-gpu-auto-handle | test-sre-gpu-auto-handle | included | physical model | - |
| qwen/qwen-t2 | tongyi-xiaomi-analysis-flash | tongyi-xiaomi-analysis-flash | included | physical model | - |
| qwen/qwen-t2 | tongyi-xiaomi-analysis-pro | tongyi-xiaomi-analysis-pro | included | physical model | - |
| qwen/qwen-t2 | unisound/unisound-u2 | unisound/unisound-u2 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-ocr | vanchin/deepseek-ocr | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-r1 | vanchin/deepseek-r1 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v3.1-terminus | vanchin/deepseek-v3.1-terminus | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v3.2-think | vanchin/deepseek-v3.2-think | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v3 | vanchin/deepseek-v3 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro-0813 | vanchin/deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro | vanchin/deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4.1-flash | vanchin/deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |
| qwen/qwen-t2 | wan2.7-image-pro | wan2.7-image-pro | included | physical model | - |
| qwen/qwen-t2 | wan2.7-image | wan2.7-image | included | physical model | - |
| qwen/qwen-t2 | wanx2.1-t2i-turbo | wanx2.1-t2i-turbo | included | physical model | - |
| qwen/qwen-t2 | xiaomi/mimo-v2.5-pro | xiaomi/mimo-v2.5-pro | included | physical model | - |
| qwen/qwen-t2 | xiaomi/mimo-v2.6-pro-ultraspeed | xiaomi/mimo-v2.6-pro-ultraspeed | included | physical model | - |
| qwen/qwen-t2 | z-image-turbo | z-image-turbo | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.1 | ZHIPU/GLM-5.1 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.2 | ZHIPU/GLM-5.2 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash | ZHIPU/GLM-5.3-Flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-FlashX | ZHIPU/GLM-5.3-FlashX | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3 | ZHIPU/GLM-5.3 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5 | ZHIPU/GLM-5 | included | physical model | - |
| qwen/qwen-t2 | cmaas-qwen3.8-flash | cmaas-qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-flash-0731 | deepseek-v4-flash-0731 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-pro-0813 | deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4-pro | deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4.1-flash-1017 | deepseek-v4.1-flash-1017 | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4.1-flash | deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | glm-5.1 | glm-5.1 | included | physical model | - |
| qwen/qwen-t2 | glm-5.2 | glm-5.2 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.6 | kimi/kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code-highspeed | kimi/kimi-k2.7-code-highspeed | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.7-code | kimi/kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k2.8-preview | kimi/kimi-k2.8-preview | included | physical model | - |
| qwen/qwen-t2 | kimi/kimi-k3 | kimi/kimi-k3 | included | physical model | - |
| qwen/qwen-t2 | MiniMax-M2.5 | MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.1 | MiniMax/MiniMax-M2.1 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.5 | MiniMax/MiniMax-M2.5 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M2.7 | MiniMax/MiniMax-M2.7 | included | physical model | - |
| qwen/qwen-t2 | MiniMax/MiniMax-M3 | MiniMax/MiniMax-M3 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/qwen-t2 | qwen-image-edit | qwen-image-edit | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-122b-a10b | qwen3.5-122b-a10b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-27b | qwen3.5-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-35b-a3b | qwen3.5-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-397b-a17b | qwen3.5-397b-a17b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash-2026-02-23 | qwen3.5-flash-2026-02-23 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash | qwen3.5-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-02-15 | qwen3.5-plus-2026-02-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-04-20 | qwen3.5-plus-2026-04-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus | qwen3.5-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-35b-a3b | qwen3.6-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash-2026-04-16 | qwen3.6-flash-2026-04-16 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash | qwen3.6-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus-2026-04-02 | qwen3.6-plus-2026-04-02 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash-2026-07-15 | qwen3.7-flash-2026-07-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash | qwen3.7-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-17 | qwen3.7-max-2026-05-17 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-20 | qwen3.7-max-2026-05-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-06-08 | qwen3.7-max-2026-06-08 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max | qwen3.7-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus-2026-05-26 | qwen3.7-plus-2026-05-26 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-2.4t-a95b | qwen3.8-2.4t-a95b | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-27b | qwen3.8-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max | qwen3.8-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-omni-flash | qwen3.8-omni-flash | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro-0813 | vanchin/deepseek-v4-pro-0813 | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4-pro | vanchin/deepseek-v4-pro | included | physical model | - |
| qwen/qwen-t2 | vanchin/deepseek-v4.1-flash | vanchin/deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | wan2.6-t2v | wan2.6-t2v | included | physical model | - |
| qwen/qwen-t2 | wanx2.1-t2i-turbo | wanx2.1-t2i-turbo | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.1 | ZHIPU/GLM-5.1 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.2 | ZHIPU/GLM-5.2 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3-Flash | ZHIPU/GLM-5.3-Flash | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5.3 | ZHIPU/GLM-5.3 | included | physical model | - |
| qwen/qwen-t2 | ZHIPU/GLM-5 | ZHIPU/GLM-5 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.preflight.baseline_mismatch.1** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/deepseek-r1-distill-llama-70b. Evidence: cases/t2.preflight.baseline_mismatch.1.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.2** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/deepseek-r1-distill-qwen-1.5b. Evidence: cases/t2.preflight.baseline_mismatch.2.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.3** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/fun-asr-flash-2026-06-15. Evidence: cases/t2.preflight.baseline_mismatch.3.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.4** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/glm-5.2-fast-preview. Evidence: cases/t2.preflight.baseline_mismatch.4.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.5** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/glm-5.3-prime. Evidence: cases/t2.preflight.baseline_mismatch.5.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.6** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/gui-plus. Evidence: cases/t2.preflight.baseline_mismatch.6.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.7** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/kimi-k2.5. Evidence: cases/t2.preflight.baseline_mismatch.7.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.8** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/MiniMax/speech-02-hd. Evidence: cases/t2.preflight.baseline_mismatch.8.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.9** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/MiniMax/speech-02-turbo. Evidence: cases/t2.preflight.baseline_mismatch.9.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.10** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/MiniMax/speech-2.8-hd. Evidence: cases/t2.preflight.baseline_mismatch.10.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.11** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/MiniMax/speech-2.8-turbo. Evidence: cases/t2.preflight.baseline_mismatch.11.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.12** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-1.8b-chat. Evidence: cases/t2.preflight.baseline_mismatch.12.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.13** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-1.8b-longcontext-chat. Evidence: cases/t2.preflight.baseline_mismatch.13.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.14** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-14b-chat. Evidence: cases/t2.preflight.baseline_mismatch.14.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.15** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-72b-chat. Evidence: cases/t2.preflight.baseline_mismatch.15.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.16** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-7b-chat. Evidence: cases/t2.preflight.baseline_mismatch.16.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.17** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-audio-3.0-asr-flash. Evidence: cases/t2.preflight.baseline_mismatch.17.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.18** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-audio-3.0-realtime-flash. Evidence: cases/t2.preflight.baseline_mismatch.18.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.19** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-audio-3.0-realtime-plus. Evidence: cases/t2.preflight.baseline_mismatch.19.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.20** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-audio-3.1-realtime-plus. Evidence: cases/t2.preflight.baseline_mismatch.20.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.21** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-coder-plus-1106. Evidence: cases/t2.preflight.baseline_mismatch.21.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.22** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-deep-research-2025-12-15. Evidence: cases/t2.preflight.baseline_mismatch.22.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.23** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-deep-search-planning. Evidence: cases/t2.preflight.baseline_mismatch.23.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.24** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-flash-character-2026-02-26. Evidence: cases/t2.preflight.baseline_mismatch.24.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.25** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-flash-character. Evidence: cases/t2.preflight.baseline_mismatch.25.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.26** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-flash. Evidence: cases/t2.preflight.baseline_mismatch.26.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.27** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-2.0-2026-03-03. Evidence: cases/t2.preflight.baseline_mismatch.27.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.28** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-2.0-pro-2026-03-03. Evidence: cases/t2.preflight.baseline_mismatch.28.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.29** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-2.0-pro-2026-04-22. Evidence: cases/t2.preflight.baseline_mismatch.29.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.30** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-2.0-pro-2026-06-22. Evidence: cases/t2.preflight.baseline_mismatch.30.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.31** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-2.0-pro. Evidence: cases/t2.preflight.baseline_mismatch.31.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.32** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-2.0. Evidence: cases/t2.preflight.baseline_mismatch.32.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.33** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-2.1-pro. Evidence: cases/t2.preflight.baseline_mismatch.33.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.34** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-image-3.0. Evidence: cases/t2.preflight.baseline_mismatch.34.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.35** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-long. Evidence: cases/t2.preflight.baseline_mismatch.35.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.36** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-max-0107. Evidence: cases/t2.preflight.baseline_mismatch.36.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.37** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-max-1201. Evidence: cases/t2.preflight.baseline_mismatch.37.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.38** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-max-longcontext. Evidence: cases/t2.preflight.baseline_mismatch.38.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.39** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-max. Evidence: cases/t2.preflight.baseline_mismatch.39.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.40** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-mt-flash. Evidence: cases/t2.preflight.baseline_mismatch.40.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.41** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-mt-lite. Evidence: cases/t2.preflight.baseline_mismatch.41.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.42** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-mt-plus. Evidence: cases/t2.preflight.baseline_mismatch.42.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.43** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-mt-uni. Evidence: cases/t2.preflight.baseline_mismatch.43.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.44** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-plus-2025-01-25. Evidence: cases/t2.preflight.baseline_mismatch.44.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.45** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-plus-2025-04-28. Evidence: cases/t2.preflight.baseline_mismatch.45.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.46** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-plus-2025-07-14. Evidence: cases/t2.preflight.baseline_mismatch.46.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.47** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-plus-2025-11-05. Evidence: cases/t2.preflight.baseline_mismatch.47.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.48** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-plus-2025-12-01. Evidence: cases/t2.preflight.baseline_mismatch.48.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.49** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-plus. Evidence: cases/t2.preflight.baseline_mismatch.49.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.50** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-turbo-0919. Evidence: cases/t2.preflight.baseline_mismatch.50.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.51** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-turbo-2024-11-01. Evidence: cases/t2.preflight.baseline_mismatch.51.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.52** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-vl-ocr-2025-11-20. Evidence: cases/t2.preflight.baseline_mismatch.52.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.53** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen-vl-ocr. Evidence: cases/t2.preflight.baseline_mismatch.53.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.54** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen1.5-0.5b-chat. Evidence: cases/t2.preflight.baseline_mismatch.54.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.55** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen1.5-1.8b-chat. Evidence: cases/t2.preflight.baseline_mismatch.55.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.56** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen1.5-110b-chat. Evidence: cases/t2.preflight.baseline_mismatch.56.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.57** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen1.5-14b-chat. Evidence: cases/t2.preflight.baseline_mismatch.57.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.58** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen1.5-32b-chat. Evidence: cases/t2.preflight.baseline_mismatch.58.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.59** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen1.5-72b-chat. Evidence: cases/t2.preflight.baseline_mismatch.59.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.60** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen1.5-7b-chat. Evidence: cases/t2.preflight.baseline_mismatch.60.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.61** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen2-0.5b-instruct. Evidence: cases/t2.preflight.baseline_mismatch.61.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.62** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen2-1.5b-instruct. Evidence: cases/t2.preflight.baseline_mismatch.62.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.63** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen2-57b-a14b-instruct. Evidence: cases/t2.preflight.baseline_mismatch.63.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.64** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen2-7b-instruct. Evidence: cases/t2.preflight.baseline_mismatch.64.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.65** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen2.5-0.5b-instruct. Evidence: cases/t2.preflight.baseline_mismatch.65.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.66** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen2.5-1.5b-instruct. Evidence: cases/t2.preflight.baseline_mismatch.66.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.67** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen2.5-math-1.5b-instruct. Evidence: cases/t2.preflight.baseline_mismatch.67.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.68** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-asr-flash-realtime. Evidence: cases/t2.preflight.baseline_mismatch.68.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.69** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-coder-flash. Evidence: cases/t2.preflight.baseline_mismatch.69.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.70** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-livetranslate-flash-2025-12-01. Evidence: cases/t2.preflight.baseline_mismatch.70.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.71** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-livetranslate-flash. Evidence: cases/t2.preflight.baseline_mismatch.71.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.72** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-omni-flash-realtime-2025-09-15. Evidence: cases/t2.preflight.baseline_mismatch.72.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.73** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-omni-flash. Evidence: cases/t2.preflight.baseline_mismatch.73.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.74** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-s2s-flash-realtime-2025-09-22. Evidence: cases/t2.preflight.baseline_mismatch.74.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.75** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-tts-flash-realtime. Evidence: cases/t2.preflight.baseline_mismatch.75.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.76** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-tts-flash. Evidence: cases/t2.preflight.baseline_mismatch.76.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.77** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3-vl-plus. Evidence: cases/t2.preflight.baseline_mismatch.77.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.78** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-livetranslate-flash-realtime-2026-05-19. Evidence: cases/t2.preflight.baseline_mismatch.78.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.79** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-livetranslate-flash-realtime. Evidence: cases/t2.preflight.baseline_mismatch.79.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.80** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-ocr. Evidence: cases/t2.preflight.baseline_mismatch.80.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.81** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-flash-2026-03-15. Evidence: cases/t2.preflight.baseline_mismatch.81.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.82** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-flash-realtime-2026-03-15. Evidence: cases/t2.preflight.baseline_mismatch.82.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.83** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-flash-realtime. Evidence: cases/t2.preflight.baseline_mismatch.83.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.84** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-flash. Evidence: cases/t2.preflight.baseline_mismatch.84.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.85** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-plus-2026-03-15. Evidence: cases/t2.preflight.baseline_mismatch.85.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.86** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-plus-realtime-2026-03-15. Evidence: cases/t2.preflight.baseline_mismatch.86.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.87** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-plus-realtime. Evidence: cases/t2.preflight.baseline_mismatch.87.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.88** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.5-omni-plus. Evidence: cases/t2.preflight.baseline_mismatch.88.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.89** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.6-27b. Evidence: cases/t2.preflight.baseline_mismatch.89.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.90** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.7-max-preview. Evidence: cases/t2.preflight.baseline_mismatch.90.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.91** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.7-text-embedding-flash. Evidence: cases/t2.preflight.baseline_mismatch.91.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.92** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.7-text-embedding. Evidence: cases/t2.preflight.baseline_mismatch.92.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.93** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.7-text-rerank. Evidence: cases/t2.preflight.baseline_mismatch.93.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.94** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.8-livetranslate-flash-realtime. Evidence: cases/t2.preflight.baseline_mismatch.94.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.95** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/qwen3.8-omni-flash-realtime. Evidence: cases/t2.preflight.baseline_mismatch.95.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.96** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/siliconflow/deepseek-r1-0528. Evidence: cases/t2.preflight.baseline_mismatch.96.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.97** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/siliconflow/deepseek-v3-0324. Evidence: cases/t2.preflight.baseline_mismatch.97.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.98** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/siliconflow/deepseek-v3.1-terminus. Evidence: cases/t2.preflight.baseline_mismatch.98.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.99** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/siliconflow/deepseek-v3.2. Evidence: cases/t2.preflight.baseline_mismatch.99.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.100** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/sre-gpu-auto-handle. Evidence: cases/t2.preflight.baseline_mismatch.100.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.101** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/stepfun/step-3.7-flash. Evidence: cases/t2.preflight.baseline_mismatch.101.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.102** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/stepfun/step-5-preview. Evidence: cases/t2.preflight.baseline_mismatch.102.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.103** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/test-sre-gpu-auto-handle. Evidence: cases/t2.preflight.baseline_mismatch.103.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.104** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/tongyi-xiaomi-analysis-flash. Evidence: cases/t2.preflight.baseline_mismatch.104.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.105** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/tongyi-xiaomi-analysis-pro. Evidence: cases/t2.preflight.baseline_mismatch.105.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.106** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/unisound/unisound-u2. Evidence: cases/t2.preflight.baseline_mismatch.106.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.107** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/vanchin/deepseek-ocr. Evidence: cases/t2.preflight.baseline_mismatch.107.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.108** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/vanchin/deepseek-r1. Evidence: cases/t2.preflight.baseline_mismatch.108.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.109** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/vanchin/deepseek-v3.1-terminus. Evidence: cases/t2.preflight.baseline_mismatch.109.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.110** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/vanchin/deepseek-v3.2-think. Evidence: cases/t2.preflight.baseline_mismatch.110.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.111** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/vanchin/deepseek-v3. Evidence: cases/t2.preflight.baseline_mismatch.111.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.112** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/wan2.7-image-pro. Evidence: cases/t2.preflight.baseline_mismatch.112.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.113** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/wan2.7-image. Evidence: cases/t2.preflight.baseline_mismatch.113.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.114** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/xiaomi/mimo-v2.5-pro. Evidence: cases/t2.preflight.baseline_mismatch.114.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.115** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/xiaomi/mimo-v2.6-pro-ultraspeed. Evidence: cases/t2.preflight.baseline_mismatch.115.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.116** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/z-image-turbo. Evidence: cases/t2.preflight.baseline_mismatch.116.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.117** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_supported_but_aicc_missing qwen/ZHIPU/GLM-5.3-FlashX. Evidence: cases/t2.preflight.baseline_mismatch.117.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.118** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised cmaas-qwen3.8-flash@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.118.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.119** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised cmaas-qwen3.8-flash@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.119.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.120** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised deepseek-v4.1-flash-1017@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.120.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.121** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised deepseek-v4.1-flash-1017@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.121.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.122** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi/kimi-k2.8-preview@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.122.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.123** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised kimi/kimi-k2.8-preview@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.123.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.124** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised qwen3.6-plus-2026-04-02@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.124.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.125** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised qwen3.6-plus-2026-04-02@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.125.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.126** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised qwen3.7-plus-2026-05-26@qwen-t2 vision.caption. Evidence: cases/t2.preflight.baseline_mismatch.126.json, effective_baseline.json
- **defect.aicc.t2.preflight.baseline_mismatch.127** (AICC, baseline_mismatch): expected official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent; observed official_not_supported_but_aicc_advertised qwen3.7-plus-2026-05-26@qwen-t2 vision.ocr. Evidence: cases/t2.preflight.baseline_mismatch.127.json, effective_baseline.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)
