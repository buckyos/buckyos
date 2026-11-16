# AIWS 同步程序的执行设施调研（B1-1、Q6）

日期：2026-11-15。源码基线：buckyos `3e0551d7a`。只读了源码，没有在真实 Zone 上运行；Deno 按域名放行的行为在本机 Deno 2.9.8 上做过最小试验。

背景：[AI Native 余下任务 TODO](aiws-ai-native-remaining-todo.md) 的 B1-1。数据源的同步程序是 Agent 或用户写的普通程序：取外部数据，可能要网络和凭据，按时间表运行，没有模型在环。已定的前提：

- 在后端运行，以工作区 owner 的身份（§6 Q4）。
- 先做“拉”（Q5）：程序把 `aiws.sync.v1` 信封写进 owner 家目录，AIWS 用 `kind: zone` 来源读取（TODO §7.2）。
- 写、测、装、升级、授予能力、改代码只有 owner 和替 owner 工作的 Agent 能做；触发同步按数据源上的 `run`（Q7、Q13）。
- 能力范围（域名、凭据、流量与频率、写入的数据源、可读的配置输入）由执行环境强制，不靠程序自觉（B4-1）。

本页回答：后端哪个设施能承担这件事，凭据放在哪里（Q6）。

## 0. 结论

| 问题 | 结论 |
| --- | --- |
| 最接近的现成设施 | Agent 宿主里 Goal 检查脚本那一套：Deno 子进程、超时与输出上限、持久的时间表、错过的槽合并成一次、结果可以直接发通知。把 Deno 权限按能力范围收窄，就是同步程序要的执行器 |
| workflow 服务 | 调度最完整，但今天不能运行用户代码；用户定义和运行只在内存里，服务重启就丢；步骤用服务自己的身份；没有凭据概念；产出只进临时桶，不进家目录 |
| node executor | 不合适：没有调度，只接受 Agent 的命令流，以节点上指定的 OS 账号运行，Linux 默认不开放 |
| 扩展工具 | 不是执行设施，是工具层（卷加启动器）。无论选哪个候选，都要用它提供的 `deno`、`curl`、`python3` |
| 凭据 | 系统里没有密钥库。现有的第三方凭据（AICC 的 API key、Telegram 的 bot token）都是明文存在 system-config 里，只靠路径 RBAC 保护。ReadWrite Agent 的 RBAC 是 `obj://*` 全部放行 |
| 建议 | 第一步在 Agent 宿主里做独立于 Goal 的“同步任务”，复用检查脚本的执行器和时间表；凭据先放在 Agent 自己的 settings 里，“Agent 自己读得到”记为已知风险（§4） |

## 1. 对比

“改造后”指按 §4 的建议改完以后的情况。

