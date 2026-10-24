# BuckyOS Agent-Runtime 集成 ComputerUse：CLI 实施 TODO

| 项目 | 内容 |
| --- | --- |
| 文档版本 | 0.2，已确定 CLI 路线的实施 TODO |
| 更新日期 | 2026-10-23 |
| 状态 | daemon 子集已实现；本文的上层 ComputerUse CLI、模型图片桥接及完整验收仍是交付目标 |
| 设计基线 | [AgentRuntime 设计](<AgentRuntime 设计.md>)、[Agent Daemon 协议](../arch/agent_daemon.md)及当前工作区源码 |
| Agent 入口 | 现有 `shell` 工具调用 `buckyos computer …` |
| 首期目标 | Windows、macOS 用户桌面上的观察—动作—再观察闭环 |
| 协议名称 | `buckyos.computer.v1`；复用现有 Executor 连接、身份和操作账本 |
| 工具数量 | 不新增 `use_computer`、`browser_*`、`desktop_*` 或专用读图 tool |

> 本文取代 0.1 草案中“新增第五个标准 Function、浏览器先交付、桌面放 P3”的决策。保留观察有效性、输入语义、去重、并发与恢复约束，将其落实到 daemon、SDK/CLI 和通用工具结果链路。
>
> 下文所有 `buckyos computer` 命令均待实现，不能当作当前可执行命令。复选框按完整交付条目记录，仅在对应实现、检查和证据齐备后勾选；daemon 子集的实现及验证进度单独记录，不据此推定完整链路已通过。

