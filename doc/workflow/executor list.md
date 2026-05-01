# BuckyOS Workflow Executor List

> 配套文档：[wokflow engine.md](./wokflow%20engine.md)（v0.4.0-draft）
>
> 本文档站在 **Executor 开发者 / 系统集成方** 的视角，说明 Workflow DSL 中的
> `executor` 应该如何分类、由哪层 adapter 实现、第一期哪些能力应先落地。

Workflow 引擎本身只认 Expr Tree（`Apply` / `Match` / `Par` / `Map` / `Await`）。
本文说的 "Executor 分类" 不是引擎内部多态，也不是调度器的长期原语，而是：

- DSL 作者在 `executor` 字段里能写什么。
- Executor 开发者应该实现哪类 adapter。
- 编排器遇到这类 `executor` 时应如何调用。
- 哪些能力属于第一期主路径，哪些只是后续扩展。

## 0. 一期范围

第一期目标是先把常见 Agent-Human-Loop workflow 跑起来，晚一点引入 Thunk 调度闭环。

> 我们认识到，定义标准的FunctionObject 不是一个容易的事情

因此一期的执行模型是：

1. `autonomous` Step 编译为 `Apply`。
2. 编排器根据 `executor` 前缀选择内置 adapter。
3. adapter 直接调用标准服务、AppService/HTTP 端点、Agent/Skill/Tool 包装服务。
4. 返回值直接回填到 `node_outputs`，再激活下游节点。
5. `human_confirm` / `human_required` 编译为 `Await`，由编排器等待人类输入。

FunctionObject / ThunkObject / Node Daemon Runner 是后续阶段的执行基础设施。它们重要，但不是第一期 executor 开发者必须实现的主路径。

术语上要区分两层：

- **定义层**：`FunctionObject` 描述一个 executor 能力，包括代码/包/脚本、参数约束、资源需求、结果类型等。
- **执行层**：`FunctionObject + params = ThunkObject`。编排器真正投递给 Scheduler / Node Daemon Runner 的是 `ThunkObject`。

## 1. 分类总表

| 类别 | DSL `executor` 形式 | Expr 类型 | 是否需要调度器 | 实际执行位置 | 幂等性默认 | 一期优先级 |
|------|--------------------|-----------|---------------|-------------|----------|------------|
| 标准服务 | `service::<name>.<method>` | `Apply` | 否 | Workflow 编排器进程 | 强幂等 | P0 |
| AppService / HTTP | `http::<endpoint-id>` / `appservice::<id>.<method>` | `Apply` | 否 | Workflow 编排器进程 | 默认非幂等 | P0 |
| Agent / Skill / Tool 语义链接 | `/agent/<name>` / `/skill/<name>` / `/tool/<name>` | `Apply` | 看最终目标 | 先展开为实际 executor 定义，再由对应 adapter 执行 | 由展开后的 executor 决定 | P0 |
| 人工输入 | `type: human_confirm` / `human_required` | `Await` | 否 | 编排器 + Msg Center | - | P0 |
| 内置算子 | `operator::<name>` | `Apply` | 否 | 编排器内置函数 | 通常幂等 | P1 |
| FunctionObject | `func::<objid>` / registry-resolved function | `Apply` |是 | Scheduler + Node Daemon Runner | 由 FunctionObject 决定 | 后续 |
| OPTask | `optask::<pkg>@<device>` | `Apply` | 是 | 指定 Node Daemon | 窄幂等 | 后续 / break-glass |

### 实际定义与语义链接

`executor` 字段里有两种不同语义：

- **实际 executor 定义**：用 `namespace::name` 表达，例如 `service::aicc.complete`、`http::file-classifier.classify`。这类名字已经能直接定位到一个 adapter 和调用协议。
- **语义链接**：用路径表达，例如 `/agent/mia`、`/skill/fs-scanner`、`/tool/image-normalizer`。它们不是最终 executor 定义，而是一个可解析的语义入口。

语义链接必须先通过 executor registry 展开为实际 executor 定义，例如：

```text
/skill/fs-scanner
  -> service::fs_index.scan
  -> http::fs-scanner.scan
  -> func::<function_object_id>
```

这让系统可以在不改大流程结构的情况下，调整某几个步骤的实现。Workflow 仍然说“这里需要扫描文件”，而 registry 可以把它切到标准服务、AppService endpoint、临时工具包或后续的 FunctionObject。

### 分类原则

