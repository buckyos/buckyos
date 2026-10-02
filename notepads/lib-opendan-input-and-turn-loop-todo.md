# lib_opendan 输入协议、Turn Loop 与长任务恢复 TODO

日期：2026-10-02

状态：待实施。依据本轮对 xAgent 输入设计、串行 / 并行等待和旧 AgentSession 的讨论，以及 `9cd6e8f5` 下的 libopendan 源码核对。实施前重新确认基线。

本 TODO 依赖的 llm_context 层修改（工具取消与时限、PendingTool 等待记录与查询接口、后台 job），以及需要 review 确认的 llm_context 事项，单独放在 [llm_context 长命令 / 长工具 TODO](./llm-context-long-tool-todo.md)。**llm_context 的修改总是先进行**，本 TODO 的宿主接入在其后。

## 1. 目标与范围

通过协议化分层，让应用、bridge、Session、LLM Context、Runtime 和 Agent State 各自承担清楚的职责，并支持独立进程、不同语言和多个 Session 协同推进。旧 OpenDAN 的复杂分支只作为反例和验收场景来源，不移植它的整体实现。

本 TODO 面向 `src/frame/lib_opendan` 的后续改进、xagent 验证和 `doc/opendan/protocol/` 的协议补全。本次只新增 TODO；后续实施也不修改 `src/frame/opendan`。TaskMgr / msg-center 的正式服务桥接继续按 xAgent 计划后移，先用模拟宿主验证协议边界。

Runtime 配置、沙箱和工具执行职责沿用 [AgentRuntime 下移 TODO](./llm-context-agent-runtime-todo.md)。libopendan 保留 Session 绑定、lease、输入提交和 run 生命周期管理，等待任务通过宿主提供的能力查询；不在 Session 内重新实现 TaskMgr、工具执行器或沙箱。

设计说明应先回答“投递什么 JSON，接收方如何处理，崩溃后如何恢复”，再给实现入口。Rust 的 `take_agent_inputs` 等函数只是协议行为的一种实现，不要求其它语言复制 Runner 的内部结构。

## 2. 当前基线与已有能力

| 项目 | 当前 libopendan | 本 TODO 的重点 |
|---|---|---|
| 输入 | `msg / event / change / control / perception`，kmsg headers + JSON payload | 按 xAgent C1–C3 收敛为 Agent 输入与 Session 控制，定稿跨语言线格式和处理语义 |
| 提交 | 输入正文与 receipt 同快照；随后提交 state、清门槛、确认输入源 | 保留现有能力，把新的事件与等待分支纳入相同提交纪律 |
| 并发 | Session / run 排他锁与单写者已有实现 | 验证跨 Session、跨进程并行及同一 Session 的独占推进 |
| 长工具 | `PendingTool` 可记录等待；恢复仍返回 `RecoveryBlocked`，`waiting_for.refs` 为空 | 补齐结果关联、等待恢复与宿主查询能力后再开启 deferred；llm_context 层的前置修改见 [llm_context TODO](./llm-context-long-tool-todo.md) |
| 半订阅 | 当前有 change / 订阅游标；xAgent 提议持久化 `pending_events` | 明确空闲保留、版本合并和精确消费，避免误删新事件 |

上述旧 OpenDAN 审查发现不能直接当作 libopendan 的现有缺陷。特别是 receipt 提交顺序已经落地，后续应扩展并验证它。

## 3. P0：把输入约定写成可手工投递的 JSON 协议

- [ ] 定稿统一逻辑 envelope 与 `msg / event / control` 的 JSON Schema。明确 kmsg headers、payload 和开发文件队列各自如何映射到同一逻辑输入；字段名、必需性、默认值、大小限制、未知类型和不支持版本的处理都有定义。
- [ ] 区分投递者与说话人：envelope 的 `from` 与 AgentMessage 的 `from` 如何对应；自报身份用于审计，写入权限来自实际调用身份 `who`。`src / index` 由输入通道提供，不由手工投递者伪造。
- [ ] Agent 输入只有 message 和 event；control 由 Runner 执行，不进入 LLM 上下文。`change` 收敛为 event 的 Observe 策略，`perception` 收敛到控制协议，按 xAgent 的 schema 升版方案处理。
- [ ] 补完整手工投递样例：普通消息、active 任务订阅事件、semi 对象变化、停止控制，以及非法输入。每个样例给出所需 Session 配置和预期行为。
- [ ] CLI 支持从文件 / stdin 投递协议 JSON，或提供等价入口；能使用 `--no-bridge` 单独验证接收方。CLI 不要求调用者了解 Rust 类型、闭包或进程内对象。

