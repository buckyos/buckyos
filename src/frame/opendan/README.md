# opendan — Agent Loader

`opendan` 以某个 Agent 的 runtime app 身份启动，托管这个 Agent 的 Session，并把 Agent State 作为服务提供出去。Agent 内核（推理循环、Context 调度、Turn、输入总线、Runtime、Agent State 文件协议）在 `llm_context` / `agent_tool` / [`libopendan`](../lib_opendan/README.md) 里；这里没有提示词、没有 behavior 名字的特判、不解析 LLM 输出、不直接改写 Session 状态。一个进程托管一个 Agent。

## 模块

| 文件 | 内容 |
|---|---|
| `main.rs` | 参数、zone 形态与 `--dev` 形态的环境装配（等待绑定的 AgentSpec，读 settings）、退出信号 |
| `loader.rs` | 启动顺序：AgentRoot 身份 → AgentRoot 同步（模板固定时跳过）→ 角色补充 → `agent.toml` → Agent State（文件实现，进程内登记）→ HTTP → Supervisor → 模块 → 写 `info`；退出时结束托管并等待 |
| `config.rs` | `agent.toml` 里 Loader 的部分：`[loader]`、`[[loader.ui]]`、`[loader.self_check]`、`[loader.self_improve]`、`[llm_context]`；配置错误在启动时报出并退出 |
| `rootfs.rs` | AgentRoot 目录布局、身份（`.meta/identity.json`，不一致时归档）、从 Agent 包同步素材（保留本地修改）、角色补充文件 |
| `records.rs` | AgentRoot 之外的 Agent 记录：system-config 的 `settings` / `profile` / `info`；`--dev` 用内存或本地目录 |
| `ui.rs` | UI Session + Message Tunnel：inbox 发现、绑定、入站桥（只放行 Owner，群聊规则与群上下文）、出站 sink |
| `tasks.rs` | TaskMgr 接入：`TurnTasks`（每个 Turn 一个 `opendan.agent_turn/v1` task，Agent 自建自跑，root task grant 给 owner）、`CancelBridge`（TaskMgr 的 cancel → Session `stop`） |
| `service.rs` | Agent State kRPC 服务、Session 观测、三个操作、Loader 状态 |
| `home.rs` | 首页用的数据：Agent 资料卡、按模型的 token 用量、UI Session 对应的会话、Agent State 登记的 Workspace 展示 |
| `web/` | WebUI：首页（资料卡、工作概况、Session 列表，按手机屏幕设计）+ 观测页（Sessions / Agent State / Loader，`#/sessions` 起），数据全部来自 kRPC |

## Agent 与它的 Owner

zone 内每个 Agent 是一个由模板构造的独立 App（AppId = AgentId，实例 `<agent_id>@<owner>`），由 control_panel 创建。Loader 启动后在 `users/<owner>/agents/*/spec` 里找绑定到本实例的 AgentSpec；还没有时一直等待（每 5 秒查一次，不退出），多于一个时报错退出。AgentRoot 在 `data/home/<owner>/.local/share/<agent_id>/agents/<agent_id>/`。

| 记录（`users/<owner>/agents/<agent_id>/…`） | Loader 的用法 |
|---|---|
| `spec` | `agent_doc.owner` 是 Owner DID；`agent_doc_object_id` / `generation` 用于 AgentRoot 身份与 `info` |
| `settings` | 启动时读一次：`role_supplement`、`template_auto_update`；`allow_group` 在收到群消息时重新读 |
| `profile` | 首页资料卡读写它（`agent.profile` / `agent.profile_set`） |
| `info` | 加载成功后写入 `{agent_doc_object_id, generation, loaded_at, template_version}`（`loaded_at` 为秒），写不进去时退避重试；control_panel 据此判断就绪 |