| | Goal 检查脚本（Agent 宿主） | workflow 服务 | node executor | 扩展工具 |
| --- | --- | --- | --- | --- |
| 谁触发、最小间隔 | 宿主的 tick；`every_secs` 或 cron，下限 60 秒 | 1 秒一轮扫描；cron 到分钟，`every_sec` 只要大于 0 | 无调度，Agent 发命令才执行 | 无 |
| 画布、浏览器关掉以后 | 照常运行 | 照常运行 | 不适用 | 不适用 |
| 服务重启以后 | `schedule.json` 持久；错过的槽合并成一次 | 时间表在 Task Manager 里，持久；用户定义在内存里，重启后触发报 `workflow_not_found` | 账本持久，没有定时项 | 不适用 |
| 进程与文件系统 | Agent 容器里的 Deno 子进程。今天 `--allow-read` 不限路径，没有写权限，不能执行命令 | 服务进程内的 tokio 任务；外部工具是子进程，`run_tool_isolated` 用 `nobody`、清空环境变量、独立网络命名空间 | 以节点指定的 OS 账号运行，看到该账号能看到的一切 | — |
| 网络 | 今天 `--allow-net` 不限主机，还带 `--unsafely-ignore-certificate-errors` | 只有开和关两档 | 宿主网络，没有策略 | — |
| 能否读到工作区的其他数据 | 今天能：子进程继承环境变量，容器环境里有 Agent 的 `BUCKYOS_APP_TOKEN`。清空环境变量以后就读不到 | 子进程默认继承 `WORKFLOW_SESSION_TOKEN`；隔离模式下读不到 | 取决于 OS 账号 | — |
| 身份与资源归属 | owner 的 Agent：App 会话的 sub 是 owner，target 是 Agent 的 AppInstance；CPU 和出网算在 Agent 容器上 | 记了 `requested_by`，实际调用都用 workflow 自己的会话；没有代某个用户行事、只碰他的空间的机制 | 节点的 OS 账号，不区分 Zone 用户 | — |
| 凭据 | 没有。Agent 的 settings 可以放，Agent 自己读得到 | 没有。放进输入会落进运行记录、Named Store 和 Task 结果 | 没有 | 清单不能带配置或密钥 |
| 产出怎样到 AIWS | 改造后由宿主把信封写进 AgentRoot。AgentRoot 就在 owner 家目录下（`home/<owner>/agents/<agent>/`），`kind: zone` 可以直接读 | 产出只进 `workflow/artifacts`（临时，30 天），不写家目录 | 能写到的是 OS 账号有权写的路径；`data/home` 下的文件属主是 root | — |
| 能强制哪些能力范围 | 改造后能强制域名、写入的数据源、可读输入和频率；流量做不到（§3） | 改造后与左边相同，凭据隔离更好；域名也要靠 Deno 的权限参数 | 几乎都不能 | — |
| 改造量 | 中 | 大 | 大，而且方向不对 | — |

## 2. 各候选的依据

### 2.1 Goal 检查脚本

- 执行器：[goal_check.rs](../src/frame/lib_opendan/src/goal_check.rs) 第 288–300 行以 `deno run --allow-read --allow-net --allow-env --unsafely-ignore-certificate-errors` 启动子进程，没有 `env_clear`；超时 30 秒（第 31 行），输出和 state 各 64 KiB，脚本 256 KiB（第 33–36 行）。脚本经 stdin 拿 `ctx` 和上次的 `state`，经 stdout 回 `{run, reason?, state?, notify?}`（第 8–10、40 行）；它只能经宿主提供的调用访问 Agent，现在只有 `memory.status`（第 150–170 行）。
- 时间表：[protocol/goal.rs](../src/frame/lib_opendan/src/protocol/goal.rs) 第 285 行下限 60 秒，第 296–306 行是 `every_secs`、cron 和时区；[host/goals.rs](../src/frame/lib_opendan/src/host/goals.rs) 第 11–14 行说明进度存在 `state/goals/<id>/schedule.json`、触发用稳定的键，第 155–158 行把错过的槽合并成一次；第 117–120 行的 `ScheduleBackend` 注释写着“第二期把 workflow 的定时任务接到同一个 trait 后面”。
- 脚本存在 AgentRoot 的 `state/goals/<id>/checks/<n>.ts`（[state/goals.rs](../src/frame/lib_opendan/src/state/goals.rs) 第 1087 行）。
- 环境：Agent 容器的环境里有 `BUCKYOS_APP_TOKEN`（[app_loader.rs](../src/kernel/node_daemon/src/app_loader.rs) 第 2132–2144 行，设备签发的 LoginAssertion，换出的会话 sub 是 owner）。检查脚本继承这个环境，所以今天的脚本能以 Agent 身份调 Zone 服务。宿主进程启动以后是否还保留这个变量，没有实测。
- AgentRoot 的宿主路径是 `data/home/<owner>/agents/<agent_id>`（app_loader.rs 第 2602–2614 行），在 owner 家目录下。
- Deno 2.9.8 实测：`--allow-net=example.com` 时访问 `127.0.0.1:8080` 报 `NotCapable`，放行这个地址后才去连；没有 `--allow-env` 时读环境变量报 `NotCapable`。