> daemon 实施进展：本仓库已实现共享 Computer 协议、复用 Executor 账本的执行入口、macOS 14+ 和 Windows 当前用户会话原生驱动。Windows 提供前台 `.exe` 宿主、ZIP 打包入口、Win32/GDI 主显示器 visual/auto 观察、点击、Unicode 输入、按键、按行滚动与 delay 等待；不提供 Session 0 服务、semantic/hybrid、drag 或像素滚动。Windows x64 已通过相关单元/生命周期测试、真实只读截图和静态 CRT debug 构建及导入核验，release 发行归档尚未构建。实际支持范围及证据见 [macOS ComputerUse](../arch/agent_daemon.md#macos-computeruse)、[Windows ComputerUse](../arch/agent_daemon.md#windows-computeruse) 与 [本机验证入口](../arch/agent_daemon.md#本机验证入口)。本草案仍是完整交付目标；下文的上层 CLI、图片桥接和 T1–T6 整体验收不能据 daemon 子集实现勾选完成，Windows 真实桌面输入与完整 Agent loop 尚未验收。

## 0. 已确定的决策

1. **能用 CLI 就沿用 CLI，Agent 不增加新的 tool。** ComputerUse 是 daemon 的执行能力，通过已有 shell 入口使用；不增加 Runtime kind，不为 GUI 扩展 `AgentRuntime` trait。
2. 命令组合、变量、循环和条件判断使用现有 shell/脚本能力。一次 ComputerUse 动作请求表达一个受控动作；daemon 不新增脚本语言，也不接受自然语言任务后自行规划。
3. CLI、SDK 和未来确有需要的模型适配器共用同一执行协议、目标绑定、操作账本及授权。专用 function、MCP 入口和厂商原生 Computer Tool 适配不在首期任务内。
4. ComputerUse 的失败概率不决定工具形态。GUI 依赖观察、焦点和共享输入资源，必须明确部分执行、未知结果、后置验证和禁止盲目重放。
5. 原生文件、应用或业务 API 可以完成的工作优先用已有 CLI/API；GUI 用于缺少接口或必须验证界面的部分。
6. Windows/macOS 原生桌面闭环进入首期。浏览器语义 Driver 是可选后续扩展，不作为桌面交付的前置依赖。
7. “模型看见截图”是首期完成条件。输出图片路径、artifact URI 或 base64 文本不算完成；补通用多模态结果通道，不因此新加读图 tool。

CLI 可以既用于逐步决策，也用于自动组合：

- **视觉步骤**：shell 调用结束 → 模型实际收到图片 → 判断下一步 → 下一次 shell 调用。
- **可检查的步骤**：脚本解析结构化状态和已知条件，使用新观察继续有限操作。
- **不确定步骤**：遇到 `partial / unknown`、审批、目标过期或焦点变化即停住，查询、重新观察或交回模型。

`set -e` 和退出码只能帮助停止脚本，不能证明 GUI 业务已经成功。

## 1. 当前实现与需要补齐的差距

以下为核对过的当前实现；各项“待补齐”不是现有保证。

| 当前事实 | 入口 | 待补齐 |
| --- | --- | --- |
| AgentRuntime 负责环境、工具装配、身份核验和中断解释，并未限定只能有四个工具 | [runtime/mod.rs](../../src/frame/agent_tool/src/runtime/mod.rs)、[Runtime 设计第 2 节](<AgentRuntime 设计.md#2-核心抽象>) | 复用已有入口，ComputerUse 不增加 trait 方法或 runtime kind |
| `executor_list / executor_call` 已按在线 daemon 目标执行操作，不改变 Session Runtime binding | [runtime/executor.rs](../../src/frame/agent_tool/src/runtime/executor.rs) | 复用目标发现、路由与原操作定位思路 |
| 控制面 kRPC 获取路由，执行数据经 WSS Command Stream 到 daemon；已有 `op_id / store_id / query / cancel`，Computer 请求与 PNG 结果复用此账本 | [agent_daemon.md](../arch/agent_daemon.md)、[computer.rs](../../src/kernel/buckyos-api/src/computer.rs) | SDK/CLI 目标绑定、派发前 locator 和模型图片结果链路 |
| daemon 已有 Linux/macOS/Windows 当前用户宿主；macOS/Windows 原生 Driver 提供主屏视觉输入子集，Linux GUI 仍 unsupported | [daemon.rs](../../src/kernel/buckyos_agent_daemon/src/daemon.rs)、[desktop.rs](../../src/kernel/buckyos_agent_daemon/src/computer/desktop.rs)、[package_agent_daemon.py](../../src/tools/package_agent_daemon.py) | Windows/macOS 真实桌面与完整 Agent loop 验收、语义/多显示器等未支持范围 |
| shell 会注入 `XLLM_CALL_ID`；简单命令可透传 `AgentToolResult`，复合脚本不自动透传 | [llm_bash.rs](../../src/frame/agent_tool/src/llm_bash.rs) | 子操作身份及不依赖 stdout 形式的附件反馈 |
| `buckyos` CLI 使用 `--output json`，输出全局 `schema_version / ok / data / error / meta` 信封，不是 `AgentToolResult` | 同级 websdk 的 `cli/core/argv.ts`、`output.ts` | 保持同一 CLI 输出合同，由消费侧做通用适配 |
| 工具结果到模型的现有路径仍存在字符串化；基础消息类型虽有图片，不能据此声称闭环已通 | [context_loop.rs](../../src/frame/llm_context/src/context_loop.rs)、[xllm.rs](../../src/frame/agent_tool/src/xllm.rs)、[aicc_client.rs](../../src/kernel/buckyos-api/src/aicc_client.rs) | 结果、快照、恢复、裁剪、Provider 全链路保留图像 |
| 通用 shell 超时说明可能建议重试；CLI 通用错误归一化可能把 timeout 标为 retryable | `llm_bash.rs`、同级 websdk 的 `cli/core/errors.ts` | 保留远端副作用状态，不能因停止 CLI 就宣称动作停止 |
| 文件工具策略不隔离任意 shell 子进程；通用 Runtime policy/approval 仍有规划项 | [Runtime 设计第 6、10 节](<AgentRuntime 设计.md>) | daemon 授权、平台权限和已有控制面逐项接入，不假设已有统一沙箱 |

现有 `buckyos executor` CLI 位于同级 `buckyos-websdk/cli/modules/executor.ts`，可参考它的“先保存 locator，再取得路由并提交”流程。新增源码放在 websdk，不能只修改 BuckyOS rootfs 中的生成副本。

## 2. 分层与权威归属

~~~mermaid
flowchart TD
    A["Agent / LLM"] --> S["现有 shell 工具"]
    S --> C["buckyos computer CLI / SDK"]
    C --> R["现有控制面 kRPC + WSS Command Stream"]
    R --> D["buckyos-agent-daemon"]
    D --> J["操作账本、Computer Session、授权、交互调度"]
    J --> H["用户桌面会话内 Driver / Helper"]
    H --> W["Windows / macOS"]
    D --> O["Observation / Artifact"]
    O --> B["CLI 与宿主的通用结果附件桥接"]
    B --> S
    S --> A
~~~

| 层 | 负责 | 不承担 |
| --- | --- | --- |
| LLM | 根据已收到的观察决定下一步、核验任务结果 | 通过参数声明自己已获授权 |
| CLI / SDK | 参数校验、持久 locator、提交和查询、统一结果输出、附件登记 | 模拟第二个 Agent；自行重试未知动作 |
| xllm / shell 宿主 | Run/tool-call 关联、通用多模态结果、持久化和中断说明 | 从任意 stdout 文本猜测可信附件或后台动作已取消 |
| daemon | Computer Session、权威操作账本、去重、授权、冲突域调度、取消和查询 | 对外部 GUI 业务承诺 exactly-once 或回滚 |
| 用户会话 Driver / Helper | 截图、可访问性读取、输入、目标核验、输入释放、平台能力报告 | 绕开 daemon 授权或自行选择另一个用户桌面 |
| 控制面 | 资源分配、授权、批准、暂停、恢复、撤销 | 由模型生成的文件或字段取得批准资格 |

CLI 本地保存的 locator 是恢复索引，daemon 的操作账本才是执行事实来源。CLI、宿主或未来适配器不能分别维护互不关联的去重日志。

`Computer Session` 与 OpenDAN `AgentSession` 是不同对象；前者描述可交互资源，后者管理 Agent 对话和执行生命周期。

## 3. 目标绑定、能力和桌面资源

### 3.1 显式 Computer binding

控制面建立授权绑定，CLI 使用 `--binding <binding_id>`。首期要求显式提供 binding；不猜“当前机器”“最新在线设备”或“当前活动窗口”。

绑定至少关联：

~~~text
binding_id
authorized_principal / agent_id
executor_id / profile_id
store_id / origin_generation
interactive_user / interactive_session_id / desktop_epoch
computer_session_id / surface_id / surface_epoch
interaction_resource_id
capabilities / policy_version / expires_at
~~~

- 当前 shell 可以在容器或另一台机器上运行，Computer binding 不随 shell 的 cwd 或 Runtime target 隐式改变。
- `generation` 标识连接代次，`store_id` 标识账本。重连后查询历史操作可以在验证同一 executor、profile 和原 store 后使用新路由；不得把旧操作重新提交到新账本。
- 用户注销、切换桌面、Driver 重建或无法证明连续性的重连使旧观察失效。新动作必须验证新的绑定和代次。
- 同一真实桌面/输入焦点是同一冲突域，不能按 CLI 进程或单独 `session_id` 各自加锁。
- 首期每个 binding 一个明确 Surface。多窗口、标签页、显示器切换后续通过显式控制命令扩展，不在动作执行时暗中切换。

### 3.2 能力与就绪状态

`inspect` 同时返回静态支持和动态就绪状态，例如：

~~~json
{
  "binding_id": "desktop-main",
  "backend_kind": "desktop",
  "capabilities": {
    "capture": {"supported": true, "ready": true},
    "input": {"supported": true, "ready": true},
    "accessibility": {"supported": true, "ready": false, "reason": "permission_required"},
    "observation_modes": ["visual"],
    "commands": ["click", "type", "keypress", "scroll", "wait_for"],
    "focus_guard": "surface"
  }
}
~~~

实际能力由平台探测，不把所有桌面都声明成拥有可访问性树。锁屏、未登录、授权撤销、UIPI 限制或 helper 不可达要返回具体状态。

### 3.3 观察模式

| 模式 | 契约 |
| --- | --- |
| `visual` | 返回可交互截图及坐标元数据；模型必须能够接收图像 |
| `semantic` | 返回可访问性/控件快照、真实元素引用及覆盖/截断信息 |
| `hybrid` | 返回树与图像，报告各自采样时间和一致性 |
| `auto` | 有可用语义能力则优先使用；否则在模型/宿主支持图像时用 visual，并报告选择原因 |

显式模式不可用时报错，不静默替换。模式只影响观察，不产生权限。首次桌面交付以 `visual` 为必需，semantic/hybrid 根据平台支持逐项完成。

默认模式随 binding 配置，每次 `observe --mode` 或动作 `feedback_mode` 可以覆盖。首期不必增加有状态的 `set_mode` CLI；减少跨调用的隐式状态。

## 4. CLI 合同

### 4.1 命令树

在同级 websdk 新增 `cli/modules/computer.ts` 并按现有 registry 注册。保持 `buckyos <module> <verb>` 两层命令树。

| 命令 | 作用 | 关键参数 |
| --- | --- | --- |
| `buckyos computer inspect` | 查询绑定、能力、权限与就绪状态 | `--binding` |
| `buckyos computer observe` | 生成新观察 | `--binding --mode`，可选 `--present` |
| `buckyos computer act` | 提交一个强类型动作并有界等待 | `--input action.json --locator operation.json`，可选 `--present --wait-ms` |
| `buckyos computer query` | 只读查询原操作当前事实 | `--locator` |
| `buckyos computer wait` | 有界等待原操作变化/结束 | `--locator --wait-ms` |
| `buckyos computer result` | 读取原操作结果；不完整时明确返回 | `--locator`，可选 `--present` |
| `buckyos computer cancel` | 请求取消原操作并报告确认程度 | `--locator` |

这些是 CLI 子命令，不是新增 LLM tools。发现入口沿用 `buckyos command describe computer act` 和 `--help`；不要另维护一套 function schema。

- 沿用全局 `--output json`、`--input <path|->`、`--timeout 30s`、trace/idempotency 参数；不引入 `--json` 或 `--agent-json` 双输出模式。
- `--input` 接收文件路径或 stdin，不使用未实现的 `@file` 语法。argv 与 input 文件重复指定同一字段仍按现有 CLI 规则拒绝。
- `--locator` 是 CLI 执行侧的文件路径；如果 shell 在远端，它不自动变为控制侧 Run 目录中的文件。宿主须登记可恢复关联或由执行侧提供持久存储。
- `--wait-ms` 限制本次等待；动作请求中的 `execution_timeout_ms` 限制 daemon 执行预算。全局 `--timeout` 只限定客户端调用总预算，不能被解释为远端取消确认。
- `--present` 请求把获准的观察实际交给当前模型调用，详见第 9 节；不是把图片路径打印成文本。

### 4.2 动作输入

统一的判别字段为 `command`，未知字段、未知枚举、非法组合和越界值必须拒绝。例：

~~~json
{
  "binding_id": "desktop-main",
  "command": "click",
  "target": {
    "kind": "point",
    "observation_id": "obs_42",
    "x": 420,
    "y": 310,
    "space": "screenshot"
  },
  "button": "left",
  "count": 1,
  "feedback": "after",
  "feedback_mode": "visual",
  "execution_timeout_ms": 10000
}
~~~

身份、授权、批准和租约不能由 action JSON 声明。CLI 从已认证会话获得授权主体，daemon 重新校验。`XLLM_CALL_ID`、trace、Run ID 只用于关联，不是凭据。

SDK 可提供便利方法，但必须归一化成同一个动作协议。服务端进行完整联合类型校验，不能只依赖 CLI 当前 JSON Schema 子集。

### 4.3 统一结果信封

保持 `buckyos` 全局信封：成功在 `data` 中携带业务返回，失败在 `error.details` 中保存已知事实。Computer 命令统一使用 `data.result` 或 `error.details.result` 承载同一个 `ComputerResult`，避免脚本面对多套业务结构。

~~~json
{
  "schema_version": 1,
  "ok": true,
  "data": {
    "result": {
      "api_version": "buckyos.computer.v1",
      "operation_id": "op_001",
      "binding_id": "desktop-main",
      "operation_state": "completed",
      "execution": {"state": "completed"},
      "feedback": {"state": "included"},
      "observation": {"observation_id": "obs_43"},
      "locator": ".computer-use/search-001/click.locator.json"
    }
  },
  "meta": {
    "command": "computer.act",
    "trace_id": "trace_001",
    "duration_ms": 180
  }
}
~~~

示例省略完整 Observation。只读 inspect/observe 不要求伪造 UI 动作 ID；未提交过动作时 `operation_id / locator` 可以不存在。

- stdout 只输出一个 JSON 信封；日志/进度走 stderr。handler 返回数据，由 CLI 框架打印，不能自行追加第二份 AgentToolResult。
- 脚本可用 `jq '.data.result // .error.details.result'` 读取业务结果；没有结果时不能推断动作尚未执行。
- 如需内部 AgentToolResult，宿主在通用消费层转换，不改变 CLI 对用户和脚本的输出合同。
- transport 错误必须保留原 locator 和 `execution.state=unknown`。未知副作用的 `error.retryable` 为 false，恢复提示为查询原操作；不能落入通用“timeout → retryable=true”的错误映射。
- 结果、错误和 artifact 的持久记录要一致；截图失败不得覆盖已经执行的事实。

### 4.4 退出码

复用现有 CLI 退出码，不新增 ComputerUse 私有的数字体系。

| 返回情形 | 退出码与约束 |
| --- | --- |
| inspect/observe 正常完成；act 确认完成且要求的反馈已就绪 | 0；使用 --present 时还须完成附件登记，不证明模型已在脚本中途看到图片或任务的业务结果成功 |
| 参数/认证/权限错误 | 2 / 3 / 4 |
| 服务不可用 | 5；如果已有派发可能，仍携带 unknown |
| 动作拒绝/失败、仍 pending、需要批准、未知执行结果 | 6，使用不同结构化错误码区分 |
| 已确认部分执行 | 7 |
| 等待/执行超时 | 8；execution 单独说明是否未开始、执行中、部分或未知 |
| 内部错误 | 9；不能自动等同未派发 |
| query 成功读取状态 | 0，即使被查询操作仍 running 或 unknown |
| wait/result | 原操作成功完成且要求的反馈已就绪才为 0；反馈 pending 按未完成处理，反馈 failed 返回操作错误并保留已完成的执行事实 |
| cancel | 0 仅表示成功处理取消请求；必须继续检查 `cancel.state` |

`act` 返回 pending/approval/unknown 时不能以 0 让普通 `set -e` 脚本继续下一动作。`query` 成功、`cancel` 成功不具有同样的动作完成含义。

## 5. 操作身份、状态与恢复

### 5.1 派发前持久记录

每次副作用动作的顺序：

1. 解析请求，验证 binding，分配独立 `operation_id`，保存规范化请求摘要。
2. 在 `--locator` 指定位置以排他创建方式持久写入完整 locator；验证落盘成功后才能发送。
3. locator 至少记录 executor/profile/store、原 generation、binding/桌面代次、operation ID、请求摘要、Run/tool-call/子调用关联。
4. Agent 调用在发送前，经宿主桥接把父 Run/tool call 到子 operation/完整 locator 的索引持久写入控制侧 Run manifest，并等待确认。登记不受 --present 开关控制；独立终端调用只要求本地 locator。未部署这条恢复桥接的远端 shell 明确拒绝 Agent 管理的 act，不能先发送后补登记。
5. daemon 先原子写入持久操作占位，再取得有效租约、校验目标并执行。
6. 更新执行事实及结果；CLI/宿主保存结果，后续只查原操作。对非 Run 终端调用，原 locator 是用户查询和恢复的入口。

同一 shell 调用内多个动作有不同 operation ID，并共享父 tool-call 关联。不能把 `XLLM_CALL_ID` 直接当每个子动作的去重键。独立终端调用没有 Run 时生成独立关联，但仍要求 locator。

`--locator` 已存在时，`act` 必须拒绝新提交并提示 query，不覆盖、不自动重放。脚本重启也不能自动删除 locator 或换新目录“重试”。首次新任务使用独立目录；已知原操作完成后，新的业务意图才创建新的操作记录。

### 5.2 状态分开表达

| 字段 | 值与含义 |
| --- | --- |
| `operation_state` | `queued / awaiting_approval / running / completed / failed / cancelled / unknown`，表示账本中的生命周期 |
| `execution.state` | `not_started / in_progress / completed / partial / unknown`，表示实际输入执行事实 |
| `feedback.state` | `pending / included / not_requested / failed`，表示 daemon 观察采集和结果 artifact 的就绪状态；等待批准/排队/执行中尚未采样时为 pending |
| `presentation.state` | `not_requested / registered / delivered / failed`，表示当前宿主调用的图像呈现状态，由 CLI/宿主记录，不改写 daemon 的动作事实 |
| `cancel.state` | `requested / confirmed / already_finished / unknown`，仅在取消相关结果中出现 |

动作完成不等于业务成功；点击“发送”完成不能直接声称邮件已发送。输入可能触发自动保存，移动鼠标也可能改变 UI，不能简单按命令名认定无副作用。

`unknown` 不是“失败且可以重试”，也不是“已经停止”。后续查询可得到更明确的事实。动作执行完成而截图/附件交付失败时，保留 completed，补观察或重新交付图片，不重复动作。

feedback included 只证明原结果的观察/artifact 已生成。CLI --present 成功登记为 presentation registered；宿主实际将该图装入并提交模型请求后才记录 delivered。图片被预算裁剪、转换失败或无法交付时必须明确记录并反馈，不能伪造 delivered。一次旧调用的 delivered 不代表新的调用/模型上下文也已收到；query 仅查询，result --present 重读时按当前调用重新登记。

### 5.3 去重与断线

- 同 ID、同请求：查已有操作、附着等待或返回保存结果，不再次派发；同 ID、不同请求返回 `request_conflict`。
- 不同 ID、相同参数：属于不同动作，不能按参数哈希合并。
- 动作发出后崩溃且无完成记录：保留 unknown，不自动重放。
- locator 损坏、丢失、账本改变、结果过期分别报告，不能统一解释成“发送前失败”。
- daemon 明确未收到原操作也不能让恢复代码自动提交；重规划后按新动作处理，并保留旧操作事实。
- 只读 query/result 可以重试；observe 重拍产生新的 observation ID，不能在旧 ID 下替换图像。
- 去重记录和 tombstone 保留时间须覆盖支持的重传窗口；过期 ID 不得悄悄作为新动作执行。
- 跨重启查询验证原 store；改变 generation 不等于账本丢失，改变 store 也不能靠相同 operation ID 冒充原执行环境。

不承诺 GUI 外部业务的 exactly-once；保证的是支持范围内不会因重传、CLI 重启或恢复而重复派发。

### 5.4 超时与取消

客户端 wait 超时仅结束等待。CLI 被杀、shell 被取消、WSS 断开都不等于 daemon 已停止动作。

- 显式 cancel 进入 daemon 的同一操作状态机，取消未执行队列并尝试终止运行输入。
- 已完成动作保持 completed；已输入一部分保持 partial；不可确认则 unknown。
- Driver 在异常路径尽力释放鼠标/修饰键，清理失败应暂停该交互资源。
- 执行截止时间由 daemon/helper 执行，不依赖客户端连接仍在。客户端等待截止时间单独处理。
- shell 的通用超时/中断说明必须保留已登记子操作，指向 query/cancel；不得建议整个 GUI 脚本自动重跑。
- 原动作是否仍在执行不明时，不能仅让租约过期就允许第二个写者进入。需完成 fencing、终止旧 worker 或确认资源可安全接管。

## 6. Observation、Target 与坐标

### 6.1 观察记录

daemon 保存不可变的观察及后端映射。建议结构：

~~~json
{
  "observation_id": "obs_42",
  "binding_id": "desktop-main",
  "surface_id": "display_01",
  "surface_epoch": 3,
  "interaction_revision": 18,
  "requested_mode": "visual",
  "effective_mode": "visual",
  "selection_reason": "explicit_request",
  "capture_started_at": "2026-10-23T22:30:00.100Z",
  "capture_completed_at": "2026-10-23T22:30:00.145Z",
  "consistency": "best_effort",
  "screenshot": {
    "artifact_ref": "artifact_img_42",
    "mime_type": "image/png",
    "width": 1280,
    "height": 720,
    "space": "screenshot",
    "interactive": true
  },
  "focus": {"guard": "surface"},
  "warnings": []
}
~~~

semantic 模式附带格式、文本/结构化节点、引用、coverage 和 truncated；hybrid 同时附带两类内容和采样区间。截图与树通常不是原子快照，检测到导航或明显漂移时重采样/报错。

`surface_epoch` 表示交互面身份代次；`interaction_revision` 表示已知的交互变化，不声称覆盖所有动画、DOM 或外部输入。

### 6.2 Target 类型

~~~typescript
type ElementTarget = {
  kind: "element";
  observation_id: string;
  ref: string;
};
type PointTarget = {
  kind: "point";
  observation_id: string;
  x: number;
  y: number;
  space: "screenshot";
};
type FocusTarget = {
  kind: "focus";
  observation_id: string;
};
~~~

- 引用由 Observation Store 产生并绑定真实后端节点。节点消失后不能按相同文字、角色或下标替换另一个节点；不能允许模型随意猜 selector。
- point 坐标来自模型实际收到的那张可交互图像，原点左上、向右/下为正，必须是有限整数且位于图内。
- daemon/Driver 保存缩放、裁剪、DPI、窗口/显示器偏移映射。full-page 拼接图可以只读展示，但不能标为可直接点击。
- 如果宿主缩放图片，应固定交付版本并维护映射及尺寸；不能让模型看到缩放图却按原图坐标执行。
- focus 至少核验授权桌面、Surface 和代次；能获取焦点节点/输入上下文时也须核验。弱焦点保护在能力中明确报告。
- 元素失败不自动猜坐标，坐标失效不自动猜同名元素。

### 6.3 有效性规则

1. 动作必须引用属于当前授权 binding、Surface、epoch 且未过期的观察。
2. 自身动作或已检测到的外部交互改变 revision；可能改变 UI 的 move 也默认使旧观察失效。
3. 每一步动作后用 feedback 或 observe 获取新观察。不能把旧坐标换绑到模型未见过的新图上。
4. 单纯生成只读观察不必使同 revision 的其它观察全部失效，但仍需 TTL 和目标校验。
5. 动作执行前重新检查真实目标/焦点/坐标映射；校验与输入尽量放在同一临界区。
6. wait_for 开始时绑定目标，等待过程中按该身份跟踪，不把正常的状态变化全部当作过期；导航或目标被另一个节点替换应结束等待。
7. 人工接管、锁屏、注销、显示布局改变和不能证明连续性的重连使相关旧观察失效。

观察 ID 不能消除检查—使用之间的 GUI 竞争。资源授权、前置核验、输入后验证仍然必要。

## 7. 动作语义

所有动作经 `computer act --input action.json` 提交，以下为 `command` 分支。首期实现支持的子集必须在 inspect 中如实声明。

| command | 输入与约束 |
| --- | --- |
| `click` | element/point；button 为 left/right/middle，count 为 1/2，可选 modifiers。执行可操作性检查，不默认 force；Accessibility Invoke 不自动等同物理点击 |
| `type` | element/focus、text、delivery=text/keys。在当前光标/选区输入，不主动清空整个字段，不自动 Enter，不隐式使用共享剪贴板 |
| `keypress` | element/focus、keys。一次组合键，修饰键加一个普通键；Control 与 Meta 区分，异常路径释放 |
| `scroll` | element/point、delta_x/y、unit=pixel/line。正值向右/下，单位按 Driver 能力声明；输入量不等于实际移动距离 |
| `move` | element/point。可能触发 hover，参与审计和观察失效 |
| `drag` | from/to、可选 duration_ms/path。同一 Surface 和 Observation，两个目标同为元素或同为坐标；失败报告 partial/unknown 并清理输入 |
| `wait_for` | condition 为 delay/text/element_state，有界且可取消；不接受任意代码 |

补充约束：

- `delivery=text` 不承诺完整逐字符键盘事件；`keys` 不支持某种文本时明确报错。`type` 不能直接映射成清空后 fill；后续 `semantic.fill` 显式表示替换。
- desktop 的 pixel 与浏览器 CSS 滚动单位不能用固定常数伪装成等价。
- `text` 等待只使用已声明的文本读取能力，不隐式启动 OCR/另一个模型；只有截图的 Driver 可先只支持 delay。
- `element_state` 只跟踪已绑定元素；hidden 可以由其移除满足，但不能把新文档同名节点当成原节点。
- 反馈默认为 `after`，可选 none；feedback_mode 仅在 after 下有效，不修改 binding 默认值。
- 超时、拖动时长、文本长度、路径点数和等待预算由 Schema 与执行端共同限制。
- Browser Driver 若后续接入 Playwright，只映射明确支持的动作。browser navigate/tabs/dialog、semantic fill/select 均通过能力扩展声明，不混入桌面默认能力。

脚本可以组合这些命令。daemon 不执行任意浏览器代码、shell 片段或批量 DSL；这是服务端命令边界，不限制 Agent 在现有 shell 中编排客户端调用。

## 8. 并发、批准、授权与回收

### 8.1 租约与人工接管

- 由 daemon 按 `interaction_resource_id` 串行调度写动作；观察与动作协调，避免把拖动中间态当成稳定观察。
- 模型思考期间不持有不可中断的按键/鼠标锁。binding 的控制租约可以有界保留/续期，但过期后新动作必须重取租约和新观察。
- helper/worker 校验 fencing token，拒绝旧执行者的迟到请求。仅修改 Broker owner 字段不够。
- 人工接管暂停新动作、取消队列并清理输入；恢复后重新观察。已送达应用的输入不能撤回。
- 首期可以使用专用测试用户桌面；直接控制用户桌面时必须具备可见状态、暂停/接管和撤销入口。浏览器 Context 不等于 OS 隔离。

### 8.2 异步批准沿用原操作

需要批准时，持久化 `awaiting_approval` 操作并返回原 locator，CLI 以非零退出码停止依赖动作。

可信控制面批准/拒绝原 operation。批准绑定主体、资源、请求摘要、目标上下文和有效期；批准后重新取得租约、检查授权及原目标。目标变化时返回未执行/过期，不偷偷重定位后继续。

CLI 通过 query/wait/result 获取结果，cancel 可以撤销等待中的操作。批准、取消和开始执行必须在同一状态机中原子仲裁，不能在取消已确认后执行。批准等待期间不持有输入锁。

不为 ComputerUse 增加独立的 LLM deferred tool 机制，也不让模型生成批准文件或自行调用无保护端点。只有策略确实要求批准时才进入该流程，不将所有桌面动作一律设为人工确认。

### 8.3 授权与数据

daemon 校验真实主体、profile、绑定资源及每项操作。观察、输入、访问应用、文件/网络、数据传出和批准规则分别落实；截图/树本身也可能含敏感数据。

CLI 入口不会削弱授权，function 入口也不会自动提供 OS 隔离。如果同一 OS 身份的任意 shell 已能直接控制桌面，不得声称 GUI 命令上的允许列表构成强安全边界；需要的 worker 隔离、IPC ACL、凭据和网络限制须明确实现和验收。

凭据优先使用已有安全注入/用户接管路径，不默认把长期密码放入 type 参数和日志。观察、artifact、输入摘要和操作日志有一致的 ACL、留存与删除策略。外部界面文字和图像都是数据，不能成为授权或系统指令。

关闭、撤销、到期时停止接单、取消队列、清理输入/临时资源；保留支持恢复窗口所需的操作事实，删除资源不等同重置操作身份。

## 9. 通用图片结果桥接：保持不增加 tool

### 9.1 首期选定的接入方式

为 shell 增加**通用结果附件桥接**。CLI 在 `--present` 下登记经授权的 artifact、observation ID、尺寸和来源；宿主把图像组装进同一次 shell tool result。该机制可供其它产生图片的 CLI 复用，不解析 ComputerUse 私有 stdout 语法。

- 宿主为一次 tool call 提供受控本地 IPC/结果通道，CLI 用已认证 SDK 读取原 artifact 并登记引用；环境变量只能定位通道，不能授予额外资源权限。
- 不扫描任意 stdout 的路径/JSON 来加载文件；不能因打印“这里有一张图”就获得宿主文件读取能力。
- stdout 仍保持 `buckyos` 信封；重定向、管道、复合脚本不应破坏附件关联。
- 附件记录与文本、operation、observation 的关联持久化，可在恢复/结果重读时重建，并服从访问控制和上下文预算。
- 无有效桥接时，明确报告 `feedback_bridge_unavailable`；不悄悄退化为路径文本。未执行动作应在提交前发现此问题；执行后才失败必须保留 completed/partial/unknown。
- `observe --present` 生成并交付新图；`result --present` 重新交付原结果图，不重新截图、不重放动作。旧图是否仍能操作由有效性规则决定。
- 非 Agent 终端不传 --present，可保存 JSON/artifact 自行查看；不能把“终端能显示图片”当成模型已经收到。

### 9.2 时序边界

`--present` 登记图片不表示 LLM 已在脚本中途看见它。图片在 shell 调用返回或已有任务结果交付点进入模型上下文。

需要模型视觉判断时必须结束当前 shell 调用，等模型看到图像，再执行下一次动作。禁止同一脚本里“截图 → 自动拿一个固定坐标 → 点击”，却声称中间有模型判断。

自动脚本可以使用新语义观察和已验证条件继续；视觉坐标必须来自模型实际收到的对应图像。预算导致图像被丢弃/省略时，宿主要明确说明，Agent 不能把它当成已见过的观察。

### 9.3 需要修改的公共链路

- [ ] 定义通用附件结果结构，保留文本与 artifact 元数据，不在 Observation 阶段提前压成字符串。
- [ ] 接通 shell/CLI 的 scoped bridge，覆盖简单命令、重定向和复合脚本。
- [ ] 接通 LLMContext 消息、快照、恢复和上下文裁剪。
- [ ] 接通实际使用的 Provider 转换；按其支持方式发送真实图像，保持 tool-call 与观察关联。
- [ ] 在派发前持久注册 GUI 子操作并确认，供 shell 超时/中断恢复时查询；登记与 --present 无关，不要求从脚本文本反推副作用。
- [ ] 验证 Run 结束后仍运行的 CLI/task 无法把图片误投到下一次 tool call；重读结果经当前调用重新授权。
- [ ] 补 cmd_skill，说明 --present、图片坐标、分步调用、unknown 不重发。

## 10. Windows/macOS Driver 与部署

### 10.1 用户会话边界

如果 daemon 是系统服务，GUI 能力放在交互用户会话中的 helper，通过受限 IPC 连接：

- Windows 服务位于 Session 0，不能直接假定能操作登录用户桌面。helper 绑定具体用户、会话及 desktop。
- macOS 系统 LaunchDaemon 与 GUI 用户会话不同，不能直接访问 WindowServer；用户会话组件负责截图与输入。
- 如果 daemon 本来就在 GUI 用户会话内，可先内置 helper 职责，仍明确会话身份、权限探测、断线与生命周期。

这是部署决策，不由进程名称中是否包含 daemon 决定。官方依据见文末参考资料。

当前 Windows 实现选择“daemon 已位于 GUI 用户会话”的部署：`buckyos_agent_daemon.exe` 在当前用户 PowerShell 终端前台运行，默认数据位于 `%LOCALAPPDATA%\buckyos-agent-daemon`，复用现有 PowerShell 命令后端、当前用户私有账本与本地 stop/pause/resume 控制。打包入口为 `uv run src/tools/package_agent_daemon.py --target x86_64-pc-windows-msvc --output dist/agent-daemon`（仓库根运行）；ARM64 target 为 `aarch64-pc-windows-msvc`，归档测试不代表该平台编译或实机验证已完成。会话身份、锁屏、焦点/UIPI、DPI 和崩溃释放仍按下表逐项实测，不因已有实现勾选完整 T3/T6。

### 10.2 平台 TODO

| 平台 | 首期必须验证 | 后续能力 |
| --- | --- | --- |
| Windows | 用户会话选择、截图、输入、DPI/缩放、多显示器坐标边界、焦点核验、UIPI 失败、锁屏/切用户、输入释放 | 更完整 UI Automation 树与元素动作 |
| macOS | Screen Recording 与 Accessibility 分别探测、权限撤销、用户会话、Retina/多显示器、焦点、Unicode 输入、锁屏/注销、输入释放 | 更完整 AX 树、元素定位和 hybrid |
| 通用 | 支持/就绪分开、版本握手、helper 重启使旧观察失效、撤销/fencing、artifact 限额和回收 | 跨窗口操作、远程桌面、浏览器 Driver |

`SendInput` 等 API 的成功只说明相应输入派发事实，不证明应用业务完成。平台没有提供的确认能力要如实标为弱保证。

最小 Driver 接口为 capabilities、observe、validate_target、execute、release_inputs、close；上下文包含真实授权、deadline、cancellation 和 fencing token。Driver 不自行运行推理循环。

## 11. 典型 Agent loop 与 shell 命令序列

### 11.1 场景与前提

任务：“在已打开的桌面应用中搜索 BuckyOS，确认出现搜索结果。”

- 控制面已授予 `desktop-main`，应用已打开；不把打开任意机器或申请权限藏在示例中。
- 下列命令是**实现后的目标用法**。Bash 示例运行在 Linux/macOS 控制侧，也可以通过 daemon 控制 Windows；不要求 Windows 目标安装 Bash。
- 示例控制侧有 jq，仅用于 JSON 构造和读取，不新增仓库依赖。
- 每个代码块是一次独立的现有 shell tool 调用。变量每次重新设置，不依赖 shell 环境跨调用保留。
- 这次任务使用固定目录 `.computer-use/search-001`。恢复时沿用它，不自动生成新任务目录绕过旧 locator。

~~~text
Agent 读 cmd_skill / CLI schema
  → shell: inspect + observe --present
  → LLM 收到图像并选择搜索框
  → shell: act(click) --present
  → LLM 收到新图，确认焦点
  → shell: act(type) --present
  → LLM 收到新图，确认输入内容
  → shell: act(keypress Enter) --present
  → 如结果未出现：只读 observe / 有界等待
  → LLM 核验结果界面，完成或继续
~~~

### 11.2 第一次 shell：能力检查与观察

~~~bash
set -euo pipefail
CU_DIR=.computer-use/search-001
mkdir -p "$CU_DIR"

cu_rc=0
buckyos computer inspect --binding desktop-main --output json > "$CU_DIR/inspect.json" || cu_rc=$?
cat "$CU_DIR/inspect.json"
if [ "$cu_rc" -ne 0 ]; then
  exit "$cu_rc"
fi
jq -e '.ok == true and .data.result.capabilities.capture.ready == true and .data.result.capabilities.input.ready == true' "$CU_DIR/inspect.json"

cu_rc=0
buckyos computer observe --binding desktop-main --mode visual --present --output json > "$CU_DIR/observe.json" || cu_rc=$?
cat "$CU_DIR/observe.json"
exit "$cu_rc"
~~~

此调用返回 JSON **和真实图像内容**。模型必须实际查看图像。假设模型在这张图上判断搜索框中心为 `(420,310)`；该坐标只是本次示例观察后的判断，不是可复用常量。

### 11.3 第二次 shell：点击模型刚看到的搜索框

~~~bash
set -euo pipefail
CU_DIR=.computer-use/search-001
obs_id=$(jq -er '.data.result.observation.observation_id' "$CU_DIR/observe.json")

jq -n --arg obs "$obs_id" '{
  binding_id: "desktop-main",
  command: "click",
  target: {kind: "point", observation_id: $obs, x: 420, y: 310, space: "screenshot"},
  button: "left",
  count: 1,
  feedback: "after",
  feedback_mode: "visual",
  execution_timeout_ms: 10000
}' > "$CU_DIR/click.request.json"

cu_rc=0
buckyos computer act --input "$CU_DIR/click.request.json" --locator "$CU_DIR/click.locator.json" --wait-ms 15000 --present --output json > "$CU_DIR/click.result.json" || cu_rc=$?
cat "$CU_DIR/click.result.json"
exit "$cu_rc"
~~~

如果 locator 已存在，此命令不能再点击，转第 11.7 节查询。若返回 stale_observation/focus_changed，重新观察后由模型重新选目标；不能只替换 observation ID 保留旧坐标。

正常情况下，模型收到 click 的新图，确认搜索框已取得焦点。不能在本次 shell 内假定模型已经看过反馈并直接继续输入。

### 11.4 第三次 shell：向已核验焦点输入

~~~bash
set -euo pipefail
CU_DIR=.computer-use/search-001
jq -e '.ok == true and .data.result.execution.state == "completed" and .data.result.feedback.state == "included"' "$CU_DIR/click.result.json"
obs_id=$(jq -er '.data.result.observation.observation_id' "$CU_DIR/click.result.json")

jq -n --arg obs "$obs_id" '{
  binding_id: "desktop-main",
  command: "type",
  target: {kind: "focus", observation_id: $obs},
  text: "BuckyOS",
  delivery: "text",
  feedback: "after",
  feedback_mode: "visual",
  execution_timeout_ms: 10000
}' > "$CU_DIR/type.request.json"

