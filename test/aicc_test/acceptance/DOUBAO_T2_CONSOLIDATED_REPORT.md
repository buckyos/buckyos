# AICC T1 / T1.5 / 豆包语音 T2 最终测试报告

测试日期：2026-10-05

测试环境：Ubuntu，BuckyOS `/opt/buckyos`。T1、T1.5 为全量测试；T2 仅覆盖 `doubao-speech`。凭证未写入报告。

## 最终结论

| 层级 | 覆盖范围 | 最终结果 |
|---|---|---:|
| T1 | AICC 全量静态、路由与配置契约 | 178 / 178 通过 |
| T1.5 | 全 Provider 高保真协议矩阵 | 1605 / 1605 通过 |
| T2 | 豆包语音 3 个非 ICL 物理模型 | 2 通过 / 1 环境失败 |

T1 与 T1.5 全量通过。T2 的 TTS 与极速 ASR 真实调用通过；标准 ASR 因 devtest 无法向豆包服务端提供可下载的公网音频 URL 而失败，不属于 AICC Provider 协议缺陷。

## T2 最终结果

| 模型 | API | 最终状态 | 结果证据 |
|---|---|---|---|
| `doubao-seed-tts-2.0` | `audio.tts` | 通过 | 真实任务成功；生成 30,765 字节 `audio/mpeg` 制品，SHA-256 为 `40b466505b073316ab5d9f2c6ea07636207c3eb575dd2aa48cde14a49ad9a62b`；请求数、字符数及人民币成本均完成归因。自动报告保留语义人工复核标记。 |
| `doubao-seed-asr-2.0-fast` | `audio.asr` | 通过 | 真实任务成功；转写为“今天的测试编号是4827。”，命中测试标记 `4827`；音频时长、请求数及人民币成本均完成归因。 |
| `doubao-seed-asr-2.0` | `audio.asr` | 环境失败 | Provider 返回 `45000006`：`[Invalid audio URI] ... audio download failed`。devtest 无法向外部 Provider 提供音频下载服务。 |

`doubao-seed-icl-2.0` 已从本轮范围排除。克隆音色属于需要单独设计的完整新功能，后续应统一设计音色资源、配置、协议参数和 AICC 接口后再纳入测试。

## 调用、费用与清理

- T2 计划 3 次、实际 3 次真实调用，无重试或额外 Judge 调用。
- 成功任务持久化计费记录：极速 ASR `¥0.00441625`，TTS `¥0.012`。标准 ASR 失败任务没有可核验的持久化金额。
- T1、T1.5、T2 cleanup 均通过；T2 临时 Provider 凭据已恢复，运行器记录的生成制品已清理。

## 最终报告证据

- T1：[`reports/acceptance/aicc-t1-2026-10-05T10-54-46-298Z-9630db6c/summary.json`](reports/acceptance/aicc-t1-2026-10-05T10-54-46-298Z-9630db6c/summary.json)
- T1.5：[`../reports/acceptance/t15-20261005115443-3294721/summary.json`](../reports/acceptance/t15-20261005115443-3294721/summary.json)
- T2：[`../reports/acceptance/aicc-2026-10-05T12-20-39-830Z-62047f7f/summary.json`](../reports/acceptance/aicc-2026-10-05T12-20-39-830Z-62047f7f/summary.json)

## 未通过项与风险

- 标准 ASR 需要豆包服务端可访问的公网 HTTPS 音频 URL；当前 devtest 环境不具备该条件，因此该项维持环境失败。
- TTS 自动报告的协议、任务、制品和计费断言已通过，但未启用 Judge，语音内容与听感质量仍保留人工复核风险。
- ICL 音色复刻能力未设计、未实现、未测试。
