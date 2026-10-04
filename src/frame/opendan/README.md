# opendan — Agent Loader

`opendan` 以某个 Agent 的 runtime app 身份启动，托管这个 Agent 的 Session，并把 Agent State 作为服务提供出去。Agent 内核（推理循环、Context 调度、Turn、输入总线、Runtime、Agent State 文件协议）在 `llm_context` / `agent_tool` / [`libopendan`](../lib_opendan/README.md) 里；这里没有提示词、没有 behavior 名字的特判、不解析 LLM 输出、不直接改写 Session 状态。一个进程托管一个 Agent。

## 模块

| 文件 | 内容 |
|---|---|
| `main.rs` | 参数、zone 形态与 `--dev` 形态的环境装配、退出信号 |
| `loader.rs` | 启动顺序：AgentRoot 同步 → `agent.toml` → Agent State（文件实现，进程内登记）→ HTTP → Supervisor → 模块；退出时结束托管并等待 |
| `config.rs` | `agent.toml` 里 Loader 的部分：`[loader]`、`[[loader.ui]]`、`[loader.self_check]`、`[loader.self_improve]`、`[llm_context]`；配置错误在启动时报出并退出 |
| `rootfs.rs` | AgentRoot 目录布局、从 Agent 包同步素材（保留本地修改） |
| `ui.rs` | UI Session + Message Tunnel：inbox 发现、绑定、入站桥、出站 sink |
| `service.rs` | Agent State kRPC 服务、Session 观测、三个操作、Loader 状态 |
| `web/` | WebUI（只读为主），数据全部来自 kRPC |

托管循环在 `libopendan::host::Supervisor`：登记表里 `driver = 本进程身份` 且未结束（或有待处理 decide）的 Session 各有一个协程循环 `drive(Idle)`；启动后的第一次扫描就是恢复；空闲超过 `idle_unload_secs` 且没有待推进工作的 Session 被卸载，有新输入时再装载。

## 运行

```bash
# zone 内（node-daemon 启动，或 ./debug_jarvis.sh）
opendan --app-id <appid> [--agent-bin <Agent 包目录>] [--service-port <n>] [--web <目录>]

# 开发形态：不需要 zone，文件队列，没有 msg-center
opendan --dev --agent-root <dir> --agent-did <did> --queue-dir <dir> \
        [--who app:opendan@local] [--agent-bin <Agent 包目录>] [--port <n>] [--web <目录>] [--poll-ms <n>]
```

端口：`--service-port`（= `--port`）→ `$BUCKYOS_SERVICE_PORT` / `$OPENDAN_SERVICE_PORT`（调度器分配）→ 4060。`xagent` 可执行文件与 `web/` 目录取自 `opendan` 所在目录。

开发形态下用 xagent 创建由 OpenDAN 驱动的 Session：

```bash
export OPENDAN_AGENT_ROOT=<dir> OPENDAN_AGENT_DID=<did> LIBOPENDAN_QUEUE_DIR=<dir> LIBOPENDAN_WHO=app:opendan@local
xagent new --objective "…" --llm-context "$LC" --msg "…" --no-run     # 创建者 = 驱动者 = app:opendan@local
OPENDAN_AGENT_ROOT= xagent list --state-url http://127.0.0.1:4060/kapi/opendan   # 经 kRPC
```

## kRPC 接口

路径 `/kapi/opendan`，标准 kRPC（`POST`，`{"method","params","sys":[seq, session_token]}`）。调用方身份 `who` 来自 session token（verify-hub 签发）：Agent 的 owner（经任何 app）与 zone root 可访问；`--dev` 形态不校验。错误的 `error` 字段里带 JSON 文本 `{"kind","message",…}`（kRPC 会加前缀 `Failed due to reason: `，从第一个 `{` 起解析）（`kind` 取值同 `OpenDanError::to_json`，如 `not_found`、`invalid_argument`、`input_full`）。

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

### 操作（都只是向 Session 的输入总线投一条记录）

| method | params | 说明 |
|---|---|---|
| `session.stop` | `{sid, reason?}` | `control(stop)` |
| `session.decide` | `{sid, decision: accept \| discard, note?}` | `control(decide)` |
| `session.post` | `{sid, text}` | 以调用方身份向有队列的 Session 投一条文本消息 |

返回 `{index}`（`session.post` 另带 `key`）。投递后服务端让 Supervisor 装载该 Session。三个操作都要求 Session 有输入队列；没有队列的 Session（如一次性的 work session）返回 `channel` 错误，它的产物验收走 `artifacts.decide`。

### Loader 状态

`loader.status` →

```jsonc
{
  "agent_did": "…", "agent_id": "…", "who": "app:…@…", "agent_root": "…",
  "started_at_ms": 0, "now_ms": 0,
  "modules": [ { "name": "ui | self_check | self_improve | agent_state_service | webui",
                 "enabled": true, "running": true, "note": "…" } ],
  "hosted":  [ { "session_id", "class", "loaded", "loaded_at_ms", "drives", "last_result": DriveResult,
                 "last_result_at_ms", "idle_unload_secs", "reason" } ],
  "ui": { "scans", "last_scan_ms", "last_error",
          "inboxes": [ { "route_key", "session_id", "delivered", "dropped", "held", "last_at_ms" } ] } | null,
  "errors": [ { "at_ms", "source", "message" } ]
}
```

`hosted` 只在内存里，重启后由登记表重建，不是协议。

## 测试

```bash
cargo test -p opendan -- --test-threads=1
```
