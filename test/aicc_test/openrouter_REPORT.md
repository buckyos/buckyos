# OpenRouter 验收报告

## 最终汇总

| 层级 | 覆盖 | 最终结果 | Cleanup |
| --- | --- | --- | --- |
| T1.5（历史定向回归） | Google Gemini 与 OpenRouter 的 Argon / Transcribe 目标协议，共 42 项 | 42/42 通过 | 通过 |
| T1.5（2026-10-20） | OpenRouter 全 Provider 范围，含 TTS、视频异步、制品和动态 variant，共 609 项 | 609/609 通过，无产品缺陷；真实模型调用为 0 | 通过 |
| T2（2026-10-16） | `google/gemini-3.5-transcribe` ASR；`google/gemini-4-argon` LLM、Caption、OCR，共 4 项 | 4 项调用成功并待人工语义复核；无协议错误或产品缺陷 | 通过 |
| T2（2026-10-20） | Gemini TTS 3 个模型、MiniMax TTS 2 个模型，共 5 项 | 5 项真实调用及音频制品、持久化 usage/route 归因检查通过；正式状态均为 `review`，等待语义复核 | 通过 |

上述 OpenRouter T2 均未启用 Judge，累计 9 项真实调用成功并待语义复核，不能计为完整 T2 通过。Argon 三项返回的实际费用合计为 USD 0.01018116；Transcribe 与本轮 5 项 TTS 均未返回可归因费用。本轮 TTS 的 USD 0.05 为未知费用的估算敞口，不是实际账单。

此前 Google Gemini Provider 的同步 Interactions 回归不改变 OpenRouter 配置或既有结论；OpenRouter 仍按其 Provider 规则为 `google/gemini-4-argon` 提交 `background=true`。该次 Interactions 回归未新增 OpenRouter 真实调用。

## 2026-10-20：TTS 定向 T2

运行 ID：`aicc-2026-10-20T08-53-40-748Z-445172e4`。通过 Zone Gateway 调用 `openrouter-t2` 实例，使用标准 T2 最小输入 `BuckyOS four eight two seven`，每个模型调用一次，不重试；全局与 Provider 并发均为 1，请求间隔 1000 ms，单例超时 120 秒，最多 5 次真实调用，总预算 USD 0.10。

| 模型 | API | 真实调用与制品检查 | 正式结果 |
| --- | --- | --- | --- |
| `google/gemini-3.1-flash-tts-preview` | `audio.tts` | 成功，音频 159360 字节 | `review` |
| `google/gemini-3.8-flash-lite-tts` | `audio.tts` | 成功，音频 147840 字节 | `review` |
| `google/gemini-3.8-flash-tts` | `audio.tts` | 成功，音频 128640 字节 | `review` |
| `minimax/speech-2.8-hd` | `audio.tts` | 成功，音频 49581 字节 | `review` |
| `minimax/speech-2.8-turbo` | `audio.tts` | 成功，音频 44397 字节 | `review` |

五个模型均经过本轮独立官方目录核对；任务完成并产生可读音频，每项持久化 usage 均记录 `request_units=1`，且 route 归因匹配目标模型与实例。没有失败、Provider 拒绝或跳过。未做 Judge 或人工听审，因此内容正确性仍待复核。

本轮 T2 验证的是默认 voice 的 TTS 链路，不覆盖显式指定 voice style，也不覆盖 native video task 的 status/result usage 合并。显式 voice style 的协议转换另由下述 T1.5 Mock 回归覆盖。

执行前运行 `uv run buckyos-build.py -s aicc`，更新并重启 AICC；构建产物与已安装可执行文件的 SHA-256 均为 `ad93d9d92ec515dbd6d9971addfa07abf42d79d8914b74cb2d187a06c6f74191`。源码为 `8fb143d9ba528bf83d77009aef5a852ad0996cda` 加本地未提交修改，包含 canonical voice 复用、OpenRouter provider metadata revision 12 和 native usage 合并回归测试。

清理通过：临时 Provider settings 与凭据已恢复，运行时临时实例恢复检查通过，5 个生成 Named Data 对象已删除，所提供的 API key 已从本地测试配置清除。新构建 AICC 保留运行，旧可执行文件备份在 `/opt/buckyos/bin/aicc/aicc.t2-before-20261020-0855`。本轮没有新增产品代码修改。

## 2026-10-20：推送前 T1.5 协议回归

运行 ID：`t15-20261020090412-2335299`。通过真实 Zone Gateway、鉴权、AICC 和 TaskMgr，连接 OpenRouter 高保真 Mock，执行全部 OpenRouter 用例与动态展开的 variant。共 609 项，执行覆盖率 100%，609 项全部通过，清理通过，真实模型调用和费用均为 0。

语音契约新增 `voice.style=warm` 输入，严格要求三个 Gemini TTS 模型下发 `voice=Sulafat`，防止模型默认音色覆盖 canonical 转换结果；期望依据 Google 官方语音列表（Sulafat 为 Warm，2026-10-20 核对）。两个 MiniMax TTS 模型也执行同一 canonical 输入并通过各自的模型协议用例。Mock 自检新增拒绝 Gemini 错误下发 `Kore` 的回归。

第一次尝试与另一轮全量 T1 同时运行，在初始化阶段返回 RPC 500，未执行协议用例；等待该 T1 结束后完成上述完整重跑。成功轮次没有失败或未执行项，未通过修改产品实现或放宽 Mock 断言消除失败。

配套验证：`cargo test -p aicc -- --test-threads=1` 为 666/666；新增契约后的 `pnpm run acceptance:self-test` 为 124/124；`cargo fmt -p aicc -- --check` 与 `git diff --check` 通过。此次不涉及 CLI、公共 API schema、运行路径或 Agent 手册的命令变更，`aigc_audio` 等 cmd_skills 无需同步修改。

## 详细报告

- [T1.5 第 1 轮：启动门禁失败](reports/acceptance/gemini-openrouter/t15-20261016160803-453890/summary.md)
- [T1.5 第 2 轮：39 通过，5 失败](reports/acceptance/gemini-openrouter/t15-20261016160923-455797/summary.md)
- [T1.5 最终轮：42/42 通过](reports/acceptance/gemini-openrouter/t15-20261016161641-467190/summary.md)
- [OpenRouter T2：4 项待复核](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-20-24-945Z-1a004d33/summary.md)
- [OpenRouter T2 费用](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-20-24-945Z-1a004d33/finance.md)
- [OpenRouter TTS T2：5 项待复核](reports/acceptance/aicc-2026-10-20T08-53-40-748Z-445172e4/summary.md)
- [OpenRouter TTS T2 费用](reports/acceptance/aicc-2026-10-20T08-53-40-748Z-445172e4/finance.md)
- [OpenRouter T1.5：609/609 通过](reports/acceptance/t15-20261020090412-2335299/summary.md)
- [OpenRouter T1.5 零费用报告](reports/acceptance/t15-20261020090412-2335299/finance.md)
