# OpenDAN AgentTool 实体化工作计划

## 1. 目标

围绕 `src/frame/agent_tools/readme.md` 中提出的方向，本次工作的目标不是再做一层“Runtime 内部伪 Bash 工具”，而是把 OpenDAN 现有 AgentTool 体系推进到下面三个结果：

1. AgentTool 以真实 CLI 的形式存在于 Bash `$PATH` 中，能够被管道、重定向、子命令、脚本直接组合。
2. 自有工具统一返回结构化 JSON 协议，支持 `success / error / pending` 三态。
3. Agent Loop、WorkLog、权限审批、长任务等待最终都收敛到同一条 `pending -> poll -> result` 流程。

当前阶段建议以 Rust 落地，优先复用 `src/frame/opendan/` 现有实现；不建议一开始就转到 TypeScript。`src/tools/buckyos-agent/readme.md` 可作为后续“Agent 可自改造工具源码”的延伸方向，但不作为本轮 MVP 的主实现。

**当前阶段工具实现约束（安全与策略）**

在本计划覆盖的各阶段中，**所有工具的具体实现**（含本地 CLI 与 Runtime 代理 CLI 的业务逻辑）遵循下列约束，避免把策略混进工具层：

1. **不内置沙盒逻辑**  
   不在工具进程内实现 chroot、路径白名单、工作区外访问拦截、资源配额、子进程隔离等“沙盒”语义；若需要，由外层部署（容器、专用用户、系统权限）或后续独立的治理层承担。

2. **不做路径合法性 / 策略裁决**  
   不因“路径是否在 workspace 内”“是否疑似系统目录”“是否跨卷”等理由在工具内拒绝或改写请求。参数所指路径按普通文件系统语义处理；访问失败时以 OS 错误（如 `ENOENT`、`EACCES`）反映即可，**不在工具层二次定义“合法路径”**。

3. **不在工具内判断是否需要用户授权**  
   工具不根据操作类型、路径敏感程度等自行决定是否进入“待审批”或返回 `pending:user_approval`。是否要走审批闸门由 **Runtime / Agent Loop / 策略服务** 在调用链外层统一决策；工具侧本阶段保持“收到已解析参数则执行，失败则 `error`”的薄语义。

说明：协议层仍可保留 `pending_reason: user_approval` 等字段，阶段 4 的审批与长任务轮询是在 **Loop 与 Runtime 任务模型** 上打通，**不是**让每个工具复制一套风险判断逻辑。

## 2. 现状基线

结合现有 OpenDAN 代码，已经有不少可直接复用的基础：

1. `src/frame/opendan/src/agent_tool.rs`
   已有统一的 `AgentTool` 抽象、`ToolSpec`、`AgentToolManager`，并支持 `support_bash / support_action / support_llm_tool_call` 三种命名空间。

2. `src/frame/opendan/src/agent_bash.rs`
   `ExecBashTool` 已经实现“按行执行”的混合路由：
   - 如果整行首 token 命中已注册 bash tool，则走 `tool_mgr.call_tool_from_bash_line_with_cwd`
   - 否则走 tmux/bash 执行

3. 现有 bash 模式工具已经存在一批可复用实现
   - `get_session`：`src/frame/opendan/src/agent_session.rs`
   - `load_memory`：`src/frame/opendan/src/agent_memory.rs`
   - `todo`：`src/frame/opendan/src/workspace/agent_todo_tool.rs`
   - `create_workspace` / `bind_workspace`：`src/frame/opendan/src/workspace/workshop.rs`
   - `create_sub_agent` / external workspace 相关工具：`src/frame/opendan/src/ai_runtime.rs`
   - `read_file`：`src/frame/opendan/src/buildin_tool.rs`

4. `src/frame/opendan/src/worklog.rs` 与 `src/frame/opendan/src/behavior/prompt.rs`
   已经有 `commit_state = PENDING` 的建模和过滤能力，说明 WorkLog 体系本身对“未完成状态”并不陌生。