- **AgentRoot 身份**：`.meta/identity.json` = `{agent_did, agent_doc_object_id}`。记录的是另一个 Agent，或同名删除后重建的 Agent（AgentDocument 不同），旧目录整体移到 `agents/.archived/<agent_id>-<毫秒时间戳>`，再从空目录初始化，不继承任何数据。没有身份文件的目录（手工准备的）直接认领。
- **模板固定**：`template_auto_update = false` 且 AgentRoot 已从包同步过（有 `.meta/rootfs_sync.json`）时不再同步，日志里记一条；`info.template_version` 保持原值。否则照常同步，`template_version` 取本次部署的包版本（环境变量 `app_instance_config` 里 `agent` 包的 `#<version>`）。
- **角色补充**：`settings.role_supplement` 写到 `.meta/role_supplement.md`（为空则删除），libopendan 的 behavior catalog 把它接在 `role.md` 之后，并计入 catalog revision。Session 创建时冻结身份文本，修改只对新 Session 生效。
- **只放行 Owner**：消息的发送者是 `record.ingress.extra.principal_did`（msg-center 解析出的本人，例如 Telegram 账号背后的 Owner），没有时是 `record.from`。不是 Owner 的记录只标记已读：不建 Session、不调用 LLM、不回复。`/stop` 等命令也只有 Owner 能发。
- **群聊**：`allow_group = false` 时群消息只标记已读。`allow_group = true` 时只有 Owner 发出、且 `mentions.dids` 含 Agent DID 的群消息触发；触发时先把同一 mailbox 最近 20 条已读消息（不含 Agent 自己的）作为上下文（`delivery.context = true`，保留各自的发言者）按时间顺序投进 Session 队列，再投触发消息。上下文消息不单独组成输入批次、不构成请求，渲染时带 `context="true"`；Session 已经看过的按 ObjId 去重。Agent 包需要 `[[loader.ui]] on = "msg.group"` 规则。

托管循环在 `libopendan::host::Supervisor`：登记表里 `driver = 本进程身份` 且未结束（或有待处理 decide）的 Session 各有一个协程循环 `drive(Idle)`；启动后的第一次扫描就是恢复；空闲超过 `idle_unload_secs` 且没有待推进工作的 Session 被卸载，有新输入时再装载。

Turn 与 task：zone 形态下每个 Turn 打开时在 TaskMgr 建 task（幂等键 `agent_turn:<sid>:<turn>`，input = `{agent_did, session_id, session_kind, turn, inputs}`），子 Session 的 Turn 挂在创建它的那个 Turn 的 task 下；Session 是 task 状态的唯一写入方（Running + message、Waiting(ChildTask / Dependency / External)、Succeeded `{summary}`、Failed、Stopped → Canceled）。`--dev` 没有 TaskMgr，不建 task。

占位：Agent 包 `i18n/<语言>.toml` 的 `[outbound]` 有 `accepted` 文案、msg-center `msg.get_edit_capability` 回答可编辑、且 Turn 打开超过 `[loader] placeholder_delay_ms`（默认 3000）或已有首次工具调用时，先发占位（带 `agent_task`），Turn 结束时以一条 edit 替换；`stopped` / `finished` 是停止与无正文结束时收尾占位的文案。其余情况只发最终回复（同样带 `agent_task`）。

## 运行

```bash
# zone 内（node-daemon 在容器里启动，或 ./debug_jarvis.sh 在宿主机上启动）；appid = AgentId
opendan --app-id <appid> [--agent-bin <Agent 包目录>] [--service-port <n>] [--web <目录>] [--trust-loopback]

# 开发形态：不需要 zone，文件队列，没有 msg-center
opendan --dev --agent-root <dir> --agent-did <did> --queue-dir <dir> \
        [--owner-did <did>] [--records <dir>] [--who app:opendan@local] [--agent-bin <Agent 包目录>] \
        [--port <n>] [--web <目录>] [--poll-ms <n>]
```

`--dev` 没有 system-config：`--records <dir>` 下的 `settings.json` / `profile.json` / `info.json` 充当 Agent 记录（缺省全部取默认值、只在内存里）；没有 AgentSpec，`identity.json` 只记 `agent_did`，不写 `info`。

