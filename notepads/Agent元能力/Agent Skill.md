# Agent Skill / Skills Mgr 设计

> 本文定义 OpenDAN / BuckyOS Agent Runtime 中新的 `Skills Mgr`。
>
> 直接输入：
>
> - [todo.md](todo.md) 中 `Skills` 章节提出的标准定义、使用、安装和 improve-skill 问题；
> - [skill in hermes agent.md](../refs/skill%20in%20hermes%20agent.md) 对 Hermes Agent skill crystallization 的研究；
> - [Agent元能力设计.md](Agent元能力设计.md) 与 [基于Agent元能力的Agent Runtime设计.md](基于Agent元能力的Agent%20Runtime设计.md) 中对 Skill 的定位；
> - [Agent Self-Improve.md](Agent%20Self-Improve.md) 中对 Skill / Shortcut Graph、安装编译和生命周期的初步设计。

---

## 0. 核心结论

一句话：

> Skill 是 Agent 与世界交互的捷径；Skills Mgr 是这些捷径的安装、召回、验证、排名、降权、归档和回滚系统。

Skill 不再被定义为一段可注入 prompt。它是一个带元数据、来源、依赖、风险、验证状态和生命周期的 **procedural memory artifact**。

新的 Skills Mgr 必须满足以下原则：

1. **Skill Source 不等于 Managed Skill**。用户安装、系统内置、Hub 下载或 Agent 自己总结出来的内容，都只是来源；必须经过编译、拆解、分类、命名、验证，才能成为 Skills Mgr 管理的 skill artifact。
2. **Self-Improve 不直接写 active skill**。已有 skill 改造只能提出维护 proposal；新 skill 提炼只能提出 candidate；正式生效必须经过验证和治理。
3. **Nothing to save 是健康结果**。没有明确可复用流程信号时，不生成 skill candidate；已有 skill 没有明确反馈信号时，不生成维护 proposal。
4. **已有 skill 默认 patch，新 skill 提炼默认 no-op**。已有 skill 的反馈闭环优先修补当前 skill、已有 umbrella skill 或 support file；新 skill 只有在没有同类覆盖且信号足够强时才创建 candidate。
5. **Skill 是 package，不是单个 markdown**。一个 skill 可能包含 `SKILL.md`、metadata、references、templates、scripts、assets、tests。
6. **Skill 使用采用 progressive disclosure**。Session 启动只知道 registry / hint；确认相关后再读取完整 skill 或子文件。
7. **Skill lifecycle 是 core，不是附加功能**。创建、验证、使用统计、排名、降权、归档、恢复、回滚都属于 Skills Mgr。
8. **Governance 必须管住 Skill**。Skill 会改变 Agent 未来行为，不能只靠 prompt 约束写权限和高风险动作。

---

## 1. 职责边界

### 1.1 Skills Mgr 做什么

Skills Mgr 负责：

| 能力 | 说明 |
|---|---|
| Skill Source 管理 | 保存系统内置、用户安装、Hub 安装、团队共享、Agent 结晶等来源 |
| Skill Compiler | 把来源文本或包拆解成 managed skill candidate |
| Skill Registry | 维护 skill index、group、scope、trigger、状态、排名和摘要 |
| Skill Recall | 根据当前 session 的 intent、objects、tags、tool availability 返回 skill hint |
| Skill Loading | 按需读取 `SKILL.md`、references、templates、scripts、assets |
| Skill Usage Tracking | 记录加载、使用、成功、失败、成本、用户反馈和产物质量 |
| Skill Verification | 对 candidate 和 active skill 做静态检查、模拟验证、真实使用验证和重测 |
| Skill Lifecycle | candidate、active、preferred、stale、archived、blocked、restored 等状态迁移 |
| Skill Curator | 合并重复 skill、归入 umbrella、降级 narrow skill、归档 stale skill |
| Skill Governance | 执行来源权限、写保护、风险分级、审批、审计和回滚 |

### 1.2 Skills Mgr 不做什么

Skills Mgr 不负责：

1. 不直接执行真实世界动作。执行仍由 Session Runtime、Object Runtime、Tool Runtime 完成。
2. 不安装工具二进制或第三方依赖。它只能声明 `requires_tools`，工具安装必须走 Tool / Package / Governance 流程。
3. 不替代 Agent Memory。Memory 保存推断和 hint，Skill 保存可复用 procedure。
4. 不替代 Agent Notebook。Notebook 保存用户明确声明的长期事实、偏好、项目状态；Skill 保存某类任务怎么做。
5. 不把 skill 全文注入每轮 prompt。默认只注入 registry/hint。
6. 不让后台 reviewer 直接修改 system/hub/team protected skills。
7. 不保证 skill 中描述的外部事实永远正确。Skill 必须能过期、重测和降权。
8. 不负责 Workflow DSL 中的 `/skill/<name>` 语义 executor 解析。Workflow 里的 `skill` 是“可执行能力语义链接”的旧术语，和本文的 Agent Skill 不是同一概念；后续应在 workflow 设计中重命名为 capability / executor semantic path 一类术语。

---

## 2. 在 Agent Runtime 中的位置

Skills Mgr 属于 `State & Recall Runtime`，但和 `Self-Improve Runtime`、`Governance Runtime` 强耦合：

```text
Session Runtime
  -> 请求 skill hint / skill content
  -> 加载并使用 skill
  -> 记录 usage report

State & Recall Runtime / Skills Mgr
  -> 保存 skill registry / package / usage history
  -> 提供 progressive disclosure 读取接口

Self-Improve Runtime
  -> 基于 report + session_history 改造已有 skill
  -> 基于 session_history 提炼新 skill candidate
  -> 提出 patch / create / block / archive 等 proposal
  -> 不直接写 active skill

Governance Runtime
  -> 决定 source 是否可信
  -> 决定 candidate 是否能 promote
  -> 限制 skill 的外部副作用和写权限
  -> 提供 audit / rollback / approval
```

Skill 与 Notebook、Memory 的边界：

| 模块 | 保存什么 | 是否可直接 apply |
|---|---|---|
| Notebook | 用户明确要求记录的长期事实、偏好、项目状态 | 否，只是事实来源 |
| Memory | Agent 从历史中推断出的 hint、对象关系、印象 | 否，只是召回线索 |
| Skill | 某类任务的可复用执行过程、坑、验证方法 | 是，但要受治理和权限限制 |

分类规则：

```text
声明事实      -> Notebook
推断印象      -> Memory
可复用过程    -> Skill
用户价值反馈  -> Value / Preference candidate
一次性历史    -> Session History
```

同一条事实不应同时写进 Notebook、Memory 和 Skill。Skill 可以引用事实来源，但不复制事实真相。

---

## 3. 核心概念

### 3.1 Skill Source

`SkillSource` 是进入系统的原始材料，不是可被直接召回和加载的 skill。

来源包括：

| source_type | 说明 |
|---|---|
| `system_builtin` | 系统内置，只读，随版本更新 |
| `hub_installed` | 从 Skill Hub 安装，有签名和版本 |
| `team_shared` | 组织或团队共享，受团队权限控制 |
| `owner_installed` | owner 手工安装 |
| `agent_discovered` | Agent 从探索或任务历史中发现 |
| `agent_curated` | Curator 从已有 skill 合并或整理得到 |
| `external_reference` | 官方文档、博客、同事建议、网页等 |

