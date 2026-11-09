# SN 图片模型 T2 重测汇总

日期：2026-11-02；基准提交 `7c9472fda`，包含本地工作区更新；能力基线 `2026-11-02.2`。真实测试仅 SN US_FULL，Flare/Sunburst 各 txt2img、img2img、inpaint。

| 测试范围 | 结果 |
|---|---|
| 本轮 T1.5 门禁 | 12/12 通过 |
| 目标任务执行、输出结构及最终 NDN 下载 | 6/6 通过 |
| 目标完整内容验收 | 4 通过、2 未通过（inpaint 蓝背景缺失） |
| 前置生成 | 2/2 任务成功、最终回读成功 |
| 实际新增模型调用 | 8（目标 6 + 前置 2），没有自动重试 |
| 原始批次报告 | 7 次模型调用；目标 failed=3、review=3；另有 cleanup 失败 |
| 零调用回读 | 原 7 个对象全部 HTTP 200；加入补全对象后 8/8 验证通过 |
| 费用 | 最终结算未知；估算敞口 $0.16 / 预算 $4 |
| 配置和临时路由 | Provider 设置已恢复；临时路由已移除 |
| 对象清理 | 8 个输出对象删除失败（HTTP 405），仍保留；未修改删除逻辑 |

以上分开统计任务、下载和内容验收，不能把“有结果且能下载”视为全部用例通过。旧连接对新增临时路由返回 403 已受控复现；本地把 NDN 入口设为独立 `localhost` origin 可避开，不修改 Gateway/NDN 实现。

## 各轮用例与证据

- [T1.5 完整用例](t15/t15-20261102040928-910231/summary.md)。
- [T2 首次批次完整用例](t2/aicc-2026-11-02T04-09-58-748Z-ca7171c7/summary.md)、[原批次调用账本](t2/aicc-2026-11-02T04-09-58-748Z-ca7171c7/finance.md)。
- [Flare img2img 单次补全](continuation/summary.md)、[合并费用](continuation/finance.md)。
- [NDN 回读及旧连接对照](readback/summary.md)。
- [最终六个单元的内容复核](final-review/summary.md)。

FlashX 后续独立 T2 已执行，当前结果见[FlashX 合并报告](../flashx-t2/summary.md)；下面保留图片重测时的身份核对结论。

## FlashX 身份结论

SN/AICC 的实际调用映射正确：`glm-5.3-flashx → glm/glm-5.3-flashx`，SN LiteLLM 再转发到 `openrouter/z-ai/glm-5.3-flashx`。没有改成 Flash。此前测试基线把 FlashX 合并到 Flash 的依据不足，现已撤销该去重，新增回归断言 Flash 与 FlashX 各保留 3 个 T2 API 单元。FlashX 尚未执行独立真实 T2，本轮没有扩大真实调用范围。

依据：[SN release 配置](https://github.com/buckyos/sn-business/blob/main/CD/cluster_config/release/cluster_config.json)、[GLM 原厂模型代码说明](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5.3-flash)。原厂单列模型代码和不同渠道可用性，仅共享架构、吞吐更快不足以判定为可跳过的逻辑别名。

静态验证：acceptance 自测 152/152、Deno 类型检查、`git diff --check` 通过。没有修改公共执行组件、NDN/Gateway 逻辑、SDK、协议共享类型或依赖；本轮更新测试基线、回归测试、配置说明及报告。`aigc_image` 手册没有 CLI/任务结果/下载公共契约变化，无需更新。未运行完整 workspace 测试或完整构建。原厂直连图片 T2 未执行。