`--trust-loopback` 只用于本机调试：来自回环地址、不带 token 的调用按 owner 处理（本机网关转发来的请求也是回环地址，所以不要在正式环境里用）。服务端口被占用时进程直接退出：占着端口的就是同一个 Agent 的另一个宿主。

端口：`--service-port`（= `--port`）→ `$BUCKYOS_SERVICE_PORT` / `$OPENDAN_SERVICE_PORT`（调度器分配）→ 4060。`xagent` 可执行文件与 `web/` 目录取自 `opendan` 所在目录。

开发形态下用 xagent 创建由 OpenDAN 驱动的 Session：

```bash
export OPENDAN_AGENT_ROOT=<dir> OPENDAN_AGENT_DID=<did> LIBOPENDAN_QUEUE_DIR=<dir> LIBOPENDAN_WHO=app:opendan@local
xagent new --objective "…" --llm-context "$LC" --msg "…" --no-run     # 创建者 = 驱动者 = app:opendan@local
OPENDAN_AGENT_ROOT= xagent list --state-url http://127.0.0.1:4060/kapi/opendan   # 经 kRPC
```

## 调试流程

日常调试不用容器，功能调对之后再用本地镜像确认发布后的效果。两步都要求 DV 环境已经在跑（`uv run start.py`）。

**1. 宿主机上调试（快）**

```bash
cd src && ./debug_jarvis.sh            # 可选：[owner] --agent <agent_id> | --no-build | --installed | --port <n> | -- <opendan 参数>
```

- 先要有一个用 Jarvis 模板创建的 Agent（桌面 Add Agent，或 `test/test_opendan/agent_target.ts --create <名字>`）。不指定 `--agent`（或 `$AGENT_ID`）时取 owner 名下第一个 ready 的 Agent，没有则取第一个已绑定的；列表来自 `rootfs/bin/service_debug.tsx agents <owner>`。
- `cargo build`（debug）后以该 Agent 的构造 App 身份（AppId = AgentId）在前台运行，Agent 包直接读 `src/apps/jarvis_runtime/agent`，WebUI 读 `frame/opendan/web/dist`（没有则用已安装的）。改了包里的提示词或 Rust 代码，Ctrl+C 再跑一次即可；已有 Session 冻结的 behavior 不变，发 `/stop` 开新一代 Session 才会用上新的。
- 脚本先停掉 node-daemon 拉起的 `buckyos-app-*` 容器，再占住 app 的服务端口，容器因此起不来（node-daemon 日志里会持续有 `docker run` 失败，属于预期）。容器抢先回来时 opendan 拒绝启动，脚本再停一次容器并重试。Ctrl+C 退出后 node-daemon 会自己把容器拉回来。
- 容器不在时 zone 网关没有到 app 的路由，WebUI 直接开 `http://127.0.0.1:<服务端口>/`，本机访问不需要登录（`--trust-loopback`）。
- 消息收发、AICC、kmsg、kevent 都是真实服务：`test/test_opendan/test_agent_loader.ts` 可以直接用（`OPENDAN_URL=http://127.0.0.1:<服务端口>/kapi/opendan`）。

**2. 本地测试镜像（发布前确认）**

```bash
./build_aios --local-test              # 只构建本机架构，镜像 local/aios-test，不推送
cd src && uv run start.py --skip-update
```

`--local-test` 构建完会把 `$BUCKYOS_ROOT/etc/devenv.json` 的 `aios` 指向这个镜像，node-daemon 重启后就用它创建 app 容器，和镜像发布后的效果一样（入口脚本、包同步、容器内路径、经网关的 SSO 都走真实路径）。删掉 `devenv.json` 里的 `aios` 就回到发布镜像；`start.py --all` 全新安装会清掉它，需要重新执行一次 `--local-test`。

## kRPC 接口