Source 可以是 markdown、目录包、压缩包、URL、git repo、对话片段或 session report。安装 source 只表示“可进入编译流程”，不表示可信或 active。

### 3.2 Managed Skill

`ManagedSkill` 是经过 Skills Mgr 编译、规范化并纳入生命周期管理的 skill artifact。

`ManagedSkill` 不等于 Agent Runtime，也不表示它一定会被当前 session 加载。真正能被普通 session 召回和加载的是其中处于 `active` / `preferred` 等允许加载状态的 loadable skill。

`loadable registry` 是 Skills Mgr 派生出的可召回索引，只包含允许普通 session 自动或显式读取的 `ManagedSkill`。

`ManagedSkill` 必须有：

- 唯一 id；
- 所属 group；
- 类型；
- 触发条件；
- procedure；
- 依赖；
- 风险等级；
- 权限范围；
- 来源；
- 生命周期状态；
- 验证状态；
- 使用统计；
- 回滚信息。

### 3.3 Skill Candidate

`SkillCandidate` 是尚未进入 loadable registry 的候选 skill。它可能来自：

- 用户安装的 Skill Source 编译；
- Self-Improve 从历史中结晶；
- Curator 合并多个旧 skill；
- active skill 的 patch proposal；
- Hub / team skill 的新版本。

Candidate 可以被查看、评审、验证、修改、丢弃、归档或提升，但默认不进入普通 Session 的自动召回。

### 3.4 Skill Package

Skill 是 package：

```text
skill-root/
  skill.yaml
  SKILL.md
  references/
  templates/
  scripts/
  assets/
  tests/
  CHANGELOG.md
```

最小可用包只需要 `skill.yaml` 和 `SKILL.md`。如果存在 support files，迁移、归档、合并和回滚必须保持 package integrity，不能只移动 `SKILL.md`。

### 3.5 Skill Group

Skill Group 表示一类任务或目标下的一组竞争 skill，例如：

```text
youtube_video_download
stock_kline_data_fetch
buckyos_system_service_dev_loop
investor_pitch_deck_writing
```

同一个 group 内可以有多个候选 skill。Skills Mgr 根据使用结果、风险、适用范围和用户反馈维护 rank。

### 3.6 Skill Hint

`SkillHint` 是注入 prompt 或返回给 session 的低成本指针：

```yaml
skill_id: skill://opendan/buckyos-system-service-dev-loop
group_id: buckyos_system_service_dev_loop
title: BuckyOS System Service Dev Loop
sentence: Build a BuckyOS system service by designing protocol, schema, implementation, integration, and DV tests in order.
score: 0.82
risk_level: medium
lifecycle_state: active
verification_status: usage_verified
```

Hint 只用于判断是否值得读取，不应包含完整 procedure。

### 3.7 Skill Usage

`SkillUsage` 是每次 session 加载或实际使用 skill 后产生的审计记录：

```yaml
usage_id: skill_usage_...
skill_id: skill://...
session_id: session_...
loaded_at: 2026-06-03T00:00:00Z
used_at: 2026-06-03T00:03:00Z
usage_mode: applied
task_result: success
user_feedback: accepted
failure_reason: null
cost:
  tokens: 3200
  wall_time_ms: 180000
output_refs:
  - object://...
```

Usage history 是 ranking、stale detection、verification 和 curator 的主要输入。

---

## 4. Skill 类型

todo.md 中提出四类 skill。新的 Skills Mgr 保留这四类，但对边界做硬化。

### 4.1 `data_acquisition` / `exploration`

信息获取 / 探索类 skill，用于更快、更可靠地观察世界。

这类 skill 不一定产生可交付产物。它可以只是帮助 Agent 更快找到某个入口、读到某类信息、确认某个本地事实、定位某个对象，或缩短探索路径。

例子：

- 下载 YouTube 视频；
- 获取股票 K 线 CSV；
- 查询国家公园开放状态；
- 从某平台 API 读取数据；
- 定位某类公开文件的稳定入口。
- 在本地代码库中定位某个功能入口；
- 查找用户电脑或 Agent workspace 中的某个具体信息。

典型字段：

```text
data_type
platform
entrypoint
parameters
format
freshness
source_trust
exploration_boundary
```

验证方式：

```text
能否拿到数据
数据是否完整
格式是否可解析
来源是否可信
是否比搜索或人工探索更便宜
是否没有产生不该有的产物或副作用
```

### 4.2 `delivery`

交付类 skill，用于把内部系统、组织流程或隐藏知识变成可发现、可复用的交付方法。

这类 skill 的特点是：不给 skill，Agent 很难从公开世界自然发现。

例子：

- BuckyOS 新系统服务开发流程；
- 内部上线、发版、评审流程；
- 某公司代码提交前必须跑哪些脚本；
- 某团队文档交付格式；
- 某产品内部 dashboard 的入口和字段含义。

典型字段：

```text
organization
project
role
required_permissions
required_artifacts
approval_steps
done_definition
```

验证方式：

```text
是否产出正确交付物
是否通过必要检查
是否符合组织规则
是否需要人工返工
```

### 4.3 `workflow`

流程类 skill，用于完成一类可重复任务。

例子：

- 修复某类 CI failure；
- 设计一个 kRPC 协议；
- 做一次代码 review；
- 为某类 app 生成测试计划；
- 完成一次线上问题排查。

workflow skill 可以是跨平台的，也可以绑定某个 object / repo / service。绑定越具体，过期风险越高。

### 4.4 `paradigm`

范式类 skill，用于改善 Agent 的思考、写作、判断和表达。

例子：

- 如何写好商业计划书；
- 如何组织投资人 deck；
- 如何做可信信息源判断；
- 如何写高信号代码审查意见。

这类 skill 最容易与用户偏好、价值判断和其它 principle 冲突。因此：

1. 不应轻易全局生效；
2. 必须声明适用上下文；
3. 必须记录冲突关系；
4. 多次正反馈后才可提升为 preferred；
5. 如果已经被模型能力自然覆盖，应降权或 block。

---

## 5. Skill Artifact 标准格式

### 5.1 `skill.yaml`

```yaml
schema_version: 1
id: skill://opendan/buckyos-system-service-dev-loop
name: buckyos-system-service-dev-loop
title: BuckyOS System Service Dev Loop
description: Develop a BuckyOS system service through protocol, durable schema, implementation, integration, and DV test stages.

type: delivery
group_id: buckyos_system_service_dev_loop
origin: owner_installed
owner_scope: agent

source_refs:
  - type: file
    uri: harness/SKILLS/implement-system-service/SKILL.md
source_event_ids: []
source_session_ids: []
object_refs:
  - object_id: object://repo/buckyos
    role: target_repo

trigger:
  when_to_use:
    - User asks to implement or integrate a BuckyOS system service.
    - Task requires BuckyOS service dev loop conventions.
  intent_tags:
    - buckyos
    - system-service
    - backend
  object_types:
    - repo
    - service
  negative_triggers:
    - Pure protocol design only.
    - UI-only work.

requires:
  tools:
    - terminal
    - git
  agent_tools: []
  permissions:
    - repo_write
  environment:
    - BuckyOS source checkout
  optional_tools: []

risk:
  risk_level: medium
  side_effects:
    - modifies_repo_files
    - runs_tests
  approval_policy: normal_repo_write
  external_impact: none

lifecycle:
  state: active
  verification_status: usage_verified
  version: 0.1.0
  created_at: 2026-06-03T00:00:00Z
  updated_at: 2026-06-03T00:00:00Z
  last_used_at: null
  last_verified_at: null
  expires_at: null
  pinned: false
  protected: false

ranking:
  rank: 1
  score: 0.75
  usage_count: 0
  success_count: 0
  failure_count: 0
  average_cost: null
  average_latency_ms: null

compat:
  min_agent_runtime: 0.1.0
  platforms:
    - darwin
    - linux
```

