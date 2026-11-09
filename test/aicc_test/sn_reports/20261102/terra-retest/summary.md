# SN GPT-6.1-Terra 历史失败 T2 重测

日期：2026-11-02；当前提交 `7c9472fda` 及已有工作区更新；能力基线 `2026-11-02.3`。仅测试 SN US_FULL 的 `gpt-6.1-terra`，未重复已有成功的 LLM/caption，未调用别名或其他 Provider。

后续状态：[2026-11-09 服务端升级后复测](../../20261109/terra-retest/summary.md)四个目标均通过。本文件保留 2026-11-02 的原始失败轮次结果。

## 最终结果

T1.5 门禁 **56/56 通过**，覆盖原厂基础 API、6 个推理预设及图片 TaskMgr 制品回归。随后 T2 **4 个目标单元均未通过**，错误与 2026-10-30 一致。

| T2 单元 | 实际执行 | 结果 | 失败原因 | runner 耗时（含排队/前置） |
|---|---|---|---|---:|
| agent.computer_use | 目标请求已发起 | 未通过 | `OpenAI response has no computer action` | 5.821s |
| image.txt2img | 目标请求已发起 | 未通过 | `Provider did not accept Responses background execution` | 9.679s |
| vision.ocr | 目标请求已发起 | 未通过 | `Provider did not accept Responses background execution` | 20.423s |
| image.img2img | 前置 txt2img 已发起；目标未发起 | 阻断/未通过 | 同模型前置生成未确认接受 background 执行 | 8.492s |

这些是协议响应验收失败，不是 900000ms 的 TaskMgr 轮询超时。本轮没有成功制品，没有进入 NDN 下载验收。不能据此判断原厂 Terra 整体不可用，已有 LLM/caption 成功记录不变。

## 调用、费用与清理

- 计划最多 6 次上游调用额度（4 个目标、1 个生成前置及 computer-use 可能的额外观察握手）；实际 **4 次**，其中 3 个目标 + 1 个图编辑前置，无自动重试、无 Judge。
- 超时 900000ms，全局并发 2、SN 并发 2、最小请求间隔 1000ms；computer-use 只验证返回动作，没有在真实桌面执行操作。
- runner 估算预算 $2；实际未知费用估算敞口 **$0.10**，最终结算金额未知。费用来自原始账本，不按错误任务或短回复推断免费。
- cleanup **passed**：Provider 临时设置/凭据恢复，0 个 fixture/output 对象需要删除，临时 NDN 路由移除；Gateway 登录及 AICC models.list 收尾检查通过。

## 每轮完整用例与证据

- [T1.5 t15-20261102044332-946973](runs/t15-20261102044332-946973/summary.md)、[零真实调用账本](runs/t15-20261102044332-946973/finance.md)。
- [T2 aicc-2026-11-02T04-44-49-736Z-70d2a443](runs/aicc-2026-11-02T04-44-49-736Z-70d2a443/summary.md)、[调用及费用账本](runs/aicc-2026-11-02T04-44-49-736Z-70d2a443/finance.md)。
- [逐任务错误与 trace](evidence/summary.md)。

本次结果仍对应 [sn-business #53](https://github.com/buckyos/sn-business/issues/53)，没有更新、关闭或追加该 issue；本轮仅重测和更新本地报告。没有产品代码/协议/依赖变更、构建或部署；只执行所选门禁与真实用例，`git diff --check` 和报告链接核对通过，未运行完整 workspace 测试或完整构建。

本地受限配置：`aicc_acceptance.sn-terra-retest-20261102.local.toml`（Git 忽略、mode 0600）；原始 T2 报告保留定向重测命令。本轮未改变原厂模型参数或 SN 渠道能力，不通过更换提示词或放宽断言消除失败。