同一服务也挂载在 `/kapi/<agent_id>`（AgentSpec 的完整 AgentId，包含域名中的点），供 Desktop 经 zone 网关的 `service_info[agent_id]` 同源访问。两条路径使用相同的 owner/root 鉴权；客户端通过 `agent.list` 的 DID → AgentId 映射定位，不把会话 token 发送到消息提供的任意 URL。

`GET /kapi/opendan` 不需要 token，返回 `{"app_id": "<托管本页面的 app>" | null}`：WebUI 用它向 zone 换取 session token（token 是签给这个 app 的）；`--dev` 形态返回 `null`。

路径 `/kapi/opendan`，标准 kRPC（`POST`，`{"method","params","sys":[seq, session_token]}`）。调用方身份 `who` 来自 session token（verify-hub 签发）：Agent 的 owner（经任何 app）与 zone root 可访问；`--dev` 形态不校验。错误的 `error` 字段里带 JSON 文本 `{"kind","message",…}`（kRPC 会加前缀 `Failed due to reason: `，从第一个 `{` 起解析）（`kind` 取值同 `OpenDanError::to_json`，如 `not_found`、`invalid_argument`、`input_full`、`queue_missing`）。

### Agent State（与 `AgentStateClient` 一一对应）

实现在 `libopendan::state::krpc`，客户端是 `KrpcAgentStateClient`（`state::connect` 的 `Krpc` 分支，`$OPENDAN_AGENT_STATE_URL`）。只提供读与带署名的写；驱动者在 lease 下的写入（`report_state`、`perception.append`、`commit_cursor`、`register_version`）与 Agent 级锁依赖 flock，不上 kRPC。

| method | params | result |
|---|---|---|
| `agent.info` | – | `{agent_did, agent_id}` |
| `sessions.lookup` | `{sid}` | `RegistryEntry \| null` |
| `sessions.query` | `{query?: RegistryQuery}` | `RegistryEntry[]` |
| `sessions.children_of` | `{parents: string[]}` | `RegistryEntry[]` |
| `sessions.register` | `{entry: RegistryEntry}`（`created_by` 必须是调用方） | `RegistryEntry` |
| `sessions.post_input` | `{sid, input: PostedInput}`（`from` 由服务端改写为调用方） | `index: number` |
| `sessions.verify` | – | `string[]`（标记为 unreachable 的 sid） |
| `activity.active` | `{me?: sid, limit?}` | `ActiveSession[]` |
| `perception.cursor` | – | `PerceptionCursor` |
| `perception.backlog` | `{cursor}` | `Backlog` |
| `perception.read` | `{item: BacklogItem}` | `PerceptionRecord[]` |
| `perception.last_seq` | `{sid}` | `number` |
| `cognition.recall_hints` | `{query: {tags, max_hints}}` | `Hint[]` |
| `cognition.notebook_append` | `{note: NotebookNote}` | `null` |
| `artifacts.list` / `artifacts.head {aid}` / `artifacts.versions {aid}` / `artifacts.version {aid, ver}` | | `ArtifactHead[]` / `ArtifactHead \| null` / `ArtifactVersion[]` / `ArtifactVersion \| null` |
| `artifacts.decide` | `{aid, ver, decision: accept \| discard}`（服务端在调用期间持有 `artifact:<aid>` 锁） | `DecideResult` |
| `behaviors.identity` / `behaviors.list` / `behaviors.get {name}` / `behaviors.revision` | | `IdentityText` / `BehaviorMeta[]` / `BehaviorConfig \| null` / `string` |

类型定义：`libopendan::protocol`（`agent_state.rs`、`state.rs`、`config.rs`、`input.rs`、`worklog.rs`、`behavior.rs`）与 `libopendan::state::{activity, perception, cognition, artifacts}`；JSON Schema 在 `doc/opendan/protocol/schema/`。

### Session 观测（只读）

`session.read {sid, report?: bool = true, worklog?: number = 40}` →