- `service::`、`http::`、`appservice::` 是第一期最重要的实际 executor 定义。它们不需要 Thunk，也足以覆盖系统服务调用、用户通知、AI 能力、AppService 接入、轻量外部 API 调用。
- `/agent/`、`/skill/`、`/tool/` 在一期是语义链接，不直接等价于 FunctionObject。它们应先通过 registry 展开到一个实际 executor 定义。
- `human_confirm` / `human_required` 不是真正的 executor，但它是 Workflow 执行语义的一等能力，所以保留在本文档中。
- `func::<objid>` 是后续 FunctionObject 定义层的底层入口。运行时会结合参数生成 ThunkObject 后再投递。第一期可以先不暴露给普通 DSL 作者。
- `optask::` 不应成为长期常规 executor。它只适合低频运维、修复、测试等 break-glass 场景。

## 2. 标准服务 Executor @ workflow

### 语义

BuckyOS 的标准服务指系统内置、有稳定 RPC schema、整个 Zone 内访问语义一致的服务。命名为 `服务名.方法名`。典型例子：

- `aicc.complete`
- `msg_center.notify_user`
- `system_config.get` / `system_config.set`
- `task_manager.create_task`
- `kb.put` / `kb.search`

### 一期执行方式

- DSL：`executor: "service::aicc.complete"`
- Expr：`Apply`
- 编排器：识别 `service::` 前缀，直接通过 buckyos-api RPC client 调用对应服务。
- 输出：RPC response 作为该 Step 的输出回填。
- Cache：若 Step 声明 `idempotent: true`，编排器可用 `executor + input hash` 做结果缓存。

标准服务原则上应具备幂等能力。若服务内部不是天然幂等，服务实现者应通过 `task_id`、request id 或业务唯一键保证重复请求得到一致结果。

### 当前代码位置

- 服务 RPC client：`src/kernel/buckyos-api/src/`
- AI 类服务：`src/frame/aicc/`
- Msg 类服务：`src/frame/msg_center/`
- 编排器 Apply 调度入口：`src/kernel/workflow/src/orchestrator.rs::schedule_apply`

### 实现状态

🟢 **直执行通道已就位**。`WorkflowOrchestrator::with_executor_registry` 接入
`ExecutorRegistry`，命中的 `service::` executor 直接由编排器同步调用 adapter，
跳过 Thunk 投递；缓存与重试 / Human fallback 路径与原有 Thunk 路径行为一致。
具体 service adapter（buckyos-api RPC 桥接、`aicc.complete` 等）仍待按服务接入。

## 3. AppService / HTTP Executor @ workflow

### 语义

这一类用于把已有 HTTP endpoint 或 Zone 内 AppService 接入 workflow。它是第一期最实用的扩展机制：很多能力不需要先包装成 Thunk，也不需要调度到特定 Node，只要有稳定 endpoint 和 schema 就能被 workflow 调用。

推荐 DSL 形式：

```json
{
  "id": "classify_file",
  "type": "autonomous",
  "executor": "http::file-classifier.classify",
  "input": {
    "file": "${scan.output.file_obj_id}"
  },
  "idempotent": true
}
```

或 Zone 内 AppService：

```json
{
  "id": "extract_metadata",
  "type": "autonomous",
  "executor": "appservice::media-tools.extract_metadata",
  "input": {
    "file": "${scan.output.file_obj_id}"
  }
}
```

### 一期执行方式

- `http::<endpoint-id>` 由 endpoint registry 解析为 URL、method、auth、request schema、response schema。
- `appservice::<id>.<method>` 由 AppService registry 或 system_config 解析为 Zone 内访问地址。
- 编排器直接发起 HTTP/kRPC 调用。
- 默认非幂等；只有显式 `idempotent: true` 时才允许自动 cache 和安全重试。

### 与标准服务的区别

| 维度 | 标准服务 | AppService / HTTP |
|------|---------|-------------------|
| 注册方 | 系统内置 | App / workflow / 用户声明 |
| Schema 稳定性 | 平台承诺 | endpoint 自己承诺 |
| 幂等性默认 | 强幂等 | 默认非幂等 |
| 调用位置 | 编排器 | 编排器 |
| 一期是否需要 Thunk | 不需要 | 不需要 |

### 实现状态

🟡 **直执行通道已就位，endpoint registry 待补**。编排器侧 `ExecutorRegistry`
对 `http::` / `appservice::` 与 `service::` 共享同一条直执行路径（命中 adapter
则同步调用、不走调度器）。仍欠：endpoint / AppService registry（URL、auth、
schema 解析）以及具体的 HTTP/kRPC adapter 实现。

## 4. Agent / Skill / Tool Semantic Links @ workflow

### 语义

`/agent/`、`/skill/`、`/tool/` 是面向 DSL 作者的人类友好语义路径。它们描述“这一步需要什么能力”，而不是直接描述“这一步由哪个 executor 实现”。