### 5.2 `SKILL.md`

`SKILL.md` 是给 LLM 读取和执行的主体。必须包含以下章节：

```markdown
# <Skill Title>

## When to Use

## Inputs

## Procedure

## Pitfalls

## Verification

## Rollback

## Report
```

其中：

| Section | 要求 |
|---|---|
| `When to Use` | 明确正触发和负触发，避免过度加载 |
| `Inputs` | 说明使用前需要从用户、对象或工具读取什么 |
| `Procedure` | 可执行步骤，优先复用既有工具和组件 |
| `Pitfalls` | 已知坑、错误路径、容易误判的信号 |
| `Verification` | 如何判断 skill 是否完成任务 |
| `Rollback` | 对有副作用流程说明如何撤销或止损 |
| `Report` | 使用后应如何向用户报告效果、风险和验证 |

### 5.3 最小字段

一个结晶出来的 skill 至少必须有：

- `trigger / when_to_use`
- `procedure`
- `dependencies / required tools`
- `pitfalls`
- `verification`
- `source_event_ids` 或其它 `source_refs`
- `risk_level`
- `owner_scope`
- `lifecycle_state`
- `verification_status`

缺少这些字段的内容只能作为 Skill Source 或 Draft Candidate，不能进入 loadable registry。

---

## 6. 生命周期状态机

### 6.1 Source 生命周期

```text
discovered / installed
  -> parsed
  -> compiled
  -> candidate_created
  -> source_archived
```

Source 不参与普通召回，除非用户显式查看来源。

### 6.2 Managed Skill 生命周期

```text
candidate
  -> verifying
  -> active
  -> preferred
  -> stale
  -> archived

candidate
  -> rejected

active / preferred
  -> needs_reverification
  -> active / stale / blocked

active / preferred / stale
  -> blocked

archived
  -> restored
```

状态含义：

| state | 说明 |
|---|---|
| `candidate` | 已编译但未验证，不进入普通自动召回 |
| `verifying` | 正在做静态检查、模拟或真实任务验证 |
| `active` | 可被普通 session 召回和使用 |
| `preferred` | 同组优先 skill，会更高排名返回 |
| `needs_reverification` | 依赖、外部平台、工具版本或失败记录触发重测 |
| `stale` | 长期未用、可能过期或低价值，默认低排名 |
| `archived` | 被归档，不默认召回，可恢复 |
| `rejected` | 候选被拒绝，保留审计 |
| `blocked` | 因安全、错误、污染或强冲突被禁止使用 |

### 6.3 Verification 状态

| verification_status | 说明 |
|---|---|
| `unverified` | 尚未验证 |
| `static_checked` | schema、依赖、权限、格式检查通过 |
| `simulated` | 在无外部副作用模式下跑过模拟验证 |
| `manual_verified` | 人类确认可用 |
| `usage_verified` | 真实 session 中多次成功 |
| `failed` | 验证失败 |
| `expired` | 依赖或外部环境过期 |
| `unsafe` | 发现安全风险，必须 blocked |

进入 loadable registry 的 skill 至少需要 `static_checked`。有外部副作用的 workflow / delivery skill，进入 preferred 前必须有 `manual_verified` 或足够的 `usage_verified`。

---

## 7. 安装流程：`llm_install_skill`

用户安装 Skill 不应直接把文本放进 prompt，也不应直接创建 active skill。

### 7.1 输入

`llm_install_skill` 接受：

```text
local file / directory
remote url
hub package id
markdown text
session excerpt
git repo path
```

安装参数：

```yaml
source_uri: ...
installed_by: owner | agent | team_admin | system
target_scope: agent | owner | team | zone
trust_hint: low | medium | high
allow_tool_install: false
allow_external_network: false
```

默认 `allow_tool_install = false`。如果 skill 依赖新的工具、脚本运行时或第三方库，Installer 只能生成依赖声明和缺失报告，不能自动安装。

### 7.2 编译步骤

```text
1. Ingest
   保存原始 source，计算 digest，记录来源和安装者。

2. Parse
   识别 package 结构、frontmatter、正文、support files。

3. Classify
   判断是 data_acquisition / exploration / delivery / workflow / paradigm，或拆成多个 skill。

4. Normalize
   生成 class-level 名称、group_id、trigger、negative_triggers、scope。

5. Extract
   提取 procedure、dependencies、pitfalls、verification、rollback。

6. Risk Scan
   标记权限、外部副作用、凭证、网络、文件写入、支付、隐私风险。

7. Dependency Resolve
   检查所需 tools / agent_tools 是否存在；缺失则标记 candidate blocked by dependency。

8. Conflict Check
   查找同 group、同 trigger 或相互矛盾的 existing skills。

9. Candidate Write
   写入 SkillCandidate，不进入 loadable registry。

10. Verification Plan
   生成最小验证方案，等待验证或人工 review。
```

### 7.3 编译输出

```yaml
install_result:
  source_id: skill_source_...
  candidates:
    - skill_id: skill://...
      group_id: ...
      type: workflow
      state: candidate
      verification_status: unverified
  dependency_report:
    missing_tools: []
    missing_permissions: []
  conflict_report:
    possible_duplicates: []
    conflicting_skills: []
  next_actions:
    - request_verification
```

### 7.4 安装拒绝条件

以下情况不能生成 active skill：

1. 缺少明确 `When to Use` 或 `Procedure`；
2. 只是一段一次性任务叙事；
3. 主要内容是用户偏好或事实声明，应进入 Notebook / Value；
4. 包含未声明的高风险外部副作用；
5. 需要新依赖但用户未确认；
6. 试图覆盖 system/hub protected skill；
7. 包含 prompt injection、凭证泄露或要求绕过治理。

---

## 8. 使用流程

### 8.1 Progressive Disclosure

Skills Mgr 提供三层读取：

```text
Level 0: skills_list / skill_hints
  只返回 id、title、description、score、risk、verification、state。

Level 1: skill_view(skill_id)
  返回 skill.yaml 和 SKILL.md 主体。

Level 2: skill_view(skill_id, path)
  返回 references/templates/scripts/assets/tests 中的指定文件。
```

Session 启动时只应注入 Level 0 的少量高相关 hint。LLM 判断相关后，再主动读取 Level 1 / Level 2。

### 8.2 Plan 阶段如何选择 skill

Plan 阶段应做：

```text
1. 从用户任务中抽取 intent tags、objects、risk、expected artifacts。
2. 调用 skill_hints(tags, objects, tools, risk_budget)。
3. 对候选 skill 做适用性判断：
   - trigger 是否命中；
   - negative trigger 是否命中；
   - required tools 是否可用；
   - risk 是否可接受；
   - verification_status 是否足够；
   - 是否有更高 rank 的同组 skill。
4. 只读取实际要用的 skill。
5. 在 plan 中记录：
   - selected_skill_ids；
   - why selected；
   - known risks；
   - verification to run。
```