### 2.2 workflow 服务

- 调度：`ScheduleSpec` 有 Cron、Once、RunEvery（[scheduled_task_manager.rs](../src/kernel/workflow/src/scheduled_task_manager.rs) 第 53–78 行），1 秒一轮（[main.rs](../src/kernel/workflow/src/main.rs) 第 304–311 行），`every_sec` 只检查大于 0（第 941–945 行）。时区是写死的表（第 1636 行起），只有 UTC、上海、洛杉矶、纽约等几个。创建时间表只校验 token，`owner` 要等于 token 的用户加 App，没有 RBAC。Agent 的 `dcrontab` 就是这组 RPC 的客户端，目标只有发提醒消息和 `agent.delegate`，后者没有执行方。
- 持久：`DefinitionStore`、`RunStore` 是内存里的 HashMap（[state.rs](../src/kernel/workflow/src/state.rs) 第 187–200、419–427 行）。用户定义重启后不在，定时触发 `workflow.run` 时报 `workflow_not_found`（[server.rs](../src/kernel/workflow/src/server.rs) 第 1897–1900 行）。系统定义不能做定时目标（第 1884–1890 行）。
- 能执行什么：只有 AICC 适配器和编译期登记的内置函数（[compiler.rs](../src/kernel/workflow/src/compiler.rs) 第 490–498 行；[function_registry.rs](../src/kernel/workflow/src/function_registry.rs) 第 1–6 行写明 Agent 或 pikg 发布函数是以后的事）。`http`、`appservice`、`operator` 命名空间能解析，但没有适配器。没有运行脚本、Deno、容器或 shell 的执行器。
- 身份：`NativeContext` 不带用户 token（[native.rs](../src/kernel/workflow/src/native.rs) 第 344–358 行），AICC 用服务自己的客户端。
- 隔离：`run_tool_env` 直接继承服务环境；`run_tool_isolated` 清空环境变量、降到 `nobody`、默认不给网络（native.rs 第 495–560 行）。这是现成的受限进程部件。
- 产出：只进 `workflow/artifacts` 桶，临时数据，保留 30 天（[workflow_service.rs](../src/kernel/buckyos-api/src/workflow_service.rs) 第 20–24 行）。

### 2.3 node executor

- 设计定位是“设备本地的执行组件”，明确不是调度器，也不是多租户沙箱（[node_exector.md](../doc/arch/node_exector.md) §0、§3）。Thunk 路径整体后置。
- 只接受 Agent 登录：App 会话、sub 等于 AppInstance 的 owner、绑定到一个获准的 Agent、Agent 在 profile 的 principals 里（[auth.rs](../src/kernel/node_executor/src/auth.rs) 第 180–254 行）。以激活时指定的 OS 账号执行（`etc/host_account.json`，[node_executor_host.rs](../src/kernel/node_daemon/src/node_executor_host.rs) 第 187–203 行）。
- 能做同步程序的“执行一下”，但调度、凭据、产出都要别处提供，隔离反而比 Agent 容器弱。

### 2.4 扩展工具

- node_daemon 的 reconciler 把工具包装到 `$BUCKYOS_ROOT/tools`，以只读方式挂进 Worker 容器（[ext_tools.rs](../src/kernel/node_daemon/src/ext_tools.rs) 第 1–9 行，app_loader.rs 第 1932–1945 行）。`paios/exttool` 镜像只用来给卷播种。
- 清单不允许带配置或密钥字段（[ext_tools.rs（api）](../src/kernel/buckyos-api/src/ext_tools.rs) 第 112–133 行，`deny_unknown_fields`）。
- 用法：同步程序用 ext-core 的 `deno`（或 `python3`、`curl`），由哪个执行设施启动是另一个问题。

### 2.5 AIWS 服务自己（不是候选）

