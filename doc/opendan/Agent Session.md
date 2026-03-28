# Agent Session 需求

## 1. 模块定位与目标

### 1.1 Session 的定义与价值

* **Session 是一个逻辑 topic**，用于归并上下文：消息、事件、todo、summary、cost/trace 等。
* **同一 Session 内的 LLM 调用必须顺序执行**（类似 thread），最小执行粒度是 **behavior step**；每 step 完成后需保存 `session.state`，支持系统重启后按 `behavior_name + step` 恢复或回退重跑。

### 1.2 “零 LLM 空转”目标

* Session 在 step 里要先 **生成输入并判空**；**无有效 input 则跳过推理**，并将 session 置为 `WAIT`，避免无意义消耗 token。

---

## 2. 功能需求清单（FR）

> 我按“Session 应该提供什么能力”来拆解，尽量可直接进 PRD/模块设计。

### FR-1 Session 状态机与状态管理

Session 最小状态集合需要支持：

* `PAUSE`：用户手工暂停
* `WAIT`：标准等待
* `WAIT_FOR_MSG`：等待特定 msg（超时后变 `READY`）
* `WAIT_FOR_EVENT`：等待特定 event（超时后变 `READY`）
* `READY`：就绪等待执行
* `RUNNING`：执行中
* `SLEEP`：长期无有效输入进入休眠（减少心跳与资源占用）

并且需要支持：

* `update_state(new_state)`：带审计/trace（至少写入 worklog/ledger 的钩子位）
* `set_wait_state(wait_details)`：用于 LLM 输出 `next_behavior=WAIT` 时设置细粒度等待条件

---

### FR-2 串行执行约束（session-level serial）

* **任意时刻一个 session 只执行一个 behavior 的一个 step**，保证状态可理解、可追踪、可恢复。
* Session 自身需要“可被 worker 安全独占”的机制（通常是 session-level lock / lease）。

---

### FR-3 输入缓冲与分类：new vs history

Session 需要维护至少两类输入容器：

* `new_msg` / `new_event`：新到达但尚未被 step 消费
* `history_msg` / `history_event`：已被 step “看过”的历史（可在 Memory 段落被编入 prompt，受预算控制）

并且要满足以下“二阶段语义”：

* Agent Loop 把 msg/event 分派给 session 后，**系统层可以标记 readed**（表示 Runtime 已收到并路由）；
* 但对 session 来说，它们仍是 `new_msg/new_event`，直到某次 step 消费后才进入 history。

---

### FR-4 `generate_input()`：基于 behavior_cfg 编译输入 + 判空（核心复杂点）

**输入来源**（文档列出的 input 关键元素）：

* new msg / new event
* Current Todo Details
* LastStep Summary（含成本与 step 计数）

**判空规则（来自文档的要求）**：

* Behavior 有 input 模板组合；**模板替换后若均为 Null，则本 step 跳过（无 input）**。
* `session.generate_input()` 判空；无有效 input 则不触发推理，session 置为 `WAIT`。

> 这里的“Null”不只是 Python 的 `None`，还应包含：空字符串、空数组、空对象、全空白字符串等（实现层面需要明确标准，见后面的伪代码）。

---

### FR-5 `update_input_used(exec_input)`：消费输入并迁移 new -> history

当某次 step 使用了输入后，必须把被使用的 msg/event 从 `new_*` 迁移到 `history_*`（避免重复进入 input）。

---

### FR-6 Behavior 指针管理（current_behavior / step_index）

Session 需要保存并更新：

* `current_behavior`
* `step_index`
* `last_step_summary`
* `session_delta` patch（LLM 输出可更新 session meta）

并支持 behavior 的切换策略：

* next_behavior = `WAIT` → 设置 wait_details
* next_behavior = `END` → 置为 `WAIT`
* 其他 → 切换 behavior 且 step_index 重置
* 无 next_behavior → step_index++，超 step_limit 回到 default_behavior 并 WAIT

> 上面是 runtime 视角，但 Session 组件至少要提供存储字段与原子更新能力。

---

### FR-7 Workspace 绑定与 local_workspace 并行锁约束

* **session 可以绑定 0 个 local_workspace 和 0..n 个 workspace**。
* 多个 session 可并行运行；但若两个 session 使用同一个 `local_workspace`，则**同一时刻只有一个能处于 RUNNING**（需要锁）。

（实现上可放在调度器，但 Session 需要暴露 `local_workspace_id / lock_key` 这类字段）

---

### FR-8 崩溃恢复与持久化（step-level save）

* **每个 step 完成后应立刻保存状态**，支持系统故障后从上一次 step 恢复。
* 可恢复内容至少包括：behavior step 状态、`last_step_summary`、session/workspace 的 worklog/todo 进度。

---

## 3. AgentSession 建议数据模型（可直接落到 struct/dataclass）

```python
class AgentSession:
    session_id: str

    # 状态机
    state: Literal["PAUSE","WAIT","WAIT_FOR_MSG","WAIT_FOR_EVENT","READY","RUNNING","SLEEP"]
    wait_details: dict | None          # WAIT_FOR_MSG/WAIT_FOR_EVENT 的过滤条件、deadline 等

    # behavior 执行指针
    current_behavior: str | None
    step_index: int
    last_step_summary: dict | None     # 上一步摘要（含成本、step count等）

    # 输入缓冲（两阶段：new -> history）
    new_msgs: list[Msg]
    new_events: list[Event]
    history_msgs: list[Msg]
    history_events: list[Event]

    # workspace 绑定
    workspace_info: dict | None
    local_workspace_id: str | None     # 用于并行锁约束

    # 可观测性
    worklog: list[dict]
    cost_trace: dict                   # token usage / action usage 等（可只存引用）
```