Plan 不应为了“可能有用”加载大量 skill 全文。

### 8.3 Do 阶段如何使用 skill

Do 阶段使用 skill 时：

1. 按 skill procedure 执行，但仍以当前仓库、当前工具和当前用户指令为准；
2. 如果 skill 与用户显式指令冲突，用户指令优先；
3. 如果 skill 与当前治理决策冲突，governance 优先；
4. 如果 skill 明显过期，应停止套用并记录 failure signal；
5. 对高风险步骤必须重新走 approval / policy check；
6. 执行完成后写 `SkillUsage`。

### 8.4 Report 阶段如何反馈

使用过 skill 的 session report 至少记录：

```yaml
used_skills:
  - skill_id: skill://...
    usage_mode: applied | referenced | rejected_after_view
    result: success | partial | failed | not_applicable
    evidence:
      - test_passed
      - user_accepted
    issues:
      - missing_step
      - outdated_dependency
```

用户可见 report 不需要暴露内部全部统计，但应在有意义时说明：

- 使用了哪个关键 skill；
- 该 skill 是否帮助完成任务；
- 是否发现 skill 过期、遗漏或风险。

### 8.5 UI Session 是否使用 skill

UI Session 使用 skill 的关键，不是“UI Session 能不能执行 skill”，而是 **UI Session 如何避免把本来可以直接回答或旁路探索的问题错误升级成 Work Session**。

UI Session 在这里应被定义为一个“无产物 Agent”：

```text
UI Session:
  面向低延迟对话、意图理解、信息查询、轻量探索和路由。
  默认不创建长期 workspace，不承诺交付物，不沉淀完整任务过程。

Work Session:
  面向有明确目标、有产物、有执行过程、有状态恢复需求的任务。
```

因此，UI Session 可以消费 skill，但主要消费两类：

1. `data_acquisition / exploration` skill；
2. 低风险的对话、澄清、问诊、routing skill。

UI Session 不应直接消费 delivery / workflow skill 来完成有产物任务。它最多用这些 skill 帮助分类、估算风险、生成 Work Session 创建参数。

### 8.6 UI Session 的分类器策略

收到用户输入后，UI Session 应先做分类：

```text
User Input
  -> direct answer?
  -> lightweight info lookup / exploration?
  -> route to existing Work Session?
  -> create new Work Session?
```

分类依据：

| 类别 | 处理方式 |
|---|---|
| 普通对话 / 简单回答 | UI Session 直接回答 |
| 读取 Notebook / Memory hint 后即可回答 | UI Session 读取后回答 |
| 获取电脑信息、查找本地文件、定位具体信息 | UI Session 调用旁路 Explorer，必要时带候选 exploration skills |
| 需要修改文件、生成产物、长期执行、可恢复状态 | 创建 Work Session |
| 需要对已有 Work Session 补充信息 | forward 到目标 Work Session |
| 高风险外部副作用 | 请求确认或创建 Work Session 并走 Governance |

这里的目标是减少 Work Session 总数：不要把“查一下在哪”“帮我找一下”“这段代码入口在哪里”“本机有没有某个文件”这类无产物探索任务都升级成 Work Session。

### 8.7 LLM Explorer 与 Skills

当前系统已有旁路 LLM Explorer。它的定位是：

```text
UI Session
  -> 判断这是探索 / 信息获取任务
  -> 选择若干候选 exploration skills
  -> 调用 LLM Explorer
  -> Explorer 使用只读 / 低副作用工具完成探索
  -> 返回普通报告给 UI Session
  -> UI Session 直接答复用户，或决定是否创建 Work Session
```

Explorer 使用 skill 的方式应和 Work Session 不同：

1. Explorer 不接收所有 skill，只接收少量候选 `data_acquisition / exploration` skill；
2. Explorer 默认只读或低副作用，不写 repo、不创建交付文件、不提交、不安装依赖；
3. Skill 对 Explorer 的作用是缩短探索路径，例如告诉它优先看哪些入口、用什么查询模式、如何验证来源；
4. Explorer 的输出是 report，不是正式产物；
5. 如果 Explorer 发现任务实际需要修改、生成、安装、发布、长期跟踪，应把结果交回 UI Session，由 UI Session 创建 Work Session；
6. Explorer 使用 skill 的过程也应写轻量 usage record，用于后续 ranking 和 curator。

也就是说，UI Session 自身不应该变成一个长任务执行器；它可以把无产物探索外包给 Explorer，并让 Skills Mgr 给 Explorer 提供候选捷径。

### 8.8 UI Session 不应自动使用的 skill

不适合 UI Session 或 Explorer 自动使用的 skill：

- 会写文件、发请求、支付、发消息、修改系统状态的 skill；
- 需要长时间执行和 checkpoint 的 workflow；
- 需要生成、保存、提交或交付正式产物的 skill；
- 依赖未授权 terminal 写操作、browser automation 或外部凭证的 skill；
- delivery skill 和高风险 workflow skill。

如果 UI Session 判断这些 skill 才能完成任务，应创建 Work Session 或请求用户显式确认。

---

## 9. Skill 改进与提炼流程

Self-Improve 对 Skill 的处理必须拆成两个一等流程：

| 流程 | 核心输入 | 目标 | 默认结果 |
|---|---|---|---|
| `improve_existing_skill` | `report + session_history + usage refs + target_skill_id` | 修补、降权、重测或禁用已有 skill | `patch` / `rank_adjust` / `mark_needs_reverification` / `block` / `archive` / `no_op` |
| `extract_new_skill` | `session_history` | 判断是否从历史中提炼新的 skill candidate | `no_op` / `create_candidate` / `duplicate_existing_skill` / `demote_to_reference` / `blocked_by_blocklist` |

这两个流程不能混成一个强写接口：

1. `improve_existing_skill` 不创建新 skill。它只处理已存在 skill 的使用后反馈闭环。
2. `extract_new_skill` 不 patch active skill。它只负责从历史中发现可复用 procedure，并生成 candidate。
3. 如果一个流程发现任务属于另一个流程，应返回 route result，而不是跨边界直接写入。
4. 两个流程都必须先检查 blocklist，再决定是否继续。

### 9.1 `improve_existing_skill`

`improve_existing_skill` 用于已有 skill 被加载、查看或实际使用后的改造。它的输入必须能定位到目标 skill：

```yaml
target_skill_id: skill://...
usage_ids:
  - skill_usage_...
session_id: session_...
report:
  result: success | partial | failed | not_applicable
  user_feedback: accepted | rejected | corrected | null
  issues:
    - missing_step
    - outdated_dependency
session_history_ref:
  from_round: 12
  to_round: 31
```

允许输出：

| proposal_type | 说明 |
|---|---|
| `patch` | 修补 `SKILL.md`、support file、trigger、pitfall、verification 或 rollback |
| `rank_adjust` | 根据 usage result 调整同组排名 |
| `mark_needs_reverification` | 标记需要重测 |
| `block` | 因 unsafe、wrong、governance conflict 等原因禁用 |
| `archive` | 提议归档低价值或被替代的 skill |
| `no_op` | 有使用记录，但没有足够信号需要修改 |
| `route_to_extract_new_skill` | 失败或反馈属于另一个 class of work，应转给新 skill 提炼流程 |