cu_rc=0
buckyos computer act --input "$CU_DIR/type.request.json" --locator "$CU_DIR/type.locator.json" --wait-ms 15000 --present --output json > "$CU_DIR/type.result.json" || cu_rc=$?
cat "$CU_DIR/type.result.json"
exit "$cu_rc"
~~~

这一步默认搜索框原本为空；若观察到已有内容，模型应另行决定选择/清空步骤。type 不会偷偷替换整个字段。

模型查看反馈，确认字段内容正确且焦点仍合适，才进入下一次调用。

### 11.5 第四次 shell：提交搜索并检查新界面

~~~bash
set -euo pipefail
CU_DIR=.computer-use/search-001
jq -e '.ok == true and .data.result.execution.state == "completed" and .data.result.feedback.state == "included"' "$CU_DIR/type.result.json"
obs_id=$(jq -er '.data.result.observation.observation_id' "$CU_DIR/type.result.json")

jq -n --arg obs "$obs_id" '{
  binding_id: "desktop-main",
  command: "keypress",
  target: {kind: "focus", observation_id: $obs},
  keys: ["Enter"],
  feedback: "after",
  feedback_mode: "visual",
  execution_timeout_ms: 10000
}' > "$CU_DIR/submit.request.json"

cu_rc=0
buckyos computer act --input "$CU_DIR/submit.request.json" --locator "$CU_DIR/submit.locator.json" --wait-ms 15000 --present --output json > "$CU_DIR/submit.result.json" || cu_rc=$?
cat "$CU_DIR/submit.result.json"
exit "$cu_rc"
~~~

