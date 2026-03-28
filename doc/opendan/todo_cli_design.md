# todo_manage CLI 设计方案

## 0. 设计原则

借鉴 todo.sh / Taskwarrior 的经验，核心三条：

1. **最短路径优先**：最常用操作 = 最少 token，Agent 输出越短越不容易出错
2. **隐式注入**：workspace_id / session_id / agent_id / actor_ctx 全部从 SessionRuntimeContext 自动注入，Agent 永远不需要手写
3. **纯位置参数 + flags**：所有操作都是 `todo <verb> [positional...] [--flags]`，不需要 JSON

---

## 1. 命令总览

```
todo <subcommand> [args...]
```

工具名从 `todo_manage` 改为 `todo`（更短，更符合 CLI 惯例）。


### 按 PDCA 阶段分组

| 阶段    | 命令                                      | 说明                        |
|---------|------------------------------------------|-----------------------------|
| PLAN    | `todo clear`                             | 清空当前列表（新计划前）       |
| PLAN    | `todo add "title"`                       | 添加一条 todo                |
| DO      | `todo start T001`                        | 开始执行                     |
| DO      | `todo done T001 "reason"`                | 标记完成                     |
| DO      | `todo fail T001 "reason"`                | 标记失败                     |
| CHECK   | `todo pass T001`                         | 验收通过                     |
| CHECK   | `todo reject T001 "reason"`              | 验收不通过                   |
| 任意    | `todo note T001 "content"`               | 追加笔记                     |
| 查询    | `todo ls`                                | 列表                        |
| 查询    | `todo show T001`                         | 详情                        |
| 查询    | `todo next`                              | 下一个可执行的 todo           |
| 查询    | `todo pending`                           | 各状态计数                   |
| Prompt  | `todo prompt`                            | 渲染 Workspace Todo 文本     |
| Prompt  | `todo current`                           | 渲染 Current Todo 详情       |

---

## 2. 各命令详细设计

### 2.1 PLAN：添加任务

#### clear（清空列表）

```bash
todo clear                            # 清空当前 workspace 的所有 todo（相当于旧的 init mode=replace 空列表）
```

新 PLAN 开始时先 `clear`，再逐条 `add`。如果是 merge 场景（长期 workspace 追加任务），直接 `add` 即可，不需要 `clear`。

#### add（添加一条 todo）

```bash
# 最简：只写标题，其余全默认
todo add "搭建项目骨架"
todo add "实现核心功能"
todo add "集成测试" --type=Bench

# 带更多选项
todo add "搭建项目骨架" --priority=0 --labels=setup --skills=git,bash
todo add "实现功能" --deps=T001
todo add "集成测试" --type=Bench --priority=10
```

**默认值策略（最大化减少 Agent 输入）**：

| 参数        | 默认值                        | 说明                             |
|------------|------------------------------|----------------------------------|
| type       | `Task`                       | 只有 Bench 需要显式写              |
| assignee   | 当前 agent DID               | 通常不需要写                       |
| priority   | 按添加顺序自增（10, 20, 30…）  | 只在需要乱序时手动指定              |
| deps       | 自动链式依赖前一条              | Bench 自动依赖所有前序非 Bench     |
| labels     | 空                            | 可选                              |
| skills     | 空                            | 可选                              |
| description| 空                            | 可选，`--desc="验收标准：..."`      |

**deps 规则（与 PRD 对齐）**：
- 不写 `--deps`：自动依赖前一条 todo（链式顺序执行）
- `--deps=T001,T003`：显式依赖指定 todo
- `--no-deps`：无依赖（独立任务，可并行执行）
- Bench 不写 deps 时：自动依赖所有前序非 Bench（确保 CHECK 时机正确）

**典型 PLAN 输出**（Agent 发一组 add 命令）：

```bash
todo clear
todo add "搭建项目骨架"
todo add "实现核心功能"
todo add "编写单元测试"
todo add "集成测试" --type=Bench
```

对比旧方式（一整块嵌套 JSON）：
```bash
todo_manage '{"action":"apply_delta","workspace_id":"ws_xxx","delta":{"ops":[{"op":"init","mode":"replace","items":[{"title":"搭建项目骨架"},{"title":"实现核心功能"},{"title":"编写单元测试"},{"title":"集成测试","type":"Bench"}]}]}}'
```

优势：
- 每行独立，Agent 逐条生成、逐条校验，出错只影响单条
- 不需要记 JSON 嵌套结构
- 输出 token 数更稳定（每条 ~6-10 token vs 整块 ~100+ token）

#### add 的事务性

PLAN 阶段的 `clear` + N 个 `add` 需要整体成功。两种策略：