| DSL `executor` | 含义 | 一期解析目标 |
|---------------|------|--------------|
| `/agent/<name>` | 一个 Agent 角色或能力（如 Mia / Jarvis / 自定义 Agent） | 实际 executor 定义，如 `service::...` / `http::...` / `func::...` |
| `/skill/<name>` | 被注册的技能语义 | 实际 executor 定义，如 `service::...` / `http::...` / 包入口 |
| `/tool/<name>` | 被注册的工具语义 | 实际 executor 定义，如 `service::...` / `http::...` / 包入口 |

第一期不要要求它们必须解析为 FunctionObject。对 executor 开发者来说，关键是实现 registry 展开：

1. 根据语义路径查到实际 executor 定义。
2. 校验 input/output schema。
3. 把调用交给实际 executor 对应的 adapter。
4. 把结果按 workflow 输出协议返回。

### 一期执行方式

`/agent/`、`/skill/`、`/tool/` 可以优先展开到三种后端之一：

- 标准服务：例如 `/agent/mia` 展开到 OpenDAN service 任务入口。
- AppService/HTTP：例如某个 skill 由 AppService 暴露 endpoint。
- 本地包入口：仅限开发/单机场景，由编排器 adapter 直接启动受控进程。

只要展开后的实际 executor 对外返回确定的 Step result，workflow 引擎不关心内部实现。这样大流程可以稳定引用 `/skill/fs-scanner`，而系统可以把它从 `http::...` 切换到 `service::...` 或后续 `func::...`。

### 实现状态

🟡 **部分设计，缺 registry**。当前 `compile_step` 只对 `executor` 字符串做 hash，没有真正把 `/agent/`、`/skill/`、`/tool/` 展开为实际 executor 定义。第一期需要先补 executor registry，而不是直接补 Thunk。

## 5. 人工输入 Executor @ workflow

### 语义

DSL 中表达为 `type: human_confirm` 或 `type: human_required` 的 Step。它不是真正的 executor，而是 `Await` 节点，由编排器原生处理。

| Step Type | 决策权 | 引擎行为 |
|-----------|-------|---------|
| `human_confirm` | 人类守门 | 展示 `subject_ref` 指向的对象，等待 approve / reject / modify |
| `human_required` | 人类执行 | 展示 prompt，等待人类按 `output_schema` 提交输出 |

### 与 Msg Center 的关系

人工输入可以理解为 `msg_center.notify_user` 的高级包装：

- 编排器生成等待事件。
- Msg Center adapter 把 prompt、schema、subject 推给用户。
- 用户响应后，编排器校验输出并继续执行。

### 当前代码位置

| 组件 | 文件 |
|------|------|
| Expr `Await` 节点处理 | `src/kernel/workflow/src/orchestrator.rs::enter_human_wait / handle_human_action` |
| 人类操作枚举 | `src/kernel/workflow/src/runtime.rs::HumanActionKind` |
| 静态校验 | `src/kernel/workflow/src/analysis.rs` |
| 通知通道 | `src/frame/msg_center/` |

### 实现状态

🟢 **核心闭环已实现**。`enter_human_wait` 和 `handle_human_action` 已覆盖 approve / modify / reject / retry / skip / abort / rollback。

🟡 **缺适配**。`step.waiting_human` 事件目前只进入内存事件流；还需要桥接 Msg Center，把 subject + prompt 推给用户。

## 6. 内置算子 Executor @ workflow

### 语义

内置算子是编排器进程内的小函数，用来做轻量、确定、无外部副作用的转换。例如：

- JSON 字段提取 / 合并
- schema normalize
- 小型路由判定
- 固定格式转换

推荐 DSL：

```json
{
  "id": "pick_files",
  "type": "autonomous",
  "executor": "operator::json.pick",
  "input": {
    "source": "${scan.output}",
    "path": "files"
  },
  "idempotent": true
}
```

### 一期执行方式

- 编排器识别 `operator::` 前缀。
- 从内置 operator table 查找函数。
- 同步或异步执行，直接回填结果。

### 边界

内置算子不应该承载重计算、长期运行、外部网络请求或有副作用的操作。这类需求应使用 `service::`、`appservice::`、`http::`，或通过 `/agent/`、`/skill/`、`/tool/` 语义链接展开。

### 实现状态

⚪ **未实现**。不是一期必须项，但对减少大量胶水 service 有价值，可作为 P1。

## 7. FunctionObject Executor @ scheduler

### 语义

FunctionObject 是后续阶段的 executor 定义对象。编排器执行某个 `Apply` 时，会用 `FunctionObject` 和已经解析好的参数构造 `ThunkObject`，再交给 Scheduler + Node Daemon Runner 执行。

两者关系是：

```text
FunctionObject + params = ThunkObject
```

其中 `FunctionObject` 属于定义层，回答“这个 executor 是什么、怎么运行、需要什么资源、参数和结果是什么”；`ThunkObject` 属于执行层，回答“这一次用这组参数执行哪个 FunctionObject”。

推荐长期入口：

