# Goal Session 继续推进 TODO：H 组等的真实 Zone DV

日期：2026-11-11（本机开发 Zone `test.buckyos.io`，owner `devtest`，Agent `jarvis.test.buckyos.io`，容器形态，真实模型）

对应 [Goal Session 继续推进 TODO](../../../opendan-goal-session-next-todo.md) §10.6。构建：`uv run buckyos-build.py`、`./build_aios --local-test`、`uv run src/start.py`（带安装更新，Jarvis 包随之同步到 AgentRoot）。kRPC 用 owner token 调 `https://test.buckyos.io/kapi/jarvis.test.buckyos.io`，脚本在 `scripts/`。

## 结果

| 条目 | 结论 | 证据 |
| --- | --- | --- |
| 升级后内置整理 Goal 拿到包里的检查脚本 | 通过。`schedule.check = {version 1, source package:goal_checks/memory_maintenance.ts}`，`timetable = every:1800`；结晶 Goal 显示 `switched_off`；`goal_checks/`、`goal_skills/` 同步进 AgentRoot。 | `01-after-upgrade.jsonl` |
| 检查脚本在 Agent 容器的 Deno 上运行 | 通过。容器里 Deno 在 `/opt/buckyos/bin/opendan/deno`（实施时按宿主布局查找会找不到，DV 前改了查找顺序）。内置整理 Goal 6 个多小时里 14 次检查全部 `false`、0 次出错、没有调用模型。 | `04-builtin-after-hours.jsonl` |
| 检查为真才推理 | 通过。测试 Goal（每 60 秒检查 `dv_inbox/`）收件箱为空时只有 `false`；放进文件后下一次检查返回 `true`，打开 1 次 Activation（`scheduled`，原因带文件名，3.2 万 token），Goal 把文件登记进 `dv_seen.txt`。 | `02-empty-inbox.jsonl`、`03-new-file.jsonl`、`01-dv-goal/activations.jsonl` 第 1 条 |
| Goal 在容器里用 `goal check run` | 通过。Goal 自己跑了 12 次，结果带 `state`。日志里混着 Deno 关闭 TLS 校验的警告（之后在代码里过滤，没有重新部署）。 | `01-dv-goal` 的 Session worklog（未拷贝） |
| 写坏的脚本静默到硬保障，再由 Goal 修好 | 通过。坏脚本连续出错 11 次，没有推理；epoch 开始 600 秒后硬保障打开 1 次 Activation，原因里带连续出错次数和错误文本；Goal 写了 v2（`source = goal_session:goal-20261111-17`），测试后安装，下一次检查 `false`（`reason: All inbox files are listed`），`check_errors` 清零。约 10 万 token。 | `05-broken-check.jsonl` |
| 页面经 zone 网关 | 通过。390 与 1280 宽度：Goals 列表、内置 Goal 的门槛面板（脚本来源、时间表、上次检查、硬保障）、检查脚本源码、测试 Goal 的检查计数与成本、Skills 页的结晶开关；390 宽度没有横向滚动。没有点开关、没有提交编辑表单。 | `06-webui/`（用例 `goal_gates_dv.spec.ts`） |

## 发现的问题

1. **硬保障设得短时会无视预算持续推理。** 测试 Goal 的硬保障设成 600 秒，环境闲置约 6.5 小时期间它每 10 分钟被唤醒一次，共 38 次 `guarantee`，周期预算 150 万用完后照样触发（硬保障压过预算是裁决），总共用了约 220 万 token。这是 DV 参数设得不当、又没有及时归档造成的；产品上值得考虑给硬保障设下限（例如不小于 1 小时，或不小于检查时间表的若干倍），待 owner 定。
2. 硬保障原因里的时长短于一小时时写成“0h”（已在代码里改成分钟，没有重新部署）。
3. **F3 的数据**：同一个 Goal 连续 `no_action` 时每次 Activation 的 token 从 2.5 万涨到 7.8 万（历史每次长约 2k），第 37 次触发了通用压缩回落到 5.9 万。

## 留在 Zone 里的东西

- 两个测试 Goal 已归档：`g-k3c2c26af84aec91068d70710`（收件箱）、`g-k019ab54faab41cf103a86a3b`（坏脚本）。
- AgentRoot 下的 `dv_inbox/report-1.txt` 和 `dv_seen.txt` 是测试文件，可以删。
- 本机 Zone 运行的是包含本次改动（除 DV 之后的两处小修）的构建。