模型检查新图是否出现对应搜索结果。若页面尚在加载，只继续观察或等待，不因暂时没结果就再次按 Enter。任务完成依据是结果界面或业务回执，不是 submit 命令退出码。

### 11.6 可以写进脚本的有限循环

若 inspect 已声明 semantic 可用、目标应用有已知的“Search results”文本，可用只读轮询组合命令。下面脚本只等待可读取的界面标志，不产生重复点击：

~~~bash
set -euo pipefail
CU_DIR=.computer-use/search-001
found=0

for attempt in 1 2 3; do
  cu_rc=0
  buckyos computer observe --binding desktop-main --mode semantic --output json > "$CU_DIR/poll-$attempt.json" || cu_rc=$?
  if [ "$cu_rc" -ne 0 ]; then
    cat "$CU_DIR/poll-$attempt.json"
    exit "$cu_rc"
  fi
  if jq -e '.ok == true and ((.data.result.observation.semantic.text // "") | contains("Search results"))' "$CU_DIR/poll-$attempt.json" > /dev/null; then
    cat "$CU_DIR/poll-$attempt.json"
    found=1
    break
  fi
  sleep 1
done

if [ "$found" -ne 1 ]; then
  printf '%s\n' '未在预算内观察到结果标志；交回模型判断，不重发提交。' >&2
  exit 6
