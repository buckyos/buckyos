# AICC T1 / T1.5 / 豆包 Agent Plan T2 最终测试报告

测试日期：2026-10-10

测试环境：Ubuntu，BuckyOS `/opt/buckyos`。凭证未写入报告。

## 最终结论

| 层级 | 覆盖范围 | 最终结果 | 真实调用 | 清理 |
|---|---|---:|---:|---:|
| T1 | AICC 全量静态清单、路由暴露与配置契约 | 178 / 178 通过 | 0 | 通过 |
| T1.5 | 15 个 Provider 的全量高保真协议矩阵 | 1605 / 1605 通过 | 0 | 通过 |
| T2 | 豆包 Agent Plan，6 个官方物理模型的 LLM API | 6 / 6 通过 | 6 | 通过 |

T1、T1.5 和本轮 T2 均通过。T2 未启用 Judge；6 个用例均由确定性协议、响应结构、标记文本、usage、持久化归因和清理断言验收，无待人工语义复核项。

## T2 最终结果

模型范围以火山方舟官方 Agent Plan 文档为基线，并优先选择配置中可试用的模型。官方来源：[Agent Plan 模型列表](https://www.volcengine.com/docs/82379/1958524?lang=zh)、[Agent Plan 使用说明](https://www.volcengine.com/docs/82379/1824718?lang=zh)、[模型免费额度说明](https://www.volcengine.com/docs/82379/1159200?lang=en)。

| 模型 | API | 调用 | Token（输入 / 输出 / 合计） | 结果 |
|---|---|---:|---:|---:|
| `deepseek-v4-flash` | `llm` | 1 | 98 / 18 / 116 | 通过 |
| `deepseek-v4-pro` | `llm` | 1 | 98 / 38 / 136 | 通过 |
| `doubao-seed-2.0-mini` | `llm` | 1 | 50 / 66 / 116 | 通过 |
| `glm-5.3` | `llm` | 1 | 28 / 309 / 337 | 通过 |
| `kimi-k2.8-preview` | `llm` | 1 | 100 / 113 / 213 | 通过 |
| `minimax-m3` | `llm` | 1 | 191 / 46 / 237 | 通过 |
| **合计** |  | **6** | **565 / 590 / 1155** | **6 / 6 通过** |

## 调用与费用

| 指标 | 结果 |
|---|---:|
| 计划 / 实际真实调用 | 6 / 6 |
| 最大预算 / 估算费用暴露 | USD 0.06 / USD 0.06 |
| 已知实际费用 | USD 0 |
| 费用未知调用 | 6 |
| 是否超预算 | 否 |

Agent Plan 响应未返回可核算的美元结算金额，因此 `已知实际费用 = USD 0` 不代表调用免费；本报告按运行器保守口径记录 USD 0.06 的估算费用暴露。

## 修复验证

| 修复项 | 验证结果 |
|---|---:|
| 长时间验收期间 sudo token 过期后自动重新认证，且并发请求共享一次刷新 | 通过 |
| 同租户重复 artifact URL 更新到当前 ProviderInstance，跨租户不可接管 | 通过 |
| Agent Plan 按官方模型能力收窄；`deepseek-v4-*`、`glm-5.3`、`kimi-k2.8-preview`、`minimax-m3` 不再误暴露视觉 API | 通过 |
| Acceptance 自测试 | 107 / 107 通过 |
| AICC 构建与 Ubuntu 部署后运行检查 | 通过（核心服务可达） |

## 报告索引

| 层级 | 轮次 | 结果 | 报告 |
|---|---|---|---|
| T1 | 初始全量轮次 | 1 个执行失败；清理通过 | [`aicc-t1-2026-10-10T03-48-08-395Z-4cc1375f/summary.json`](../reports/acceptance/aicc-t1-2026-10-10T03-48-08-395Z-4cc1375f/summary.json) |
| T1 | 最终全量轮次 | 178 / 178 通过；清理通过 | [`aicc-t1-2026-10-10T03-53-26-950Z-b9b6b849/summary.json`](../reports/acceptance/aicc-t1-2026-10-10T03-53-26-950Z-b9b6b849/summary.json) |
| T1.5 | 初始全量轮次 | 1577 / 1605 通过；清理通过 | [`t15-20261010040037-2623351/summary.json`](../reports/acceptance/t15-20261010040037-2623351/summary.json) |
| T1.5 | Artifact 定向复测 | 1 / 1 通过；清理通过 | [`t15-20261010044035-2678347/summary.json`](../reports/acceptance/t15-20261010044035-2678347/summary.json) |
| T1.5 | 最终全量轮次 | 1605 / 1605 通过；清理通过 | [`t15-20261010044054-2678826/summary.json`](../reports/acceptance/t15-20261010044054-2678826/summary.json) |
| T2 | 初始零调用全矩阵检查 | 2 个能力基线差异；0 次真实调用；清理通过 | [`aicc-2026-10-10T05-11-54-561Z-6db0b95a/summary.json`](../reports/acceptance/aicc-2026-10-10T05-11-54-561Z-6db0b95a/summary.json) |
| T2 | 修复后零调用定向预演 | 6 个 case 均按设计跳过；0 次真实调用；无能力差异；清理通过 | [`aicc-2026-10-10T05-19-26-458Z-0cdb4f23/summary.json`](../reports/acceptance/aicc-2026-10-10T05-19-26-458Z-0cdb4f23/summary.json) |
| T2 | 最终真实调用轮次 | 6 / 6 通过；清理通过 | [`aicc-2026-10-10T05-19-50-091Z-5a533960/summary.json`](../reports/acceptance/aicc-2026-10-10T05-19-50-091Z-5a533960/summary.json) |

## 剩余风险与未覆盖项

- T2 仅对 6 个物理模型执行了 LLM 真实调用；`doubao-seed-2.0-mini` 的视觉 API 已由 T1/T1.5 覆盖，但本轮未产生真实 Provider 视觉调用。
- Agent Plan API 未返回可核算费用字段，最终结算金额需以供应商账单为准。
- Agent Plan 控制面模型清单需要独立 AK/SK；本轮使用带版本的官方文档基线，并由运行时模型清单做双向核对。
