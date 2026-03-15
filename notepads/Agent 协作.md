# 需求文档：基于 Workshop/Workspace 的协作型 Agent 系统（v0.2）

> 本版（v0.2）是在 v0.1 基础上，根据你提供的 review 反馈进行修订：补齐了 **Session↔Workspace 绑定语义、并发最小策略、Todo 状态机定义、Router/Resolver 可解释性细化、SubAgent 生命周期、Worklog 粒度约束、Remote WorkspaceService 同步策略**，并新增了异常流程示例与术语力度统一。

## 1. 背景与目标

### 1.1 背景

系统需要支持 Agent 在“本地执行 + 远端交付/协作”的环境下完成复杂任务，并支持三类协作：

1. Agent ↔ SubAgent（内部可控委托）
2. Agent ↔ Agent（外部协作，消息保底）
3. Agent ↔ Human（人类协作，平台化优先）

同时系统希望通过可信 Worklog 降低“观测成本”，但必须支持回退到传统“扫描文件”策略。

### 1.2 目标

* 建立清晰的 **注意力单元（Session）**、**私有执行区（Workshop）**、**执行基座（Local Workspace）**、**交付/协作目标（Remote WorkspaceService）** 的职责边界
* 定义三类协作的最小闭环协议（Todo + Patch/PR + Verify）
* 确保运行时“路由到正确 Session/Workspace”可解释、可确认，避免选错 Workspace 造成无效劳动

---

## 2. 范围与非范围

### 2.1 范围（In Scope）

* 概念关系与绑定语义（Session/Workshop/Local/Remote）
* Router/Resolver 路由、Workspace 解析与确认
* Todo 数据结构、状态机、权限边界
* Patch/PR 交付物、验收与合并
* SubAgent 生命周期与隔离工作树
* Worklog 记录粒度与回退策略
* Remote WorkspaceService 同步/刷新策略

### 2.2 非范围（Out of Scope）

* UI 细节与视觉交互
* 具体 Git 实现细节（worktree/rebase/patch 格式）
* 经济/声誉/支付体系
* 全量安全审计与攻防（仅给需求边界）

---

## 3. 术语与概念定义

### 3.1 核心实体

* **Agent**：对用户负责的主执行体，具备最终合并与验收责任。
* **SubAgent**：内部委托执行体，非公开、无消息端点、隔离工作树、受限写入。
* **Human**：人类协作者，通过消息或平台接口参与。
* **Session**：注意力/上下文单元，聚焦一个 topic/问题。
* **Workshop**：Agent 私有执行区（Data 分区内的 `workshop/`），仅该 Agent 可写。
* **Local Workspace**：Workshop 内的本地工作目录（如 codespace），承载 PDCA 循环与中间态。
* **Remote WorkspaceService**：远端交付/协作目标（GitHub、发布服务、SaaS 平台等），以服务接口形式存在。
* **Todo**：协作锚点与任务状态机载体。
* **Patch**：最小交付单位之一（也可对应 PR/commit），用于合并变更。
* **Worklog**：系统记录的变更日志（加速器，不是替代品）。

### 3.2 新增：Workspace 语义字段（用于“绑定不松散”）

每个 Local Workspace **必须**具备以下元信息（Workspace Record）：

* `workspace_id`
* `owner_agent_id`（Workspace 所属 Agent，唯一）
* `creator_session_id`（创建该 Workspace 的 Session）
* `origin_type`：`CREATED`（内生创建）/ `IMPORTED`（外部导入/clone）
* `trust_level`：`HIGH`（内生）/ `BOOTSTRAP`（导入后初始化中）/ `LOW`（外部只读）
* `binding_sessions`（引用该 workspace 的 Session 列表，可多）
* `default_write_session`（当前持有写权的 Session，可为空）
* `last_activity_at`

> 说明：
>
> * “Owner”永远是 Agent（一个 workspace 不跨 agent 所有权）。
> * “Creator Session”用于解释来源与默认关联。
> * 多 Session 可引用同一 workspace，但**写入权**在最小策略下必须被控制（见 6.1 并发策略）。


