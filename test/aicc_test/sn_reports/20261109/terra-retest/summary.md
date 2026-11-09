# SN 升级后 GPT-6.1-Terra T2 复测汇总

日期：2026-11-09（北京时间）；run ID 保留 UTC。BuckyOS 基准 `7c9472fda` 及已有工作区更新；能力基线 `2026-11-02.3`。服务端修复说明见 [#53 评论](https://github.com/buckyos/sn-business/issues/53#issuecomment-6734963329)：Terra 改为直接调用 OpenAI。当前 SN 部署提交和 LiteLLM 版本未独立回读；以下结论来自授权 SN US_FULL 的真实功能复测。

## 最终结果

T1.5 **56/56 通过**后执行四个历史失败 T2 单元，**4/4 最终验收通过**。background 创建、异步任务完成、图片制品读取，以及 computer-use 返回动作均可完成，上次的两类协议错误没有再出现。

| API | 目标任务 | 最终内容/动作验收 | runner 单元耗时（含排队/前置） |
|---|---|---|---:|
| agent.computer_use | Succeeded | 通过：按钮范围内的左键点击，要求下一次观察 | 5.838s |
| image.txt2img | Succeeded | 通过：蓝色方块及完整 4827 | 24.554s |
| image.img2img | Succeeded | 通过：保留构图并增加暖色光照；前置生成和目标均成功 | 44.753s |
| vision.ocr | Succeeded | 通过：完整 `BUCKYOS-DV-4827` | 26.598s |

原始自动结果为 passed=1、review=3；人工复核得到以上 4 项通过。另有独立 cleanup failure=1，不能将 runner 退出码 1 解释为这四个模型用例仍失败。未重复两个已通过的基础 LLM/caption T2 单元，未扩大到其他账号、别名或 Provider；T1.5 包含它们的协议覆盖。

## 调用、费用与清理

- runner 推理调用 **5 次**：4 个目标 + 1 个生成前置；computer-use 的一次调用记录 `usage.request_units=2`，因此本轮按该记录计 **6 次上游交互**，达到计划上限 6。
- 无自动重试、无 Judge 调用，单次 timeout 900000ms，全局/SN 并发上限 2，最小间隔 1000ms。
- runner 估算预算 $2，未知费用估算敞口 **$0.12**；Provider 未给出可确认的最终结算金额，实际账单未知。
- 临时 Provider 凭据和设置已恢复，临时 NDN 路由已移除；Gateway 登录、AICC models.list 收尾检查通过。
- 3 个生成输出对象（含前置）均已下载；删除接口仍返回 **HTTP 405**，对象保留，资源清理未通过。该本地限制独立于 SN #53。本轮零模型调用的清理复核逐一确认了这 3 个 405。

## 每轮详细报告

- [T1.5 t15-20261109075222-2920985](runs/t15-20261109075222-2920985/summary.md)、[零真实调用账本](runs/t15-20261109075222-2920985/finance.md)。
- [T2 aicc-2026-11-09T07-53-24-398Z-9b9e94eb](runs/aicc-2026-11-09T07-53-24-398Z-9b9e94eb/summary.md)、[调用与费用账本](runs/aicc-2026-11-09T07-53-24-398Z-9b9e94eb/finance.md)。
- [四项最终验收、任务、trace 与制品证据](manual-review/summary.md)。

本轮保持 2026-11-02 的提示、异步 operation 与验收标准，没有同步回退；没有修改产品代码、原厂参数、Provider 配置或 Mock 契约，也没有构建部署。只更新本地受限执行配置、测试结果和合并报告。`git diff --check`、报告链接和白名单检查通过；未运行完整 workspace 测试或完整构建。测试对应的 [SN #53](https://github.com/buckyos/sn-business/issues/53) 四个历史失败单元在本次 US_FULL 范围内已验证恢复。