```jsonc
{
  "entry": RegistryEntry,
  "state": SessionState,          // 含 open_turn、waiting_for、process_stack、pending_events、outbox、reply、live_run
  "report": "…",                  // report.md
  "worklog": [WorklogEntry],      // 尾部，新→旧（按 seq 排序）
  "note": "…",                    // 目录不可读等说明；此时没有 state
  "config": { "session": {…}, "runtime": {…}, "workspace": …, "subscriptions": […], "channels": {…},
              "behavior": "…" | null, "frozen": { "catalog_rev", "frozen_at_ms", "frozen_by", "behaviors": ["…"] } },
  "binding": Binding | null,      // runtime 绑定
  "statistics": SessionStatic | null,
  "runs": ["<run_id>", …],
  "lease_holder": LockInfo | null,
  "children": ["<sid>", …],
  "hosted": HostedStatus | null
}
```

没有任何对 Session 目录的写接口。

`session.worklog {sid, turn, before?: number, after?: number, limit?: number = 50}` 按指定 turn 读取已提交的执行记录：

```jsonc
{
  "agent_did": "…", "session_id": "…", "turn": 1,
  "entries": [WorklogEntry],
  "next_before": 1234, "next_after": 5678, "committed": 5678,
  "complete": false, "task_id": "…",
  "children": [{"created_by_call": "<run_id>/<call_id>", "session_id": "…", "name": "…"}]
}
```

`entries` 按 seq 从旧到新排列，仅含这个 turn。首次读最近一页；`before` 反向翻页，`after` 从上一响应的 `next_after` 向前补读。游标是 worklog 的字节边界，不允许同时传入或越过 `state.worklog.committed_bytes`；`limit` 范围 1–200。单次最多检查 2000 条记录，因此空页仍可能有 `next_before`，实时补读的 `next_after` 也可能小于 `committed`，客户端需继续读取。`next_before = null` 表示该 turn 的更早记录已读尽。翻页期间日志继续追加不会改变既有游标。

`complete` 根据 `turn_seq / open_turn` 判断，包括失败、停止等终态；不使用只统计成功轮次的 `turns_completed`。`task_id` 来自仍保留的 `turn_tasks` 绑定，历史绑定被清理时可为 null，日志仍可读。`children` 来自 session 登记表的 `origin.created_by_call`，按 run/call 精确挂在创建调用上；子 session 的首个 turn 是其 work task 的入口。此接口沿用整个 OpenDAN 的 owner/root 读权限，单有 TaskMgr grant 不授予 worklog 权限。

### 操作（都只是向 Session 的输入总线投一条记录）

| method | params | 说明 |
|---|---|---|
| `session.stop` | `{sid, reason?}` | `control(stop)` |
| `session.decide` | `{sid, decision: accept \| discard, note?}` | `control(decide)` |
| `session.post` | `{sid, text}` | 以调用方身份向有队列的 Session 投一条文本消息 |

返回 `{index}`（`session.post` 另带 `key`）。投递后服务端让 Supervisor 装载该 Session。三个操作都要求 Session 有输入队列；没有队列的 Session（如一次性的 work session）返回 `channel` 错误，它的产物验收走 `artifacts.decide`。

### 首页

| method | params | result |
|---|---|---|
| `agent.profile` | – | `{agent_did, agent_id, display_name, avatar, bio, owner_did, desktop_url}` |
| `agent.profile_set` | `{display_name?, avatar?, bio?}` | 同 `agent.profile` |
| `usage.models` | – | `{now_ms, since_ms, models: [{model, hour, day, all}]}`，`hour` / `day` / `all` 为 `{input, output, total}` |
| `ui.bindings` | – | `[{session_id, to, to_session, kind}]` |
| `home.workspaces` | – | Agent State 中全部已知 Workspace 的展示字段，包含归档和不可访问记录，不输出私有备注与策略引用 |