---

## 4. 核心原则

1. **Session 管注意力，Workspace 管执行，Remote 管交付**
2. **本地永远可执行**：私有 Workshop + Local Workspace 是万能执行基座
3. **协作保底用 Message，密集协作用平台化 WorkspaceService**
4. **所有外来变更必须验收**（SubAgent/外部 Agent/人类）
5. **Workspace 选择高风险**：解析必须可解释，必要时必须用户确认
6. **Worklog 是加速器，不是替代品**：允许回退到文件扫描
7. **并发最小策略优先保证正确性**：先保证“不写错/不污染”，再追求吞吐

---

## 5. 系统组件概览（需求级）

* **Workshop Manager**：管理 `workshop/`、Local Workspace 创建/索引/清理
* **Session Manager**：Session 创建/加载；维护 Session ↔ Workspace 引用
* **Router/Resolver**：将输入/消息路由到正确 Session+Workspace；输出解释与证据；触发确认
* **Todo Service**：任务创建、指派、状态机、交付物引用、验收记录
* **Delivery Integrator**：下载/应用 patch、创建/更新 PR、验证、回写状态
* **Worklog Service**：记录变更摘要；支持按时间点/Session 查询；支持回退策略
* **WorkspaceService Driver（可扩展）**：对接 GitHub 等远端平台的接口适配层

---

## 6. 功能需求

### 6.1 Session ↔ Workspace 绑定模型（收紧语义 + 并发最小策略）

#### 6.1.1 Session 引用模型

**R-6.1.1** Session **必须**支持引用：

* `0..1` 个 `default Local Workspace`（执行主环境）
* `0..N` 个 `Remote WorkspaceService`（交付/协作目标）

**R-6.1.2** 多个 Session **可以**共享同一 Local Workspace（同一项目多 topic 拆分），但写入必须遵循 6.1.3 的最小策略。

#### 6.1.2 Workspace 的归属与来源

**R-6.1.3** 每个 Local Workspace **必须**记录：

* `owner_agent_id`（唯一）
* `creator_session_id`
* `origin_type` 与 `trust_level`（用于观测与信任策略）

#### 6.1.3 并发写入最小可行策略（必须明确）

**R-6.1.4** 系统 **必须**提供最小并发策略，以避免多个 Session 并行写同一 Local Workspace 导致污染。最小策略为：

* ** 单写者 Lease（默认）**

  * 写操作前，Session **必须**获取该 workspace 的写 Lease（排他）。
  * 实现方法: 确保没有别的session的todo是工作中的状态
  * session长时间没有工作，会进入sleep状态，作为TTL释放

**R-6.1.5** “串行 apply”如何保证：

* 主 Agent 是唯一合并者（Single Integrator）。
* 所有 patch/PR 必须在主 Agent 的控制下串行进入集成分支或主工作树。

---

### 6.2 Router/Resolver：路由、解析、确认与可解释性（更具体）

#### 6.2.1 必经流程

**R-6.2.1** 对以下输入，系统 **必须**先 resolve（定位 Session + Workspace）再执行：

* 用户输入（尤其是模糊指令，如“我要调整一下”）
* 任意外部消息（A2A）
* Todo 状态变更Event（DELIVERED/超时/失败等）

#### 6.2.2 Resolve 输出（系统解释 vs 用户解释）

**R-6.2.2** Resolve **必须**输出两类解释：

* **面向用户的解释（User Explanation）**

  * 自然语言摘要
  * 展示 1~3 个候选 Workspace/Session
  * 展示关键证据（例如：创建时间、关联项目名、最近变更、关联 Todo/交付目标）

* **面向审计/系统的解释（Audit Explanation）**

  * 结构化记录：候选列表、置信度、证据项、风险等级、最终选择、是否经过确认

#### 6.2.3 绑定确认（必要时 double-confirm）