`improve_existing_skill` 的默认策略是：

```text
默认 patch，不默认 create。
默认修补当前 loaded/viewed skill。
如果当前 skill 不合适，先查同 group / umbrella skill。
如果应该是新 class，只返回 route_to_extract_new_skill。
```

### 9.2 `extract_new_skill`

`extract_new_skill` 用于从 session history 中提炼新的 skill candidate。它不需要已有 `target_skill_id`，但必须先证明历史中存在明确 skill signal：

1. 用户纠正了某类任务的 workflow、步骤顺序、检查方法；
2. 用户表达不满，且不满可归因到某类任务流程；
3. 本轮发现非平凡 technique、fix、workaround、debugging path、tool pattern；
4. 同类任务多次重复，且成功路径稳定；
5. 用户显式要求“保存为 skill”或“以后这类事按这个流程做”；
6. 某个高 attention object 周围出现可复用捷径。

没有这些信号时，输出：

```text
No skill candidate.
```

这不是失败，是防止 skill 噪声的健康路径。

### 9.3 不应提炼成新 skill 的内容

以下内容不得生成 skill：

1. 一次性任务叙事；
2. 临时环境失败，如缺 binary、fresh install、credential 未配置；
3. 对工具能力的负面断言，如“某工具不能用”，除非已经验证为长期机制限制；
4. 已经通过 retry 恢复的 transient error，最多记录 retry pattern；
5. 用户偏好或事实声明，除非明确绑定某类任务 procedure；
6. 外部网站、政策、API 的短期状态；
7. 没有验证路径的泛化结论；
8. 与 system / owner / governance 明确冲突的做法。

### 9.4 新 skill 提炼优先级

`extract_new_skill` 必须遵守：

```text
1. 先查找已有同 group / 同 trigger / umbrella skill。
2. 如果已有 skill 覆盖该流程，返回 duplicate_existing_skill 或 route_to_improve_existing_skill。
3. 如果知识太细，返回 demote_to_reference，不生成 narrow skill。
4. 只有没有任何已有 skill 覆盖该 class，才创建新的 class-level candidate。
```

压缩成规则：

```text
默认 no_op，不默认 create。
默认 generalize，不默认 snapshot。
默认归入 umbrella，不默认生成 one-session skill。
```

### 9.5 Proposal 生成

两个流程都输出 proposal，但 proposal type 的合法集合不同。

`improve_existing_skill` proposal 示例：

```yaml
flow: improve_existing_skill
proposal_type: patch
reason: User corrected the BuckyOS service integration sequence.
target_skill_id: skill://...
candidate_skill_id: skill_candidate_...
usage_ids:
  - skill_usage_...
source_session_ids:
  - session_...
source_event_ids:
  - event_...
confidence: 0.72
risk_level: medium
requires_review: true
```

`extract_new_skill` proposal 示例：

```yaml
flow: extract_new_skill
proposal_type: create_candidate
reason: Session discovered a repeatable workflow for diagnosing a CI failure.
candidate_skill_id: skill_candidate_...
group_id: buckyos_ci_failure_fix
source_session_ids:
  - session_...
source_event_ids:
  - event_...
confidence: 0.64
risk_level: medium
requires_review: true
```

Self-Improve 只能写 candidate store 和 proposal log。它不能直接：

- 修改 active skill；
- 删除 skill；
- 安装 tool；
- 调用 terminal 修改 skill store；
- 对外发消息；
- 触发有外部副作用的验证。

### 9.6 Blocklist

Blocklist 是两个流程的核心输入，而不是 Curator 的附属功能。至少需要区分两类：

| blocklist | 作用 |
|---|---|
| `skill_blocklist` | 禁用已有 skill、skill group 或 trigger pattern |
| `extraction_blocklist` | 禁止把某类历史、错误、偏好或短期状态提炼成 skill |

典型 `BlockEntry`：

```yaml
block_id: block_...
target_type: skill | group | trigger_pattern | extraction_pattern
target_id: skill://... | group_id | pattern
scope: agent | owner | team | zone
reason_code: unsafe | outdated | duplicate | transient | low_value | user_disabled | governance_conflict
reason: Human readable reason.
source_refs:
  - session_...
created_by: owner | governance | curator | self_improve
created_at: 2026-06-03T00:00:00Z
expires_at: null
```

`improve_existing_skill` 命中 `skill_blocklist` 时，应返回 `blocked` 或只允许生成 unblock / reverify proposal。`extract_new_skill` 命中 `extraction_blocklist` 时，应返回 `blocked_by_blocklist`，不能生成 candidate。

---

## 10. Curator：整理、合并和遗忘

只要允许自动生成 skill，就必须有 curator。否则 skill 库会变成重复、过时、互相冲突的 prompt 噪声。

### 10.1 Curator 输入

Curator 读取：

- active skills；
- candidates；
- archived skills metadata；
- usage history；
- failure reports；
- conflict notes；
- source trust；
- pin / protected 标记；
- group ranking。

### 10.2 Curator 动作

Curator 可以提出：

| action | 说明 |
|---|---|
| `merge` | 把多个 narrow skill 合并进 umbrella skill |
| `demote_to_reference` | 把一次性细节降级成 support file |
| `patch` | 修补遗漏步骤、pitfall、verification |
| `rank_adjust` | 调整同组排名 |
| `mark_stale` | 标记可能过期 |
| `archive` | 归档低价值或被合并 skill |
| `block` | 对 unsafe / harmful skill 提出禁用 |
| `restore` | 从 archived 恢复 |

Curator 默认不 delete。最大破坏性动作是 archive，且必须可恢复。

### 10.3 Curator 硬规则

1. 不碰 `protected = true` 的 skill；
2. 不修改 system builtin / hub installed skill 的原始包，只能提出 overlay patch 或 upgrade request；
3. 不因为 usage count 低就简单归档；
4. 不因为 trigger 不同就拒绝合并，要判断人类维护者会写成多个 skill 还是一个 skill 的多个小节；
5. 合并 package 时必须迁移 references/templates/scripts/assets/tests；
6. 每次真实 curator pass 前生成 backup；
7. 支持 dry-run，输出 would-take actions；
8. 归档后保留 redirect，让旧 usage history 能解析到新 umbrella skill。

### 10.4 Umbrella 判断

Curator 判断是否合并时，核心问题是：

```text
这几条 skill 是否都在回答同一个 class of work？
如果由人维护，会写成 N 个 skill，还是一个 skill 的 N 个小节？
```

如果答案是后者，应合并成 umbrella skill，并把细节放入：

```text
SKILL.md labeled subsection
references/
templates/
scripts/
tests/
```

---

## 11. 权限与治理

### 11.1 Skill 来源权限

| origin | 默认写权限 | 说明 |
|---|---|---|
| `system_builtin` | 只读 | 随系统版本更新 |
| `hub_installed` | 只读原包 | 更新走签名和版本；本地可有 overlay |
| `team_shared` | 团队权限 | owner / team admin 控制 |
| `owner_installed` | owner 可改 | agent 可提 candidate |
| `agent_discovered` | agent 可提 candidate | promote 需验证 |
| `agent_curated` | curator 可提 proposal | 不直接改 active |

