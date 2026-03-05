# OpenDAN Agent 开发指南

面向开发 Agent / Behavior / Skill 的工程师，记录容易踩坑的配置细节与默认行为。

## 1. Tools 与 Actions 的可用性

### 1.1 判断逻辑概览

一个 action 在 prompt build 时被视为**可用**，需同时满足：

| 条件 | 含义 |
|------|------|
| 在 `requested_actions` 中 | 来自 `default_load_actions` 或当前 loaded skills 的 `actions` |
| 在 `action_specs` 中 | 已在 `tool_mgr` 中注册 |
| 通过 `allowed_tools` 过滤 | 由 behavior 的 `tools` 配置决定（见下） |

### 1.2 `tools` 配置（Behavior 层）

位置：`BehaviorConfig.tools` / `toolbox.tools`

| `tools.mode` | 行为 |
|--------------|------|
| **all** | 全放开，`tool_mgr` 中注册的都能用 |
| **allow_list** | 白名单，仅 `names` 中的 tools 可用 |
| **none** | 全部禁用，返回空列表 |

### 1.3 默认值（不配置时）

**`tools` 不配置时**，使用 `BehaviorToolsConfig::default()`：

- `mode: all`


即：**默认启用所有 tools**，

### 1.4 无 Behavior 配置时的兜底

当 `behavior_cfg_cache` 中**没有**当前 behavior 的配置时，`AgentPolicy::allowed_tools` 直接返回 `tool_mgr.list_tool_specs()` 的全部结果，即**全放开**。

---

## 2. 多层级影响（当前 vs 理想）

| 层级 | 当前是否参与 | 说明 |
|------|--------------|------|
| **Behavior** | ✅ | `cfg.tools.filter_tool_specs()` |
| **Toolbox / Skills** | ✅ | `default_load_actions` + 各 skill 的 `actions` |
| **Workspace** | ⚠️ 间接 | 仅影响 skill 加载路径，不参与 action 限制 |
| **Agent** | ❌ | `AIAgentConfig` 无 tool/action 相关配置 |

未来可扩展：Workspace 级、Agent 级的 allowed/deny 策略。

---

## 3. Toolbox 与 Skills

### 3.1 `requested_actions` 来源

```text
requested_actions = default_load_actions ∪ (各 loaded_skill 的 actions)
loaded_skills = effective_skills() ∪ session.loaded_skills
```

### 3.2 未解析的 actions

在 `requested_actions` 中但不在 `action_specs` 中的 action 会被记为 `unresolved_actions`，并打 warning：

```text
prompt.build_toolbox unresolved_actions_ignored=...
```

### 3.3 工程约束（提示词工程师必看）

- 优先使用 `skills` 和 `default_load_skills`
- 使用 `allow_tools`、`default_load_actions` 时需清楚与当前加载 skills 的关系，避免冲突
- 一个 Behavior 只给完成目标所需的最小工具集，不要“为了保险”扩大集合

---

## 4. 相关代码位置

| 模块 | 路径 |
|------|------|
| Behavior 配置 | `src/frame/opendan/src/behavior/config.rs` |
| `allowed_tools` | `src/frame/opendan/src/agent_tool.rs` |
| `build_toolbox` / `select_action_specs` | `src/frame/opendan/src/behavior/prompt.rs` |
| 行为执行 | `src/frame/opendan/src/behavior/behavior.rs` |

---

## 5. 参考文档

- `notepads/LLM_Behavior.md` - LLMBehavior 职责与模块结构
- `notepads/opendanv2.md` - 完整 OpenDAN 设计（含 toolbox 模式）
- `notepads/opendan关键类型.md` - 关键类型索引