---

## 4. 关键伪代码：`generate_input()` 判空（你关注的复杂点）

### 4.1 判空标准（建议写成统一函数）

* `None` → Null
* `""` / `"   "` → Null
* `[]` / `{}` → Null
* 其他对象：如果定义了 `.is_empty()` / `.empty` 也可纳入

### 4.2 输入模板驱动（与文档一致）

文档强调：“input 模板组合（模板替换后若均为 Null 则无 input）”。

所以建议 `generate_input()` 做两件事：

1. **按 behavior_cfg.input_template 需要的 slot 去取数据**（避免无谓准备）
2. **所有 slot 都为 Null → 返回 None**（触发零空转）

### 4.3 伪代码（AgentSession 组件内部）

```python
def is_null_value(v) -> bool:
    if v is None:
        return True
    if isinstance(v, str):
        return len(v.strip()) == 0
    if isinstance(v, (list, tuple, set, dict)):
        return len(v) == 0
    return False


class AgentSession:

    def generate_input(self, behavior_cfg) -> dict | None:
        """
        按 behavior_cfg 的 input 模板编译输入：
        - 模板替换后若均为 Null => None
        - None => Runtime 将跳过推理，并把 session 置为 WAIT（零 LLM 空转）
        """
        # 1) 解析模板需要哪些 slot（示例：new_msg/new_event/current_todo/last_step_summary）
        slots = behavior_cfg.required_input_slots()

        rendered = {}

        # 2) 逐 slot 填充（只填模板要求的）
        if "new_msg" in slots:
            rendered["new_msg"] = self._select_new_msgs(behavior_cfg)

        if "new_event" in slots:
            rendered["new_event"] = self._select_new_events(behavior_cfg)

        if "current_todo" in slots:
            rendered["current_todo"] = self._select_current_todo(behavior_cfg)

        if "last_step_summary" in slots:
            # 注意：它可能为 None（比如第一步）
            rendered["last_step_summary"] = self.last_step_summary

        # 3) 判空：所有 slot 都是 Null => 无有效 input
        if all(is_null_value(v) for v in rendered.values()):
            return None

        # 4) 返回结构化输入（推荐包含用于消费/幂等的游标信息）
        exec_input = {
            "session_id": self.session_id,
            "behavior": self.current_behavior,
            "step_index": self.step_index,
            "payload": rendered,
            "consumable_ids": self._extract_consumable_ids(rendered),  # msg_id/event_id/todo_id
        }
        return exec_input

    def _select_new_msgs(self, behavior_cfg):
        # 若当前处于 WAIT_FOR_MSG，可按 wait_details 过滤（只选择匹配的 msg）
        msgs = self.new_msgs
        if self.state == "WAIT_FOR_MSG" and self.wait_details:
            msgs = [m for m in msgs if match_wait_filter(m, self.wait_details)]
        return msgs

    def _select_new_events(self, behavior_cfg):
        events = self.new_events
        if self.state == "WAIT_FOR_EVENT" and self.wait_details:
            events = [e for e in events if match_wait_filter(e, self.wait_details)]
        return events
```

### 4.4 与 Runtime 的配合点（文档要求）

当 `exec_input is None`：

* **必须不触发推理**
* 并将 session 状态置为 `WAIT`

（Runtime 侧伪代码你可以这样对齐）

```python
exec_input = session.generate_input(cfg)
if exec_input is None:
    session.update_state("WAIT")
    return  # 本 step 跳过（零 LLM 空转）
```

---

## 5. 关键伪代码：`update_input_used(exec_input)`（new -> history）

```python
class AgentSession:

    def update_input_used(self, exec_input: dict) -> None:
        """
        将本次 step 实际使用的 new_* 输入迁移到 history_*。
        对齐文档：new_msg/new_event 在“看过”后进入 history。:contentReference[oaicite:26]{index=26}
        """
        used = exec_input.get("consumable_ids", {})

        used_msg_ids = set(used.get("msg_ids", []))
        if used_msg_ids:
            still_new = []
            for m in self.new_msgs:
                if m.id in used_msg_ids:
                    self.history_msgs.append(m)
                else:
                    still_new.append(m)
            self.new_msgs = still_new

        used_event_ids = set(used.get("event_ids", []))
        if used_event_ids:
            still_new = []
            for e in self.new_events:
                if e.id in used_event_ids:
                    self.history_events.append(e)
                else:
                    still_new.append(e)
            self.new_events = still_new
```

---

## 6. 验收用例（重点覆盖你关心的判空）

1. **无 new_msg、无 new_event、无 current_todo、last_step_summary 为空**

   * `generate_input()` 返回 `None`
   * Runtime 将 session 置 `WAIT`，不触发 LLM

2. **只有 new_msg**

   * `generate_input()` 返回非空
   * step 执行后 `update_input_used()` 将该 msg 迁移到 history

3. **处于 WAIT_FOR_MSG，来了不匹配的 msg（例如来源不对/类型不对）**

   * `_select_new_msgs()` 过滤后为空
   * 若其他 slot 也为空，则 `generate_input()` 仍返回 `None`（保持零空转）

4. **两个 session 绑定同一个 local_workspace**

   * 调度器/锁保证同一时刻最多一个 session 为 RUNNING

5. **step 完成后持久化**

   * 每 step 后保存 session（支持崩溃恢复）