5. `src/frame/opendan/src/behavior/behavior.rs`
   已经有对 AICC 异步任务的等待逻辑，说明 Runtime 内部并非完全同步模型，后续可以复用 TaskMgr / wait / poll 的工程经验。

## 3. 关键缺口

虽然基础不错，但离 README 里的目标还有几个核心缺口：

1. 现有 bash tool 不是“真实 PATH 命令”
   现在仍然是 Runtime 先解析命令，再决定是否走内置工具。工具本身并不存在于系统 `$PATH` 中。

2. 当前 bash 路由只识别“整行首命令”
   `tokenize_bash_command_line()` 只是简单分词，不理解完整 Bash 语法树，因此无法正确进入：
   - 管道
   - 命令替换
   - 子 shell
   - 复杂重定向
   - `&& / || / ;` 组合

3. 工具输出协议未统一到三态模型
   当前 `AgentToolResult` 更偏向 prompt/render 视角，核心字段是 `cmd_line / result / stdout / stderr / details`，尚未标准化为 `success / error / pending`。

4. `pending -> check_task` 还没有贯穿工具层
   WorkLog 和异步 LLM 任务已有局部能力，但 AgentTool 还没有统一的任务 ID、轮询入口、等待策略。

5. 本地工具与 Runtime 代理工具尚未系统分层
   哪些命令可完全本地执行，哪些必须 RPC 回调 Runtime，目前主要体现在具体实现里，还没有抽象成明确边界。

6. 写类工具尚未 bash CLI 化
   `edit_file`、`write_file` 当前是 action 模式，不是 bash 命令；这与 README 中“纯本地工具直接作为 CLI 跑”的目标不一致。

## 4. 实施原则

1. 先兼容现有 OpenDAN ToolSpec / AgentTool 体系，再做实体化
   不要另起一套完全平行的工具定义。CLI 化应建立在现有 `ToolSpec + AgentToolManager + 已有工具实现` 之上。

2. 先做协议和边界，再做工具搬运
   如果不先定义清楚“本地执行 / RPC 回调 / pending / poll / 权限审批”边界，后面 CLI 数量越多，返工越大。  
   **与 §1 约束一致**：这里的“权限审批”指 **Runtime / Loop / 策略层** 与协议的契约边界；**工具实现本身**在阶段 0–3 及阶段 4 的工具代码路径中仍不内嵌沙盒、路径策略裁决或授权闸门（见 §1「当前阶段工具实现约束」）。

3. 保持 tmux 基座不变
   本轮不替换 `exec_bash` 的 tmux 会话模型；重点是把工具从“Runtime 拦截”迁移为“Bash 原生命令”。

4. 先做 Rust MVP，再考虑 TS 镜像实现
   当前仓库中真正可复用、可上线的实现都在 Rust；TS 版本应是第二阶段能力，不应阻塞 MVP。

5. 保持向后兼容
   在真实 CLI 跑通前，LLM tool call / action / 旧 bash 拦截路径都应继续可用，避免一次性切换导致 Agent 退化。

6. 工具层保持“薄实现”
   与 §1 一致：CLI 与 RPC 工具只负责 **协议、参数解析、调用核心能力、映射 JSON 结果**；**不**在工具内叠加沙盒、路径合法性判断、风险分级与用户授权决策。治理能力后续可集中在单一策略与调用编排层，避免 N 个工具各写一套规则。

## 5. 推荐技术路线

建议把实现拆成三层：

### A. 协议层

新增一套 AgentTool CLI 通用结果协议，建议字段如下：

```json
{
  "status": "success|error|pending",
  "summary": "human readable summary",
  "detail": {},
  "task_id": "optional",
  "pending_reason": "long_running|user_approval|external_callback",
  "estimated_wait": "optional",
  "check_after": 5
}
```

同时保留与现有 `AgentToolResult` 的桥接：

- Runtime 内部仍可继续使用 `AgentToolResult`
- CLI 输出层负责把 `AgentToolResult` 映射到统一协议
- Prompt / WorkLog 再根据统一协议生成压缩视图