- 资料：读写系统配置里的 Agent `profile`（与 control_panel 的 `agent.profile.*` 是同一份）；没有昵称时 `display_name` 是 Agent 用户名（AgentId 的第一段）。`agent.profile_set` 只改传入的字段，传空字符串清除该字段。`display_name` ≤ 64 字符，`bio` ≤ 500 字符，`avatar` 是图片 data URL，≤ 128 KiB。`owner_did` 是 AgentSpec 里的 Owner DID（`--dev` 取 `--owner-did`），`desktop_url`（zone 桌面的地址，MessageHub 在其 `/messagehub`）在 `--dev` 形态下为 `null`。
- 用量：汇总登记表里各 Session 的 `usage.jsonl`（[Session Directory Protocol](../../../doc/opendan/protocol/Session%20Directory%20Protocol.md) §4），按 `all.total` 降序；`hour` / `day` 是最近 1 小时 / 24 小时。`model` 是 AICC 路由最终选中的模型，取不到时是请求里的模型别名。没有 `usage.jsonl` 的旧 Session 不计入，`since_ms` 是最早一条记录的时间。
- `ui.bindings`：UI Session 的回复去向（`session_config.json` 的 `channels.outbound`）。WebUI 据此把 Session 链接到 MessageHub：对方是 owner 时打开 owner 自己与 Agent 的会话（`/messagehub?entityId=<agent>&sessionId=<to_session | dm:<agent>>`），否则以观察模式打开 Agent 的会话（`ownerDid=<agent>&mode=observe&entityId=<对方>&sessionId=<收件箱会话>`）。没有绑定会话的 Session（work、self_check 等）在 WebUI 里进入自己的详情页。
- `home.workspaces` 从 Workspace Manager 查询已知目录，不按 Session 是否使用过或本机目录是否存在过滤。首页展示稳定 ID、Runtime、目录、使用属性、生命周期、最近可用性检查、错误与冲突，按稳定 ID 关联 Session 和产物。检查访问调用 `workspaces.check`；重新定位调用 `workspaces.discover` 并同时提交目标 ID 和预期登记修订，失败保留原登记。重新定位只供新的 Session 使用，既有 Session 的绑定保持不变。

### Loader 状态

`loader.status` →

```jsonc
{
  "agent_did": "…", "agent_id": "…", "who": "app:…@…", "agent_root": "…",
  "started_at_ms": 0, "now_ms": 0,
  "modules": [ { "name": "ui | task_mgr | self_check | self_improve | agent_state_service | webui",
                 "enabled": true, "running": true, "note": "…" } ],
  "hosted":  [ { "session_id", "class", "loaded", "loaded_at_ms", "drives", "last_result": DriveResult,
                 "last_result_at_ms", "idle_unload_secs", "reason" } ],
  "ui": { "scans", "last_scan_ms", "last_error",
          "inboxes": [ { "route_key", "session_id", "delivered", "dropped", "held", "last_at_ms" } ] } | null,
  "errors": [ { "at_ms", "source", "message" } ]
}
```

`hosted` 只在内存里，重启后由登记表重建，不是协议。

zone 内启动时，进程把 `AGENT_TOOL_HOST_ID` 设为 `<device did>/<app instance id>`（已设置则不动）：容器每次重建都会换 hostname，Session 的 runtime 绑定与 lease 用这个稳定身份。

## 测试

```bash
cargo test -p opendan -- --test-threads=1

# 真实 zone（DV 环境）：登录、Loader 状态、一次对话往返、经 Loader 观察 Session
# Agent：BUCKYOS_TEST_AGENT_CREATE=<名字>（没有就创建）| BUCKYOS_TEST_AGENT_ID=<agent_id> | 当前用户第一个 ready 的
cd test/test_opendan && deno run --config ../deno.json --allow-net --allow-env \
  --unsafely-ignore-certificate-errors test_agent_loader.ts ["<消息>"]     # OPENDAN_FOLLOW_S=300 继续打印后续消息（work session 的结果）

# WebUI 经 zone 网关（SSO 登录）
cd src/frame/opendan/web && pnpm exec playwright test -c playwright.zone.config.ts
```
