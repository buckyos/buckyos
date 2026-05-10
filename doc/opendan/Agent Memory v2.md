# Agent Memory Module — 需求设计文档 v2.8

> **CLI 优先**:核心接口以单一可执行 `agent-memory` 暴露
>
> ```
> # 两参形态(短 content,推荐用于单行/无特殊字符)
> agent-memory [--root <memory_root>] set    <key> <content> --reason <reason>
>
> # 单参 + stdin 形态(长/多行 content)
> agent-memory [--root <memory_root>] set    <key> --reason <reason>           # content 经 stdin
>
> agent-memory [--root <memory_root>] remove <key> [--reason <reason>]
> agent-memory [--root <memory_root>] load   <tag1,tag2,tag3> [--max-bytes N] [--max-records N]
> agent-memory [--root <memory_root>] list   [/dir1/dir2]
> ```
>
> 上层(Claude Code skills、其它 agent 框架)以 shell 命令形式调用;**接口不接收 JSON 入参**。`--root <memory_root>` 总是可选覆盖;不传时 CLI 只按默认本地 memory root / `AGENT_MEMORY_ROOT` 定位目录。

---



## 1. 概述

### 1.1 文档目的

定义 Agent Memory 模块的功能需求、数据模型、接口设计、存储布局与生命周期管理策略。Memory 模块作为 Agent 的基础设施,为跨会话持久化记忆提供统一的能力,支撑日程管理、用户偏好学习、任务跟踪、知识沉淀(KB)等上层场景。

### 1.2 核心设计哲学

Memory 模块遵循 **"基础设施 + Agent 自主决策"**:

* **CLI + skills 优先**:上层一律通过 `agent-memory` 命令行调用;接口不消费 JSON 入参,CLI 保持(语义上)无状态,只读取和修改本地 `memory_root` 目录状态。
* **单一写入口原则**:Agent 只需要掌握一个写入子命令 `agent-memory set`,两种调用形态:短 content 用 `set <key> <content> --reason <r>`,长/多行 content 用 `set <key> --reason <r>` + stdin。`agent-memory remove <key>` 是失效语义的语法糖。
* **人机分工原则**:
  * LLM 擅长:决定记什么、如何命名 key、如何措辞 content(自然语言)、为何写入/失效、查询时使用哪些 tags/关键词。
  * 系统擅长:可靠落盘、本地内容文件与FTS5 倒排索引构建、默认读取排序、过期过滤、一致性修复、压缩重写。
* **content 纯文本**:content 必须是纯文本(自然语言短句/段落),便于 LLM 直接消费、便于 grep。少量系统字段(`importance`、`expired_at`)以约定的"文本前言"承载,系统按行解析(见 §4.2)。
* **主语言原则(English-only)**:Memory 内容统一使用英语。这一约束的依据见 §1.5——简而言之,Memory 是 Agent 的工作记忆,用户对话语言可能多变,但 Agent 抽取出的事实必须归一到统一语言才能跨对话稳定 retrieve。`primary_language` 字段在 init 时写入 `.meta/meta.json`,**不可修改**;v2.8 仅支持 `"en"`。Agent 的责任是把多语言对话翻译/抽取为英语 memory(由 prompt 工程保障,见 §1.5)。系统不做翻译,也不做语言检测。
* **真相源 = 文件 + 最新 envelope**:在线状态由 `memory_root` 根目录下的业务内容文件 **联合** `.meta/log.jsonl` 的最新一条 envelope 决定;两者矛盾时按最新 envelope 的 `valid` 与 `ts` 判定。索引数据库是派生缓存,可任意丢弃重建。
* **元数据集中隐藏**:审计日志、锁文件、快照、归档等元数据统一放在 `.meta/` 下;`.meta/` 不参与默认读取 / `agent-memory load` 的正常路径。
* **会话状态外置**:CLI 每次执行只看本地 `memory_root` 目录;当前会话上下文、tags 构造等由上层 session 模块管理,本模块不参与。
* **轻量存储原则**:本地文件系统 + SQLite (含 FTS5 模块) 即可,不依赖向量库或外部全文搜索服务。
* **单目录初始化原则**:模块以单一 `memory_root` 目录初始化,所有持久化状态都落在该目录下;目录布局是**跨语言实现的兼容契约**(见 §4.1),任何语言都可以读/写同一个 `memory_root`。
* **自我进化原则**:Agent 在 self-improve 阶段进行语义整理;系统侧用确定性算法提供一致性与 compaction 兜底。

### 1.3 核心理念:浮现逻辑(Surfacing,not Retrieval)

Memory 模块的所有接口设计、排序策略、写入语义,都围绕一个核心理念展开——**浮现逻辑**。

#### 1.3.1 比喻:人聊天时下意识的"想起"

人与人聊天时,对方提到一个话题,我们脑中**会下意识浮现**与之相关的过往记忆——不是显式去"检索"(没有内心独白说"让我搜索一下我对这个人的所有记忆"),而是当前话题构成的语义场自动激活相关长期记忆。Memory 模块要复刻的就是这个机制。

#### 1.3.2 push 范式:不是 RAG,而是浮现

主流 RAG (Retrieval-Augmented Generation) 是 **pull 范式**:用户提问 → 系统检索 → 返回相关。LLM 知道自己缺什么,主动去拿。

本模块是 **push 范式**:LLM 没"提问",但每个对话回合都会留下少量轻量信号(英语 tags),由系统主动把相关历史**推到** LLM 视野里——下一次推理时,记忆已经在 prompt 上下文中了。

这个差异在哲学上深刻:RAG 假设 LLM 对自己的知识空白有元认知(知道自己不知道什么),浮现假设 LLM 不需要这种元认知——**只需要在每个回合产出一组 tags,就能让相关记忆自动到位**,这与人脑的工作方式更接近。

#### 1.3.3 协作模型:LLM ↔ session ↔ Memory

整个流程涉及三个角色,每段都是低成本的:

```
   ┌────────── LLM (每回合) ──────────┐
   │ 副产物:产出英语 tags(几个 token 成本) │
   └──────────────┬───────────────────┘
                  │ tags
                  ▼
   ┌──────── Session 合并器(上层职责) ────────┐
   │ 累积新 tags、衰减/淘汰旧 tags、维持上限 N    │
   │ 输出有序优先级列表                          │
   └──────────────┬─────────────────────────┘
                  │ ordered tag list
                  ▼
   ┌────────── Memory 模块(本文档) ──────────┐
   │ load 接口接受有序 tag list                │
   │ FTS5 召回 + 优先级 boost + 排序            │
   │ 截断到 token/byte limit                   │
   └──────────────┬──────────────────────────┘
                  │ 记忆片段
                  ▼
              下一轮 prompt 的"记忆区"
                  │
                  ▼
                  LLM(看到记忆已浮现,无需主动检索)
```

**LLM 侧**:每个对话回合除了正常生成回复,作为副产物输出一小组英语 tags(典型 3–10 个,每个 ≤ 32 字节)代表"这个回合涉及的概念"。这是浮现的"种子",几乎零 token 成本。

**Session 合并器**:tags 不是一次性的。新 tags 进来,旧 tags 衰减或被淘汰,维持一个**有上限、有优先级的 tag 列表**反映"当前会话的语义场"。具体合并算法(滑窗/频次衰减/语义聚类等)**由上层 session 模块自行决定**,不属于 Memory 模块的职责——Memory 只通过 `agent-memory load <tag1,tag2,...>` 的有序优先级 list 接口与之配合。

**Memory 模块**:在下次 LLM 唤醒前,系统拿这个 tag 列表去 Memory 召回相关条目,塞进 prompt 的记忆区。LLM 不是"主动想起来",而是"打开眼睛就看到了"——记忆已经浮现。

#### 1.3.4 写入与浮现的异步解耦

读是浮现式的、主动权在系统;写入也不必同步。每个 session 可有不同的写入策略:

| Session 类型     | 写入策略                                                   |
| -------------- | ------------------------------------------------------ |
| 高信任(主人对话)      | 允许 LLM 在回合中主动 `set`(实时写入)                              |
| 普通(对外服务)       | 回合中不写,会话结束后 review/self-improve 阶段统一 `set`            |
| 只读(临时/沙盒会话)    | 完全不写;Memory 仅供查询                                       |

**这是 session 层的策略选择,不是 Memory 模块的强约束**——Memory 始终允许 set,但调用方可在 prompt/skill 层决定是否调用。这种解耦让"何时记下来"成为 session 信任级别的产物,而不是技术限制。

#### 1.3.5 设计含义(为什么前面所有决策长这样)

浮现逻辑作为核心理念,统一解释了文档中多处看起来零散的设计选择:

| 设计                            | 浮现逻辑下的解释                                                          |
| ----------------------------- | -------------------------------------------------------------------- |
| tags 是查询词项,不是 memory 元数据      | 浮现是 retrieve-time 触发,LLM 不需要在 set 时打 tag——session 合并器在用时构造,见 §3.4 |
| `load` 接受有序 tag list,优先级位置加权 | session 合并器输出的就是有序优先级列表;Memory 接口形态与之契合,见 §4.1.4.2                 |
| 召回是 any 并集,不是 all 交集          | 浮现允许噪声——只要语义相邻就该浮现,不必每个 tag 都精确命中,见 §3.4.1                          |
| 不用向量检索,用 BM25                 | 浮现是 push,不是 pull;短英语 tags + BM25 已足够,且确定性、可解释、可 grep,见 §10        |
| Memory 主语言英语                  | 同一事实在不同对话语言下都能被同一 tag 命中,这是浮现的基础,见 §1.5                            |
| 默认读取(passive retrieval)默认开启   | 浮现是默认行为,LLM 不必显式调用任何工具,见 §5                                          |
| 写入与读取异步解耦                     | 浮现侧从不写,写入由 session 信任级别决定;两条路径独立演化                                  |

#### 1.3.6 责任边界(防止理念落地走样)

| 角色             | 职责                                              | 不做                                                         |
| -------------- | ----------------------------------------------- | ---------------------------------------------------------- |
| LLM(prompt 层) | 每回合输出英语 tags;在合适回合 `set`/`remove`                | 不维护 tag 状态;不做检索决策                                          |
| Session 合并器    | 累积、衰减、淘汰 tags;维持有上限有优先级的 list                   | 不查询 Memory;不参与排序                                           |
| Memory 模块(本)   | 提供 set/remove/load CLI;FTS5 召回 + boost 排序;持久化   | 不维护 session 状态;不构造 tags;不翻译;不做 LLM 元认知                  |

> 把"浮现"做对的关键不在 Memory 模块,而在 session 合并器与 prompt 工程的协作。本模块的职责是**让浮现廉价、可靠、确定**——剩下的,交给上层去赋形。

---

### 1.4 典型场景:Agent 日程管理(非专用、可涌现)

日程管理不是专用模块,而是 Agent 利用 Memory 基础设施 + 浮现逻辑(§1.3)形成的一种行为模式。下面这个场景体现两条主线:**写入由 LLM 在合适回合主动触发**,**读出由系统通过浮现自动完成**。

**写入侧**:

1. 用户提出日程/提醒需求(如用户用中文说"明天 10 点牙科复诊")。
2. Agent 选择 key(如 `/user/calendar/2026-02-23_10-00_dental_followup`),调用 `agent-memory set` 写入一条英语记录(content 是 `Dental follow-up appointment on 2026-02-23 at 10:00.`,reason 中可保留中文原话作为 provenance)。
3. 若用户要求高精度,Agent 调用上层能力 `set_timer`(不属于 Memory 模块)注册精确 Timer;否则依赖 **3 分钟保底轮询**(best-effort)。
4. 这一回合 LLM 同时输出 tags(浮现副产物):`dental, appointment, calendar, reminder, 2026-02-23`,session 合并器把这些 tags 累积进当前语义场。

**浮现侧**(关键体现):

5. 一周后,用户在另一次对话中随口提到"我牙最近有点酸"。LLM 这一回合的 tags 副产物里出现 `dental, tooth, pain`。session 合并器把这些累积进语义场。
6. **下一次 LLM 推理前**,系统拿合并后的 tag list `dental, tooth, ...` 调用 `agent-memory load`。FTS5 命中之前写入的 `/user/calendar/2026-02-23_10-00_dental_followup`,记忆被推到 prompt 的记忆区。
7. LLM 看到 prompt 里已经有"用户上周记录的牙科复诊"这条,**无需主动检索**就能主动提醒:"你上周记的牙科复诊在 2026-02-23,牙最近不舒服要不要提前去看?"

**主动触发侧**(Timer 唤醒):

8. Timer Event 在 2026-02-23 09:57 唤醒 Agent。Agent 通过默认读取或 `agent-memory load`(必要时 bash)定位相关记忆,执行 SendMsg。
9. 一次性提醒完成后,Agent 调用 `agent-memory remove <key>` 使其失效;重复提醒可更新内容(例如推进 next_trigger_at)。

> **精度声明**:准点提醒的强保证来自 `set_timer`;3 分钟轮询属于 best-effort(允许延迟与偶发遗漏),符合"通用系统"定位。
>
> **浮现声明**:第 5–7 步展现的"用户提到牙痛 → 系统浮现一周前的牙科记录 → LLM 看到上下文已有该信息"是这套设计的核心价值——记忆不是被检索出来的,而是顺着话题浮现出来的。如果第 5 步用户没提"牙",这条 memory 也不会浮现到 prompt;它会安静地等到 2026-02-23 由 Timer 触发,或者下一次任何含 `dental/tooth/calendar` 等相邻概念的对话激活它。

---

### 1.5 主语言决策:Memory 内容统一为英语

#### 1.5.1 决策依据

英语主语言是浮现逻辑(§1.3)的直接推论:**同一事实必须能被任意对话语言触发的浮现命中,这要求所有事实在 Memory 内归一到统一语言**。具体推演与跨对话语义稳定性的论证见 §1.3.5。

工程层面的具体例子:

* 德国客户说 `"I have an appointment Tuesday"`,法国客户说 `"j'ai un rendez-vous mardi"`,日本客户说 `"火曜日に予約があります"`。如果 Memory 跟随对话语言,这三个事实在浮现时根本无法互通。统一英语后,session 合并器在任何语言对话中产出的 tag 都是 `appointment, tuesday, ...`,均可命中同一条 memory。
* 用户偏好"喜欢简洁中文"是个**事实**,不是中文专属;它该被表达为 `User prefers concise responses in Chinese.`,这样英语对话里讨论"如何回复用户"时也能浮现。

#### 1.5.2 协议约束

* `.meta/meta.json` 必含 `primary_language` 字段(BCP 47 language tag),init 时写入,**之后不可修改**。
* v2.8 只接受 `"en"`;遇到其它值的 memory_root 拒绝挂载并提示"unsupported primary_language; v2.8 only supports en"。未来版本可放开。
* CLI 不在每次 set 时检查 content 语言(避免运行时开销与不可靠的语言检测);**写英语是 Agent 的责任,系统只在 init/挂载时校验 `primary_language` 字段值**。

#### 1.5.3 上层 prompt 工程的责任

以下规则不是 CLI 强制,而是上层 Agent system prompt / skill 应当遵循的约定:

1. **content 主体一律英语**:无论对话使用什么语言,写入 memory 的 content 必须翻译/抽取为英语自然语言句子。
2. **专有名词允许夹带原文**:人名、地名、产品名、项目术语等不可逆信息允许在英语后用括号标注原文,如:
   ```
   User's name: Zhang Wei (张伟)
   Home address: Zhongguancun district, Beijing (中关村)
   Project term: Intent Engine (意图引擎) — see /glossary/intent_engine
   ```
   FTS5 unicode61 会丢弃括号内的 CJK 字符(它们不是 alnum),所以原文只是"备查",不参与索引。索引仍走英语,这正是期望行为。
3. **reason 字段可以保留原文来源**:`--reason` 不参与索引,适合放对话原句、URL、文件路径等 provenance 信息,可不翻译。
4. **建议建立 `/glossary/` 命名空间**:对项目专有术语固定英语规范写法,避免 Agent 在不同对话中翻译漂移(如 `intent engine` / `intent-driven engine` / `intention engine` 三种译法)。例:
   ```bash
   agent-memory set /glossary/intent_engine \
       "Intent Engine (意图引擎): OpenDAN's failure-as-first-class-citizen subsystem; see CYFS docs" \
       --reason "project glossary;canonical translation"
   ```
   后续 Agent set 涉及该术语的 memory 时,先 retrieve glossary 取规范写法,再写入。

> 这些规则的执行质量决定了 Memory retrieval 的质量,但不影响 Memory 模块的正确性——本模块只保证"你写什么进去就索引什么"。

---

## 2. 系统架构:职责边界

### 2.1 Agent 侧职责(LLM 驱动,容错型)

| 职责                       | 说明                                                                                              |
| ------------------------ | ----------------------------------------------------------------------------------------------- |
| 调用 `agent-memory set`    | 主写入口:新增/更新                                                                                      |
| 调用 `agent-memory remove` | 失效/删除                                                                                            |
| 决定 key                   | 自行组织命名空间与层级,如 `/user/calendar/...`、`/user/preference/...`、`/kb/...`                             |
| 组织 content / reason      | content 必须纯文本;调用 `set` 时给出写入原因                                                                  |
| 调用 `agent-memory load`   | 在需要主动召回时传入 tags/关键词集合,基于 `key + content` 派生的FTS5 倒排索引批量取回                                          |
| 提供 reason                | 对外部/网络/工具信息必须在 reason 中给 provenance;对用户对话也推荐给可回溯来源                                              |
| 主动查询                     | 通过 bash(ls/find/grep/cat)检索 `memory_root/` 下的业务内容文件(跳过 `.meta/` 与 `memory.sqlite`)              |
| self-improve             | 在整理阶段合并冗余、调整重要度、路径重构/归档(通过 set/remove 写回)                                                       |

### 2.2 系统侧职责(确定性算法,可靠型)