**策略 A（推荐：宽松模式）**：每个 `add` 独立 apply，单条失败不影响其他。Agent 在 observation 中看到失败后可以修正重试。

**策略 B（严格模式）**：系统识别 `clear` 后连续的 `add` 序列，攒成一个事务。需要一个显式的 "commit" 信号，或依赖 behavior step 边界作为事务边界。

建议先实现策略 A，简单可靠。

---

### 2.2 DO：执行任务

```bash
todo start T001                    # WAIT -> IN_PROGRESS
todo done  T001 "实现完成"          # -> COMPLETE
todo fail  T001 "编译报错"          # -> FAILED
```

**设计决策**：
- 位置参数 1 = todo_code，位置参数 2 = reason（可选引号字符串）
- `start` 不需要 reason（默认 "started"）
- `done` / `fail` 的 reason 必须提供（PDCA 审计要求）
- `fail` 可追加 `--error='{"code":"E001","message":"..."}'`（可选 last_error）

```bash
# 带 last_error 的 fail
todo fail T001 "编译报错" --error='{"code":"compile_err","message":"..."}'
```

**Agent 最短输出**：

```bash
todo done T001 "实现完成"
```

对比现有：
```bash
todo_manage '{"action":"apply_delta","workspace_id":"ws_xxx","delta":{"ops":[{"op":"update:T001","to_status":"COMPLETE","reason":"实现完成"}]}}'
```

---

### 2.3 CHECK：验收

```bash
todo pass   T001                   # COMPLETE -> DONE (Bench: WAIT -> DONE)
todo reject T001 "测试不通过"       # COMPLETE -> CHECK_FAILED (Bench: WAIT -> CHECK_FAILED)
```

**设计决策**：
- `pass` 不需要 reason（默认 "verified"）
- `reject` 必须有 reason
- 与 DO 的 `done` / `fail` 语法一致

---

### 2.4 Notes：追加笔记

```bash
todo note T001 "已生成产物 xxx"
todo note T001 --kind=result "关键结果: ..."
todo note T001 --kind=error  "错误详情: ..."
```

**设计决策**：
- kind 默认 `note`，可选 `result` / `error`
- content 为最后一个位置参数

---

### 2.5 查询

#### ls（列表）

```bash
todo ls                            # 默认：活跃状态，按 priority+order 排序
todo ls --all                      # 包括 DONE
todo ls --status=IN_PROGRESS       # 筛选状态
todo ls --type=Bench               # 筛选类型
todo ls --assignee=did:od:web      # 筛选负责人
todo ls --label=setup              # 筛选标签
todo ls -q "关键词"                 # 搜索 title/description
todo ls --limit=10 --offset=0      # 分页
todo ls --sort=priority            # 排序：priority / order / updated_at(默认)
```

**设计决策**：
- 无参数时默认只显示未完成状态（WAIT/IN_PROGRESS/COMPLETE/FAILED/CHECK_FAILED）
- `--all` 是 `--status=*` 的快捷方式
- 多值用逗号：`--status=WAIT,IN_PROGRESS`

**Agent 最短输出**：

```bash
todo ls
```

#### show（详情）

```bash
todo show T001                     # 完整详情 + notes + deps
```

#### next（下一个可执行 todo）

```bash
todo next                          # 返回最高优先级且依赖已满足的 WAIT todo
```

这是 Agent DO 循环中最频繁的调用，0 参数。session_id/agent_id 全部从 ctx 注入。

#### pending（计数）

```bash
todo pending                       # 各状态计数
todo pending --status=WAIT,IN_PROGRESS  # 只关心特定状态
```

---

## 3. 隐式注入规则

以下字段由系统从 `SessionRuntimeContext` 自动注入，Agent 永远不写：

| 字段           | 来源                          | 覆盖方式（调试用，可选）   |
|---------------|------------------------------|------------------------|
| workspace_id  | session 绑定的当前 workspace    | `--ws=xxx`             |
| session_id    | ctx.session_id               | `--session=xxx`        |
| agent_id      | ctx.agent_name               | `--agent=xxx`          |
| actor_ctx     | 从 ctx 推导 kind/did/session  | 不可覆盖               |
| op_id         | 系统自动生成                   | `--op-id=xxx`          |

**Agent 写出的命令里不会出现这些字段**。

---

## 4. 状态流转速记表

给 Agent prompt 用的极简参考：

```
PLAN:  add     -> 新 todo 全部 WAIT
DO:    start   -> WAIT => IN_PROGRESS
       done    -> IN_PROGRESS => COMPLETE  (或 WAIT => COMPLETE)
       fail    -> IN_PROGRESS => FAILED    (或 WAIT => FAILED)
CHECK: pass    -> COMPLETE => DONE         (Bench: WAIT => DONE)
       reject  -> COMPLETE => CHECK_FAILED (Bench: WAIT => CHECK_FAILED)
ADJUST:start   -> CHECK_FAILED/FAILED => IN_PROGRESS
       done    -> FAILED => COMPLETE
```