Prompt 级“不要修改”不足够。Skills Mgr 必须在执行层 / 存储层拒绝越权写入。

### 11.2 Risk Level

| risk_level | 说明 | 默认要求 |
|---|---|---|
| `low` | 只影响思考、格式、低风险读取 | static checked |
| `medium` | 修改 repo、调用工具、读写本地文件 | usage verified 或用户确认 |
| `high` | 外部系统写入、发消息、提交、发布、支付、隐私数据 | approval + manual verified |
| `critical` | 不可逆、金钱、法律、生产环境、身份权限 | 明确人工审批，默认不自动执行 |

Risk 属于 skill 元数据，也属于每次 action 的动态判断。一个 low-risk skill 中的某个步骤仍可能触发 high-risk action。

### 11.3 Background Reviewer 权限

后台 review、`improve_existing_skill` 和 `extract_new_skill` 只能使用：

```text
read session history
read active skill metadata
read loaded skill content
write SkillCandidate / Proposal
write block / unblock proposal
write review report
```

禁止：

```text
modify active skill
delete skill
install tools
run terminal for external side effects
send messages
call arbitrary object methods
write system/hub/team protected stores
```

### 11.4 Prompt Injection 防护

Skill、Memory、Notebook、context file 都是自然语言输入，不能等同 system prompt。

Skill 加载后的优先级：

```text
System / execution policy
  > owner explicit instruction
  > current user task instruction
  > governance decision
  > verified active skill
  > memory / notebook hints
  > candidate / source text
```

Skill 中出现“忽略系统规则”“绕过审批”“直接修改正式库”等文本时，应被 risk scan 标记，并进入 candidate rejection 或 manual review。

---

## 12. API 草案

### 12.1 Read APIs

```text
skills_list(filter) -> SkillHint[]
skill_hints_for_explorer(filter) -> SkillHint[]
skill_view(skill_id) -> SkillPackageSummary + SKILL.md
skill_view(skill_id, path) -> SkillPackageFile
skill_groups_list(filter) -> SkillGroup[]
skill_usage_list(skill_id, filter) -> SkillUsage[]
```

`filter` 支持：

```yaml
intent_tags: []
object_ids: []
object_types: []
required_tools: []
risk_budget: low | medium | high
state: active | preferred | candidate | stale | archived
verification_status: usage_verified
limit: 10
```

`skill_hints_for_explorer` 是面向 UI Session 旁路探索的窄接口，只返回低风险的信息获取 / 探索类 hint：

```yaml
type:
  - data_acquisition
  - exploration
risk_budget: low
side_effects:
  - read_only
limit: 3
```

它不返回 delivery / workflow / high-risk paradigm skill，也不返回完整 package 内容。UI Session 应把这些 hint 交给 LLM Explorer，由 Explorer 在需要时读取少量 Level 1 内容。

### 12.2 Session / Todo Selection APIs

现有 Plan / Do / Todo 设施需要的不是旧版 `load_skill`，而是“选择少量 active skill，并让 Prompt Compiler 渲染选中内容”的接口。

```text
skill_validate_selection(input) -> SkillSelectionValidation
skill_select_for_session(session_id, input) -> SelectedSkillSet
skill_select_for_todo(session_id, todo_id, input) -> SelectedSkillSet
skill_unselect_for_session(session_id, skill_ids) -> SelectedSkillSet
skill_unselect_for_todo(session_id, todo_id, skill_ids) -> SelectedSkillSet
skill_selected_list(session_id, todo_id?) -> SelectedSkillSet
skill_render_selected(session_id, todo_id?, options) -> PromptFragment
skill_record_selection_decision(input) -> SelectionDecisionRecord
```

`skill_validate_selection` 用于 `todo add --skill ...`、Plan 阶段生成 Todo、或用户显式指定 skill 时做同步校验：

```yaml
scope:
  agent_id: ...
  session_id: ...
  todo_id: null
requested:
  - skill://opendan/buckyos-system-service-dev-loop
  - buckyos-system-service-dev-loop
max_count: 3
allowed_states:
  - active
  - preferred
risk_budget: medium
```

返回：

```yaml
ok: true
resolved:
  - requested: buckyos-system-service-dev-loop
    skill_id: skill://opendan/buckyos-system-service-dev-loop
    title: BuckyOS System Service Dev Loop
    state: active
    verification_status: usage_verified
rejected:
  - requested: unknown-skill
    reason: not_found | not_loadable | blocked | too_many | risk_exceeded | permission_denied
```

`skill_select_for_todo` 是当前 Todo 模型的正式后端。breaking change 后，Todo 中的 `skills` 字段应保存规范化后的 `skill_id`，不是任意短名；CLI / UI 可以继续接受短名，但必须先经 `skill_validate_selection` 解析。

`skill_render_selected` 是 Prompt Compiler 使用的接口：

```yaml
level: 1
max_skills: 3
token_budget: 12000
include_metadata: true
include_usage_instructions: true
```

它只渲染当前 session / todo 已选择且仍处于 loadable 状态的 skill。渲染时必须自动写入 `skill_record_loaded` 或返回可写入的 `usage_handle`，让 Do 阶段后续可以调用 `skill_record_used` / `skill_record_result`。

Plan / Do 的建议接入方式：

```text
Plan:
  skills_list / skill_hints
  -> skill_validate_selection
  -> todo add 保存规范化 skill_id

Do:
  current_todo.skills
  -> skill_render_selected
  -> Prompt Compiler 注入少量 Level 1 内容
  -> 完成后 skill_record_result
```

### 12.3 Install / Candidate APIs

```text
skill_source_install(input, options) -> InstallResult
skill_source_install_from_pkg(pkg_id, media_root, options) -> InstallResult
skill_compile(source_id, options) -> SkillCandidate[]
skill_propose_existing_improvement(input) -> SkillProposal
skill_extract_new_candidate(input) -> SkillProposal
skill_candidate_view(candidate_id) -> CandidateDetail
skill_candidate_patch(candidate_id, patch) -> CandidateDetail
skill_candidate_reject(candidate_id, reason) -> RejectResult
```

`skill_propose_existing_improvement` 只接受已有 skill 的使用反馈：

```yaml
target_skill_id: skill://...
usage_ids: []
session_id: session_...
report: {}
session_history_ref: {}
```

它不能返回 `create_candidate`。如果发现需要新 skill，应返回 `route_to_extract_new_skill`。

`skill_extract_new_candidate` 只接受历史提炼输入：

```yaml
session_id: session_...
session_history_ref: {}
source_object_ids: []
```

它不能 patch active skill。如果已有 skill 覆盖，应返回 `duplicate_existing_skill` 或 `route_to_improve_existing_skill`。

`skill_source_install_from_pkg` 用于接入 `pkg_list.agent_skills`、Hub package 或系统内置 package。Package Mgr 负责签名、版本和解压；Skills Mgr 负责把本地 media root 作为 `SkillSource` ingest / compile，不直接信任为 active skill。

### 12.4 Verification / Promotion APIs

```text
skill_request_eval(skill_or_candidate_id, eval_plan) -> EvalRun
skill_record_eval_result(eval_run_id, result) -> EvalResult
skill_promote(candidate_id, options) -> ManagedSkill
skill_mark_needs_reverification(skill_id, reason) -> ManagedSkill
```