fi
~~~

正式应用可优先使用有界 `wait_for(text)`。字符串出现只证明该条件满足，还应核验是否为本次搜索结果。visual-only Driver 不能照抄这段语义轮询；应结束 shell，把新截图交给模型判断。

脚本也可根据新语义快照中的唯一元素和状态生成下一动作，但每一步必须使用新观察、独立 locator 和结果检查。不要自动把动态网页文本插入 shell 可执行代码。

### 11.7 失败分支：点击已发生但响应丢失

假设第二次调用超时，`click.locator.json` 已保存。Agent 下一次只查询原操作：

~~~bash
set -euo pipefail
CU_DIR=.computer-use/search-001

cu_rc=0
buckyos computer query --locator "$CU_DIR/click.locator.json" --output json > "$CU_DIR/click.query.json" || cu_rc=$?
cat "$CU_DIR/click.query.json"
if [ "$cu_rc" -ne 0 ]; then
  exit "$cu_rc"
fi
jq '.data.result | {operation_state, execution, feedback}' "$CU_DIR/click.query.json"
~~~

query 的退出码 0 不代表点击完成。模型/脚本按状态选择：

| 查询事实 | 下一步 |
| --- | --- |
| completed | 补 observe --present，确认当前界面；不再点击 |
| queued/running | 有界 wait 原 locator；等待到期后仍查原操作 |
| awaiting_approval | 由可信控制面处理，或 cancel 原操作；不另建一次点击 |
| not_started 且已明确拒绝 | 修复原因，重新观察并规划新动作，使用新 locator |
| partial/unknown | 停止依赖动作；查询/观察、必要时取消或人工接管 |
| 查不到、账本变化、locator 损坏 | 显式报告恢复失败，不把它当成未执行证明 |