AIWS 已经有刷新计时器：每 5 分钟检查一次到期的来源，以 owner 的内部身份刷新，`auth_mode: hosted` 的来源跳过（[source.rs](../src/frame/aiworkspace/server/src/source.rs) 第 66–98 行）。AIWS 服务调用 Agent Service 也有先例：托管运行用 `goals.create` 加 `goals.activate`（[exec.rs](../src/frame/aiworkspace/server/src/wish/exec.rs) 第 351–366 行）。浏览器优先 M1–M2 定了服务不运行程序，所以它只负责读产出和转发“立即同步”。

## 3. 能力范围由谁强制（B4-1）

| 范围项 | Agent 宿主（改造后） | 说明 |
| --- | --- | --- |
| 可访问的域名 | `--allow-net=<声明的域名>`，Deno 按主机放行 | 不带 `--unsafely-ignore-certificate-errors`。workflow 的隔离模式只有网络开关，也要用这个参数 |
| 可用的凭据 | 宿主经 stdin 交给子进程，日志按原值脱敏 | 宿主和 Agent 的 shell 在同一个容器、同一个身份下，Agent 自己读得到（§4） |
| 频率 | 宿主的时间表下限设为 900 秒（B5-2） | — |
| 流量 | 做不到 | 只能用时长和产出大小兜底（B5-3 的 32 MiB、30 秒）。真要计量，要让子进程经宿主的代理出网 |
| 写入的数据源 | 子进程没有写权限，宿主把信封写到任务登记的路径 | 程序换不了写入目标 |
| 可读的配置输入 | 宿主放进 `ctx`；`--allow-read` 只给脚本自己的目录；清空环境变量 | 不清空的话，继承来的 `BUCKYOS_APP_TOKEN` 能调 AIWS，违反 B4-4 |

## 4. 凭据放在哪里（Q6）

现有做法：AICC 的 key 在 `services/aicc/settings`（[settings/mod.rs](../src/frame/aicc/src/settings/mod.rs) 第 20 行），预留的 `secret_ref` 一用就报“没有配置 secret resolver”（[settings_runtime.rs](../src/frame/aicc/src/service/settings_runtime.rs) 第 335–338 行）。Telegram 的 bot token 在 Agent 的 `users/<u>/agents/<a>/settings`（[app_schema.rs](../src/kernel/buckyos-api/src/app_schema.rs) 第 501 行）。system-config 没有逐键 ACL，也没有加密，靠的是路径 RBAC（[rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs)）：`security/*` 只有 kernel、ood、root、su_admin 读得到（第 109–110 行）；ReadWrite Agent 的运行身份拿到 `agent_read_write`，即 `obj://*` 全部放行（第 92 行，[system_config_agent.rs](../src/kernel/scheduler/src/system_config_agent.rs) 第 1639–1643 行）。verify-hub 只存 Zone 自己的会话，没有外部 OAuth token。

| 方案 | 今天有没有 | 谁读得到 | 复制工作区时 | 改造量 |
| --- | --- | --- | --- | --- |
| 1. Agent 的 settings（照 Telegram bot token） | 有 | Agent、owner、管理员与 `system` 组的服务 | 不跟着走：凭据挂在 owner 的 Agent 上 | 小：owner 专用的写入入口、宿主读出后交给子进程 |
| 2. AIWS 保管（工作区的 local 库，不导出、不复制），每次运行交给执行环境 | 无 | AIWS 服务；执行环境运行时拿到 | 不跟着走 | 中。“拉”方案里程序按自己的时间表运行，宿主每次都要向 AIWS 取，效果和方案 1 差不多 |
| 3. 系统密钥库：AICC `secret_ref` 预留的 resolver，加 `security/` 那样的拒绝规则 | 无 | 只有指定的服务 | 不跟着走 | 大，但 AICC 也能用 |