下面只是目标线格式示意，具体字段编码应随 schema 定稿；当前 CLI 不保证直接接受这些 JSON。

```json
{
  "type": "msg",
  "key": "message:m1",
  "from": "did:bns:alice",
  "at_ms": 1790899200000,
  "payload": {
    "from": "did:bns:alice",
    "text": "查询构建进度",
    "attachments": []
  }
}
```

```json
{
  "type": "event",
  "key": "task:t1:revision:7",
  "from": "app:task-bridge@alice",
  "at_ms": 1790899200000,
  "payload": {
    "subscription_id": "watch-t1",
    "source": { "kind": "task", "id": "t1" },
    "event": "updated",
    "seq": 7,
    "summary": "任务 t1 的状态发生变化",
    "terminal": false
  }
}
```

例中的 `watch-t1` 必须对应接收 Session 的订阅。active 决定 Wake，semi 决定 Observe；投递者不填写 `wake_llm: true` 来替接收方决定策略。未匹配订阅时，按定稿的模板默认规则处理。

## 4. P0：统一 Session 的机械判断与输入处理

- [ ] 用规则表说明 `type + subscription + waiting_for + Session 模板` 的处理结果，覆盖 drive 入口、空闲等待和运行中的观察边界；渲染模板只改变正文，不改变输入路由和消费语义。
- [ ] 明确区分“队列变化唤醒 Runner”与“形成可推理输入后运行 LLM”。kevent 只通知队列可能变化，重复通知和漏通知不影响已持久化输入的消费。
- [ ] message 的业务意图交给 LLM；鉴权、去重、排序、暂存和终态拒绝仍按协议机械处理。普通自然语言消息不能被直接解释为停止控制命令。
- [ ] event 按结构化字段机械匹配；`summary` 是给 LLM 的说明，不用于解析任务状态或推导等待条件。
- [ ] 挂起调用的结果依赖与普通事件订阅分别登记和匹配。同一事件如同时满足两者，明确是否还需向上下文提供通知及如何避免重复注入。不能因事件来自 Task 就把未匹配调用的事件一律当成孤儿丢弃。
- [ ] 明确未知、已删除订阅和无订阅系统事件的处理。沿用 xAgent 的目标默认值：已知 active → Wake，semi → Observe；未匹配时由模板决定，默认 timer → Wake、其它 → Observe。订阅变更的生效顺序与迟到事件行为纳入协议。
- [ ] 挂起工具期间，普通 msg / 无关 Wake event 保留待处理，不能填补缺失 ToolResult 或隐式开启替代 run；控制命令继续可处理。停止、审批和结果完成之间的先后关系必须有确定规则（建议规则与待决项见 llm_context TODO §9 第 7 项）。
- [ ] 挂起调用与事件的匹配使用 llm_context TODO §4 的 `wait.source{kind, id}`，与 `AgentEvent.source` 共用词汇，按 `(kind, id)` 相等匹配。
- [ ] 工具执行期间也要能处理 stop 并维持 activity 心跳。当前控制输入只在 `before_inference` 边界读取；Runner 需要一个与工具调用并行的监视任务，读到 stop 后调用 interrupt handle（依赖 llm_context TODO §3 的工具取消）。监视任务只查看控制输入，不确认 / 消费，也不写 state；控制由驱动者在下一个边界执行并提交（§5 单写者纪律）。

| 输入 / 情况 | 机械处理 | 对 LLM / Turn 的影响 |
|---|---|---|
| 可处理的 message | 校验、去重、组批 | 开启新 Turn 或并入未关闭 Turn |
| 匹配普通 active 订阅的 event | 解析 Wake 策略、组批 | 在可推理时进入上下文；挂起工具期间按等待规则暂存 |
| semi / Observe event | 持久化待观察状态 | 不单独触发推理，在观察边界或下一批输入时注入 |
| 匹配 pending call 的任务通知 | 重新查询权威状态、收集结果 | 满足快照所需结果后以 ToolResults 恢复原 run / Turn |
| control | 校验后由 Session 驱动者执行并提交 | 不作为 Agent 输入；停止等命令可改变 Session / Turn 状态 |

## 5. P0：事件身份、合并和消费提交

