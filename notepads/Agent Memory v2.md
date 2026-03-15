# Agent Memory Module — 需求设计文档 v2.1


> **`set_memory(key, json_content, source)`**



## 1. 概述

### 1.1 文档目的

定义 Agent Memory 模块的功能需求、数据模型、接口设计、存储布局与生命周期管理策略。Memory 模块作为 Agent 的基础设施，为跨会话持久化记忆提供统一的文件系统能力，支撑日程管理、用户偏好学习、任务跟踪、知识沉淀（KB）等上层场景。

### 1.2 核心设计哲学

Memory 模块遵循 **“基础设施 + Agent 自主决策”**：

* **单一工具原则**：Agent 只需要掌握一个写入口：`set_memory(key, json_content, source)`
  读操作依赖系统的默认读取 + Agent 的 bash 主动查询。
* **人机分工原则**：

  * LLM 擅长：决定记什么、如何命名 key、如何组织 json_content、何时写入/失效。
  * 系统擅长：可靠落盘、索引构建、默认读取排序、过期过滤、一致性修复、压缩重写。
* **轻量存储原则**：本地 JSONL + 文件系统目录索引；不依赖 DB/向量库。
* **自我进化原则**：Agent 在 self-improve 阶段进行语义整理；系统侧用确定性算法提供一致性与 compaction 兜底。

### 1.3 典型场景：Agent 日程管理（非专用、可涌现）

日程管理不是专用模块，而是 Agent 利用 Memory 基础设施形成的一种行为模式：

1. 用户提出日程/提醒需求（如“明天 10 点牙科复诊”）。
2. Agent 选择 key（如 `/user/calendar/2026-02-23_10-00_牙科复诊`），调用 `set_memory` 写入一条记录。
3. 若用户要求高精度，Agent 应调用上层能力 `set_timer`（不属于 Memory 模块）注册精确 Timer；否则可依赖 **3 分钟保底轮询**（best-effort）。
4. Timer Event 唤醒 Agent 后，Agent 可通过默认读取 +（必要时）bash 主动查询定位相关记忆，决定是否提醒并执行 SendMsg。
5. 一次性提醒完成后，Agent 可对对应 key 执行 tombstone（`set_memory(key, null, source)`）使其失效；重复提醒可更新内容（例如推进 next_trigger_at）。

> **精度声明**：准点提醒的强保证来自 `set_timer`；3 分钟轮询属于 best-effort（允许延迟与偶发遗漏），符合“通用系统”定位。

---

## 2. 系统架构：职责边界

### 2.1 Agent 侧职责（LLM 驱动，容错型）

| 职责              | 说明                                                                  |
| --------------- | ------------------------------------------------------------------- |
| 调用 set_memory   | 唯一写入口：新增/更新/删除（tombstone）                                           |
| 决定 key          | 自行组织命名空间与层级，如 `/user/calendar/...`、`/user/preference/...`、`/kb/...` |
| 组织 json_content | 任意 JSON（建议对象），包含 type/tags/importance/expired_at 等可选字段              |
| 提供 source       | 对外部/网络/工具信息必须给 provenance；对用户对话也推荐给可回溯来源                            |
| 主动查询            | 通过 bash（ls/find/grep/jq/cat）检索 `memory_root`                        |
| self-improve    | 在整理阶段合并冗余、调整重要度、路径重构/归档（通过 set_memory 写回）                           |

### 2.2 系统侧职责（确定性算法，可靠型）

| 职责            | 说明                                                               |
| ------------- | ---------------------------------------------------------------- |
| 原子追加写入        | 将 set_memory 调用持久化为 JSONL 追加（并发安全）                               |
| key→目录索引      | 根据 key 构建目录索引文件（单文件对应单 key 的最新有效状态）                              |
| 默认读取构造        | 按 token_limit 与排序策略返回嵌入 prompt 的 Memory 片段                       |
| 过滤与 LWW       | 过期过滤（expired_at），tombstone 过滤（valid=false），同 key Last Write Wins |
| 一致性修复         | 整理阶段修复 JSONL 与目录索引不一致（以日志或 state 为准，按策略）                         |
| Compaction/压缩 | 生成 state 快照、归档旧日志、原子替换与重建索引                                      |

---

## 3. 接口设计

## 3.1 set_memory（唯一写入口）

### 3.1.1 工具签名

`set_memory(key: string, json_content: any | null, source: object | string)`

* `key`：逻辑路径（见 4.2），标识“一个记忆槽位/条目”的唯一身份。
* `json_content`：任意 JSON（推荐 JSON object）。
* `source`：来源/证据链。对外部/工具/网络信息为强制字段（见 3.2）。