| 职责              | 说明                                                                       |
| --------------- | ------------------------------------------------------------------------ |
| 双写落盘            | `set`/`remove` 同时更新**本地内容文件**、**审计日志**与**索引数据库**(含FTS5 倒排索引)                  |
| 审计 JSONL        | 每次写入追加到 `.meta/log.jsonl`,并发安全;**作为真相源的一部分**(见 §4.4.3)                        |
| key→本地内容文件      | 根据 key 构建本地内容文件(单文件对应单 key 的最新有效内容)                                      |
| FTS5 倒排索引            | 维护 `term → keys` 轻量索引,term 从 `key + content` 派生,供 `agent-memory load` 高效查询  |
| 默认读取构造          | 按 token_limit 与排序策略返回嵌入 prompt 的 Memory 片段                               |
| 过滤与 LWW         | 过期过滤(`expired_at`),tombstone 过滤,同 key Last Write Wins                    |
| 一致性修复           | 真相源 = 文件存在性 + 最新 envelope;两者矛盾时按 envelope 的 `valid`/`ts` 判定;索引从真相源重建        |
| Compaction/压缩   | 归档审计日志、重建索引、原子替换                                                          |

---

## 3. 接口设计(CLI 优先)

### 3.0 接口形态约束

* **唯一可执行**:`agent-memory`,所有能力以子命令暴露。
* **无 JSON 入参**:CLI 不接受 JSON 字符串参数。
* **`set` 支持双形态**:短 content 可作为 positional 直接传(`set <key> <content> --reason <r>`);长/多行 content 走 stdin(`set <key> --reason <r>` + stdin)。形态由 positional 数量**显式区分**,不做隐式 tty 检测。详见 §3.2。
* **CLI 语义无状态**:CLI 每次执行只看本地 `memory_root` 目录状态;不持有任何会话级状态(如当前会话 tags),这些由上层 session 模块管理。锁文件 `.meta/lock` 是同步原语,不视为"状态"(见 §4.1.6)。
* **无 JSON 出参(默认)**:默认输出对 LLM/人友好的纯文本(见 §3.6)。
* **退出码语义**:
  * `0`  成功(包括幂等成功:remove 不存在的 key、重复 init)
  * `1`  一般错误(参数非法/校验失败,如 key 含 `..`、segment 超长、reason 缺失、单参 set 但 stdin 为 tty/空)
  * `2`  写者锁/并发冲突(`.meta/lock` 被占用且超过等待时长)
  * `3`  真相源损坏需修复(envelope 与文件无法对齐且无法自动恢复)
  * `64–78` 同 `<sysexits.h>`

### 3.1 全局选项

```
agent-memory [--root <memory_root>] [--quiet] <verb> [...]
```

* `--root`:总是可选。传入时覆盖默认 memory root;不传时从 `AGENT_MEMORY_ROOT` 或运行时约定的本地默认目录推导。
* `--quiet`:抑制非错误日志,仅打印结果。

### 3.2 set — 写入 / 更新

```
# 形态 A:两参(content positional)— 推荐用于短文本
agent-memory [--root <memory_root>] set <key> <content> --reason <reason>

# 形态 B:单参 + stdin — 用于长/多行 content
agent-memory [--root <memory_root>] set <key> --reason <reason>
# content 经 stdin 传入(必填,不可为空)
```

#### 3.2.1 两种形态的消歧规则(关键)

CLI **只看 positional 数量**,不去检测 stdin 是否 tty:

| positional 数 | 行为                                                                  |
| ------------ | ------------------------------------------------------------------- |
| 2(`<key> <content>`) | content 取自 positional;**忽略 stdin**(即使被管道喂了内容)            |
| 1(`<key>`)            | content 必须从 stdin 读;**stdin 是 tty 或读到 0 字节 → 退出 1**,提示用户 |
| 0 或 ≥3              | 退出 1                                                                |

为什么不自动检测 stdin tty:那种隐式行为在 LLM/skill 调用场景下不可预测——LLM 可能写出 `agent-memory set /x` 但忘记接 heredoc,导致命令悬挂等输入。**显式 positional 数 = 显式选择形态**,LLM 容易学,排错也容易。

#### 3.2.2 何时用哪种形态

* **用形态 A(两参)**:content 是单行、≤ 1KB、不含 `\n`、反引号、`$`、嵌套引号。
* **用形态 B(stdin)**:content 含换行、长度超过 ~1KB、含特殊 shell 字符。**经验法则:content 含换行 → 一定走 stdin**。

#### 3.2.3 调用示例

```bash
# 形态 A:短 content
agent-memory set /user/preference/style "User prefers concise responses in Chinese." \
    --reason "user conversation;original=用户喜欢中文、偏好简洁;conversation=c1,message=m9"

# 形态 B:多行 content(LLM 生成场景的首选)
agent-memory set /user/calendar/2026-02-23_10-00_dental_followup \
    --reason "user conversation;original=明天10点牙科复诊;conversation=c1,message=m10" <<'EOF'
Dental follow-up appointment on 2026-02-23 at 10:00.
If not completed by 2026-02-24, ignore.
EOF

# 形态 B:管道喂入
some_command | agent-memory set /agent/notes/run_42 --reason "tool output"
```

> heredoc 使用 `<<'EOF'`(带单引号)可避免 shell 对 content 内的 `$`、反引号做展开,这是 LLM 生成命令时的稳妥默认。

#### 3.2.4 行为语义

| 语义               | 调用方式                                              | 说明                          |
| ---------------- | ------------------------------------------------- | --------------------------- |
| 新增/更新(Upsert,短) | `set <key> <content> --reason <r>`                | 同 key 覆盖更新;以最后写入为准(LWW)     |
| 新增/更新(Upsert,长) | `set <key> --reason <r>` + stdin                  | 同上,适合多行/长文                  |
| 失效/删除(Tombstone) | `agent-memory remove <key>`                       | 见 §3.3                      |

> **重要约定**:set 的 content 不可为空。两参形态的 content 不能是空字符串;单参形态 stdin 读到 0 字节 → 退出 1。要清除一个 key,请使用 `remove`。

#### 3.2.5 系统侧写入行为(确定性,顺序敏感)

收到 `set` 后系统必须执行:

1. **校验**:key 规范化(§4.3)、reason 必填且符合 §3.5 强制规则、content 非空且字节数 ≤ NFR 上限、所有 segment ≤ 200 字节(详见 §4.1.5)。
2. **追加审计**:将一条标准 envelope 追加写入 `<memory_root>/.meta/log.jsonl`,`O_APPEND` + `fsync`(见 §4.2)。**这一步成功即视为写入完成**——envelope 是真相源的一部分。
3. **写本地文件**:将 key 对应的本地内容文件原子写入最新内容(见 §4.1.6)。
4. **更新索引数据库**:刷新 `key → 元信息` 主表与从 `key + content` 派生的FTS5 倒排索引。
5. 若步骤 3、4 失败:**不影响真相源**(envelope 已落盘);下一次 compaction/启动校验会从 envelope + 文件重建数据库与缺失文件。

> 顺序由真相源定义反推:envelope 是 append-only、原子的,作为最终判定依据放在第一步;文件与索引是派生形态,可重建。

#### 3.2.6 reason 与 content 长度上限

* `--reason <text>`:必填。短文本,通过 argv 传入(reason 通常 < 200 字节,适合 flag)。
* 当内容来自 `web/tool/file` 或 key 落在 KB 命名空间时,reason 必须包含可回溯来源(见 §3.5)。
* 形态 A 的 content 受 `ARG_MAX` 隐式限制(典型系统 ~128KB);超长 content **必须**用形态 B,实现可在形态 A 检测到 content > 64KB 时打印警告建议改用 stdin。
* content 建议上限见 §9 NFR(单条 ≤ 256KB,超出仅 warning)。Memory 是 Agent 的工作记忆,不是图书馆——海量长文档应放在外部存储,Memory 里只保留摘要 + 引用。

---

### 3.3 remove — 失效(Tombstone)

```
agent-memory [--root <memory_root>] remove <key> [--reason <reason>]
```

* 等价于"对该 key 写入 tombstone envelope"。
* 系统行为:
  1. 追加 `valid=false` 的 envelope 到 `.meta/log.jsonl`(真相源生效)
  2. 删除业务内容文件
  3. 从FTS5 倒排索引清理该 key
  4. 索引主表标记 `valid=0`(或物理删除,实现可选)
* 删除不存在的 key 不报错(幂等),退出码 0。
* `--reason` 可选,记录失效原因,有助于后续审计。

---

### 3.4 load — 按 tags 批量召回

```
agent-memory [--root <memory_root>] load <tag1,tag2,tag3> [--max-bytes N] [--max-records N]
```

#### 3.4.1 行为语义