**R-6.2.3** 当 Session 没有 default Local Workspace，且 Resolver 通过历史解析得到候选 workspace 时：

* 系统 **必须**要求用户确认后才能绑定为 default Local Workspace
* 当风险等级为高（例如多个候选置信度接近、或 workspace 属于外部导入且 trust_level 非 HIGH），系统 **应该**触发 double-confirm（两步确认）

#### 6.2.4 解析质量目标（作为实现目标而非硬性 SLA）

**R-6.2.4** 系统 **应该**满足以下目标（可随数据迭代调整）：

* 明确引用（用户提到项目名/仓库名/显式链接）时：Top-1 解析正确率 ≥ 95%
* 模糊指令（“调整一下”“继续做”）时：Top-1 ≥ 70%，不足则依赖确认交互保证最终正确
* 任何情况下：必须允许用户在 ≤2 次确认交互内完成绑定（对应 S1 的产品指标）

---

### 6.3 Todo：状态机（定义 entry condition / trigger action / 退回路径）

#### 6.3.1 Todo 字段（最小集）

**R-6.3.1** Todo 项 **必须**包含：

* `id`
* `title/description`
* `session_id`
* `local_workspace_id`（可空但建议填）
* `assignee_type`：`MAIN_AGENT | SUBAGENT | EXTERNAL_AGENT | HUMAN`
* `assignee_id`（若适用）
* `status`
* `deliverables[]`（patch/PR/artifact URL）
* `verification[]`（验证记录：测试、构建、review、apply 结果）
* `timestamps`（created/updated/delivered/verified/completed）
* `rejection_reason`（可选）
* `retry_policy`（可选：重试次数/是否自动重新指派）

#### 6.3.2 状态定义（强语义）

建议状态集合（最小闭环）：

1. **OPEN**

* entry：创建 todo
* action：可分配 assignee；可等待用户补充信息

2. **IN_PROGRESS**

* entry：assignee 接单或主 Agent 开始处理
* action：执行；可更新进展；可生成中间交付

3. **BLOCKED**

* entry：需要额外信息/外部依赖/冲突待解
* action：记录阻塞原因；等待用户/外部消息/资源

4. **DELIVERED**（已交付，未验收）

* entry：deliverables 至少包含一项（patch/PR/url）
* action（主 Agent 触发）：开始验收流程（下载/apply/检查/测试）

5. **VERIFIED**（已验证通过，未必已合并/发布）

* entry：deliverables 已被成功 apply 到集成环境（或 PR 检查通过），验证项全部通过
* action：准备合并/发布；生成合并记录草案

6. **COMPLETED**（已合并/已发布/已关闭）

* entry：变更已合并到目标分支或已发布到目标环境，且任务关闭
* action：关闭 todo；写入最终链接（merge commit / release / deployment）

7. **REJECTED**（验收失败/退回）

* entry：apply 冲突不可自动解决、测试失败、review 未通过、安全检查失败等
* action：必须记录 `rejection_reason` + 建议修复方向
* next transitions（必须定义）：

  * `REJECTED → IN_PROGRESS`（退回给原 assignee 修复）
  * `REJECTED → OPEN`（重新分诊/改派）
  * `REJECTED → BLOCKED`（等待用户澄清或依赖）
  * （可选）`REJECTED → COMPLETED`（标记 wontfix/不做，需显式理由）

8. （可选但推荐）**CANCELLED**

* entry：用户取消或任务失效
* action：关闭但保留审计记录

> 关键澄清：
>
> * `VERIFIED` vs `COMPLETED`：前者强调“技术验证通过”，后者强调“已合并/发布并关闭”。两者不应混用。

---

## 7. 协作方式需求

### 7.1 Agent ↔ SubAgent（内部委托协作）

#### 7.1.1 可见性与通信

**R-7.1.1** SubAgent **必须**非公开：无 message endpoint，用户不可直接对话或指派。

#### 7.1.2 隔离执行（强约束）