### B. 执行层

新增一个 BusyBox 风格的 AgentTool CLI 二进制，推荐模式：

1. 一个主二进制，例如 `agent-tools`
2. 通过软链接或 argv[0] 暴露多个命令名，例如：
   - `get_session`
   - `load_memory`
   - `todo`
   - `create_workspace`
   - `bind_workspace`
   - `read_file`
   - `write_file`
   - `edit_file`
   - `check_task`

这样可以最大化复用现有工具注册和命令分发逻辑。

**部署约束补充**

- 最终部署目标是**只交付一个主二进制**，命令名通过软链接、硬链接、wrapper 或 `argv[0]` 分发暴露。
- 若迁移过程中为了测试/灰度方便短暂产出多个 Rust `[[bin]]`，应视为过渡态，不作为最终制品模型。
- 验收和收口时，应回到“单 binary + 多别名”的发布方式，避免把当前开发便利性方案误认为长期架构。

### C. Runtime 适配层

为 CLI 提供两种后端：

1. 本地后端
   - `read_file`
   - `write_file`
   - `edit_file`
   - 其他纯文件系统工具

2. Runtime RPC 后端
   - session / workspace / subagent / external workspace 相关
   - 需要权限审批的操作
   - 需要任务持久化和 `pending` 状态流转的操作

CLI 通过 OpenDAN 注入的环境变量拿到上下文：

- `OPENDAN_SESSION_ID`
- `OPENDAN_AGENT_ID`
- `OPENDAN_AGENT_ENV`
- `OPENDAN_RUNTIME_RPC`
- `OPENDAN_SESSION_TOKEN`
- `OPENDAN_TRACE_ID`
- `OPENDAN_STEP_ID`

字段名可调整，但要在第一阶段固定下来。

## 6. 分阶段计划

### 阶段 0：设计收敛与资产梳理

目标：冻结边界，避免直接开写后二次重构。

任务：

1. 产出现有工具清单矩阵
   - 维度：tool 名称、当前 namespace、是否已有 bash 语法、是否纯本地、是否需要 Runtime、是否需要 pending
   - 若矩阵涉及沙盒、路径策略或审批：单列「策略归属层」（如 Runtime / 部署 / 待定），并约定 **工具实现本身不承载** 这些逻辑（见 §1）

2. 产出协议草案
   - CLI stdout JSON
   - stderr 约定
   - exit code 约定
   - `pending` 的退出语义

3. 产出环境变量契约
   - Tmux 启动时要注入哪些变量
   - 哪些变量必须有，哪些可选

4. 决定 MVP 工具范围
   建议第一批：
   - `read_file`
   - `write_file`
   - `edit_file`
   - `get_session`
   - `load_memory`
   - `todo`
   - `create_workspace`
   - `bind_workspace`
   - `check_task`

交付物：

- 工具矩阵
- 协议文档
- MVP 名单

验收标准：

- 团队对“哪些走本地、哪些走 RPC、哪些支持 pending”达成一致

### 阶段 1：抽取共享协议与分发骨架

目标：先把“实体化所需的公共层”搭好。

任务：

1. 在 `src/frame/opendan/` 或新建共享模块中抽取：
   - CLI result 协议结构体
   - `AgentToolResult -> CLI result` 转换器
   - 命令分发器
   - 公共错误码和退出码约定

2. 设计 BusyBox 风格入口
   - 根据 argv[0] 或 subcommand 定位实际工具
   - 支持 `--help`
   - 支持统一 JSON 输出

3. 抽取现有 bash 解析中可复用的部分
   - 保留现有 `todo` 等工具已有 CLI 参数规则
   - 避免复制两套 parser

4. 设计 `check_task` 抽象
   - 统一 task_id 格式
   - 统一轮询返回协议

交付物：

- 可编译的 CLI 主程序骨架
- 统一 result 协议实现
- `check_task` 协议定义

验收标准：

- 任意一个 demo 命令可通过新 CLI 输出标准 JSON