### 12.5 Usage APIs

```text
skill_record_loaded(skill_id, session_id, context) -> UsageHandle
skill_record_used(usage_id, evidence) -> SkillUsage
skill_record_result(usage_id, result) -> SkillUsage
```

### 12.6 Curator APIs

```text
skill_curate(dry_run: true, filter) -> CuratorPlan
skill_curate_apply(plan_id, approval) -> CuratorResult
skill_archive(skill_id, reason) -> ArchiveResult
skill_restore(skill_id, reason) -> RestoreResult
skill_rollback(skill_id, version_or_backup_id) -> RollbackResult
```

### 12.7 Governance APIs

```text
skill_check_permission(actor, action, skill_id) -> Decision
skill_check_risk(skill_id, action_context) -> RiskDecision
skill_blocklist_check(input) -> BlockDecision
skill_blocklist_list(filter) -> BlockEntry[]
skill_block(input) -> BlockResult
skill_unblock(block_id, reason) -> UnblockResult
skill_audit_log(filter) -> AuditEvent[]
```

`skill_blocklist_check` 必须同时支持：

```yaml
target_type: skill | group | trigger_pattern | extraction_pattern
flow: improve_existing_skill | extract_new_skill | usage | promote | curate
scope: agent | owner | team | zone
```

---

## 13. 存储模型

逻辑目录建议：

```text
agent_root/
  skills/
    sources/
      <source_id>/
        source.yaml
        raw/

    candidates/
      <candidate_id>/
        skill.yaml
        SKILL.md
        references/
        templates/
        scripts/
        assets/
        tests/

    active/
      <skill_id>/
        skill.yaml
        SKILL.md
        references/
        templates/
        scripts/
        assets/
        tests/

    archived/
      <skill_id>/

    indexes/
      registry.json
      groups.json
      triggers.json

    usage/
      usage.jsonl
      eval_runs.jsonl
      curator_runs.jsonl
      proposals.jsonl
      blocklist.jsonl
      audit.jsonl

    backups/
```

实现层不一定要严格使用文件系统目录；也可以接入统一 Item Store / RDB。无论底层如何，必须保留：

1. 原始 source；
2. active package；
3. candidate package；
4. usage log；
5. eval log；
6. curator proposal；
7. improve / extraction proposal；
8. blocklist；
9. audit log；
10. backup / rollback point。

`registry`、`groups`、`triggers` 可以是派生索引，可删除重建；`skill.yaml`、`SKILL.md`、usage/eval/proposal/blocklist/audit log 是语义真相源。

这是一次 breaking change。`<agent_root>/skills/<category>/<skill_dir>` 不再作为运行时直接加载目录保留；新的 `<agent_root>/skills/` 整体归 Skills Mgr 管理，并采用上面的 `sources / candidates / active / archived / indexes / usage / backups` 布局。

旧目录或旧 package 中的 `meta.json + skill.md` 只能在升级 / 安装阶段作为 `SkillSource` 导入。导入完成后，普通 Session、Todo、Prompt Compiler、Self-Improve 和 Curator 都只能通过 Skills Mgr API 访问 managed skill，不能再直接扫描旧目录。

### 13.1 工程实现约束

文件系统实现也必须满足以下约束：

1. 所有写操作必须原子化：先写临时文件，再 rename / replace。
2. 修改 `skill.yaml`、`SKILL.md`、support files、index 或 log 时必须写 audit event。
3. Candidate / active skill 的修改 API 必须带 `actor`、`request_id` 和 `expected_version` 或 `base_digest`，避免 Self-Improve、Curator、用户手工操作互相覆盖。
4. `indexes/` 是派生缓存，崩溃或损坏后必须能从 package 与 log 重建。
5. `usage/*.jsonl`、`proposals.jsonl`、`audit.jsonl` 只能 append，不允许原地改写；需要 compact 时必须生成 backup。
6. promote、archive、restore、rollback 前必须生成 backup 或可追溯的 previous version。
7. 并发执行时，同一 skill package 的写操作必须串行化；不同 skill 可并行。
8. 写失败不得留下 half-written package。启动时如发现临时文件或不完整 package，应进入 repair / quarantine 流程。

---

## 14. 评分和排名

Skills Mgr 需要给同组 skill 排名，但评分不能只看成功次数。

### 14.1 通用指标

```text
success_count
failure_count
last_used_at
last_verified_at
user_feedback_score
average_cost
average_latency
average_retry_count
risk_penalty
conflict_penalty
staleness_penalty
source_trust
verification_strength
```

### 14.2 类型差异

| type | 主要指标 |
|---|---|
| `data_acquisition` | 数据拿到率、完整性、可信来源、解析稳定性、freshness |
| `delivery` | 交付物正确性、检查通过率、返工率、权限问题、组织规则符合度 |
| `workflow` | 成功率、耗时、步骤遗漏率、失败恢复能力 |
| `paradigm` | 用户反馈、产物修改次数、复用率、冲突率、上下文适配度 |

### 14.3 降权触发

以下情况应降权或标记重测：

1. 最近失败率上升；
2. required tool 版本变化；
3. 外部平台、API、页面或组织流程变化；
4. 用户多次拒绝使用结果；
5. 与更高优先级 skill / principle 冲突；
6. 长期未使用且非 pinned；
7. verification 过期。

### 14.4 Block 触发

以下情况应 block：

1. 造成错误外部副作用；
2. 引导绕过审批或权限；
3. 泄露凭证或隐私；
4. 持续输出错误 procedure；
5. 被 owner 或治理策略显式禁用；
6. 被证明是 prompt injection 或恶意 package。

---

## 15. 与 Agent Tool 的关系

安装 Skill 和安装 Agent Tool 都是扩展 Agent 能力的主要方式，但二者不同：

| 项 | Skill | Agent Tool |
|---|---|---|
| 本质 | procedure / procedural memory | 可调用能力 / executable interface |
| 内容 | 何时用、怎么做、坑、验证 | API schema、权限、实现、side effect |
| 安装结果 | candidate 或 active skill | tool registration |
| 是否可执行 | 不直接执行 | 可被调用 |
| 依赖关系 | 声明 requires_tools | 可被 skill 依赖 |
| 风险 | 改变未来行为 | 改变世界状态 |

Skill 可以声明依赖某个 tool，但不能把 tool 安装藏在 skill install 里。引入新的 tool、第三方依赖或通用组件时，必须走单独确认和治理流程。

---

## 16. 与现有 `session_skills` 机制的迁移

旧版 Agent Skill 机制的核心是：

```text
__OPENDAN_VAR(session_skill_list, ...)
__OPENDAN_CONTENT(session_skills)
load_skill <skillname> <behavior|session>
unload_skill <skillname>
session.load_skills
behavior.load_skills
$agent_root/skills/$skill_name/meta.json
$agent_root/skills/$skill_name/skill.md
```

这个机制把 skill 视为 prompt 注入片段，适合早期手工加载，但缺少 source、risk、verification、usage、curator 和 governance。

本版本是 breaking change，不保留旧运行时语义。旧格式只作为升级 / 安装阶段的输入格式，进入系统后必须转换成 managed skill package。

