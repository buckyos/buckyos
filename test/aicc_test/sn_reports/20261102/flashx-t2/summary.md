# GLM-5.3-FlashX 独立 T2 汇总

日期：2026-11-02；基准提交 `7c9472fda` 及工作区更新。执行能力基线 `2026-11-02.2`；当前仓库 `2026-11-02.3` 只更新了“尚无独立 T2”的过时说明，模型身份、API 和去重规则相同。

| 范围 | 目标数 | 任务/协议成功 | 完整内容通过 | 内容未通过 |
|---|---:|---:|---:|---:|
| SN GLOBAL_OPEN | 3 | 3 | 2 | 1 |
| SN US_FULL | 3 | 3 | 3 | 0 |
| 合计 | 6 | 6 | 5 | 1 |

GLOBAL_OPEN 的 OCR 预期 `BUCKYOS-DV-4827`，实际 `BUCKY05-DV-4827`（原文带 Markdown 粗体），因此未通过完整标记验收。US_FULL OCR、两个实例的 LLM/caption 均通过。六个任务都是 `Terminal/Succeeded`；没有协议错误或 product_defects，不能将任务成功等同于识别完全正确。

FlashX 使用独立 exact model 和自己的模型/API 单元，未合并到 Flash；LLM 的持久化 provider source 为 `origin_model: glm-5.3-flashx`、`origin_provider: glm`、`provider_profile_id: sn`。本轮没有调用原厂 GLM 或 OpenRouter 直连实例。

T1.5 门禁 12/12（基础 API 及 low/high/max 推理预设）通过后执行 T2。T2 仅基础物理模型，6 次真实调用、无重试、无 Judge 调用、无生成前置。每实例调用上限 3、估算预算 $0.50、timeout 900000ms；合计估算敞口 $0.12 / 预算 $1，最终结算金额未知。每轮 global/provider 并发上限 2；Provider 配置实际最小间隔 750ms。两个实例顺序执行，配置事务不重叠。

两轮 cleanup 均 passed：临时 Provider 凭据/设置恢复，0 个 fixture/output 对象需要删除；临时 NDN 路由均移除。Gateway 登录和 AICC models.list 收尾检查通过。没有修改产品代码、协议、共享类型、依赖或原厂参数；既有质量失败保持不变。

## 各轮完整用例报告

- [T1.5：t15-20261102042624-928527](runs/t15-20261102042624-928527/summary.md)。
- [GLOBAL_OPEN：aicc-2026-11-02T04-26-54-706Z-33f9975f](runs/aicc-2026-11-02T04-26-54-706Z-33f9975f/summary.md)、[调用账本](runs/aicc-2026-11-02T04-26-54-706Z-33f9975f/finance.md)。
- [US_FULL：aicc-2026-11-02T04-27-06-314Z-dc3bca00](runs/aicc-2026-11-02T04-27-06-314Z-dc3bca00/summary.md)、[调用账本](runs/aicc-2026-11-02T04-27-06-314Z-dc3bca00/finance.md)。
- [六个目标单元的人工内容复核](manual-review/summary.md)。

静态检查：acceptance 自测 152/152；`git diff --check` 通过。没有运行完整 workspace 测试或构建，本轮没有产品实现变更。

定向重测配置是被 Git 忽略的 `aicc_acceptance.sn-flashx-global-open-20261102.local.toml`、`aicc_acceptance.sn-flashx-us-full-20261102.local.toml`；各原始报告包含 `targeted_retest_command`，新的真实执行仍需遵循授权范围。