1. 将 `<tag1,tag2,tag3>` 按逗号拆分为 tags/关键词集合;每个 tag 必须满足:**长度 2–32 字节,不含控制字符**(空格/标点允许出现在 phrase tag 内,如 `"phone case"`)。CLI 在解析阶段做校验,超出范围报错退出 1。
2. **召回为并集(any)**:在FTS5 倒排索引中按 any 策略求并,得到候选 key 列表。FTS5 倒排索引由系统从 `key + content` 派生,不要求写入时显式提供 tags。
3. 过滤:`valid=false`、`expired_at` 已过期、安全检查不通过的 key 被剔除。
4. **排序**:按 (关联度、新鲜度 ts、importance) 加权排序(详见 §5.4)。tag 数组顺序表示查询优先级,**靠前的 tag 命中权重更高**——但召回范围不变,不会因为排在后面而被"踢出"。
5. 截断与限制:
   * `--max-records N`:最多返回 N 条(默认 50)
   * `--max-bytes N`:返回 stdout 总字节数上限(默认 64KB);单条 content 超过 4KB 时,截断为 `<前 4KB> + "...[truncated, total=<size>B]"`,头部 `TRUNCATED 1`
   * 两者**同时生效**,任一触达即停止追加新记录
6. **不传 tags 或传入 `*`**:行为为全量候选,关联度恒为 0,实际按 ts/importance 排序返回。
7. CLI **不处理 token 限制**——token 是 LLM 概念,需要 tokenizer。调用方根据返回字节数自行折算。

#### 3.4.2 tags 的来源

`load` 必须显式传入 tags/关键词集合,CLI 不维护任何 tags 状态。tags 从哪里来由调用方决定(用户/LLM 输入、上层 session 模块的当前会话上下文、固定主题等),本模块不规定。

> tags 是**查询词项**,不是 memory 的显式元数据字段。用户不会给每条 memory 打 tag;系统只依赖 `key + content` 构建可查询词项。
> tags 的顺序是调用方表达意图的一部分:`calendar,dental,reminder` 与 `reminder,calendar,dental` 的召回候选相同,但排序权重不同。

#### 3.4.3 与默认读取的关系

* **默认读取**(§5):每轮推理前由系统自动构造、写进 prompt 的 best-effort 片段,受 `token_limit` 严格限制。
* **`agent-memory load`**:上层 skills 主动调用,传入 tags,返回更完整的 key→content 列表,作为默认读取的补充与替代。

---

### 3.5 reason / provenance 规则(防污染硬约束)

#### 3.5.1 强制适用范围

满足任一条件时,**`reason` 必须包含可回溯来源**,否则禁止写入长期区(拒绝或隔离到 untrusted 命名空间):

* content 来自 `web | tool | file`
* key 落在 KB 命名空间(如 `/kb/...` 或约定的长期知识区)

#### 3.5.2 reason 内容建议

| 来源类型 | reason 建议                                  |
| ---- | ------------------------------------------ |
| 用户对话 | 包含 conversation/message id 或可回溯上下文          |
| tool | 包含工具名、调用参数摘要、结果 id 或内容 hash                |
| web  | 包含 URL、站点名、抓取时间                            |
| file | 包含文件路径、版本、digest 或 mtime                   |
| agent | 包含推理原因和触发事件                                 |

> CLI 只暴露一个 `--reason` 参数,避免引入嵌套结构。需要复合定位(如 `conversation_id` + `message_id`)时,在 reason 字符串中以 `key=value` 拼接(如 `用户确认;conversation=c1,message=m9`)。

#### 3.5.3 示例

```bash
# web 来源(短 content,两参形态)
agent-memory set /kb/product/iphone16/name "iPhone 16" \
    --reason "web source;site=apple.com;url=https://apple.com/iphone16;retrieved_at=2026-02-22T10:00:00Z"

# user 对话(短 content,两参形态;原话可放在 reason)
agent-memory set /user/preference/style "User prefers concise responses in Chinese." \
    --reason "user conversation;original=用户喜欢中文、偏好简洁;conversation=c1,message=m9"

# web 来源(长 content,stdin 形态)
agent-memory set /kb/product/iphone16/spec \
    --reason "web source;site=apple.com;url=https://apple.com/iphone16/spec;retrieved_at=2026-02-22T10:00:00Z" <<'EOF'
iPhone 16 specifications summary

- Display: 6.1-inch OLED
- Chip: A18
- Camera: dual-camera system, 48MP main
...
EOF
```

---

### 3.6 输出格式

#### 3.6.1 text(默认,对 LLM/人友好)

`load` 输出每条记录使用**长度前缀**,可无歧义解析:

```
KEY <key>
SIZE <bytes>
TRUNCATED <0|1>
MATCHED <tag1,tag2,...>         # 可空,命中的查询 tag
TS <iso8601>
---
<恰好 SIZE 字节的 content>
```

* `SIZE` 单位:**UTF-8 编码后的字节数**,与 `content_size` 字段一致(见 §4.1.4)。
* 记录之间无分隔符;解析端按 `KEY` 行识别下一条;读完 SIZE 字节后回到等待 `KEY` 状态。
* content 中的换行/特殊字符不会破坏解析(依赖字节计数,不依赖分隔符)。
* 文件名/key 已禁止换行与 NUL(§4.1.5)。

### 3.7 辅助子命令

```
agent-memory [--root <memory_root>] init           # 初始化目录(写 .meta/、memory.sqlite);幂等
agent-memory [--root <memory_root>] get <key>      # 直接打印一个 key 的 content(仅 stdout,无前缀)
agent-memory [--root <memory_root>] list <prefix>  # 列出某逻辑前缀下的全部 key
agent-memory [--root <memory_root>] verify         # 校验/修复本地文件 ↔ envelope ↔ 索引一致性
agent-memory [--root <memory_root>] compact        # 触发 compaction(见 §7)
```

* **`init`**:幂等。已初始化的 `memory_root` 重复调用 → 退出码 0,无副作用(可选 `--quiet` 抑制提示)。
* **`list`**:每行输出一个绝对 key(以 `/` 开头),不含前缀剥离;`list` 无参等价于 `list /`。例:
  ```
  /user/calendar/2026-02-23_10-00_dental_followup
  /user/preference/style
  ```

`get` / `list` 纯按 key/前缀工作,便于 bash 流水线调试。

---

## 4. 数据模型与存储布局

### 4.1 初始化与目录布局(跨语言兼容契约)

#### 4.1.1 初始化签名

Memory 模块以**单一目录路径** `memory_root` 进行初始化:

```
AgentMemory(memory_root: string)
```

* `memory_root` 必须是绝对路径,指向一个存在或可创建的目录。
* 模块负责在该目录下维护下文规定的布局;**该布局是跨语言实现的兼容契约**——任何语言/进程读到一个由其它实现写入的 `memory_root` 时,都能正确重建在线状态。
* 同一 `memory_root` **只允许一个写者**(见 §4.1.6);只读访问无并发限制。

#### 4.1.2 目录树(规范)

```
<memory_root>/
├── user/                        # 示例:业务内容目录,来自 key=/user/...
│   └── ...
├── kb/                          # 示例:业务内容目录,来自 key=/kb/...
│   └── ...
├── agent/                       # 示例:业务内容目录,来自 key=/agent/...
│   └── ...
├── memory.sqlite                # 推荐:索引数据库(派生缓存,schema 见 4.1.4)
└── .meta/                       # 必选:隐藏元数据目录,不参与在线读取
    ├── meta.json                # 必选:模块元信息(版本、编码方案、分词算法)
    ├── log.jsonl                # 必选:审计日志(追加写,**真相源的一部分**)
    ├── lock                     # 必选:写者进程文件锁
    ├── state.jsonl              # 可选:启动加速快照
    └── archive/                 # 可选:审计日志归档
        └── log_YYYYMMDD.jsonl
```

约束:
* `.meta/`、`.meta/meta.json`、`.meta/log.jsonl`、`.meta/lock` 必须存在;`memory.sqlite` 推荐存在,可从真相源重建。
* 业务目录直接位于 `memory_root` 根目录,不再额外包一层索引目录。
* `memory_root` 根目录保留 `.meta/` 与 `memory.sqlite`;业务 key 的第一段不得为 `.meta` 或 `memory.sqlite`。
* `.meta/` 是唯一隐藏元数据入口;默认读取与 `agent-memory load` 不扫描 `.meta/`。
* 跨语言读取 `memory_root` 时,**只读 `.meta/meta.json` + `.meta/log.jsonl` + 根目录业务内容文件**即可恢复完整在线状态;`memory.sqlite`、`.meta/state.jsonl` 均为派生缓存,可任意丢弃重建。

#### 4.1.3 .meta/meta.json 契约(自描述)

`.meta/meta.json` 在第一次初始化时写入,描述本 `memory_root` 的版本、编码方案与主语言。**任一项不兼容时挂载方必须拒绝写入**(可降级为只读):

```json
{
  "schema_version": "2.6",
  "primary_language": "en",          // BCP 47;init 后不可修改;v2.8 仅支持 "en"
  "writer": {
    "lang": "rust",
    "impl": "agent-memory-rs",
    "version": "0.6.0"
  },
  "encoding": {
    "key_to_path": "percent",        // key→相对路径编码方案
    "max_segment_bytes": 200,        // 单段长度上限(UTF-8 字节);超出直接拒写
    "filename_format": "bare"        // 文件名格式:bare = 无后缀、无 hash
  },
  "compaction_strategy": "snapshot", // "snapshot" | "log_only",见 §7.3
  "created_at": "2026-05-09T10:00:00Z"
}
```