### 阶段 2：本地工具 CLI 化

目标：先把最不依赖 Runtime 的工具跑通，验证 PATH 可组合能力。

任务：

1. CLI 化 `read_file`
2. CLI 化 `write_file`
3. CLI 化 `edit_file`
4. 调整 OpenDAN 启动 / tmux 环境，让工具目录进入 `$PATH`
5. 增加组合验证场景：
   - `read_file foo.txt | jq ...`
   - `cat a.txt | some_tool`
   - `grep ... $(read_file ...)` 类失败场景的兼容约束说明

说明：

- 这一步不要再依赖 `ExecBashTool` 的“首 token 拦截”能力
- 让 Bash 直接找到真实命令
- `read_file` / `write_file` / `edit_file` 仅按参数做 IO，**不**在实现中增加沙盒、路径白名单或“是否允许访问该路径”的二次判断（见 §1）

交付物：

- 本地工具真实可执行文件
- PATH 注入逻辑
- 组合调用测试

验收标准：

- 在 tmux/bash 中直接运行工具名即可执行
- 这些命令可被管道和重定向组合
- 结果输出满足统一 JSON 协议

### 阶段 3：Runtime 代理工具 CLI 化

目标：把现有 session / memory / todo / workspace 能力迁移到真实 CLI。

任务：

1. 为 Runtime 增加稳定的本地 RPC 入口
   - `tool.invoke` 或按领域拆分 RPC
   - `tool.check_task`

2. CLI 化已有 Runtime 工具
   - `get_session`
   - `load_memory`
   - `todo`
   - `create_workspace`
   - `bind_workspace`
   - `create_sub_agent`
   - external workspace 相关

3. 明确认证和上下文透传
   - session_id
   - agent_id
   - trace_id
   - 权限 token（用于 **RPC 身份与调用边界**，不是让各工具在 CLI 内再实现一套“操作是否需用户批准”的判断；见 §1）

4. 保持旧接口兼容
   - 现有 `AgentToolManager` 仍可被 behavior/tool_call 使用
   - CLI 和 Runtime tool 尽量共用核心逻辑，不复制业务实现

交付物：

- Runtime 代理型 CLI 工具集
- 本地 RPC 契约
- 兼容层

验收标准：

- CLI 调用与原先 Runtime 内调用得到等价结果
- Agent 不需要知道工具后端是本地还是 RPC

### 阶段 4：统一 pending / poll / approval 流程

目标：把 README 中最关键的三态协议真正接入 Agent Loop。

任务：

1. 为工具返回结果引入 `pending`
   - `pending_reason`
   - `task_id`
   - `summary`
   - `check_after`

2. 为 Runtime 增加任务持久化与查询能力
   - 长任务
   - 用户审批
   - 外部回调

3. 改造 Behavior / Tool Loop
   - 遇到 `pending` 不再等同失败
   - 记录 task_id
   - 按策略轮询 `check_task`
   - 允许去做其他独立任务

4. 接入 WorkLog
   - `pending` 记录写入 worklog
   - prompt 压缩优先展示 `summary`
   - 状态流转可追踪

5. 接入权限审批（Runtime / Loop 层，非工具内判断）
   - 由 **Runtime 或策略编排** 在调用工具前/后统一决定是否进入 `pending:user_approval`（例如基于方法名、参数摘要、会话策略），**不是**在各工具实现里写 `if 高风险路径` 等分支。
   - 批准后由任务模型转 `pending:long_running` 或 `success`；工具二进制仍不内嵌沙盒与授权逻辑（与 §1、§4 第 6 点一致）。

交付物：

- 三态协议落地
- `check_task` 运行链路
- Agent Loop 调度改造
- WorkLog/Prompt 适配

验收标准：

- 长任务不会阻塞整个 session
- 用户审批能与长任务共用同一轮询机制
- WorkLog 能看到状态流转

### 阶段 5：替换旧路径并收口

目标：让“真实 CLI”成为主路径，Runtime 拦截退为兼容层。