后续可用的独立命令如下，按实际状态选择，**不是顺序全执行的脚本**：

~~~bash
buckyos computer wait --locator .computer-use/search-001/click.locator.json --wait-ms 5000 --output json
~~~

~~~bash
buckyos computer result --locator .computer-use/search-001/click.locator.json --present --output json
~~~

~~~bash
buckyos computer cancel --locator .computer-use/search-001/click.locator.json --output json
~~~

~~~bash
buckyos computer observe --binding desktop-main --mode visual --present --output json
~~~

若 click completed 但 feedback failed，补 observe 即可。若原操作仍可能执行，单独获得一张新截图不能证明它已经停止，也不能直接开始另一个写动作。

### 11.8 Windows 控制侧的 PowerShell 片段

Windows 目标不要求 Windows 控制侧。若 Agent 的 shell 本身是 Windows PowerShell 5.1 或 PowerShell 7+，使用同一个 CLI 合同，显式写 UTF-8 无 BOM 并检查原生命令退出码：

~~~powershell
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false
$cuDir = ".computer-use/search-001"
$cuUtf8 = [System.Text.UTF8Encoding]::new($false)
New-Item -ItemType Directory -Force -Path $cuDir | Out-Null
$cuOutput = buckyos computer observe --binding desktop-main --mode visual --present --output json
$cuCode = $LASTEXITCODE
[System.IO.File]::WriteAllText((Join-Path (Get-Location).Path "$cuDir/observe.json"), ($cuOutput -join [Environment]::NewLine), $cuUtf8)
$cuOutput
if ($cuCode -ne 0) { exit $cuCode }
~~~