不论哪个方案，程序都是 Agent 写的，作者有意把凭据打印出来或发出去，保管方式挡不住。能做到的是：凭据不进聊天、运行记录、模板，也不给协作者；只能发往声明的域名（Deno 放行）；日志脱敏。Agent 测试或修复程序时要经宿主的试跑入口（类似 `goal check run`），不在 shell 里直接拿凭据，否则凭据会进会话记录。只有执行环境与 Agent 不是同一个身份时（例如将来放到 workflow，方案 3），“只交给被授权的执行环境”才严格成立。

## 5. 建议

**设施：Agent 宿主，做成独立于 Goal 的“同步任务”。** 复用 `DenoCheckRunner` 和 `LocalScheduleBackend`，不新建 Goal（Goal 是珍贵的资源）。理由：

1. 要的部件大多已经有了：子进程、超时、输出上限、持久的时间表、错过的槽合并、通知。
2. 作者和修理者就是这个 Agent（B2-3、B6-2），写、试跑、修复都在同一处。程序失效时，可以按现有的通知告诉 owner，再唤醒 Agent 去修。
3. Deno 的权限参数能在环境层强制域名、写入目标和环境变量，B4-1 的五项里能做到四项。
4. 产出写进 AgentRoot，就在 owner 家目录下，B-内核的 `kind: zone` 能直接读，AIWS 不接触用户代码。

要做的事：任务登记（程序版本、能力范围、时间表，放在 AgentRoot）；收窄权限的执行器（清空环境变量，超时和产出上限按 B5-3）；宿主写信封（失败时写 `status: failed`）；owner 专用的安装、授权、录入凭据入口（照 `skills.*` 的做法，Agent 只交草稿）；试跑命令；Agent Service 上的“立即运行”，供 AIWS 的 `source.refresh` 调用（Q13）；手册（B6-3）。

已知风险：身份是 owner 的 Agent，不是 owner 本人的会话；owner 没有 Agent，或 Agent 停着，就不同步；凭据挡不住 Agent 自己；多节点 Zone 里 AIWS 和 Agent 不在同一节点时，AIWS 读不到对方节点上的 AgentRoot。

**以后：** 做“推”（B3-2）或者要严格的凭据隔离时，再看 workflow。前提是它先补齐持久的用户定义、按用户的身份、脚本执行器（在 `run_tool_isolated` 上加 Deno 权限参数），以及方案 3 的密钥库。

**凭据（Q6）：** 第一步用方案 1，“Agent 自己读得到”记为已知风险（先能力再安全）；方案 3 等 workflow 或 AICC 需要时一起做。

## 6. 需要 owner 决定

1. 设施选 Agent 宿主（建议），还是 workflow 服务？
2. Q4 的“以 owner 的身份”，可以按“owner 的 Agent 的 App 会话（sub 是 owner）”落地吗？资源算在 Agent 容器上。
3. owner 没有 Agent，或 Agent 停用时：数据源停在最近一个有效版本，显示“执行环境不可用”。可以吗？
4. 同步任务独立于 Goal（建议）。程序失效时用通知告诉 owner，并唤醒 Agent 修复。走哪个入口（某个 Goal、对话、新的 Work）？
5. Q6：选方案 1，并接受“Agent 自己读得到”（建议）？还是现在就做方案 3？
6. 流量预算第一步不计量，只用时长和产出大小兜底。可以吗？
7. 第一步只支持 AIWS 与 Agent 在同一节点（单 OOD）。可以吗？

## 7. 没有核对的

- Agent 容器的出网有没有别的限制（Docker 网桥、网关），没有看。
- 宿主进程里 `BUCKYOS_APP_TOKEN` 启动以后是否还在：检查脚本继承环境变量是从代码看出来的，没有实测。
- macOS、Windows 上 Agent 以宿主进程运行时，同样的做法没有容器这层边界；Deno 的权限参数照样有效。
- Deno 按主机放行只试了直连。重定向和 DNS 的行为没有测。
- workflow 的部分行号来自一次源码检索，抽查了其中八处。