**R-7.1.2** SubAgent **必须**在隔离工作树（自己的 local workspace）中执行，不直接写入主 Local Workspace。

#### 7.1.3 共享状态与最小写权限

**R-7.1.3** SubAgent **必须**通过 Todo 作为主要共享状态通道：

* 可读任务相关上下文
* 仅可写其被委托 Todo 项（状态/交付物/简要说明）

#### 7.1.4 交付物

**R-7.1.4** SubAgent 完成后 **必须**：

* 生成 patch（或等价本地目录/文件）
* 将 Todo 状态置为 `DELIVERED`
* 在 deliverables 中写入交付物路径

#### 7.1.5 主 Agent 验收与合并

**R-7.1.5** 主 Agent 发现 `DELIVERED` 后 **必须**：

* 获取 patch → apply 到集成/主 workspace（需持有 Lease）
* 运行验证（build/test/check）
* 通过后：`VERIFIED → COMPLETED`（合并/发布并关闭）
* 失败：进入 `REJECTED` 并记录原因

---

### 7.2 SubAgent 生命周期管理（补齐缺失项）

- SubAgent 的生命周期逻辑于标准Agent一致，会长期持有自己的session和workshop
- SubAgent 的Session生命周期逻辑于标准Agent Session 一致 
- 相对来说Sub Agent更容易Sleep，也不容易因为外部input而激活

---

### 7.3 Agent ↔ Agent（外部协作）

#### 7.3.1 保底通信（A2A）

这是Agent将自己的TODO分配给另一个Agent执行的模式。核心流程和SubAgent一致。

**R-7.3.1** 系统 **必须**支持 A2A message 作为保底通道：消息可携带 `session_ref / todo_ref / workspace_ref / deliverable_url`。

#### 7.3.2 外部交付物（URL Patch / PR）

**R-7.3.2** 外部 Agent 完成任务后 **必须**支持提供至少一种交付形式：

* patch 下载 URL
* 或 PR URL（平台化交付）

#### 7.3.3 主 Agent 统一路由与验收

**R-7.3.3** 主 Agent 收到外部消息后 **必须**：

* 先 resolve 到正确 Session + Local Workspace + Todo
* 再执行下载/apply/验证/状态回写
* 验证通过后才允许进入 `VERIFIED/COMPLETED`

---

### 7.4 Agent ↔ Human（人类协作）

#### 7.4.1 Message 保底

**R-7.4.1** 系统 **必须**支持人类通过消息提交需求、反馈、交付链接；处理流程同 7.3.3。

#### 7.4.2 平台型协作（WorkspaceService：GitHub 类）

**R-7.4.2** 系统 **应该**支持 Session 绑定 Remote WorkspaceService（如 GitHub repo），用于长期密集协作（PR/Review/Issue）。

---

## 8. Worklog：写入时机与粒度（给出上下界）

**R-8.1** Worklog **必须**至少记录“文件路径级别”的变更摘要（下界要求），包含：

* 时间戳、actor（Session/Agent/SubAgent）、workspace_id
* 操作类型（create/update/delete/rename）
* 变更文件路径列表
* 关联 todo/session（若可得）

**R-8.2** Worklog **不强制**记录行级 diff（上界约束，避免“地图比文件大”）。

* 行级 diff 可作为可选能力，仅在需要时生成（如验收审计、回滚分析）。

**R-8.3** 系统 **应该**支持关键节点快照（snapshot），至少包括：

* apply patch 前的快照点（便于回滚）
* 合并/发布前的快照点（便于追溯）

**R-8.4** 回退机制：

* Agent **必须**能够忽略 Worklog，直接扫描 workspace 文件作为最终真相来源。

---

## 9. Remote WorkspaceService：交付阶段加载的补充约束（避免“过时状态”）


**R-9.1** 在任何交付动作前（创建 PR / push / merge / release），系统 **必须**进行远端状态同步（Pre-Delivery Sync）：

