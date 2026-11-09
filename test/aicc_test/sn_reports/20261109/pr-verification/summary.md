# PR 推送前验证汇总

日期：2026-11-09。基于上游 `803d0ee87` 的工作分支；Rust 验证当前源码，Gateway Mock 验证使用本机已部署 AICC 与本轮 metadata/fixture，没有重新构建或部署。真实模型调用 0，费用 $0。

- AICC Rust library：783/783 通过。
- acceptance 自测：152/152 通过；静态 preflight、JSON、报告链接及差异检查通过。
- SN T1.5：1149 通过、70 按禁用策略跳过、0 失败；跳过不计通过。
- 相关渠道定向 T1.5：102/102 通过（OpenAI 12、Kimi 12、GLM 24、OpenRouter 51、Qwen 3）。
- 路由 T1：91/91 通过。

扩大范围的首轮 T1.5 不是全绿：OpenAI 视频生成及视频编辑各有 invalid_request/not_found 两个错误响应单元未满足 `provider_error` 稳定错误码断言，共 4 项；随后 `provider:openai:cases` 阶段出现 `NdmProxyApiError: Method not allowed`，runner 记 1 项失败并中断，后续渠道未执行。该轮总计 1268 通过、70 跳过、5 失败。四个视频断言的根因尚未定位，不能视为模型质量失败或已证明的本 PR 回归。后续 102 项只补齐本次参数更新直接涉及的渠道模型，不包含这四个视频错误单元，也不将它们改为通过。

各轮 cleanup 均通过：临时 Provider/Mock 配置删除，T1 系统设置逐字恢复。405 中断保留在原轮报告，不因配置清理成功而删除。

| 轮次 | 结果 | 报告 |
|---|---|---|
| t15-20261109084937-3000441 | passed=1268, skipped=70, failed=5；cleanup=passed | [完整用例](runs/t15-20261109084937-3000441/summary.md)、[费用](runs/t15-20261109084937-3000441/finance.md) |
| t15-20261109090608-3020106 | passed=12；cleanup=passed | [完整用例](runs/t15-20261109090608-3020106/summary.md)、[费用](runs/t15-20261109090608-3020106/finance.md) |
| t15-20261109090615-3020328 | passed=12；cleanup=passed | [完整用例](runs/t15-20261109090615-3020328/summary.md)、[费用](runs/t15-20261109090615-3020328/finance.md) |
| t15-20261109090621-3020512 | passed=24；cleanup=passed | [完整用例](runs/t15-20261109090621-3020512/summary.md)、[费用](runs/t15-20261109090621-3020512/finance.md) |
| t15-20261109090629-3020718 | passed=51；cleanup=passed | [完整用例](runs/t15-20261109090629-3020718/summary.md)、[费用](runs/t15-20261109090629-3020718/finance.md) |
| t15-20261109090643-3021056 | passed=3；cleanup=passed | [完整用例](runs/t15-20261109090643-3021056/summary.md)、[费用](runs/t15-20261109090643-3021056/finance.md) |
| aicc-t1-2026-11-09T09-05-00-230Z-fdc23c4c | passed=91；cleanup=passed | [完整用例](runs/aicc-t1-2026-11-09T09-05-00-230Z-fdc23c4c/summary.md)、[费用](runs/aicc-t1-2026-11-09T09-05-00-230Z-fdc23c4c/finance.md) |

未执行完整 workspace 测试、完整系统构建、其他平台或新真实 T2；历史 T2 内容质量及旧读取失败记录保持原结果。