* `schema_version` 采用 SemVer-lite(`major.minor`);major 不一致 → 拒绝挂载,minor 不一致 → 允许只读。
* `encoding` 中任一字段不被支持 → 拒绝写入,允许只读。
* **`primary_language` 是协议级硬约束**:init 后不可修改(没有 CLI 子命令支持修改);v2.8 仅识别 `"en"`,其它值的 memory_root 直接拒绝挂载并报错 `unsupported primary_language; v2.8 only supports en`。这条约束的依据见 §1.5。
  * 实现侧:`primary_language` 在 init 之外的任何路径都不应被读取或检查——避免运行时开销与启发式语言检测。所有跨语言互操作只在挂载阶段(§4.1.7)校验一次。
* **倒排索引算法不在 meta 中声明**:由本文档 §4.1.4.1 定义为强契约(SQLite FTS5 unicode61)。所有跨语言实现必须遵循。

#### 4.1.4 memory.sqlite schema(强契约)

数据库位于 `<memory_root>/memory.sqlite`,是派生缓存。SQLite + FTS5 模块的可用性由分发机制保证;**实现必须使用 SQLite 3.34+ 且启用 FTS5**。

schema:

```sql
-- 主表:每个 key 一条记录,承载非全文字段
CREATE TABLE memory (
  key            TEXT PRIMARY KEY,
  file_path      TEXT NOT NULL,         -- 相对 memory_root 的业务内容文件路径
  ts             TEXT NOT NULL,         -- ISO8601,来自最新 envelope
  valid          INTEGER NOT NULL,      -- 0/1
  importance     INTEGER,               -- 可空,从 content 文本前言解析
  expired_at     TEXT,                  -- 可空,从 content 文本前言解析
  source_summary TEXT,
  content_size   INTEGER NOT NULL       -- UTF-8 字节数
);

CREATE INDEX idx_memory_ts  ON memory(ts);
CREATE INDEX idx_memory_imp ON memory(importance);

-- FTS5 虚表:倒排索引,bm25 排序
-- key 列 UNINDEXED,作为 join 主键,不参与全文检索
-- key_text / content_text 是预处理后的 token 流(见 §4.1.4.1)
CREATE VIRTUAL TABLE memory_fts USING fts5(
  key UNINDEXED,
  key_text,
  content_text,
  tokenize = 'unicode61 remove_diacritics 2'
);
```

**写入流程**(每次 set):

1. 对 `key` 末段(filename)与 `content` 主体(去除前言)分别做 §4.1.4.1 的预处理 → 得到 token 流。
2. `INSERT OR REPLACE INTO memory ...` 更新主表。
3. `DELETE FROM memory_fts WHERE key = ?; INSERT INTO memory_fts ...` 重建该 key 的 FTS5 行。

**查询流程**(load):

1. 对每个输入 tag 做同样的预处理 → 得到查询 token。
2. 构造 FTS5 MATCH 表达式(详见 §4.1.4.2)。
3. `SELECT key, bm25(memory_fts, 4.0, 1.0) AS bm25_score FROM memory_fts WHERE memory_fts MATCH ? JOIN memory USING(key) WHERE valid=1 AND (expired_at IS NULL OR expired_at > ?) ORDER BY bm25_score`。
4. 在外层应用 tag 优先级 boost(详见 §5.4)。

#### 4.1.4.1 分词与 token 流(强契约)

主语言为英语,FTS5 unicode61 tokenizer 的默认行为已经足够:

* 按非 alnum 字符切分(空格、标点、控制字符均为分隔符)
* lowercase
* 移除变音符号(`remove_diacritics 2`)

实现把 `key`(末段)与 `content` 主体直接喂给 FTS5 即可,**无需任何预处理**。

```sql
CREATE VIRTUAL TABLE memory_fts USING fts5(
  key UNINDEXED,
  key_text,
  content_text,
  tokenize = 'unicode61 remove_diacritics 2'
);
```

**写入语义**:`key_text` 写入 percent-encoding 还原后的 key(逻辑路径,如 `/user/calendar/2026-02-23_10-00_dental_followup`);`content_text` 写入 content 主体(去除文本前言)。整个 content 主体都喂给 FTS5,不做长度截断——遵循 §9 NFR 的 256KB 建议上限,FTS5 索引开销可以忽略。

**查询时的预处理**:`load tag1,tag2` 收到 tag 后,直接构造 FTS5 MATCH 表达式(见 §4.1.4.2)。tag 本身就是英语词项,FTS5 内部会做 lowercase 等规范化。

**确定性保证**:unicode61 tokenizer 在 SQLite 3.34+ 跨平台行为一致。给定同一字符串,所有遵循本契约的实现产出的 token 流必须**完全相同**。

> v2.8 协议只支持英语 memory(`primary_language: "en"`)。Agent 写入时夹带的 CJK/其它非 alnum 原文(用于专有名词标注,见 §1.5.3)会被 unicode61 自然丢弃,这正是期望行为——这些原文留作人工审计/翻译还原用,不参与索引。

#### 4.1.4.2 FTS5 MATCH 表达式与排序

**召回(MATCH)**:把所有 tag 用 `OR` 连接,构造一个 phrase 查询:

```
tags = ["dental", "appointment", "reminder"]
==>
MATCH '"dental" OR "appointment" OR "reminder"'
```

> 用 phrase 查询(双引号包住 tag)避免 FTS5 对 tag 做 prefix 展开。tag 内的多个 token(如 `"phone case"`)按 phrase 完整匹配。

**FTS5 自带排序**:`bm25(memory_fts, 4.0, 1.0)` 给 `key_text` 列权重 4、`content_text` 列权重 1。这部分由 SQLite 内置算法保证。

**外层 boost(tag 优先级)**:bm25 把所有查询 token 视为等权,**这无法表达"前 3 个 tag 比后面更重要"**。因此在 SQL 之外做一层重排:

1. 拿到候选(key, bm25_score)列表后,逐 key 检查它的 `key_text + content_text` 包含哪些**输入 tag**(简单子串/词项匹配即可)。
2. 对命中 tag 集合计算 boost 权重:
   - 第 0 个 tag 命中:`+8`
   - 第 1 个 tag 命中:`+4`
   - 第 2 个 tag 命中:`+2`
   - 第 ≥3 个 tag 命中:每个 `+1`
3. 最终 score = `boost - bm25_score`(bm25_score 越小越相关,所以减号;boost 越大越优先)。
4. 按 score 降序排列;tie-break 顺序:`ts desc`、`importance desc`、`key asc`(稳定排序)。

**为什么 boost 在外层而不是 SQL 内**:bm25 的列权重是固定的,无法按运行时输入做 per-query 加权;FTS5 也不支持"哪个 token 来自哪个 tag"的归属信息。外层 Python/Rust 重排成本可忽略(候选通常 < 100 条)。

**步骤 1 的实现细节**:对每个候选 key 取其 `key_text + content_text`(直接从 FTS5 表查询返回),lowercase 后对每个输入 tag 做子串/词边界匹配;命中即贡献该 tag 的 boost 权重。无需冗余存 token 流。

#### 4.1.5 key → 物理文件名(默认编码方案)

`.meta/meta.json.encoding` 约束下的默认方案:

1. `key` 必须以 `/` 开头,按 `/` 切分为 segments;连续 `/` 规范化为单 `/`;空 segment 与 `..` 段直接拒绝(退出 1)。
2. 第一段不得为 `.meta` 或 `memory.sqlite`,避免与根目录保留项冲突。
3. **每个 segment 的 UTF-8 字节数必须 ≤ 200**;超出直接拒写(退出 1),不做截断。
   > 这个约束让我们能去掉 `@<hash>` 后缀:不存在因截断引起的碰撞。LLM 在合理 key 命名规则下不会触及这个上限。
4. 每个 segment 做 RFC3986-style **percent-encoding**:保留 unreserved 字符;编码 `/`、NUL(`%00`)、换行(`%0A`)、控制字符等。
5. **文件名无扩展名、无 hash 后缀**:物理文件名直接是 percent-encoded 的 key 末段。
6. 物理路径 = `<memory_root>/<encoded_seg1>/<encoded_seg2>/.../<encoded_filename>`。

示例(key = `/user/calendar/2026-02-23_10-00_dental_followup`):

```
<memory_root>/user/calendar/2026-02-23_10-00_dental_followup
```

> v2.8 协议约定 key 使用英语命名(主语言英语)。percent-encoding 仍在协议中保留,作为对意外非 ASCII 字符或特殊符号的健壮性兜底——但合理的 Agent 写出的 key 几乎不会触发实际编码,文件名在文件系统里就是裸的 ASCII 字符串,`ls` / `cat` 体验自然。

#### 4.1.6 原子写入与并发约束

* **业务内容文件**:写到同目录的 `<file>.tmp.<rand>` 后 `rename`(POSIX 原子)。
* **`memory.sqlite`**:使用事务;整库重建时写到 `memory.sqlite.new` 后 `rename`。
* **`.meta/log.jsonl`**:`O_APPEND` + 单条 envelope `fsync`;**append-only 是真相源完整性的保证**。
* **`.meta/meta.json`**:仅初始化时写入;后续升级使用 `<file>.tmp + rename`。
* **写者锁**:写入端在 `<memory_root>/.meta/lock` 持有 POSIX `flock` / Windows `LockFileEx`;同一 `memory_root` 同时只允许一个写者。
* **只读端**:可不加锁,但必须容忍瞬时不一致——遇到业务内容文件、envelope、`memory.sqlite` 三者不一致时,按 §4.4.3 真相源规则判定。