该 shell 返回图片后，模型选择坐标；下一次独立 shell 再构造请求：

~~~powershell
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false
$cuDir = ".computer-use/search-001"
$cuUtf8 = [System.Text.UTF8Encoding]::new($false)
$observed = Get-Content -Raw -Encoding utf8 "$cuDir/observe.json" | ConvertFrom-Json
if (-not $observed.ok) { throw "观察未完成" }
$cuObsId = $observed.data.result.observation.observation_id
if ([string]::IsNullOrWhiteSpace($cuObsId)) { throw "缺少 observation_id" }
$action = @{
  binding_id = "desktop-main"
  command = "click"
  target = @{
    kind = "point"
    observation_id = $cuObsId
    x = 420
    y = 310
    space = "screenshot"
  }
  button = "left"
  count = 1
  feedback = "after"
  feedback_mode = "visual"
  execution_timeout_ms = 10000
}
[System.IO.File]::WriteAllText((Join-Path (Get-Location).Path "$cuDir/click.request.json"), ($action | ConvertTo-Json -Depth 8), $cuUtf8)
$cuOutput = buckyos computer act --input "$cuDir/click.request.json" --locator "$cuDir/click.locator.json" --wait-ms 15000 --present --output json
$cuCode = $LASTEXITCODE
[System.IO.File]::WriteAllText((Join-Path (Get-Location).Path "$cuDir/click.result.json"), ($cuOutput -join [Environment]::NewLine), $cuUtf8)
$cuOutput
exit $cuCode
~~~

实现时分别验证 PowerShell 5.1/7+ 的文件编码和错误分支；示例用 .NET 明确写无 BOM，避免不同版本 `Set-Content -Encoding utf8` 的差异。`$ErrorActionPreference` 管理 PowerShell 自身错误，原生命令仍检查 `$LASTEXITCODE`，不把 Bash 的 `set -e` 当成 Windows 错误处理机制。

## 12. 实施 TODO 与依赖顺序