- [ ] 分别定义：逻辑输入去重 `key`、通道投递位置 `(src, index)`、事件来源 `source`、状态合并键 `(subscription_id, source)` 和来源版本 `seq`。同一逻辑输入重投沿用 key，不同变化使用不同 key。
- [ ] 明确去重记录的保留范围，避免把有界 `recent_keys` 描述成无限期去重保证；通道消费游标与 receipt 承担恢复时的幂等消费。
- [ ] Observe 状态按同一来源的版本合并，迟到旧版本不能覆盖新版本。版本不可比、缺少 seq、来源重启等情况先规定行为，不用接收时间冒充来源顺序。
- [ ] 不对所有 Wake event 默认做快照合并；需要保留每次发生的事件按流处理，可覆盖的状态事件由协议声明合并语义。terminal 用于保留和注入优先级，不能代替权威任务状态判断。
- [ ] `pending_events` 在空闲时保留；只在消费、明确拒绝或按规则覆盖后清除。容量 / 预算不足时定义延期或拒绝行为，不能统一丢掉所有 event。
- [ ] receipt 记录本次实际消费的投递位置及事件版本。清理待观察状态时，只清掉 receipt 覆盖的版本；处理 v7 期间收到 v8，提交 v7 不得删除 v8。
- [ ] 新的输入批次、观察注入和结果补齐继续遵守已有顺序：正文与 receipt 同快照 → run 门槛 → state 提交 → 清门槛 → 确认输入源。禁止在可恢复提交前出队，也禁止结束时按来源键再次删除本轮期间新到达的事件。
- [ ] 对仅更新待观察状态的投递，先提交该状态再确认输入源；区分“已接收保存”与“已注入上下文”，receipt 不声称 LLM 已看到尚未注入的变化。
- [ ] 保持 lease 下单写者纪律；bridge 和生产者通过输入通道提交，由驱动者修改 Session state，避免多个异步写入覆盖彼此的磁盘状态。

## 6. P1：串行等待、并行后台任务与可靠恢复

这里的“串行 / 并行”指 Agent 的后续工作是否依赖后台任务。后台任务要能独立于 Session 驱动进程存活并提供可查询状态；普通本地 exec 的未知执行结果仍沿用现有停止旧执行 / 记录 unresolved 的恢复纪律，不自动重放。

本节依赖 [llm_context TODO](./llm-context-long-tool-todo.md) 的修改（§3 工具取消与时限、§4 等待记录与 resolver、§5 后台 job 与 run 的所有权分离），它们先实施。

### 6.1 串行等待：补齐未完成的工具调用

- [ ] 完整定义 PendingTool 的宿主约定：工具返回 Pending 让出控制权，最终 ToolResult 尚未交付；持久化 run、call_id、等待对象引用、截止时间及已收集结果。每个调用的等待记录（`wait.source` / `wait.class` / 截止时间）以快照为准，由 llm_context TODO §4 定义；`waiting_for.refs` 在提交挂起时从中生成结构化摘要（call_id、source、deadline），用于状态展示和 §4 的事件匹配，不再是空 refs。
- [ ] 删除未使用的 `pending_task_calls: Vec<Value>`（当前只有定义与默认值），不另定 schema；等待关联只认快照中的等待记录与由它生成的 `waiting_for.refs`。
- [ ] 查询和恢复能力由宿主提供，Session 只依赖接口。Task、审批票据等挂起原因各自解析结果，不把所有 PendingTool 都转换为 TaskMgr 任务；审批沿用 xAgent 的控制协议设计。接口 `DeferredResolver` 定义在 llm_context 层（xllm 单独运行也能用），实现由宿主 / Runtime 按 `wait.source.kind` 提供；run.json 记录所需 resolver 种类，接手方缺少时拒绝（见 §7 交接）。
- [ ] 在 drive 恢复入口先对齐快照、提交现场和等待对象，再处理新输入。任务已终态就收集结果，仍在执行才继续等待；通知到达和兜底定时检查使用同一查询路径。
- [ ] 收齐快照实际需要的结果后，使用 `ResumeFill::ToolResults` 恢复同一 run 和仍打开的 Turn；轮询未完成任务无需调用 LLM。
- [ ] 等待条件独立于 inbox 是否为空。所有等待出口、CLI 重入和常驻模式都能继续检查任务，不能只有“已经收到部分输入”的分支才做轮询。
- [ ] 明确查询失败、任务不存在和结果不可读取的处理；保留现场并暴露可诊断状态，不删除挂起快照后静默另起任务。
- [ ] 支持结果恢复和崩溃场景后再开启 `allow_deferred`。宿主缺少对应能力时明确拒绝 / 返回 RecoveryBlocked，保留原始现场。这里指已处于挂起态的快照；派发时宿主未开 deferred 而工具返回 Pending 的情况，按 llm_context TODO §4 降级处理，不报 Internal 错误。

### 6.2 并行等待：工具已经返回，后台任务继续