#### 4.1.6.1 写者锁与无状态 CLI 的张力

CLI 每次启动都需要抢 `.meta/lock`,这在批量写场景(如 self-improve 阶段一次性整理几百条)下会有 fork+lock+fsync+exit 的固定开销。

**实现策略**:

* **默认模式(MVP)**:CLI-per-call,每次抢锁。fork+lock+fsync+exit 的开销与 LLM 推理相比可以忽略,无需额外优化。
* **可选 daemon 模式(扩展)**:实现可提供常驻 `agent-memory daemon`,通过 unix socket 接受子命令,daemon 持有锁,CLI 子命令转发请求。daemon 的存在不改变协议——`memory_root` 布局完全一致,daemon 崩溃后任何 CLI 进程都能直接接管。**daemon 是可选优化,不是协议要求**——只有在明确量化到瓶颈时才需要引入。
* **锁等待**:写者锁默认等待 5s,超时退出码 2。`--quiet` 不影响该退出码。

#### 4.1.7 跨语言互操作步骤

任何语言实现接管一个已存在的 `memory_root` 时,规范操作:

1. 读取 `.meta/meta.json`,校验 `schema_version`、`primary_language`、`encoding`、`compaction_strategy`;不兼容则报错或降级为只读。**`primary_language` 不在 `"en"` 时 v2.8 直接报错退出 1,信息:`unsupported primary_language; v2.8 only supports en`**。
2. 获取 `.meta/lock`(写者)或跳过加锁(只读)。
3. (可选)扫描 `.meta/log.jsonl` + 业务内容文件重建 `memory.sqlite`;或直接信任既有 `memory.sqlite` 但保证启动期做一次轻量校验。
4. 后续 `agent-memory set` / `remove` 严格按 §3.2.3 / §4.1.5 / §4.1.6 写入。

> 这套契约的目标:让任何语言(Rust / Python / TS / Go ……)只要遵循 §4.1.2–§4.1.6,就能在同一 `memory_root` 上互操作而不破坏数据。

---

### 4.2 审计 JSONL:envelope 与 content 文本前言

#### 4.2.1 envelope(真相源)

系统内部每次写入(包括 tombstone)都以 envelope 形式追加到 `.meta/log.jsonl`:

* 路径:`memory_root/.meta/log.jsonl`
* **真相源的一部分**——与本地业务文件联合判定在线状态。
* 当业务文件丢失或与 envelope 不一致时,按 envelope 的 `valid`/`ts` 重建。

envelope 结构:

```json
{
  "key": "/user/preference/style",
  "ts": "2026-02-22T10:00:00Z",
  "valid": true,
  "source": "user conversation;original=用户喜欢中文、偏好简洁;conversation=c1,message=m9",
  "content_digest": "blake3:abcd1234...",
  "content_size": 43
}
```

* `key`:身份主键(LWW 单位)
* `ts`:系统写入时间(用于新鲜度排序与 LWW 判定)
* `valid`:`false` 表示 tombstone(remove 触发)
* `source`:即 `--reason` 的原文(不参与索引,可保留多语言原话作为 provenance,见 §1.5.3)
* `content_digest`:content 的 blake3 摘要;用于校验业务文件未被篡改
* `content_size`:UTF-8 字节数

> envelope **不直接承载 content**——content 永远只在业务文件里(避免日志膨胀;内容与索引各得其所)。`content_digest` + `content_size` 提供校验链。

#### 4.2.2 content 文本前言(可选系统字段)

content 是纯文本,但允许在文件**开头**用约定的"前言"承载少量系统字段。前言由零行或多行 `Key: Value` 组成,跟随一个空行,空行之后是真正的 content 主体:

```
Importance: 5
Expired-At: 2026-02-24T00:00:00Z

Dental follow-up appointment on 2026-02-23 at 10:00.
```

规则:

* 前言**完全可选**。content 第一行不含 `:` 或不匹配 `^[A-Z][A-Za-z0-9-]*: ` 模式时,视为没有前言,整个 content 即主体。
* 已识别字段:`Importance`(整数,默认 0)、`Expired-At`(ISO8601)。其它字段忽略,但不报错(为未来扩展留口)。
* 前言由系统在 `set` 时解析并写入 `memory.sqlite` 主表的 `importance` / `expired_at` 字段;Agent 可像写普通文本一样写。
* `agent-memory get` 输出**完整 content,包括前言**;`load` 输出的 SIZE 也包含前言字节数;LLM 看到的也是完整文本(前言对 LLM 可读、对系统可解析)。
* 决策依据:Git commit trailer 风格,人类与机器都能直接读;不引入 JSON 结构。

---

### 4.3 key:逻辑路径 vs 物理路径(安全映射必须明确)

#### 4.3.1 key 定义

* key 是逻辑路径,形式类似 URL path:`/dir1/dir2/.../filename`。
* 推荐命名空间示例:
  * `/user/...`:用户相关(偏好、日程、长期事实)
  * `/kb/...`:外部知识沉淀(强制 provenance)
  * `/agent/...`:Agent 自身状态(可选)

#### 4.3.2 物理落盘规则(必须)

系统必须保证所有业务内容文件落在 `memory_root` 根目录下,并按 §4.1.5 的编码方案规范化:

* 禁止 `..`、禁止 NUL、禁止换行等危险字符
* 连续 `/` 规范化为单 `/`
* 禁止使用根目录保留项 `.meta` 与 `memory.sqlite`
* 任一 segment > 200 UTF-8 字节直接拒写
* 对不可安全落盘的字符做可逆 percent-encoding,确保文件不会逃逸 `memory_root`
* 物理路径示例:
  `memory_root/user/calendar/2026-02-23_10-00_dental_followup`(无扩展名,无 hash 后缀)

> 编码方案在 `.meta/meta.json.encoding` 中声明,必须:**可预测、可重复、安全、不会逃逸 memory_root**。

---

### 4.4 三层存储结构(本地文件 + 索引数据库 + 审计日志)

#### 4.4.1 必选:本地内容文件

* 路径:`memory_root/` 根目录下的业务内容路径(无扩展名)。
* 每个 key 对应一个文件,内容是纯 UTF-8 文本(可选前言 + 主体,见 §4.2.2),无 BOM。
* key 的逻辑层级直接体现在根目录下的业务目录结构中。
* `.meta/` 与 `memory.sqlite` 是保留路径,不属于业务内容。
* tombstone 后业务文件被删除;tombstone 状态由 `.meta/log.jsonl` 最新 envelope 承载。
* `ls memory_root/` 必须能直接浏览顶层业务命名空间。

#### 4.4.2 必选:审计日志(真相源的一部分)

* 路径:`memory_root/.meta/log.jsonl`
* 所有 `set` / `remove` 调用追加 envelope;append-only + fsync,并发安全。
* compaction 阶段可归档为 `memory_root/.meta/archive/log_YYYYMMDD.jsonl`(见 §7)。
* **envelope 是真相源的一部分,不能在 compaction 时丢失全部 tombstone 历史**——见 §7.3。

#### 4.4.3 真相源与冲突解决(强约束)

**真相源定义**:在线状态 = (`.meta/log.jsonl` 最新 envelope per key) + (业务内容文件)。

**冲突解决**(按优先级):

1. **envelope 说 `valid=false`,文件存在** → 文件应被删除,以 envelope 为准(可能是 tombstone 后 crash 导致文件未删干净)。
2. **envelope 说 `valid=true`,文件不存在** → 状态丢失。降级处理:在 `verify`/启动时报告该 key 不可读,但不主动恢复(因为 content 不在 envelope 里)。运营层面应配合定期备份。
3. **envelope 与文件 `content_digest` 不一致** → 文件被外部篡改。报告异常,以文件为准(假设外部修改是有意的)并在审计日志追加一条 `valid=true` + 新 digest 的 envelope 修复记录。
4. **本地文件 vs `memory.sqlite` 不一致** → `memory.sqlite` 是派生缓存,从真相源重建。
5. **多份业务文件冲突**(理论上不应发生) → 按 mtime 最新者为准,审计日志追加冲突事件。

> 这条相比 v2.2 的关键变化:**v2.2 说"以文件为准"**——会把已 tombstone 的 key 在 crash 场景下复活;**v2.8 说"envelope + 文件联合判定"**,tombstone 跨 crash 安全。

#### 4.4.4 必选:索引数据库(派生缓存)

SQLite + FTS5(分发机制保证可用),承载:

* **`memory` 主表**:`key → {file_path, ts, valid, importance, expired_at, source_summary, content_size}`。
* **`memory_fts` FTS5 虚表**:`key + key_text + content_text` 的倒排索引,用于 `agent-memory load` 的高效查询。tokenizer 用 unicode61;CJK 文本由实现在写入前预处理为 2-gram token 流(见 §4.1.4.1)。
* 可选辅助索引:`(namespace, ts)`、`(importance, ts)` 等。