---

## 5. 命令-状态映射表

| 命令      | 触发状态迁移              | reason 必填 | 默认 reason      |
|----------|--------------------------|------------|-----------------|
| start    | WAIT→IN_PROGRESS         | 否         | "started"       |
| done     | IP→COMPLETE / W→COMPLETE | 是         | —               |
| fail     | IP→FAILED / W→FAILED     | 是         | —               |
| pass     | COMPLETE→DONE / W→DONE   | 否         | "verified"      |
| reject   | COMPLETE→CHECK_FAILED    | 是         | —               |

> IP = IN_PROGRESS, W = WAIT

---

## 6. 实现要点

### 6.1 在 TodoTool 上实现自定义 exec()

当前 `TodoTool` 使用默认 `exec()` → `parse_default_bash_exec_args` → `call()`。
需要覆写 `exec()`，解析位置参数+flags，然后构造 JSON 调用 `call()`。

```
exec(ctx, line, shell_cwd):
  tokens = tokenize(line)      # ["todo", "done", "T001", "实现完成"]
  subcmd = tokens[1]           # "done"
  match subcmd:
    "ls"      -> build_list_args(tokens[2..], ctx)
    "show"    -> build_get_args(tokens[2..], ctx)
    "clear"   -> build_clear_args(ctx)
    "add"     -> build_add_args(tokens[2..], ctx)
    "start"   -> build_update_args("IN_PROGRESS", tokens[2..], ctx, reason_required=false)
    "done"    -> build_update_args("COMPLETE", tokens[2..], ctx, reason_required=true)
    "fail"    -> build_update_args("FAILED", tokens[2..], ctx, reason_required=true)
    "pass"    -> build_update_args("DONE", tokens[2..], ctx, reason_required=false)
    "reject"  -> build_update_args("CHECK_FAILED", tokens[2..], ctx, reason_required=true)
    "note"    -> build_note_args(tokens[2..], ctx)
    "next"    -> build_next_args(tokens[2..], ctx)
    "pending" -> build_pending_args(tokens[2..], ctx)
    "prompt"  -> build_prompt_args(tokens[2..], ctx)
    "current" -> build_current_args(tokens[2..], ctx)
    _         -> error("unknown subcommand")
```

#### add 的内部映射

`todo add "title" [--flags]` 在 exec() 内部构造为：

```json
{
  "action": "apply_delta",
  "workspace_id": "<auto>",
  "delta": {
    "ops": [{
      "op": "init",
      "mode": "merge",
      "items": [{
        "title": "<title>",
        "type": "<--type or Task>",
        "priority": "<--priority or auto>",
        "deps": "<--deps or auto>",
        ...
      }]
    }]
  }
}
```

`todo clear` 构造为一个 `init` + `mode=replace` + `items=[]`（空列表替换）。
或新增一个轻量的 `clear` action 直接清空 workspace 的 todo 表。

### 6.2 workspace_id 自动注入

在 `exec()` 中从 ctx 获取 session 绑定的 workspace：

```rust
// 优先级：--ws=xxx > session 绑定的 workspace
let workspace_id = extract_flag("ws", &tokens)
    .or_else(|| ctx.resolve_workspace_id())  // 需要新增此方法
    .ok_or_else(|| "no workspace bound to session")?;
```

### 6.3 别名注册

在 `workshop.rs` 注册时，同时注册 `todo` 和 `todo_manage` 两个 bash 命令名：

```rust
TOOL_TODO_MANAGE => {
    let tool = TodoTool::new(...)?;
    tool_mgr.register_tool(tool)?;
    // todo 别名指向同一个 tool
}
```

或者直接把 TOOL_TODO_MANAGE 常量改为 `"todo"`。

### 6.4 Usage / Help

```bash
todo --help
# 输出：
# Usage: todo <command> [args...]
#
# Plan:
#   clear                           Clear all todos (before new plan)
#   add   "title" [--type=Bench]    Add a todo item
#
# Do:
#   start  T001                     Begin task (WAIT -> IN_PROGRESS)
#   done   T001 "reason"            Mark complete (-> COMPLETE)
#   fail   T001 "reason"            Mark failed (-> FAILED)
#
# Check:
#   pass   T001                     Verify done (-> DONE)
#   reject T001 "reason"            Verify failed (-> CHECK_FAILED)
#
# Notes:
#   note   T001 "content"           Append note
#
# Query:
#   ls     [--status=X] [--all]     List todos
#   show   T001                     Show details + notes + deps
#   next                            Get next ready todo
#   pending                         Status counts
#
# Prompt:
#   prompt  [--budget=N]            Render for prompt
#   current [T001]                  Render current details
```