- [ ] 正常 ToolResult 返回任务引用后，Agent 可以继续其它工作；后续通知按普通 active / semi 订阅处理，不走补齐旧 ToolResult 的路径。
- [ ] Agent 在收到变化通知后通过工具重新读取任务状态，决定继续等待、处理结果或改方案。空闲时的定期检查由持久化 timer / 宿主调度提供机会，不能假定 LLM 自己持续轮询。
- [ ] task 提交与订阅建立之间可能错过终态通知，建立订阅后立即查询一次；进程恢复后也先查询，再重新等待。
- [ ] 明确无输入队列模板的能力边界：串行等待可由查询能力恢复；需要后台通知驱动后续推理的 Session 必须有相应输入 / 调度通道，缺少能力时给出明确结果。
- [ ] 后台 job（llm_context TODO §5）的 Session 侧处理：
  - 以并行模式启动的 job，由 Session 自动登记订阅并立即查询一次；以串行等待使用的 job 不登记订阅，避免同一完成既回填结果又作为事件注入（对应 §4 “同一事件同时满足两者”）。
  - job 完成通知由 Runner 内置 bridge 写入 `pending_events`，与 Session 来源订阅相同，不依赖输入队列；无队列模板是否允许并行 job 见 llm_context TODO §9 第 6 项。
  - Session finished / discard 时调用 llm_context 层接口停止并清理其 job；work session 的并行 job 不会比 Session 活得久，需在工具结果中告知 Agent。

### 6.3 Task bridge 与任务创建的恢复关联

- [ ] 将 TaskMgr 当前 `phase / outcome` 与结果引用转换成稳定的宿主结果 / AgentEvent；映射在适配层完成，Session 不读取旧 `to_status`，不解析服务专有事件路径来推断工具依赖。
- [ ] 通知只加速检查，Task / 状态 API 才是权威来源。原始 kevent 的丢失通过重新查询收敛；需要保留每次发生的事件必须有持久来源或可靠投递，不能仅靠弱通知承诺至少一次。
- [ ] 创建任务前持久化 dispatch intent；根据 Session / run / call 的稳定身份生成幂等键；创建后持久化 task 绑定。崩溃在创建和登记之间时，按该身份找回同一个任务。llm_context 层的后台 job 采用同一原则（`job_id` 由 `(session, run, call_id)` 推导，见 llm_context TODO §5）。
- [ ] 任务创建、结果收集、ToolResults 恢复及旧等待关联清理之间设定可恢复提交点；链式 PendingTool 不得被上一批的清理操作抹掉。
- [ ] 首先用模拟任务服务 / 宿主验证上述协议，正式 TaskMgr bridge 作为后续接入项，不要求修改现有 OpenDAN 来完成验证。

## 7. P1：分层与并行推进的验收

- [ ] 多个 Session 可由不同进程并行推进；每个 Session 仍只有一个持 lease 的驱动者、一个 active run。后台任务并行不要求同一 Session 同时启动多个 LLM run。
- [ ] bridge 只转换来源和可靠投递；Session 判断消费与等待；LLM Context 推进推理 / 工具调用并给出 Outcome；Runtime 执行工具；Agent State 提供跨 Session 状态。Runner 复用现有组件，不把这些职责收回到一个大循环。
- [ ] xagent 与 xllm 交接时遵守 Session / run 锁和提交门槛，不能同时推进同一 run；挂起结果的提供方及能力不足时的行为也需明确：接手方按 run.json 记录的 resolver 种类判断能否继续等待，缺少时拒绝接手（llm_context TODO §4）。
- [ ] 跨语言共享 schema、处理规则与 fixtures。先验证独立进程生产 JSON、Rust Runner 消费和恢复；TS Runner 后续用相同 fixture 验证，不要求本轮实现第二套 Runner。
- [ ] xAgent 主文以 JSON 样例、规则表和恢复步骤解释设计，详细伪代码作为实现参考；主循环只呈现阶段边界，具体协议约定不隐藏在闭包或模板分支中。

## 8. 验收场景与实施顺序

以下是未来实施必须覆盖的场景，优先扩展现有 `tests/crash.rs`、Runner 测试和协议 fixtures；不是本次已经运行的验证。