> 数据库**仅是缓存**,可随时由真相源(`.meta/log.jsonl` + 业务文件)重建;启动/compaction 阶段必须做"真相源→数据库"的一致性校验。

#### 4.4.5 推荐:状态快照

为大规模场景兼顾启动速度,可选生成 `memory_root/.meta/state.jsonl`(隐藏快照):

* 每个 key 一行,最新有效 envelope。
* 仅作为加速首次启动的可选缓存,**不影响真相源约束**。

---

## 5. 默认读取(Passive Retrieval)

### 5.1 功能描述

每次 Agent 被唤醒(用户消息或 Timer Event),系统自动从 Memory 中构造一段"可嵌入 prompt 的记忆片段"。

默认读取是 **best-effort**:受 token_limit 限制,不承诺覆盖全量记忆。

### 5.2 输入参数

| 参数             | 类型        | 说明                                                     |
| -------------- | --------- | ------------------------------------------------------ |
| token_limit    | number    | 允许嵌入提示词的最大 token(由调用方根据其 tokenizer 估算)                 |
| tags           | string[]  | 与当前上下文相关的 tags/关键词集合(由调用方提供,可选);数组顺序从高优先级到低优先级 |
| current_time   | timestamp | 当前时间,用于 expired 过滤等                                    |

### 5.3 生效规则(过滤 + LWW)

* 同 key 多次写入:以最后一条 envelope 为准(按 `ts` 或写入顺序)
* `valid=false` 的 key 不进入默认读取
* content 前言含 `Expired-At` 且 `current_time > Expired-At`:过滤掉
* content 前言含 `Importance`:用于排序加权;缺省按 0 处理

### 5.4 排序策略

默认读取与 `agent-memory load` 共享同一套排序优先级:

| 优先级 | 维度          | 说明                                                       |
| --- | ----------- | -------------------------------------------------------- |
| P0  | 关联度          | 综合 FTS5 bm25 与 tag 优先级 boost(详见 §4.1.4.2);**无 tags 时关联度恒为 0,实际只按 P1+P2 排序** |
| P1  | 新记忆优先       | `ts` 越近优先                                                 |
| P2  | 重要记忆优先      | `Importance`(若存在)越高优先                                    |

> tags 在默认读取中是"辅助信号";**强检索/批量召回**请使用 `agent-memory load`。

**关联度计算细节**:见 §4.1.4.2。归纳要点:

- 召回:FTS5 MATCH(所有 tag 预处理后的 token 用 OR 连接),由 bm25 给出基础分(列权重 key=4, content=1)。
- 外层 boost:按 tag 优先级位置加权——前 3 个 tag 命中分别 +8/+4/+2,第 ≥3 个 tag 命中各 +1。
- 最终排序:`(boost - bm25) desc, ts desc, importance desc, key asc`(稳定)。

### 5.5 输出格式(建议)

系统应输出紧凑、可追溯的片段,建议包含:

* key(用于 grep/ls 定位)
* importance(若 content 前言存在)
* summary:content 主体(去除前言)的前若干字符

示例:

```
[Agent Memory]
- /user/preference/style User prefers concise responses in Chinese.
- /user/calendar/2026-02-23_10-00_dental_followup [imp=3] Dental follow-up appointment on 2026-02-23 at 10:00.
```

---

## 6. 主动查询(Active Query)

### 6.1 功能描述

Agent 可使用两种方式主动查询:

1. **`agent-memory load <tag1,tag2,tag3>`**:CLI 子命令,结构化、走FTS5 倒排索引(推荐)。
2. **bash 通用工具**对 `memory_root/` 下的业务内容文件做查询(兜底,自由)。

### 6.2 实现要求

* `agent-memory load` 由 FTS5 倒排索引驱动。
* FTS5 倒排索引的数据源只允许来自真相源中的 `key` 与 `content`;不得要求调用方或用户显式维护每条 memory 的标签。
* `load` 的输入是 tags/关键词集合,不是任意自然语言文本;调用方应先把上下文压缩成少量稳定 tags(每个 tag 2–32 字节)。
* bash 路径不新增专用查询工具,复用 `ls/find/grep/cat`。
* `memory_root/` 必须可被 `ls` 直接浏览;业务内容目录直接位于根目录,查询时跳过 `.meta/` 与 `memory.sqlite`。

### 6.3 典型用法(CLI + bash)

```bash
# —— CLI 路径(推荐) ——

# 按 tags / 关键词集合召回(英语 tags;中文用户输入由 Agent 在外层翻译为英语 tags)
agent-memory --root "$MEMROOT" load calendar,reminder,dental

# 列出某前缀下全部 key
agent-memory --root "$MEMROOT" list /user/calendar

# 直接打印一个 key 的 content
agent-memory --root "$MEMROOT" get /user/preference/style

# —— bash 兜底路径 ——

# 浏览顶层业务命名空间
ls "$MEMROOT/"

# 直接查看某条记忆(content 是纯文本,无扩展名)
cat "$MEMROOT/user/preference/style"

# 全文 grep
grep -r "dental" "$MEMROOT/" --exclude-dir=.meta

# 审计日志(reason 中可能保留多语言原文)
grep "牙科" "$MEMROOT/.meta/log.jsonl"
```

---

## 7. 整理淘汰(Memory Compaction)

### 7.1 目标

* 控制 `.meta/log.jsonl` 增长、修复索引数据库一致性。
* 同时满足审计/回放与在线状态读取的需求。

### 7.2 系统侧确定性处理(必须)

| 操作                | 说明                                                                          |
| ----------------- | --------------------------------------------------------------------------- |
| 真相源→数据库重建         | 以最新 envelope per key + 业务内容文件为真相源,扫描重建主表与FTS5 倒排索引                            |
| LWW 归并            | 多条 envelope 历史归并为最新状态(每个 key 保留最新一条作为快照,其余移入归档)                            |
| tombstone 生效      | `valid=false` 的 envelope 对应 key,从数据库与FTS5 倒排索引中移除;若业务文件残留,删除                  |
| expired 处理        | 若 content 前言 `Expired-At` 过期:从数据库与FTS5 倒排索引过滤;业务文件按策略保留或删除(见 7.3)              |
| 一致性修复             | 按 §4.4.3 真相源规则                                                              |
| 原子重建              | 采用"写 `memory.sqlite.new` + rename"方式替换,并刷新FTS5 倒排索引                                |
| 审计日志归档            | 见 §7.3——**必须保留 tombstone 完整性**                                              |

### 7.3 审计日志归档策略

**核心约束**:tombstone envelope 是真相源的一部分,**不能丢**——否则一个 key 被 remove 后又在归档轮转中"失忆",在某些恢复路径下会复活。

**方案 A(推荐):快照 + 归档**

* `.meta/log.jsonl` 按时间或行数阈值归档:旧 log 移到 `.meta/archive/log_YYYYMMDD.jsonl`,新建空 `.meta/log.jsonl`。
* **归档前**生成 `.meta/state.jsonl` 快照:每个 active key 一行最新 envelope,包括 `valid=true` 与有意义的 `valid=false`(被 remove 但其他 key 引用过的 tombstone)。
* **真相源 = 最新 envelope per key**,无论它在 `log.jsonl` 还是 `state.jsonl`。重建时优先读 `state.jsonl`,再叠加 `log.jsonl` 的增量。
* 归档文件永久保留(由运营周期清理),不影响在线状态。

**方案 B(简单):log 永久保留**

* `.meta/log.jsonl` 永不删除,只按行数阈值切到 `.meta/archive/log_part_NNN.jsonl` 但保持 append 链完整。
* 重建时按时间合并所有 archive + 当前 log。
* 优点:零状态;缺点:重建时间随历史线性增长。

> 实现至少二选一,并在 `.meta/meta.json` 加一个 `compaction_strategy: "snapshot" | "log_only"` 字段声明。跨语言挂载方必须能识别两种策略。

---

## 8. 记忆生命周期

| 阶段     | 触发方式                                              | 说明                                      |
| ------ | ------------------------------------------------- | --------------------------------------- |
| 创建/更新  | `set <key> <content> --reason <r>` 或 `set <key> --reason <r>` + stdin    | 同 key Upsert,envelope+文件+索引三写             |
| 活跃     | 默认读取 / `agent-memory load` / 主动查询                  | 被纳入 prompt 或被 Agent 查询使用                |
| 触发(可选) | Timer Event                                       | 日程类记忆到点后 Agent 执行提醒(准点依赖 set_timer)     |
| 失效     | `agent-memory remove <key>`                       | tombstone envelope 写入,业务文件删除,FTS5 倒排索引清理     |
| 整理     | compaction/self-improve                           | 系统从真相源重建数据库;Agent 做语义合并/降级(通过写回)        |

---

## 9. 非功能性需求(NFR)

性能层面本模块不规定具体指标——**与 LLM 推理的开销相比,本地文件 + SQLite 的传统计算开销可以忽略**。下面只列出可靠性、容量与一致性约束。