---

## 7. 对比：Agent 输出 Token 开销

| 操作              | 旧 (JSON)                            | 新 (CLI)                       | Token 节省 |
|-------------------|--------------------------------------|---------------------------------|-----------|
| 列表              | `action=list workspace_id=ws1`       | `todo ls`                       | ~80%      |
| 下一个            | `action=get_next... workspace_id=... session_id=... agent_id=...` | `todo next` | ~90%      |
| 标记完成          | `'{"action":"apply_delta",...嵌套3层JSON...}'` | `todo done T001 "完成"` | ~85% |
| 追加 note         | 类似上面的 JSON                       | `todo note T001 "content"`      | ~85%      |
| PLAN 3 条任务     | 嵌套 3 层 JSON，~120 token            | 3 行 `todo add`，每行 ~8 token  | ~80%      |
| 查 pending        | `action=query_pending workspace_id=...` | `todo pending`               | ~80%      |

### PLAN 阶段对比（3 条任务）

**旧**：
```bash
todo_manage '{"action":"apply_delta","workspace_id":"ws_xxx","delta":{"ops":[{"op":"init","mode":"replace","items":[{"title":"搭建骨架"},{"title":"实现功能"},{"title":"集成测试","type":"Bench"}]}]}}'
```

**新**：
```bash
todo clear
todo add "搭建骨架"
todo add "实现功能"
todo add "集成测试" --type=Bench
```

新方案逐条独立，Agent 生成更可控，单条出错可定位修复。

---

## 8. 向后兼容

不用向后兼容,exec逻辑是新实现

---

## 9. 与 Taskwarrior / todo.sh 的设计对比

| 设计点              | Taskwarrior          | todo.sh              | 本方案                  |
|---------------------|---------------------|----------------------|------------------------|
| 任务引用            | 数字 ID             | 行号                  | `T001` 短号            |
| 创建                | `task add "..."`    | `todo.sh add "..."`  | `todo add "..."`       |
| 标记完成            | `task 1 done`       | `todo.sh do 1`       | `todo done T001 "r"`   |
| 列表                | `task list`         | `todo.sh ls`         | `todo ls`              |
| 清空/重置           | `task purge`        | 手动清文件            | `todo clear`           |
| 依赖                | `depends:2`         | ❌                    | 自动链式 / `--deps`    |
| 状态机              | pending/done        | done/not done        | 6-state PDCA           |
| 隐式上下文          | ❌                  | ❌                    | ✅ workspace/session   |
| 审计                | undo.data           | ❌                    | oplog + sqlite          |

本方案保留了 todo.sh / Taskwarrior 的「`add` 逐条创建 + 短号引用」风格，同时：
- 上下文全部隐式注入（Agent 不写 workspace_id）
- PDCA 有语义化动词（start/done/fail/pass/reject）
- deps 默认链式，Bench 自动推导，Agent 几乎不需要手动写依赖

---

## 10. 未来扩展

- `todo pri T001 0`：改 priority（借鉴 todo.sh 的 `pri` 命令）
- `todo assign T001 did:od:web-agent`：改 assignee
- `todo cancel T001 "reason"`：取消任务（需新增 CANCELLED 状态）
- `todo deps T001`：查看依赖图
- `todo timeline T001`：查看状态流转历史（从 oplog 派生）
- `todo edit T001 --desc="new description"`：修改已有 todo 的描述/标签等

---

## 11. 完整 PDCA 示例

一个完整任务的 Agent 输出序列（全程无 JSON、无 workspace_id）：

```bash
# ── PLAN ──
todo clear
todo add "克隆仓库并搭建开发环境"
todo add "实现用户注册接口" --skills=rust,api
todo add "实现登录接口" --skills=rust,api
todo add "编写单元测试" --skills=rust,test
todo add "集成测试" --type=Bench

# ── DO ──
todo next                              # → T001
todo start T001
todo done T001 "环境搭建完成"
todo next                              # → T002
todo start T002
todo note T002 "使用 JWT 方案"
todo done T002 "注册接口实现完成"
todo next                              # → T003
todo start T003
todo done T003 "登录接口实现完成"
todo next                              # → T004
todo start T004
todo fail T004 "3 个测试用例失败"
todo start T004                        # 重试
todo done T004 "测试全部通过"

# ── CHECK ──
todo pass T001
todo pass T002
todo pass T003
todo pass T004
todo pass T005                         # Bench: WAIT -> DONE

# ── 查看状态 ──
todo ls
todo pending
```