| 场景 | 必须观察到的行为 |
|---|---|
| 独立进程手工投递 msg / event / control | schema 与规则决定处理方式，不依赖进程内句柄 |
| 同一事件投给 active 与 semi Session | 前者形成可推理输入，后者保存变化且空闲时保留 |
| 非法输入 / 不支持版本 | 按协议拒绝并记录原因；持久现场不被猜测性改写 |
| 队列 kevent 重复或丢失 | 已持久化输入仍可被轮询发现并正确消费 |
| 先到 v8，后到 v7 | v7 不覆盖 v8；事件流按其声明的规则处理 |
| 消费 v7 时 v8 到达 | 提交 v7 后，尚未注入的 v8 仍可被消费 |
| 快照 / state / 清门槛 / ack 各窗口崩溃 | 恢复后不漏输入、不重复注入，同一打开的 Turn 延续 |
| Task 在 Session 离线期间完成 | 无新通知也能查询终态并恢复；inbox 为空不阻止恢复 |
| TaskMgr 的成功 / 失败 / 取消及重复通知 | 都按当前权威状态映射；结果与 call_id 精确关联 |
| 普通后台任务订阅没有 pending call | 通知正常进入 active / semi 路径，不按任务路径前缀丢弃 |
| 串行等待期间收到消息与停止控制 | 消息保留；控制按定义处理，不破坏挂起调用关联 |
| Task 已创建、绑定未提交时崩溃 | 按稳定幂等身份找回同一 Task，不创建重复任务 |
| ToolResults 恢复后再次 PendingTool | 新等待关联保留，旧结果不会重复补齐 |
| 两个 Session 并行、两个进程争抢同一 Session | 不同 Session 正常推进；同一 Session 仅一个驱动者成功 |
| 长 exec 执行中收到 stop | 工具被取消；快照含配对的 Cancelled 结果，恢复后不重跑 |
| 同步 exec 执行中 Runner 被 kill | 恢复时先停止旧进程，注入“结果未知”并说明已被停止，不重放 |
| 串行等待 job 期间 kill -9 | 恢复后继续等待，终态回填到同一 run / Turn |
| 并行 job：Turn 关闭后 job 完成，期间 Runner 被杀 | job 不受影响；完成事件唤醒 active session |
| 启动 job 后、结果持久化前崩溃 | 按稳定身份找回同一 job，不重复启动 |
| 挂起期间收到 stop | 按定稿规则取消等待并回填，run 以 Stopped 结束 |

建议实施顺序：

0. [llm_context TODO](./llm-context-long-tool-todo.md) 先行：其 §3（工具取消与时限，P0）不依赖输入协议，立即开工，与下面第 1、2 步并行；§4 / §5 在第 3 步之前完成。
1. 定稿输入语义与目标 JSON 样例，和 xAgent C1–C3 对齐；实现对应类型与处理逻辑后反写最终 Spec / Schema / fixtures。
2. 补事件版本和精确消费，扩展既有 receipt / 崩溃验证，确认跨进程并行纪律。
3. 在 llm_context TODO §4 / §5 完成的基础上，实现宿主结果查询、挂起关联和恢复路径；用模拟任务服务验证串行 / 并行两种语义，再开启 deferred。
4. 接入 xagent 手工投递和常驻验证，随后按计划接 TaskMgr 等正式 bridge；更新主文、实现计划与 fixtures。

实现阶段在 `src/` 下运行相应的 `cargo test -p libopendan -- --test-threads=1`；触及共享 waist / 宿主装配时增加对应 crate 的检查。schema 改动按 beta 2.2 breaking change 规则显式升版并拒绝旧版本，不增加旧 OpenDAN 格式的兼容代码。

## 9. 参考与入口

- [xAgent 设计](../doc/opendan/xAgent.md)：§4 输入、§9 Turn Loop、§11 C1–C3 / C8 / C12。
- [Agent Session SDK 实现计划](<../doc/opendan/Agent Session SDK 实现计划.md>)、[协议索引](../doc/opendan/protocol/README.md)、[Session Input Protocol](<../doc/opendan/protocol/Session Input Protocol.md>)。
- [长任务与可靠等待](<../doc/opendan/OpenDAN Long Task & Sub-Agent.md>)、[事件订阅语义](<../doc/opendan/Agent Session的事件订阅.md>)。
- 当前实现入口：`lib_opendan/src/protocol/{input,state}.rs`、`runner/{drive,hook,live,receipts,outcome}.rs`、`channel/` 与 `tests/`。
- 旧实现审查来源：`opendan/src/agent_session.rs` 的提前出队、按事件来源键清理、Task 事件分流与等待分支；`task_dispatch.rs` 的任务创建关联。只用于设计验证，不纳入改动范围。
- llm_context 层：[llm_context 长命令 / 长工具 TODO](./llm-context-long-tool-todo.md)（先于本 TODO 实施，含对 xAgent.md 的修改清单与待 review 事项）、[AgentRuntime 下移 TODO](./llm-context-agent-runtime-todo.md)。