### 3.1.2 操作语义（由参数组合隐式决定）

| 语义               | 调用方式                                          | 说明                              |
| ---------------- | --------------------------------------------- | ------------------------------- |
| 新增/更新（Upsert）    | `set_memory(key, json_content!=null, source)` | 同 key 覆盖更新；以最后写入为准（LWW）         |
| 失效/删除（Tombstone） | `set_memory(key, null, source)`               | `valid=false`，目录索引移除该 key 的有效文件 |

> **重要约定（避免歧义）**：
> “json_content 为空”在本规范中推荐明确使用 **JSON `null`** 表示 tombstone。
> **`{}` 不是 tombstone**（它是合法的空对象内容，仍视为有效写入）。

### 3.1.3 系统侧写入行为（确定性）

收到 `set_memory` 后系统必须执行：

1. 将一条**标准 envelope**追加写入日志 JSONL（见 4.1）。
2. 若 `json_content != null`：更新该 key 的目录索引文件为最新有效内容。
3. 若 `json_content == null`：将该 key 置为无效，并删除/移除目录索引中的对应文件（或写入 tombstone 标记文件）。

### 3.1.4 轻量校验（action-aware）

* `key` 必须以 `/` 开头，且通过安全规范化（见 4.2）。
* `source` 必须存在（object 或 string）；**当判定为外部/工具/网络写入时，必须满足 provenance 最小字段集**（见 3.2）。
* `json_content == null` 允许（tombstone）。
* 对 `json_content != null` 的写入：建议限制最大尺寸（NFR 中定义）。

---

## 3.2 source/provenance 规则（防污染硬约束）

### 3.2.1 强制适用范围

满足任一条件时，`source` 必须提供可回溯 provenance，否则禁止写入长期区（拒绝或隔离到 untrusted 命名空间）：

* 来源是网络（网页、在线文档、API）
* 来源是外部工具输出（搜索、抓取、第三方系统导入、文件解析）
* 写入目标属于 KB 命名空间（如 `/kb/...` 或你们约定的长期知识区）

### 3.2.2 provenance 最小字段集（建议规范）

如果 `source` 为 object，建议至少包含（字段名可按你们风格调整，但语义要具备）：

* `kind`：`user | tool | web | file | system | agent`
* `name`：来源名称（工具名、站点名、文件名、会话来源等）
* `retrieved_at`：获取时间（ISO8601）
* `locator`：可回溯定位信息之一：URL / 文件路径 / 工具 query / 事件 id / 内容 hash

示例（web）：

```json
{
  "kind":"web",
  "name":"example_site",
  "retrieved_at":"2026-02-22T10:00:00Z",
  "locator":{"url":"https://...","title":"..."}
}
```

示例（user 对话）：

```json
{
  "kind":"user",
  "name":"chat",
  "retrieved_at":"2026-02-22T10:00:00Z",
  "locator":{"conversation_id":"c123","message_id":"m456"}
}
```

---

## 4. 数据模型与存储布局

## 4.1 系统侧落盘：标准 envelope（固定字段）+ content（自由字段）

系统内部每次写入（包括 tombstone）都以 envelope 形式写入 JSONL：

```json
{
  "key": "/user/preference/style",
  "ts": "2026-02-22T10:00:00Z",
  "valid": true,
  "source": { ... },
  "content": { ... }   // == json_content（原样保存）
}
```

* `key`：身份主键（LWW 单位）
* `ts`：系统写入时间（用于新鲜度排序与 LWW 判定）
* `valid`：当 `json_content == null` 时写入 `false`
* `source`：来源/证据链
* `content`：自由 JSON（推荐 object）

> 说明：这套 envelope 保证系统可以做一致性、过滤、排序；content 的自由度保证业务扩展不受 schema 约束。

---

## 4.2 key：逻辑路径 vs 物理路径（安全映射必须明确）

### 4.2.1 key 定义

* key 是逻辑路径，形式类似 URL path：`/namespace/category/name...`
* 推荐命名空间示例：

  * `/user/...`：用户相关（偏好、日程、长期事实）
  * `/kb/...`：外部知识沉淀（强制 provenance）
  * `/agent/...`：Agent 自身状态（可选）

### 4.2.2 物理落盘规则（必须）

系统必须保证所有目录索引文件落在 `memory_root` 下，并进行安全编码/规范化：

* 禁止 `..`、禁止 NUL、禁止换行等危险字符
* 连续 `/` 规范化为单 `/`
* 对不可安全落盘的字符做可逆编码（例如 percent-encoding）
* 对过长 segment 做截断 + hash（避免文件名长度限制）
* 物理路径示例：
  `memory_root/index/user/calendar/2026-02-23_10-00_牙科复诊@a1b2c3.json`

