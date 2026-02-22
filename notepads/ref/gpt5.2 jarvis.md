

## 当前文档的主要缺口（会导致工程师实现时“各写各的”）



### 缺口 2：TODO 数据结构与状态机不够“可执行”

你提到 Complete/Failed/Done/CHECK_FAILED/WAIT/Bench，但没把以下锁死：

* TODO 的字段（id、title、deps、type、skills、assignee、subagent、artifacts、acceptance_criteria…）
* 合法状态流转图（比如 WAIT→IN_PROGRESS→COMPLETE→DONE；CHECK_FAILED→ADJUSTING→…）
* Check 阶段对 Bench 的特殊规则（“WAIT 变 Done”）需要更精确条件：**何时判定 Bench 可测？**

### 缺口 5：等待（wait_events）的**事件名与触发条件**没有标准化

比如：

* `WAIT_USER_INPUT` / `WAIT_USER_APPROVAL(tool=...)` / `WAIT_SUBAGENT(task_id=...)`
  如果没有统一枚举，工程师会各自定义，后续不好维护。

### 缺口 6：Memory 编译的预算策略只有概念，没有“工程可实现规则”

你写了段落优先级，但没有：

* 每段最大 token/字符预算的默认值
* “压缩触发条件”（超过预算？超过轮次？Self‑Improve 触发？Plan 触发？）
* 压缩输出写回到哪里（AgentMemory / Session Summary / Workspace Summary 的边界）

### 缺口 7：Do 阶段“自修复尝试”需要硬规则

“多次失败后标记 Failed”——工程上要定义：

* 最大重试次数 N
* 什么算失败（tool error / test fail / lint fail / check fail）
* 每次重试必须改变什么（参数、策略、工具选择），避免死循环

### 缺口 8：Self‑Improve 的“写入目标”需要明确接口

你写：

* 修改 self.md
* 给 KB Agent 发消息
  工程师需要明确：
* self.md 存在哪个路径？通过什么工具写？是否要 PR/commit？
* 给 KB Agent 发消息的 tool 名、payload 结构、可靠性语义（至少一次？幂等？）


### 3）TODO 模型（最小字段）

```json
{
  "todo_id": "string",
  "title": "string",
  "type": "TASK|BENCH",
  "status": "NEW|WAIT|IN_PROGRESS|COMPLETE|DONE|FAILED|CHECK_FAILED",
  "deps": ["todo_id"],
  "skills": ["string"],
  "assignee": "MAIN|SUBAGENT:<name>",
  "can_start_immediately": "boolean",
  "acceptance_criteria": ["string"],
  "artifacts": ["path_or_url"],
  "worklog_refs": ["worklog_id"],
  "blockers": ["string"]
}
```


## 6 个提示词实现：工程师可照抄的“Prompt Skeleton”


---

### 0）Router / Resolve-Router（合并成一个“路由提示词”也可）

**process_rules 关键点**

* 只做：识别 session_id、判断是否需要多步澄清、给 quick reply（可选）
* 禁止：创建 TODO、改 workspace
* 若信息不足：输出 `resolve_router` 并 `next_behavior=PLAN`，同时 `is_wait=true` 要求补充（例如“你想继续哪个 session？”）

**policy 关键点**

* 不臆测 session_id；没有就明确 `create_new_session=true`
* new_event-only：若无需回复用户，可 `should_reply_user=false`

**input 模板应包含**

* new_msg（原文、解析出的 metadata、是否含 session_id）
* new_event（类型、payload）
* 当前 session 列表摘要（最近 N 个 session 的 topic + id）

---

### 1）PLAN 提示词

**process_rules**

* Workspace readonly
* 目标：把 Input → 可执行 TODO 列表（含 skills、deps、acceptance_criteria）
* 必要时向用户请求确认/补信息/授权（通过 `SEND_MSG` + `is_wait=true`）
* 初始化 workspace（若 session 尚未绑定 workspace）
* 对可并行的 TODO：用 `DISPATCH_SUBAGENT`，并把 TODO.assignee 改为对应 subagent，状态变 WAIT（等待 subagent event）

**policy**

* 不要在 Plan 中写交付物到 workspace（只允许创建结构、写 todo、写摘要）
* 不要承诺“后台完成”；必须显式等待事件

**memory 模板建议顺序与预算（示例）**

1. AgentMemory（高优先）
2. Session Summary
3. Workspace Summary
4. Workspace Todo（必须）
5. 最近 K 条 History Message（低优先，预算不足则裁剪）

---

### 2）DO 提示词

**process_rules**

* 输入是“当前可执行 todo”（deps 全部 DONE 或由 subagent 负责且已完成）
* 多 step：每步基于 LastStep Summary 决策下一步 action
* 要求：最后必须做一次自检（lint/test/验收点对照）
* 自修复：同一 todo 允许最多 N 次修复尝试；每次必须记录变更点；超过 N → FAILED
* 若 todo 依赖 subagent 且 subagent 未回：输出 `is_wait=true`，等待 `WAIT_SUBAGENT`

**policy**

* 工具调用必须可追踪：每次 `CALL_TOOL` 都写入 worklog（或由系统自动写）
* 不要在 Do 阶段扩展 scope（新增 todo 必须回到 Plan 或 Adjust）

---

### 3）CHECK 提示词

**process_rules**

* 输入：status=COMPLETE 的 todo + workspace 交付物引用
* 只检查，不修复
* 先整体验证（结构/一致性/规范），再边缘验证（极端/异常/契约）
* 失败立刻：todo → CHECK_FAILED，next_behavior=ADJUST
* Bench 规则：把 type=BENCH 且 status=WAIT 的 todo 在 Check 阶段执行集成测试，满足条件则 DONE

**policy**

* 不写代码、不修改交付（除非写“检查报告”到 worklog/summary）
* 检查结果必须结构化输出到 last_step_summary（便于 Adjust 分析）

---

### 4）ADJUST 提示词

**process_rules**

* readonly 为主（允许：更新 todo 计划、补充 acceptance_criteria、增加 blockers）
* 深度归因：路径问题/信息缺失/技能不足/任务过难
* 若缺授权/缺信息：SEND_MSG 请求并 WAIT
* 若缺工具：提议“新 session 构建工具”（输出清晰的 session 切换建议）
* 输出：调整后的计划（哪些 todo 回到 IN_PROGRESS / 是否拆分新 todo）

**policy**

* 不做修复实现；把修复留给 Do
* 不要用模糊原因（如“模型不好”）；必须对照失败证据（Check 报告、日志、workspace 文件）

---

### 5）SELF_IMPROVE 提示词

**process_rules**

* 输入通常是 new_event 或 session 收尾信号
* 做三类整理：

  * Memory 压缩（更新 AgentMemory/SessionSummary/WorkspaceSummary）
  * Session 整理（清理历史、归档关键决策）
  * Workspace 整理（worklog、todo 状态一致性）
* 可选：更新 self.md（改变工作方式）
* 可选：发消息给 KB Agent（只描述需求，不自己维护 KB）

**policy**

* 只做“可复用的抽象”，不要把用户隐私/敏感数据写进长期记忆
* 修改 self.md 要求可回溯（建议走 git 提交或至少写入变更记录）

---