| 旧机制 | 新机制 |
|---|---|
| `meta.json` | `skill.yaml` |
| `skill.md` | `SKILL.md` |
| bash 查找目录 | `skills_list(filter)` / registry index |
| `load_skill` | 移除；由 `skill_select_for_session` / `skill_select_for_todo` + `skill_render_selected` 替代 |
| `unload_skill` | 移除；由 `skill_unselect_for_session` / `skill_unselect_for_todo` 替代 |
| `session.load_skills` | 移除；由 session selected skill set 替代 |
| `behavior.load_skills` | 移除；由 behavior-scoped recommended / pinned skill hints 替代 |
| `__OPENDAN_CONTENT(session_skills)` | Prompt Compiler 按 selected skills 渲染 Level 1 内容 |
| `__OPENDAN_VAR(session_skill_list)` | `skills_list` 的 ranked Level 0 hints |

迁移规则：

1. 旧目录中的 `meta.json + skill.md` 可作为 `SkillSource` 导入；
2. Installer 把旧 metadata 规范化为 `skill.yaml`，把 `skill.md` 改名为 `SKILL.md`；
3. 导入完成后旧目录不再作为 loadable skill store 使用；
4. 旧 `load_skill`、`unload_skill`、`session.load_skills`、`behavior.load_skills` 不提供兼容 shim；
5. Todo / Session 中保存的 skill 引用必须是规范化后的 `skill_id`；
6. 如果某个 behavior 需要强制 skill，应声明 `pinned = true`、`protected = true` 和明确 scope；
7. Prompt Compiler 不再拼接全部可用 skill，只渲染已选择的少量 active skill；
8. 旧 skill 没有 verification 字段时，导入后状态为 `candidate` 或 `active + needs_reverification`，不能直接 preferred。

这次迁移的目标不是兼容旧行为，而是把旧的 prompt 片段模型一次性收敛到带生命周期的 artifact 模型。

---

## 17. 最小落地阶段

### Phase 1：Skill Source 与 Managed Skill 分离

目标：

- 支持安装 Skill Source；
- 支持 LLM Compiler 生成 candidate；
- 支持 data_acquisition / exploration / delivery / workflow / paradigm 分类；
- 支持 `skill.yaml` + `SKILL.md` 最小包；
- 支持 registry 和 basic skill_hints。

不做：

- 自动 promote；
- 自动安装工具；
- curator 自动应用。

### Phase 2：Session 使用记录

目标：

- Work Session 记录 loaded / used / rejected skill；
- UI Session / LLM Explorer 记录轻量 exploration skill usage；
- Session report 写入 usage log；
- 按 group 展示 basic rank；
- 失败时可标记 missing step / outdated / not applicable。

### Phase 3：Verification 与 Promote

目标：

- candidate 静态检查；
- 人工或测试式 verification；
- promote candidate 到 active；
- active skill 重测；
- block / archive / restore。

### Phase 4：已有 Skill 改造

目标：

- 基于 `report + session_history + usage refs` 生成已有 skill 的维护 proposal；
- 支持 patch / rank_adjust / mark_needs_reverification / block / archive / no_op；
- 支持 route_to_extract_new_skill；
- 引入 `skill_blocklist`；
- 不直接写 active。

### Phase 5：新 Skill 提炼

目标：

- 从 session history 生成 create_candidate proposal；
- 支持 `No skill candidate`；
- 强制 no-op first / duplicate check / umbrella-first；
- 支持 duplicate_existing_skill / demote_to_reference / blocked_by_blocklist；
- 引入 `extraction_blocklist`；
- 不 patch active。

### Phase 6：Curator

目标：

- dry-run curator；
- cluster / merge proposal；
- archive stale skill；
- support package integrity；
- backup / rollback。

---

## 18. 已决策与保留问题

### 18.1 已决策

| 问题 | 决策 |
|---|---|
| Skill Package 存储位置 | 先使用 Agent Root 下的文件目录实现，不进入统一 Item Store。 |
| `skill.yaml` schema | 先使用 JSON Schema，不单独定义 Rust / TypeScript schema。 |
| Hub skill 的签名、版本和升级 | 由安装器 / Package Mgr 作为前置流程处理。Skills Mgr 假设本地拿到的是已经验证并解压好的 skill package。 |
| 是否允许 agent-specific overlay patch hub skill | 允许。进入本地 Skills Mgr 后，所有来源的 skill 在使用层面平等，最终质量由使用效果、验证和 ranking 决定。 |
| Candidate promote 是否必须人工确认 | 低风险 candidate 尽量自动 promote；人工确认只用于高风险、低置信或治理策略要求的场景。 |
| Team / Zone scope skill 权限继承和撤销 | Skill 是方法，不直接绑定权限。权限应在使用时由 Governance / Tool Runtime 实时判断，不能固化在 skill package 中。 |
| Skill 中的 scripts 是否允许执行 | 允许，但必须走当前 session 的 sandbox / tool execution policy，不给 skill 自己开独立绕行通道。 |
| Skills Mgr 是否暴露为 kRPC system service | 第一阶段先作为 Agent Runtime 内部模块实现，不对外暴露 kRPC system service。 |
| Workflow DSL 中 `/skill/<name>` 与 Agent Skill 的关系 | 二者不是同一概念。Workflow 侧应修改术语；Skills Mgr 不提供 workflow semantic executor registry。 |
| Todo / Session selected skills | 属于 Skills Mgr 职责。现有 Todo / Plan / Do 设施通过 selection API 校验、保存和渲染 selected skill。 |
| 旧 `<agent_root>/skills/<category>/<skill_dir>` 布局 | 不保留运行时兼容。升级 / 安装时作为 `SkillSource` 导入，然后统一使用新的 managed store。 |
| 并发、版本和原子写入 | 属于 Skills Mgr 的工程实现硬要求；所有写 API 必须有 actor、request_id、base version / digest 和 audit。 |

### 18.2 保留问题

| 问题 | 当前处理 |
|---|---|
| Paradigm skill 与 Value / Principle 的转换边界 | 暂时保留到下一阶段。这个问题偏价值系统和自我模型，不阻塞 Skills Mgr 第一阶段。 |
| Skill eval 是否复用 DV Test / workflow test | 暂不处理。Skill 如何测试本身是独立大问题，第一阶段只保留 verification_status 和 usage history，不设计完整 eval framework。 |

---

## 19. 参考原则摘录

Hermes 的经验可以压缩成两句话：

```text
Hermes 证明了 skill crystallization 是必要的；
Hermes 的问题证明了 crystallizer 必须受治理。
```

对 OpenDAN 来说，最终设计不应照搬 `skill_manage(create/edit/delete)` 这种强写接口，而应采用：

```text
skill_source_install
skill_compile
skill_propose_existing_improvement
skill_extract_new_candidate
skill_request_eval
skill_promote
skill_record_usage
skill_blocklist_check
skill_block
skill_unblock
skill_curate
skill_archive
skill_restore
skill_rollback
```

也就是说：

```text
Session 使用 skill 并记录 usage；
Self-Improve 对已有 skill 只提出维护 proposal；
Skill Extraction 从 history 中提炼新的 candidate；
Verification 决定 candidate 能不能 promote；
Blocklist 阻止已知坏 skill 或低价值提炼；
Curator 负责合并和遗忘；
Governance 管住权限、风险、block 和回滚。
```