> 注：这里的 `@a1b2c3` 用于避免同名冲突并提高稳定性；具体编码方案由实现决定，但必须：**可预测、可重复、安全、不会逃逸 memory_root**。

---

## 4.3 双重存储结构（日志 + 目录索引，可选 state）

### 4.3.1 必选：日志（Log）

* `memory_root/log.jsonl`：所有 set_memory 调用的 envelope 追加写入（审计/回放）

### 4.3.2 必选：目录索引（Index）

* `memory_root/index/...`：按 key 层级展开的目录树
* 每个 key 对应一个“最新有效内容文件”（json 或 text 均可）
* tombstone 后移除该文件或标记为无效

### 4.3.3 推荐：状态快照（State）

为兼顾性能与默认读取稳定性，推荐引入：

* `memory_root/state.jsonl`：每个 key 的最新有效版本（compaction 产物）

> 如果不引入 state，也必须在 compaction 时归档旧 log，以免无限增长。

---

## 5. 默认读取（Passive Retrieval）

### 5.1 功能描述

每次 Agent 被唤醒（用户消息或 Timer Event），系统自动从 Memory 中构造一段“可嵌入 prompt 的记忆片段”。

默认读取是 **best-effort**：受 token_limit 限制，不承诺覆盖全量记忆。

### 5.2 输入参数

| 参数           | 类型        | 说明                     |
| ------------ | --------- | ---------------------- |
| token_limit  | number    | 允许嵌入提示词的最大 token       |
| tags         | string[]  | 上下文标签（可选），用于相关性排序的辅助信号 |
| current_time | timestamp | 当前时间，用于 expired 过滤等    |

### 5.3 生效规则（过滤 + LWW）

* 同 key 多次写入：以最后一条为准（按 `ts` 或写入顺序）
* `valid=false` 的 key 不进入默认读取
* 若 content 中存在 `expired_at` 且 `current_time > expired_at`：过滤掉（视为过期）
* 若 content 中存在 `importance`、`tags`：用于排序加权；缺省则按默认值处理

### 5.4 排序策略（保持 v2.0 的三原则，但适配自由 content）

默认读取在 token_limit 内返回综合最优的记忆，排序优先级：

| 优先级 | 维度     | 说明                                |
| --- | ------ | --------------------------------- |
| P0  | 新记忆优先  | `ts` 越近优先                         |
| P1  | 重要记忆优先 | `content.importance`（若存在）越高优先     |
| P2  | 相关记忆优先 | `content.tags` 与输入 tags 的匹配度（若存在） |

> tags 在检索中是“辅助信号”，不是硬检索；强检索需求由 Agent 主动 bash 查询完成。

### 5.5 输出格式（建议）

系统应输出紧凑、可追溯的片段，建议包含：

* key（用于 grep/ls 定位）
* type（若 content.type 存在）
* summary（若 content.summary 存在；否则从 content 提取短文本）

示例：

```
[Agent Memory]
- user/preference/style preference 用户喜欢中文、偏好简洁
- user/calendar/2026-02-23_10-00_牙科复诊 reminder 明天10点牙科复诊（一次性）
```

---

## 6. 主动查询（Active Query）

### 6.1 功能描述

Agent 使用标准 bash 工具对 `memory_root` 做查询，作为默认读取的补充，适用于“找特定记忆/批量浏览/定位日程列表”等场景。

### 6.2 实现要求

* 不新增专用查询工具，复用 `ls/find/grep/jq/cat` 等。
* 目录索引必须保证 `ls memory_root/index/user/calendar/` 能列出日程条目（按 key 层级）。
* JSONL 仍可 grep（用于审计与回放）。

### 6.3 典型用法（示例）

```bash
# 浏览所有日程条目（目录索引）
ls memory/index/user/calendar/

# 查找关键词（日志）
grep "牙科" memory/log.jsonl

# 用 jq 过滤有效提醒（如果 content 有 type/importance 等字段）
cat memory/log.jsonl | jq 'select(.valid==true and .content.type=="reminder")'
```

---

## 7. 整理淘汰（Memory Compaction）

### 7.1 目标

* 控制 log 增长、提升默认读取性能、修复索引一致性。
* 同时满足审计/回放与在线状态读取的需求。

### 7.2 系统侧确定性处理（必须）

