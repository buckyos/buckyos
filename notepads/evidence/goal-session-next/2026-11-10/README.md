# Goal Session 继续推进 TODO：A 组真实 Zone 补验

日期：2026-11-10（本机开发 Zone `test.buckyos.io`，owner `devtest`，Agent `jarvis.test.buckyos.io`，容器形态，真实模型）

对应 [Goal Session 继续推进 TODO](../../../opendan-goal-session-next-todo.md) 的 A1（DV 部分）与 A3。只验证，没有为了通过而改代码；发现的问题列在最后，没有处理。

## 环境

- 第 1 条的前半部分取自重建之前已经在跑的 Zone：镜像 `local/aios-test`，2026-11-10 17:36 构建（`01-maintenance-running-zone/image.txt`），早于提交 `667468998`。
- 其余各条在重建之后跑：`uv run buckyos-build.py`、`./build_aios --local-test`（镜像 revision `667468998…-dirty`，包含本次 A1、A2 的改动）、`uv run src/start.py`。重启后 Goal 数仍是 1，没有重复播种。
- 页面操作用 Playwright 经 zone 网关登录（`playwright.zone.config.ts`，Chromium 桌面宽度）；其余用 owner token 调 Agent 的 kRPC（`https://test.buckyos.io/kapi/jarvis.test.buckyos.io`）。

## 结果

| 条目 | 结论 | 证据 |
| --- | --- | --- |
| A3-1 整理 Goal 播种一次 | 通过。只有一个 `memory_maintenance` Goal，`changes.jsonl` 只有一条 `created`，重启后仍是一个。 | `01-maintenance-running-zone/` |
| A3-1 无材料时不调用模型 | 通过。8 个定时槽被 gate 跳过（`no_work` / `no_change`），没有对应的 Activation；周期用量里只有 3 次 Activation。新镜像上的第一个槽（996880）同样是 `no_work`。 | `schedule.json`、`03-sleep/dv_cleanup.log` 末行 |
| A3-1 由 30 分钟定时触发的真实整理 | 通过。3 次 Activation 的原因都是 `scheduled`（槽 996869、996871、996874），结论 `acted`，各处理 1 条材料，4.2 万到 10.6 万 token。 | `activations.jsonl` |
| A3-2 WebUI 经网关登录后的状态操作 | 通过。休眠、唤醒、restart（epoch 2）、归档、reopen（epoch 3）都成功，没有权限拒绝和页面错误；内置 Goal 的归档按钮禁用并有说明。 | `02-webui/*.png`、`changes.jsonl`、`goal_dv.spec.ts` |
| A3-3 Activation 进行中置休眠 | 通过。22:00:01 置休眠时 Goal 正在跑一条 `sleep 40`；这条命令返回后的推理边界上 Activation 以 `stopped`（`turn stop requested`）结束，约 20 秒。 | `03-sleep/dv_sleep_1.log` |
| A3-3 派生的 Work 随之暂停、唤醒后恢复 | 通过。Work 的 `paused` 变为 `goal`（`paused: goal … is dormant`），休眠的 60 多秒里 Goal 和 Work 都没有推理；唤醒后 Work 继续并 `succeeded`，结局只被吸收一次。 | `dv_sleep_1.log`、`dv_sleep_2.log`、`activations.jsonl` |
| A3-3 Session 级 stop 等于休眠 | 通过。对 Goal Session 发 `session.stop{scope: session}` 后 Goal 变为 `paused`，Session 没有结束。 | `dv_sleep_3.log` |
| A1 Goal 建的 Work 是 `work_simple` | 通过。Work 冻结的 behavior 是 `work_simple`，名下没有子会话。 | `03-sleep/work-20261111-14.session_config.json`（`prompt.behavior`）、`dv_sleep_3.log` |
| A3-4 结晶 Goal 对真实模型 | 通过，结论 `no_action`。读了用量记录里全部 6 个 Work（报告加最多 40 条 worklog），没有改任何手册、没有写草稿、没有发通知；`cmd_skills` / `plan_skills` / `goal_skills` / `skills_drafts` 下 28 个文件的摘要前后一致。它只在自己的 Session 目录里写了 `evidence/` 临时文件。用了 18.2 万 token。 | `04-crystallize/` |

## 没有验的

- 手机宽度和真机：页面操作只在桌面宽度的 Chromium 上跑过。
- 休眠是从 kRPC 置的（页面上的休眠按钮在 A3-2 里验过，但那个 Goal 当时没有 Activation 在跑）。
- 结晶 Goal 只跑了一次，材料是 6 个没有带手册的 Work；它“修订手册”和“写草稿”两条路径没有被触发，越权改写只能说这一次没有发生。
- 整理 Goal 的“无材料不调用模型”是从记录推断的（跳过的槽没有 Activation），没有抓模型调用日志。

## 发现的问题（未处理）

1. **停掉的 Activation 留下的任务会在唤醒时打开新的 Activation。** 第 1 次 Activation 被休眠停掉时，它的 `sleep 40` 没有被取消（实际跑了 80 秒）；任务结束的事件在唤醒后成了第 2 次 Activation 的原因，同一批原因里还有它自己后续几条 `sleep 40` 的结束事件。这就是 TODO 的 B6（Q-G7），这里只是在真实 Zone 上确认。
2. **Work 的 `needs_input` 也会给 Goal 一次运行机会。** `work_simple` 的 Work 在等自己的后台任务时关闭了 Turn（`waiting for input: sleep 240 is running`），Goal 收到 `child … needs_input` 并为此读了一遍报告。Work 并没有结束，这次 Activation 没有新信息。
3. **Work 的任务结束事件出现在 Goal 的 Activation 原因里。** 第 3 次 Activation 的原因含 `task:local:shell:call_jxigsAHxNfFTlQzqSSnZ5Tj1:terminal`（`exit=0 in 240238ms`），这是 Work 自己的 `sleep 240`，不是 Goal 发起的任务。来源没有查。
4. **成本。** 这个测试 Goal 三次 Activation 用了 37.8 万 token（其中 Work 6.9 万）：每条 `sleep 40` 的结束都带着整段上下文再推理一次。测试目标是故意这样写的，但它说明 Goal 里跟长命令很贵，和 B1（Q-G1）有关。结晶 Goal 一次 `no_action` 用了 18.2 万 token，周期上限是 60 万。
5. **结晶 Goal 关掉以后，Goal 和它的 Session 留着。** 开关还原为 `enabled = false` 之后，Goal 仍是 `active`，gate 为 `disabled`，Session 停在 `waiting`；这是设计（关掉后不建新 Session、不检查），但页面上它看起来仍是一个活动 Goal。和 E5（N8）一起考虑。

## 留在 Zone 里的东西

- 两个测试 Goal 已归档：`g-k6f69faf00d8dc40a5ae8df29`（页面操作）、`g-ke9f2b178f19c59ba1eb73f47`（休眠语义，带 Work `work-20261111-14`）。
- 内置结晶 Goal `g-k7e8eaf27017ec857fbf59d9e` 是这次打开开关时播种的，内置 Goal 不能归档；开关已经还原（容器内直接改的 AgentRoot `agent.toml`，下次模板同步也会覆盖回去）。
- 本机 Zone 现在运行的是包含 A1、A2 未提交改动的构建。