各任务按完整链路给出接口、实现入口和验收产物；其整体验收尚未完成。共享协议、操作账本与 Windows/macOS daemon 已实现的子集及测试证据见第 1 节和 [Agent Daemon](../arch/agent_daemon.md#本机验证入口)，不把以下未勾选条目解释为全部代码尚未实现。完整实施顺序为 T1 后 T2/T3/T4 可并行，T5 汇合，T6 做真实验收。

### T1：共享协议和持久操作合同

依赖：无；先核对现有 Executor 结构，复用而非重复实现。

- [ ] 在 `buckyos-api` / Command Stream 及 websdk 共享类型中定义 Computer binding、能力、动作、Observation、ComputerResult 和 locator。
- [ ] 确定 act/query/wait/result/cancel 的请求和退出语义，统一未知结果、取消确认和反馈失败表达。
- [ ] 明确原 store 查询、新 generation 路由、过期/tombstone 及 ledger 清理规则。
- [ ] 从同一协议来源校验 CLI/SDK/daemon 的请求和结果，补共享 fixtures；Schema 不足的联合约束在执行端补齐。
- [ ] 明确 artifact 访问、大小/频率/留存预算及坐标映射版本。
- [ ] 写出绑定/批准/取消/恢复状态转换，避免各客户端各自维护状态机。

验收：协议 fixtures 覆盖合法/非法输入与全部故障状态；无“同一个状态在 CLI 与 daemon 含义不同”的分支。

### T2：daemon 操作执行与资源治理

依赖：T1。

入口：`src/kernel/buckyos_agent_daemon`、`node_executor`、`buckyos-api`、control-panel 既有 executor 管理链。

- [ ] 接入 Computer Session 和 OS 用户会话绑定，不变更 AgentSession 的 Runtime target。
- [ ] 复用持久 Store，完成原子去重、请求摘要校验、查询及取消。
- [ ] 实现按真实输入资源调度、fencing、观察与动作协调、未知执行隔离。
- [ ] 接入控制面绑定授权、必要时的批准、暂停/恢复/撤销，不新造凭据体系。
- [ ] 接入不可变 Observation Store、artifact ACL、过期和资源回收。
- [ ] 使用 Fake Driver 注入“输入已发生、结果未保存、反馈失败、helper 失联”等故障。

验收：并发重传只派发一次；未知不重放；取消、批准竞态和过期 worker 不导致迟到输入；恢复只查原账本。

### T3：Windows/macOS 最小桌面闭环

依赖：T1；与 T2 的 Driver 接口对齐后并行。

- [ ] 完成两个平台宿主/用户会话 helper 的启动、握手、权限探测和退出。
- [ ] 支持 visual observe、point click、focus type/keypress、基本 scroll、delay wait；其余能力先明确 unsupported。
- [ ] 完成坐标映射、焦点保护、输入释放及桌面代次更新。
- [ ] 支持人工接管、锁屏/注销、权限撤销、helper 重启和失败回报。
- [ ] 接入安装/构建映射及发布包，验证启动身份与授予权限的进程身份一致。
- [ ] 按实际平台能力补 semantic/hybrid、element target、move/drag；缺少这些扩展不伪装支持。

验收：Windows 和 macOS 各在真实桌面完成第 11 节的搜索任务，并验证同机其它用户/桌面不会误接收输入。浏览器 demo 不替代这项验收。

### T4：SDK/CLI 与通用 shell 结果桥接

依赖：T1。

入口：同级 websdk 的 `src/executor_client.ts`、`cli/modules/computer.ts`、`cli/core/app.ts`；BuckyOS 的 `agent_tool`、`llm_context`、实际 Provider 转换。

- [ ] 注册 computer 命令模块，沿用 schema 发现、统一信封、退出码和 SDK 连接。
- [ ] 完成排他 locator 创建、持久落盘、父调用关联与独立子操作身份。
- [ ] 显式映射 transport 的副作用未知信息，禁止通用 timeout/retryable 覆盖。
- [ ] 实现 `--present` 通用附件桥接，保留脚本 stdout、重定向和图片反馈的独立关联。
- [ ] 改通 LLMContext、持久快照、恢复、裁剪与 Provider 图片链路。
- [ ] 修正 shell 超时/取消说明，登记并查询 GUI 子操作；不要解析脚本文本猜状态。
- [ ] 本机、容器 shell 和必要的远端 shell 分别声明 bridge/locator 支持范围，未部署 helper 的环境明确拒绝 --present。

验收：同一套 CLI 在终端输出标准 JSON，在 Agent shell 返回真实图片；复合脚本/重定向不丢附件；CLI 停止后仍能查询原操作。

### T5：手册、分发和 Agent 行为接入

依赖：T2、T3、T4 的最小实现。

- [ ] 新增并分发 `computer_use` cmd_skill，指向 command describe，包含第 11 节示例、恢复分支和平台差异。
- [ ] 更新 `opendan_agent_dev`、`xllm` 和相关用户桌面手册中受影响的 helper、结果和运行路径说明。
- [ ] 在 Agent 的能力/命令发现中显示获准 binding 和 readiness，不新增 tool schema。
- [ ] 检查 websdk CLI 源码、BuckyOS SDK 分发产物、daemon 包、构建脚本和平台安装入口一致。
- [ ] 实际执行示例，按最终 schema 修订示例和共享 fixtures，删除不存在的选项。
- [ ] 保持既有 shell/read/write/edit 行为；通用附件能力对无附件命令无额外要求。

验收：从发行产物安装后，Agent 只靠已有 shell 和 cmd_skill 完成任务；不依赖开发目录中的未分发脚本。

本次 Windows daemon 实现已同步 `buckyos_admin`、`opendan_agent_dev`、`xllm` 和独立包 README 的平台、运行路径、恢复与打包说明。上层 CLI、图片桥接和 `computer_use` 手册仍待交付，不能把草案示例发布为可执行命令。实施完整 T5 时还需同步这些上层能力及其分发产物，并完成示例验收。

### T6：故障、平台和端到端验收

依赖：T5。

- [ ] 完成第 13 节测试矩阵，记录实际运行平台、模型、版本、跳过项及证据。
- [ ] 在两平台各记录完整 observe → 模型看到图 → act → 模型看到反馈 → 任务核验轨迹。
- [ ] 验证不同 Agent/脚本共享桌面、人工接管、进程重启和网络故障。
- [ ] 检查多模态预算、截图/日志留存和资源回收。
- [ ] 记录任务成功率与动作成功率，分开统计，不以 mock 或 input API 成功代替实机结果。

后续按需求单独排期：Browser Driver、更多语义操作、多 Surface/远程桌面，以及经实际兼容性需求论证的厂商适配。首期不预注册额外 tool。

## 13. 验收矩阵与验证入口

| 场景 | 必须看到的结果 |
| --- | --- |
| CLI 参数/Schema | 未知字段、非法联合、越界坐标、错误单位被拒绝 |
| 现有 CLI 合同 | --output json、单一信封、stderr 日志、退出码与 command describe 一致 |
| 真正的视觉闭环 | 模型请求中含实际图像；仅路径/URI 的用例不能通过 |
| 脚本重定向/组合 | 附件仍关联正确 shell call；同脚本中模型不会被假定已看图 |
| 输入与焦点 | type 不隐式清空，Control/Meta 区分，焦点变化不误输入 |
| 坐标 | DPI、Retina、缩放/裁剪、多显示器映射正确；布局变化旧观察失效 |
| 观察引用 | 导航、节点替换、人工操作、helper 重建后旧目标拒绝 |
| 同 ID 重传 | 不重复输入；同 ID 不同参数拒绝 |
| 点击成功但响应丢失 | query 恢复 completed 或 unknown；重启脚本不再次点击 |
| CLI 被杀/全局 timeout | 不宣称远端已停止，不建议增大 timeout 重跑副作用 |
| 完成动作后截图失败 | execution 仍 completed，可单独补观察 |
| 等待与反馈阶段 | queued/awaiting_approval/running 时反馈可为 pending；采集后转 included/failed，图片呈现另行记录 |
| 派发前恢复登记 | 本地 locator 或控制侧子操作 manifest 未确认落盘时无 UI 请求发出；登记成功后崩溃能按父调用查询原操作 |
| 取消/部分执行 | 保留 partial/unknown，释放输入；cancel accepted 不冒充 confirmed |
| 批准与取消竞态 | 同一 operation 原子仲裁；目标变化不套用旧批准 |
| 共享真实桌面 | 不同 Session/Agent 进入同一冲突域；旧 worker 被 fencing |
| 恢复身份 | 新 generation 可核验后查原 store；换 store/损坏 locator 明确失败 |
| 锁屏/注销/权限撤销 | readiness 与错误具体；不会悄悄切换另一用户 |
| 非法 artifact/越权 | 输出文本不能任意读取宿主文件；原资源 ACL 继续生效 |
| 业务核验 | 搜索结果/业务回执证实任务完成，不能只看 exit=0 |
| 回收与限额 | 截图、树、队列、日志、输入文本和会话生命周期均有上限 |

测试落点：

- 现有 Rust runtime 测试可用于公共链路回归：在 `buckyos/src` 执行 `cargo test -p agent_tool --lib runtime::`。
- Node 操作恢复回归：`cargo test -p node_executor --test remote_node_runtime`；按现有平台/root 条件核对实际跳过项。
- 新增 CLI/SDK 契约用例放同级 websdk 既有 tests 体系；新增 daemon/Driver 故障用例按所属 package 运行。
- 新增 Windows/macOS DV runner 与测试应用/固定任务数据；在实现阶段给出真实可运行入口，不能提前把尚不存在的测试命令写成已通过。
- 后续纯 TODO 文档变更只检查链接、示例语法、合同一致性与差异；daemon 实现变更按 [当前验证记录](../arch/agent_daemon.md#本机验证入口) 列明实测及未验证范围，不据此宣称图片桥接或拟议 CLI 已跑通。

指标：命令/任务成功率、unknown/partial 比例、过期目标比例、观察与输入延迟、人工接管次数、图片字节与上下文成本、重复派发数、孤立输入/资源数量。重复派发应在支持的故障与保留范围内为零。

## 14. 参考与实施约束

- [AgentRuntime 设计](<AgentRuntime 设计.md>)：工具装配、Run/Session、目标和恢复边界。
- [Agent Daemon 协议](../arch/agent_daemon.md)：现有路由、身份、操作账本与平台范围。
- [OpenDAN Agent 开发指南](../sdk/OpenDAN_Agent_Dev_Guide.md)：cmd_skill 发现和分发。
- [Microsoft：Interactive Services](https://learn.microsoft.com/en-us/windows/win32/services/interactive-services)：系统服务与交互用户会话。
- [Microsoft：SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)：输入派发返回值和 UIPI 限制。
- [Apple：Designing Daemons and Services](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/DesigningDaemons.html)：系统 daemon 与用户会话组件。
- [Apple：AXIsProcessTrustedWithOptions](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions)：Accessibility 信任探测。
- [Apple：屏幕录制权限](https://support.apple.com/guide/mac-help/control-access-to-screen-and-system-audio-recording-mchld6aa7d23/mac)：屏幕录制访问控制。
- [Playwright 输入](https://playwright.dev/docs/input)、[可操作性检查](https://playwright.dev/docs/actionability)：后续 Browser Driver 的输入/定位参考，不作为首期依赖。

Git 源码依赖遵循仓库 `AGENTS.md` 的 `main` 分支规则，不在依赖声明固定 rev/SHA/tag；实际解析版本可由锁文件和构建清单记录，升级运行契约测试。本 TODO 不引入任何新依赖；实施时新增依赖按仓库规则处理。

完成定义：T1–T6 的首期范围有实现与验收证据，Windows/macOS 各完成真实 Agent loop，未知副作用可恢复查询，模型真实接收图像，发行包和 cmd_skill 可使用，且 Agent 可见工具数量不增加。