| 操作           | 说明                                                       |
| ------------ | -------------------------------------------------------- |
| LWW 归并       | 同 key 多条记录，归并为最新状态                                       |
| tombstone 生效 | `valid=false` 的 key 在 state/index 中移除                    |
| expired 处理   | 若 content.expired_at 过期：按策略从 state/index 过滤（日志是否保留见 7.3） |
| 一致性修复        | 对比 state 与目录索引（或 log 与目录索引），修复不一致                        |
| 原子重建         | 采用“写新文件 + rename”方式替换 state，并重建 index                    |

### 7.3 历史策略（二选一，必须明确）

为避免“历史追溯”与“删除无效记录”冲突，必须选一种：

* **方案 A（推荐）**：log 永久保留 + state 作为快照

  * `log.jsonl` 不删除（或按周期归档但不丢）
  * compaction 产出 `state.jsonl`（在线使用）
* **方案 B**：log 归档轮转

  * compaction 前将旧 `log.jsonl` 移到 `archive/log_YYYYMMDD.jsonl`
  * 在线仅保留最新 log 或 state

---

## 8. 记忆生命周期

| 阶段     | 触发方式                               | 说明                                      |
| ------ | ---------------------------------- | --------------------------------------- |
| 创建/更新  | `set_memory(key, content, source)` | 同 key Upsert                            |
| 活跃     | 默认读取/主动查询                          | 被纳入 prompt 或被 Agent 查询使用                |
| 触发（可选） | Timer Event                        | 日程类记忆到点后 Agent 执行提醒（准点依赖 set_timer）     |
| 失效     | `set_memory(key, null, source)`    | tombstone 生效，目录索引移除                     |
| 整理     | compaction/self-improve            | 系统清理归并；Agent 做语义合并/降级（通过 set_memory 写回） |

---

## 9. 非功能性需求（NFR）

| 项目    | 要求                                                        |
| ----- | --------------------------------------------------------- |
| 写入可靠性 | JSONL 追加必须原子化；并发写入必须加锁或等效机制，保证不产生半行 JSON                  |
| 一致性   | index 与 state（或 log）不一致时可在 compaction 中确定性修复              |
| 性能    | 默认读取 ≤ 500ms；`ls` 浏览 index 近似即时；万级记录 grep ≤ 1s（视硬件可调）     |
| 可扩展性  | content 结构完全自由；系统只依赖 envelope 与少量可选字段做排序/过滤               |
| 可观测性  | set_memory、compaction、默认读取构造过程记录日志（至少含 key、ts、valid、来源摘要） |
| 无外部依赖 | 仅依赖本地文件系统与标准 Unix 工具                                      |

---

## 10. 约束与边界

* 本模块不提供向量检索或语义搜索；相关性只使用 tags（可选）与目录结构（key）。
* 本模块不实现 set_timer / Timer 调度，只服务于被唤醒后的记忆读取与写入。
* 本模块不包含 SendMsg 等业务动作。
* 单个在线 log 建议上限（如 100,000 行）触发 compaction 或归档轮转（具体阈值可配）。
* Agent 的 key 设计、content 质量、importance/tags 的标注质量依赖上层 prompt 策略与 LLM 能力。

---

## 附录 A：示例（对外工具调用层）

### A.1 写入用户偏好（对话来源，推荐也带 source）

```python
set_memory(
  "/user/preference/style",
  {"type":"preference","summary":"用户喜欢中文、偏好简洁","importance":6,"tags":["language","style"]},
  {"kind":"user","name":"chat","retrieved_at":"2026-02-22T10:00:00Z","locator":{"conversation_id":"c1","message_id":"m9"}}
)
```

### A.2 写入日程（通用存储；准点靠 set_timer）

```python
set_memory(
  "/user/calendar/2026-02-23_10-00_牙科复诊",
  {"type":"reminder","text":"明天10点牙科复诊","importance":8,"tags":["health"],"trigger_at":"2026-02-23T10:00:00-08:00"},
  {"kind":"user","name":"chat","retrieved_at":"2026-02-22T10:01:00Z","locator":{"conversation_id":"c1","message_id":"m10"}}
)
# 若用户要求准点：Agent 另外调用 set_timer(trigger_at, key) ——不属于 Memory 模块
```

### A.3 失效（tombstone）

```python
set_memory(
  "/user/calendar/2026-02-23_10-00_牙科复诊",
  None,
  {"kind":"agent","name":"reminder_done","retrieved_at":"2026-02-23T10:02:00-08:00","locator":{"reason":"sent"}}
)
```

### A.4 写入 KB（外部来源强制 provenance）

```python
set_memory(
  "/kb/product/iphone16/spec",
  {"type":"kb","data":{...},"summary":"iPhone16 主要规格汇总"},
  {"kind":"web","name":"apple.com","retrieved_at":"2026-02-22T10:05:00Z","locator":{"url":"https://..."}}
)
```