* 拉取远端最新元信息（默认至少：HEAD、PR 列表、最近提交、CI 状态）
* 检测本地基线是否落后或分叉

**R-9.2** 若检测到远端有新变更：

* 系统 **必须**采取其中一种策略（可配置）：

  * `UPDATE_LOCAL_BASELINE`：更新本地基线后再继续（pull/rebase/merge）
  * `CREATE_DIVERGENCE_TASK`：创建一个 Todo 处理分叉（先不交付）
  * `ASK_CONFIRM`：风险高时要求用户确认继续或先同步

**R-9.3** 刷新策略（降低 token 且避免过时）：

* 系统 **应该**支持：

  * 事件驱动刷新（收到 PR comment / new commit 通知时）
  * 时间阈值刷新（超过 T 时间未 sync，交付前强制 sync）

---

## 10. 典型流程（含异常流程）

### 10.1 Happy Path：SubAgent UI 优化

1. 主 Agent 创建 Todo（OPEN）→ 指派 SubAgent → IN_PROGRESS
2. SubAgent 在隔离 workspace 完成 → 生成 patch → DELIVERED
3. 主 Agent 获取 Lease → apply patch → 验证通过 → VERIFIED
4. 合并/发布 → COMPLETED（关闭）

### 10.2 异常：patch apply 冲突

1. Todo 已 DELIVERED
2. 主 Agent apply 失败（冲突）
3. Todo → REJECTED（reason=apply conflict），并自动：

* 生成冲突摘要
* 创建子任务（可选）让 SubAgent/外部协作者解决冲突

4. 冲突解决后回到 IN_PROGRESS → 再次 DELIVERED → 验收

### 10.3 异常：SubAgent 超时

1. SubAgent RUNNING 超过 TTL → TIMED_OUT
2. Todo → BLOCKED（reason=timeout），并提供选项：

* 重新指派同一 SubAgent 重试（回到 IN_PROGRESS）
* 改派外部 Agent 或 Human
* 取消（CANCELLED）

### 10.4 异常：用户拒绝所有候选 Workspace

1. 用户输入“继续调整”触发 resolve
2. Resolver 给出候选 1~3 个 workspace
3. 用户全部拒绝
4. 系统必须提供下一步：

* 新建 Local Workspace（origin_type=CREATED）
* 或引导用户提供更明确锚点（repo/url/项目名）再 resolve（不阻塞主流程）

---

## 11. 非功能性需求（补齐并发与解释）

* **NF-1 可解释性**：所有 workspace/session 路由必须可解释（用户解释 + 审计解释）。
* **NF-2 正确性优先**：宁可要求确认，也不允许静默绑定到低置信 workspace。
* **NF-3 单合并者**：主 Agent 是唯一集成者，所有外来变更必须串行进入集成分支/主工作树。
* **NF-4 并发控制**：默认 Lease 排他写；并行通过派生工作树解决。
* **NF-5 可降级**：Worklog/Resolver 失效时可回退到扫描文件与手动选择。

---

## 12. 成功指标（可验收）

* **S1**：模糊输入在 ≤2 次确认交互内绑定到正确 workspace（若不确定必须确认而不是猜）。
* **S2**：SubAgent 任务闭环：Todo → patch → apply → verify → close，且主 workspace 不被直接污染。
* **S3**：外部协作闭环：A2A 消息携带交付物链接，主 Agent 可稳定 resolve 并完成验收。
* **S4**：平台化协作闭环：PR/Review/Issue 支持多轮迭代，交付前强制 sync 防止过时基线。
* **S5**：Worklog 可用时减少观测成本；不可用时可无损回退到全量扫描。

---

## 13. 开放问题（保留到后续版本）

1. Worklog 的压缩与索引策略（避免膨胀）
2. 外部交付物的安全校验（签名、来源信誉、恶意变更检测）
3. WorkspaceService Driver 的能力声明与最小权限模型
4. Todo 与远端 Issue/PR 的双向同步一致性策略

