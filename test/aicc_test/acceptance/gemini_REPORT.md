# AICC Gemini / Gemma 修复最终测试报告

测试日期：2026-10-17

测试环境：Ubuntu，BuckyOS `/opt/buckyos`。凭证未写入报告。

## 最终汇总

| 层级 | 覆盖范围 | 最终结果 |
|---|---|---:|
| T1 | Gemini 4 / Gemma 4 受影响路由矩阵 | 8 / 8 通过 |
| T1.5 | Gemini 4 / Gemma 4 / Qwen Kimi K3 / Qwen video-input 协议矩阵 | 12 / 12 通过 |
| T2 | Gemini 4 / Gemma 4 / Qwen Omni / Qwen Kimi K3：21 个模型/API 单元 | 6 passed，15 review，0 failed；人工复核后 21 / 21 通过 |

Gemini 4 保持原厂 1M context / 1M output 声明；Google Gemini Provider 的同步调用上限收窄为 65536 output tokens。Gemma 4 两个托管模型使用 262144 context、32768 output，并且只暴露 `minimal` / `high` thinking level。

同一 Kimi K3 基础模型在 Qwen Provider 中存在两个服务入口，按实际 Provider model ID 区分 reasoning 能力：阿里云直供 `kimi-k3` 支持 `low` / `high` / `max`，月之暗面直供 `kimi/kimi-k3` 只支持 `max`。Qwen Responses 的视频输入按规范化 MIME 判定并编码为 `input_video`，不依赖文件名或 URL 后缀；Kimi tool continuation 会保留并回放 assistant `reasoning_content`。

本次 T2 精确执行 21 / 21 次真实调用，0 failed，临时 Provider 凭据已原样恢复。机器可直接判定 6 个 LLM 单元为 passed；其余 15 个视觉/ASR 单元因关闭 Judge 保留为 review，经人工按 runner rubric 复核均通过。财务报告记录已知实际支出 0.000246 USD、其余 20 次按未知价格估算 0.200000 USD，总暴露 0.200246 USD，未超过 0.500000 USD 预算。

## 详细报告链接

### T1

- [定向清单：0 个匹配单元](../reports/acceptance/aicc-t1-2026-10-17T09-48-37-510Z-f7bcadf0/summary.md)
- [Gemma 路由契约：3 failed](../reports/acceptance/aicc-t1-2026-10-17T09-51-22-174Z-9e8c28f6/summary.md)
- [最终受影响路由矩阵：8 / 8](../reports/acceptance/aicc-t1-2026-10-17T09-58-10-812Z-3333950b/summary.md)

### T1.5

- [Gemini/Gemma 变体：7 passed；Qwen inventory fail-closed](../reports/acceptance/t15-20261017100053-1881503/summary.md)
- [Qwen inventory 收敛检查：1 runner failure](../reports/acceptance/t15-20261017100146-1882700/summary.md)
- [Qwen Kimi K3 / video-input：5 / 5](../reports/acceptance/t15-20261017100406-1885912/summary.md)
- [最终受影响协议矩阵：12 / 12](../reports/acceptance/t15-20261017100427-1886388/summary.md)

### T2

- [Gemini 4 / Gemma 4 / Qwen Omni / Qwen Kimi K3：21 个单元，0 failed](../reports/acceptance/aicc-2026-10-17T10-05-49-527Z-eb0bbf4f/summary.md)