| 项目             | 要求                                                                         |
| -------------- | -------------------------------------------------------------------------- |
| 写入可靠性          | JSONL 追加必须原子化;并发写入由 `.meta/lock` 串行化,保证不产生半行 JSON                          |
| 真相源约束          | envelope + 文件联合判定;两者矛盾按 §4.4.3 处理                                          |
| 一致性(浮现语义)     | `load` 与默认读取**不保证快照一致性**——读取过程中并发的 set/remove 可能让结果反映半新半旧的状态。这是浮现逻辑(§1.3)天然不需要的属性,接受 SQLite 默认行为即可 |
| 写者唯一性          | 同一 `memory_root` 同时只允许一个写者(`.meta/lock`);只读端无并发限制                          |
| key segment 上限 | ≤ 200 UTF-8 字节;超出退出 1                                                       |
| tag 长度          | 2–32 字节,不含控制字符;超出退出 1                                                      |
| content 建议上限   | **单条 content 建议 ≤ 256KB**。Memory 是 Agent 的工作记忆,不是图书馆——海量长文档应放在外部存储,Memory 里只保留摘要 + 引用。CLI 在 content > 256KB 时打印 warning,但不报错;硬上限由系统资源决定(stdin 流式读取无固定上限) |
| 可观测性           | set、remove、load、compaction 子命令的执行过程记录日志(key、ts、tags、命中词项、来源摘要)              |
| 无外部依赖          | 仅依赖本地文件系统、SQLite ≥ 3.34(含 FTS5)与标准 Unix 工具;FTS5 可用性由分发机制保证                       |

---

## 10. 约束与边界

* 本模块不提供向量检索、语义搜索或任意文本全文搜索;相关性来自 `key + content` 派生的FTS5 倒排索引与目录结构(key)。
* 本模块不实现 set_timer / Timer 调度,只服务于被唤醒后的记忆读取与写入。
* 本模块不包含 SendMsg 等业务动作。
* `.meta/log.jsonl` 单文件建议上限(如 100,000 行)触发 compaction 或归档轮转(具体阈值可配)。
* CLI 不持有任何会话级状态(锁文件除外,锁是同步原语而非状态);调用方负责把当前需要的 tags 传入 `agent-memory load`。跨会话语义请通过 `agent-memory set` 持久化。
* Agent 的 key 设计、content 质量、importance 标注质量依赖上层 prompt 策略与 LLM 能力。

---

## 附录 A:示例(CLI 调用层)

> 以下示例假设 `export AGENT_MEMORY_ROOT=/path/to/memory_root` 已设置;省略 `--root` flag。
> v2.8 主语言英语:所有 key 与 content 使用英语,reason 字段可保留多语言原文作为 provenance。

### A.1 初始化(幂等)

```bash
agent-memory init
# init 时写入 .meta/meta.json,primary_language="en",此后不可修改
```

### A.2 写入用户偏好(单行 content,两参形态)

```bash
# 用户中文对话:"我喜欢简洁的中文回复"
# Agent 抽取为英语事实写入 memory,原话留在 reason
agent-memory set /user/preference/style "User prefers concise responses in Chinese." \
    --reason "user conversation;original=用户喜欢中文、偏好简洁;conversation=c1,message=m9"
```

### A.3 写入日程(多行 content + 前言,stdin 形态)

```bash
# 用户中文对话:"明天10点牙科复诊,如果没去就算了"
agent-memory set /user/calendar/2026-02-23_10-00_dental_followup \
    --reason "user conversation;original=明天10点牙科复诊;conversation=c1,message=m10" <<'EOF'
Importance: 3
Expired-At: 2026-02-24T00:00:00Z

Dental follow-up appointment on 2026-02-23 at 10:00.
Discardable after 2026-02-24 if not completed.
EOF

# 若用户要求准点:另外调用 set_timer ——不属于 Memory 模块
```

### A.4 失效(remove)

```bash
agent-memory remove /user/calendar/2026-02-23_10-00_dental_followup \
    --reason "reminded and confirmed completed"
```

### A.5 写入 KB(外部来源强制 provenance)

```bash
agent-memory set /kb/product/iphone16/spec \
    --reason "web source;site=apple.com;url=https://apple.com/iphone16/spec;retrieved_at=2026-02-22T10:00:00Z" <<'EOF'
Importance: 2

iPhone 16 specifications summary

- Display: 6.1-inch OLED, ProMotion
- Chip: A18 Bionic
- Camera: 48MP main, 12MP ultrawide
- Battery: ~22 hours video playback
EOF
```

### A.6 写入专有名词(术语词典 + 括号原文标注)

```bash
# 项目术语:Intent Engine 是 OpenDAN 的一个子系统,中文叫"意图引擎"
# 建立 glossary,固定英语规范写法,避免后续翻译漂移
agent-memory set /glossary/intent_engine \
    "Intent Engine (意图引擎): OpenDAN's failure-as-first-class-citizen subsystem; sits between Agent Loop and tool layer." \
    --reason "project glossary;canonical translation;source=internal docs"

# 用户人名(包含原文备查):
agent-memory set /user/profile/owner_name \
    "Owner's name: Zhang Wei (张伟); prefers being addressed as Mr. Zhang in formal contexts." \
    --reason "user profile;conversation=c0,message=m1"
```

### A.7 按 tags 召回

```bash
# 上层(用户/LLM/session 模块)给出英语 tags,调用 load:
agent-memory load calendar,dental,reminder

# 输出(默认 text 格式,按 §3.6.1 解析,SIZE 为 UTF-8 字节数):
# KEY /user/calendar/2026-02-23_10-00_dental_followup
# SIZE 145
# TRUNCATED 0
# MATCHED calendar,dental
# TS 2026-02-22T10:01:00Z
# ---
# Importance: 3
# Expired-At: 2026-02-24T00:00:00Z
#
# Dental follow-up appointment on 2026-02-23 at 10:00.
# Discardable after 2026-02-24 if not completed.
```

### A.8 大 content + 截断

```bash
agent-memory load product,iphone16,spec --max-bytes 8192

# 单条 content 超过 4KB 时,body 末尾会出现:
# ...[truncated, total=18234B]
# 同时头部 TRUNCATED 1
```

### A.9 浏览与查找(bash 兜底)

```bash
# 顶层命名空间
ls "$AGENT_MEMORY_ROOT/"
# user  kb  agent  glossary

# 某前缀下文件
ls "$AGENT_MEMORY_ROOT/user/calendar/"
# 2026-02-23_10-00_dental_followup

# 直接看 content
cat "$AGENT_MEMORY_ROOT/user/preference/style"
# User prefers concise responses in Chinese.

# 全文 grep(英语)
grep -r "dental" "$AGENT_MEMORY_ROOT/" --exclude-dir=.meta

# 审计日志可能保留多语言原话
grep "牙科" "$AGENT_MEMORY_ROOT/.meta/log.jsonl"
```

---

## 附录 B:跨实现一致性测试 fixture(必备)

v2.8 协议下倒排索引由 SQLite FTS5 unicode61 tokenizer 直接处理,**无需自实现分词**。下表是 unicode61 (`remove_diacritics 2`) 在标准输入下的预期 token 化结果,所有遵循本契约的实现 CI 应验证 SQLite 行为一致。

| 输入                                              | token 流(空格分隔)                                |
| ----------------------------------------------- | --------------------------------------------- |
| `dental`                                        | `dental`                                      |
| `Dental Followup`                               | `dental followup`                             |
| `iPhone16`                                      | `iphone16`                                    |
| `iPhone 16 specifications`                      | `iphone 16 specifications`                    |
| `2026-02-23_10-00_dental_followup`              | `2026 02 23 10 00 dental followup`            |
| `Hello, world!`                                 | `hello world`                                 |
| `naïve résumé café`                            | `naive resume cafe`(变音符号被移除)                  |
| 空字符串                                            | (空)                                           |

**用户夹带原文的行为**(协议允许的 §1.5.3 用法):

| 输入                                                              | token 流                                       |
| --------------------------------------------------------------- | --------------------------------------------- |
| `User's name: Zhang Wei (张伟)`                                   | `user s name zhang wei`(`张伟` 不是 alnum,被丢弃) |
| `Intent Engine (意图引擎): subsystem`                              | `intent engine subsystem`(中文被丢弃)            |

> CJK 字符在 unicode61 默认分类下不属于 alnum,会被当分隔符丢弃——这正是 v2.8 想要的:**括号原文标注只为人类审计/翻译还原服务,不参与索引**。索引完全走英语 token,跨对话语义稳定性靠英语主语言保证。

**容易踩坑的 corner case**:

- `iPhone16`:中间无分隔符,整段是 ASCII+数字,lowercase 后保留为 `iphone16`,**不切**。要让 `iphone` 单独可查,Agent 写 content 时应该写 `iPhone 16` 带空格(或在 glossary 里固定写法)。
- 变音符号(`naïve`、`résumé`):tokenizer 配置了 `remove_diacritics 2`,产出 `naive`、`resume`。这对欧洲语言专有名词友好。
- 空字符串、纯标点输入产出空 token 流——CLI 端 tag 长度 ≥ 2 字节的校验已经挡掉这种情况。

> CI 必备项:把上表写成 SQL fixture,在任意 SQLite 3.34+ 上跑 `SELECT * FROM memory_fts WHERE memory_fts MATCH ?` 验证召回行为一致。