```json
{
  "id": "embed_batch",
  "type": "autonomous",
  "executor": "func::<function_object_id>",
  "input": {
    "batch": "${plan.output.batch_obj_id}"
  },
  "idempotent": true
}
```

`/agent/`、`/skill/`、`/tool/` 未来也可以通过 registry 展开到 `FunctionObject`，但它们不是底层原语。

### FunctionObject 类型

当前代码里的 `FunctionType` 包括：

| FunctionType | 含义 | Runner hint |
|--------------|------|-------------|
| `ExecPkg` | 可执行包 | `package-runner` |
| `Script(<lang>)` | 脚本 | `script-runner:<lang>` |
| `OPTask(<lang>)` | 运维脚本 | `op-task-runner:<lang>` |
| `Operator` | 算子 | `operator-runner` |

### 当前代码位置

| 组件 | 文件 |
|------|------|
| FunctionObject / ThunkObject | `src/kernel/buckyos-api/src/thunk_object.rs` |
| Apply + params → ThunkObject 构造 | `src/kernel/workflow/src/orchestrator.rs::build_thunk` |
| Scheduler thunk runner | `src/kernel/scheduler/src/thunk_runner.rs` |
| Node 执行器 | `src/kernel/node_daemon/src/node_exector.rs` |

### 实现状态

🟡 **基础链路已有，但不作为一期主路径**。

已存在：

- `FunctionType` 枚举。
- Scheduler runner hint。
- NodeExecutor 对 `ExecPkg` / `Script` / `OPTask` 的执行计划。

未完成：

- executor registry：语义链接到实际 executor 定义 / FunctionObject 的解析。
- named_store 参数存在性检查。
- `Operator` 在 NodeExecutor 中仍未支持。
- 一期编排器侧的直接 adapter 还没有补齐。

## 8. OPTask Executor @ node_daemon

### 语义

OPTask 是设备视角的低频运维任务，典型用途包括修配置文件、安装依赖、重启服务、临时修 bug。

它不应作为普通 workflow 的长期一等 executor。更推荐的方向是把可变状态版本化，把运维动作表达成普通 Function Instance：

```text
exec(hash("deploy.sh"), node_state("node3", 3701))
```

这样调度器看到的仍是函数求值，而不是一条命令式运维指令。

### 使用边界

`optask::` 只建议用于：

- break-glass 修复。
- 测试环境运维。
- 迁移期临时工具。
- 尚无法表达为 service/function 的底层操作。

### 当前代码位置

| 组件 | 文件 |
|------|------|
| FunctionType `OPTask` | `src/kernel/buckyos-api/src/thunk_object.rs` |
| runner hint | `src/kernel/scheduler/src/thunk_runner.rs::build_scheduling_hint` |
| Node 端执行计划 | `src/kernel/node_daemon/src/node_exector.rs::build_execution_plan` |

### 实现状态

🟡 **基础执行分支存在，但完整语义未实现**。

缺口：

- `optask::` DSL 前缀解析。
- 目标 device 绑定。
- Device State Version 注入和拒绝重复执行。
- 脚本白名单 / FileObjectID 审计。

## 9. 一期实现优先级

| 优先级 | 类别 | 下一步动作 |
|-------|------|------------|
| P0 | `service::` | 编排器 `schedule_apply` 加 service adapter，直接 RPC 调用 |
| P0 | `http::` / `appservice::` | 定义 endpoint registry，编排器直接调用 |
| P0 | `/agent/` / `/skill/` / `/tool/` | 定义 executor registry，先展开到实际 executor 定义 |
| P0 | `human_confirm` / `human_required` | 桥接 Msg Center 推送与响应 |
| P1 | `operator::` | 增加编排器内置 operator table |
| 后续 | `func::<objid>` | 接入 FunctionObject registry；运行时由 Apply 参数构造 ThunkObject，再投递 Scheduler / Node Daemon Runner |
| 后续 | `optask::` | 仅按 break-glass 能力推进，避免成为常规 workflow executor |

## 10. 选择建议

给 DSL 作者和 executor 开发者的粗略指引：

```text
要调用 BuckyOS 内置能力？
└── service::<name>.<method>

要接入已存在的 AppService 或 HTTP API？
└── appservice::<id>.<method> 或 http::<endpoint-id>

要调用 Agent / Skill / Tool？
└── /agent/<name> / /skill/<name> / /tool/<name>
    └── 一期通过 executor registry 展开到实际 executor 定义

要等待人类确认或输入？
└── type: human_confirm / human_required

只是做轻量数据转换？
└── operator::<name>

需要调度到具体节点、利用数据亲和性、跑重计算？
└── 后续使用 func::<objid> 引用 FunctionObject；运行时生成 ThunkObject

需要低频运维修复？
└── 后续 optask::<pkg>@<device>，并标记为 break-glass
```