任务：

1. 调整 `exec_bash` 默认策略
   - 优先依赖 `$PATH` 中真实命令
   - 仅对未迁移工具保留兼容拦截

2. 清理旧文档和旧提示词
   - 工具使用说明
   - Agent prompt 中的工具介绍
   - 开发文档

3. 保留灰度开关
   - 新旧路径切换
   - 失败回退

交付物：

- 默认 CLI 路径上线
- 兼容开关
- 文档更新

验收标准：

- 真实 CLI 成为默认路径
- 不影响现有 Agent/Behavior 的基本可用性

## 7. 测试计划

至少覆盖以下测试：

1. 单元测试
   - 协议序列化
   - `pending` 状态转换
   - `check_task` 结果解析
   - argv[0] / subcommand 分发

2. 集成测试
   - 在 tmux 中直接运行 CLI
   - PATH 注入是否生效
   - 管道 / 重定向 / 子命令是否可用
   - CLI 与旧 Runtime 调用结果是否一致

3. 回归测试
   - 现有 `tool_call`
   - 现有 `action`
   - 现有 `exec_bash`
   - worklog prompt 渲染

4. 人工验收场景
   - 文件读取和写入
   - todo 的增删改查
   - workspace 创建与绑定
   - 一个长任务 + 一个短任务并行推进
   - 由 **Runtime/策略层** 触发的用户审批流程（验证 Loop 与 `check_task`，**不**要求各工具内置风险判断）

## 8. 风险与决策点

1. BusyBox 单二进制还是多二进制
   建议先单二进制 + 软链接，降低发布和版本管理复杂度。
   即使迁移中短暂保留多 `bin` 便于联调，最终也应收口到单主二进制。

2. CLI 直接链接 Runtime crate 还是只走 RPC
   建议混合方案：
   - 纯本地工具直接链接本地逻辑
   - 需要 session/runtime context 的走 RPC

3. `AgentToolResult` 是否直接替换
   不建议一步到位替换。先做桥接层，等 CLI 协议稳定后再考虑收敛内部模型。

4. `pending` 是否立即接入所有工具
   不建议。先让 `check_task` 跑通，再挑长任务和审批型工具接入。

5. TypeScript 工具链何时进入
   不应阻塞 Rust MVP。待 Rust CLI 与协议稳定后，再评估把部分工具镜像到 `src/tools/buckyos-agent/`。

6. 沙盒与路径策略放在哪一层
   本阶段明确 **不在工具实现内** 做沙盒与路径合法性裁决（§1）。若产品上需要，优先：部署隔离（用户/容器）、Runtime 统一策略引擎、或 Bash 外层包装；避免每个工具重复实现半套安全策略。

## 9. 建议的首轮落地顺序

为了最快得到可见成果，建议按以下顺序推进：

1. 阶段 0 和阶段 1
2. 阶段 2 的 `read_file / write_file / edit_file`
3. 阶段 3 的 `todo / get_session / create_workspace / bind_workspace`
4. 阶段 4 的 `check_task + pending`
5. 最后再处理 `create_sub_agent`、external workspace、审批型操作

这样做的原因是：

- 先拿到真实 PATH 命令和组合能力，尽快验证路线正确
- 再迁移已有高频工具
- 最后处理最复杂的 pending / approval / 调度改造

## 10. 里程碑定义

### M1：CLI MVP 可运行

标准：

- 至少 3 个本地工具成为真实命令
- 在 tmux/bash 中可直接运行
- 输出统一 JSON

### M2：常用 Runtime 工具迁移完成

标准：

- `todo`、`get_session`、workspace 基础操作 CLI 化
- PATH 调用与旧 Runtime 路径行为一致

### M3：异步协议打通

标准：

- `pending -> check_task -> success/error` 全链路可跑
- Agent Loop 不再把 pending 视为失败

### M4：默认路径切换完成

标准：

- 真实 CLI 成为主路径
- 旧拦截逻辑仅作为兼容兜底
