# Agent Memory Module — 需求设计文档 v2.2


> **CLI 优先**：核心接口以单一可执行 `agent-memory` 暴露
>
> ```
> agent-memory [--root <memory_root>] set    <key> <content> <reason>
> agent-memory [--root <memory_root>] remove <key> [reason]
> agent-memory [--root <memory_root>] load   <tag1,tag2,tag3 ...>
> agent-memory [--root <memory_root>] list   [/dir1/dir2]
> ```
>
> 上层（Claude Code skills、其它 agent 框架）以 shell 命令形式调用；**接口不接收 JSON 入参**。`--root <memory_root>` 总是可选覆盖；不传时 CLI 只按默认本地 memory root / `AGENT_MEMORY_ROOT` 定位目录。



## 1. 概述

### 1.1 文档目的

定义 Agent Memory 模块的功能需求、数据模型、接口设计、存储布局与生命周期管理策略。Memory 模块作为 Agent 的基础设施，为跨会话持久化记忆提供统一的能力，支撑日程管理、用户偏好学习、任务跟踪、知识沉淀（KB）等上层场景。

v2.2 在 v2.1 的基础上引入：

* **CLI 优先 + 无 JSON 入参**：核心能力以单一可执行 `agent-memory` 暴露；写入/查询参数均为 positional。skills 通过 shell 命令直接调用，无需序列化 JSON。
* `agent-memory remove` 显式删除子命令（等价 `set` 写入空 content）。
* `agent-memory load` 基于倒排索引的标签批量召回。
* **content 默认纯文本**：通过 `agent-memory set <key> <content> <reason>` 直接提供；鼓励直接写入自然语言文本。
* **本地文件为唯一真相源**：`set`/`remove` 同时落地到本地索引文件与索引数据库（含倒排索引），冲突时**以本地文件为准**。`.log.jsonl` 仅作审计/回放，默认是隐藏文件（以 `.` 开头）。

### 1.2 核心设计哲学

Memory 模块遵循 **"基础设施 + Agent 自主决策"**：

* **CLI + skills 优先**：上层一律通过 `agent-memory` 命令行调用；接口不消费 JSON 入参，CLI 保持无状态，只读取和修改本地 `memory_root` 目录状态。
* **单一写入口原则**：Agent 只需要掌握一个写入子命令：`agent-memory set <key> <content> <reason>`；`agent-memory remove <key> [reason]` 是其失效语义的语法糖。
* **人机分工原则**：

  * LLM 擅长：决定记什么、如何命名 key、如何措辞 content（建议自然语言）、为何写入/失效、写入时挂哪些 tag、查询时给哪些 tag。
  * 系统擅长：可靠落盘、目录索引与倒排索引构建、默认读取排序、过期过滤、一致性修复、压缩重写。
* **content 文本优先**：建议 content 是**纯文本**（自然语言短句/段落），便于 LLM 直接消费、便于 grep。必须结构化时，也应先由上层把结构压成可读文本后传入 `<content>`。
* **本地文件即真相**：所有"在线状态"均由 `memory_root/index/...` 下的本地文件承载；索引数据库（含倒排索引）是这些文件的派生缓存，**冲突时以文件为准**。
* **审计文件隐藏**：`log.jsonl` 默认命名为 `.log.jsonl`（隐藏文件），仅用于审计与回放，不参与默认读取 / `agent-memory load` 的正常路径。
* **会话状态外置**：CLI 全无状态，每次执行只看本地 `memory_root` 目录；当前会话 tag、上下文权重等由上层 session 模块管理，本模块不参与。
* **轻量存储原则**：本地文件系统 + 一个轻量索引数据库（SQLite/嵌入式 KV 即可），不依赖向量库。
* **单目录初始化原则**：模块以单一 `memory_root` 目录初始化，所有持久化状态都落在该目录下；目录布局是**跨语言实现的兼容契约**（见 §4.1），任何语言都可以读/写同一个 `memory_root`。
* **自我进化原则**：Agent 在 self-improve 阶段进行语义整理；系统侧用确定性算法提供一致性与 compaction 兜底。

### 1.3 典型场景：Agent 日程管理（非专用、可涌现）

日程管理不是专用模块，而是 Agent 利用 Memory 基础设施形成的一种行为模式：

1. 用户提出日程/提醒需求（如"明天 10 点牙科复诊"）。
2. Agent 选择 key（如 `/user/calendar/2026-02-23_10-00_牙科复诊`），调用 `agent-memory set` 写入一条记录（content 可直接是文本"明天10点牙科复诊"）。
3. 若用户要求高精度，Agent 应调用上层能力 `set_timer`（不属于 Memory 模块）注册精确 Timer；否则可依赖 **3 分钟保底轮询**（best-effort）。
4. Timer Event 唤醒 Agent 后，Agent 可通过默认读取 / `agent-memory load` /（必要时）bash 主动查询定位相关记忆，决定是否提醒并执行 SendMsg。
5. 一次性提醒完成后，Agent 调用 `agent-memory remove <key>` 使其失效；重复提醒可更新内容（例如推进 next_trigger_at）。

> **精度声明**：准点提醒的强保证来自 `set_timer`；3 分钟轮询属于 best-effort（允许延迟与偶发遗漏），符合"通用系统"定位。

---

## 2. 系统架构：职责边界

### 2.1 Agent 侧职责（LLM 驱动，容错型）

| 职责                       | 说明                                                                                              |
| ------------------------ | ----------------------------------------------------------------------------------------------- |
| 调用 `agent-memory set`    | 主写入口：新增/更新                                                                                      |
| 调用 `agent-memory remove` | 失效/删除（等价 set 写入空 content）                                                                       |
| 决定 key                   | 自行组织命名空间与层级，如 `/user/calendar/...`、`/user/preference/...`、`/kb/...`                             |
| 组织 content / reason      | **优先纯文本**；调用 `agent-memory set <key> <content> <reason>` 时给出写入原因                                      |
| 调用 `agent-memory load`   | 在需要主动召回时传入逗号分隔 tag 集合批量取回（tag 集合的来源由上层 session 模块决定）                                      |
| 提供 reason                | 对外部/网络/工具信息必须在 reason 中给 provenance；对用户对话也推荐给可回溯来源                                                |
| 主动查询                     | 通过 bash（ls/find/grep/cat）检索 `memory_root/index/`                                                |
| self-improve             | 在整理阶段合并冗余、调整重要度、路径重构/归档（通过 set/remove 写回）                                                       |

### 2.2 系统侧职责（确定性算法，可靠型）

| 职责              | 说明                                                                       |
| --------------- | ------------------------------------------------------------------------ |
| 双写落盘            | `set`/`remove` 同时更新**本地索引文件**与**索引数据库**（含倒排索引）                          |
| 审计 JSONL        | 同步追加写入隐藏 `.log.jsonl`，并发安全                                               |
| key→目录索引        | 根据 key 构建目录索引文件（单文件对应单 key 的最新有效内容）                                      |
| 倒排索引            | 维护 `tag → keys` 倒排索引，供 `agent-memory load` 高效查询                          |
| 默认读取构造          | 按 token_limit 与排序策略返回嵌入 prompt 的 Memory 片段                               |
| 过滤与 LWW         | 过期过滤（`expired_at`），tombstone 过滤，同 key Last Write Wins                    |
| 一致性修复           | **以本地索引文件为唯一真相源**，从文件重建索引数据库与倒排索引                                        |
| Compaction/压缩   | 归档隐藏审计日志、重建索引、原子替换                                                       |

---

## 3. 接口设计（CLI 优先）

### 3.0 接口形态约束

* **唯一可执行**：`agent-memory`，所有能力以子命令暴露。
* **无 JSON 入参**：CLI 不接受 JSON 字符串参数，也不要求调用方传复杂结构。
* **CLI 无状态**：CLI 每次执行只看本地 `memory_root` 目录状态；不持有任何会话级状态（如当前会话 tag），这些由上层 session 模块管理。
* **content / reason 通过 positional 文本提供**：`set` 的三个业务参数固定为 `<key> <content> <reason>`。
* **无 JSON 出参（默认）**：默认输出对 LLM/人友好的纯文本（见 §3.6）。
* **退出码语义**：`0` 成功；`1` 一般错误（参数非法/校验失败）；`2` 写者锁/并发冲突；`3` 真相源损坏需修复；`64–78` 同 `<sysexits.h>`。

### 3.1 全局选项

```
agent-memory [--root <memory_root>] [--quiet] <verb> [...]
```

* `--root`：总是可选。传入时覆盖默认 memory root；不传时从 `AGENT_MEMORY_ROOT` 或运行时约定的本地默认目录推导。
* `--quiet`：抑制非错误日志，仅打印结果。

### 3.2 set — 写入 / 更新

```
agent-memory [--root <memory_root>] set <key> <content> <reason>
```

#### 3.2.1 行为语义

| 语义               | 调用方式                                                       | 说明                          |
| ---------------- | ---------------------------------------------------------- | --------------------------- |
| 新增/更新（Upsert）    | `agent-memory set <key> <content> <reason>`                | 同 key 覆盖更新；以最后写入为准（LWW）     |
| 失效/删除（Tombstone） | `agent-memory remove <key> ...`                            | 见 §3.3                      |

> **重要约定**：set 的 content 不可为空（不接受空字符串）。要清除一个 key，请使用 `remove`。

#### 3.2.2 系统侧写入行为（确定性）

收到 `set` 后系统必须执行（顺序敏感）：

1. **追加审计**：将一条标准 envelope 追加写入 `<memory_root>/.log.jsonl`（见 §4.2，隐藏文件）。
2. **写本地文件（真相源）**：将该 key 对应的目录索引文件原子写入最新内容（见 §4.4）。
3. **更新索引数据库**：刷新 `key → 元信息` 主表与 `tag → keys` 倒排索引。
4. 若步骤 3 失败：**不影响真相源**；下一次 compaction/启动校验会从文件重建数据库。

#### 3.2.3 轻量校验

* `key` 必须以 `/` 开头，且通过安全规范化（见 §4.3）。
* `reason` 必填；当内容来自 `web/tool/file` 或 key 落在 KB 命名空间时，reason 必须包含可回溯来源（见 §3.5）。
* content 字节数受 NFR 限制（默认上限见 §9）。
* tag 仅由 `load <tag1,tag2,tag3>` 查询时提供，必须满足 `[A-Za-z0-9_\-一-鿿]+`，禁止空白与控制字符。

#### 3.2.4 用法

```
agent-memory set /user/preference/style "用户喜欢中文、偏好简洁" "用户在会话 c1:m9 中明确表达"
```

---

### 3.3 remove — 失效（Tombstone）

```
agent-memory [--root <memory_root>] remove <key> [reason]
```

* 等价于"对该 key 写入 tombstone 标记"。
* 系统行为：追加 `valid=false` 的 envelope 到 `.log.jsonl`；删除/无效化本地索引文件；从倒排索引中清理该 key 的所有 tag 关联。
* 删除一个不存在的 key 不报错（幂等），退出码 0。
* 可选的reason可以保存在工作日志中

---

### 3.4 load — 按 tag 批量召回

```
agent-memory [--root <memory_root>] load <tag1,tag2,tag3> [--limit 4096]
```

#### 3.4.1 行为语义

1. 将 `<tag1,tag2,tag3>` 按逗号拆分为 tag 集合。
2. 在倒排索引中按 any 策略求并，得到候选 key 列表。
3. 过滤：`valid=false`、`expired_at` 已过期、安全检查不通过的 key 被剔除。
4. 排序：按 (新鲜度 ts、importance、命中 tag 数) 加权排序（详见 §5.4）。
5. 按实现默认上限截取并逐条从本地索引文件读取 content。
6. 单条 content 超过实现默认截断阈值时，截断为 `<前 N 字节> + "...[truncated, total=<size>B]"`。
7. 可以通过--limit传入对返回结果的token限制
8. 如果不传tag列表，行为为"*": 相当于所有的item都满足


#### 3.4.2 tag 集合的来源

`load` 必须显式传入逗号分隔 tag 列表，CLI 不维护任何 tag 集合状态。tag 集合从哪里来由调用方决定（用户/LLM 输入、上层 session 模块的当前会话上下文、固定主题等），本模块不规定。

#### 3.4.3 与默认读取的关系

* **默认读取**（§5）：每轮推理前由系统自动构造、写进 prompt 的 best-effort 片段，受 `token_limit` 严格限制。
* **`agent-memory load`**：上层 skills 主动调用，传入一组 tag，返回更完整的 key→content 列表，作为默认读取的补充与替代。

---

### 3.5 reason / provenance 规则（防污染硬约束）

#### 3.5.1 强制适用范围

满足任一条件时，**`reason` 必须包含可回溯来源**，否则禁止写入长期区（拒绝或隔离到 untrusted 命名空间）：

* content 来自 `web | tool | file`
* key 落在 KB 命名空间（如 `/kb/...` 或你们约定的长期知识区）

#### 3.5.2 reason 内容建议

| 来源类型 | reason 建议 |
|---|---|
| 用户对话 | 包含 conversation/message id 或可回溯上下文 |
| tool | 包含工具名、调用参数摘要、结果 id 或内容 hash |
| web | 包含 URL、站点名、抓取时间 |
| file | 包含文件路径、版本、digest 或 mtime |
| agent | 包含推理原因和触发事件 |

> CLI 只暴露一个 `reason` positional 参数，避免引入嵌套结构。需要复合定位（如 `conversation_id` + `message_id`）时，建议在 reason 字符串中以 `key=value` 拼接（如 `来自用户确认；conversation=c1,message=m9`）。

#### 3.5.3 示例

```
# web 来源
agent-memory set /kb/product/iphone16/spec "iPhone 16 规格摘要..." "web 来源；source=apple.com；url=https://apple.com/iphone16；retrieved_at=2026-02-22T10:00:00Z"

# user 对话
agent-memory set /user/preference/style "用户喜欢中文、偏好简洁" "用户对话；conversation=c1,message=m9"
```

---

### 3.6 输出格式

#### 3.6.1 text（默认，对 LLM/人友好）

`load` 输出每条记录使用**长度前缀**，可无歧义解析：

```
KEY <key>
SIZE <bytes>
TRUNCATED <0|1>
TAGS <tag1,tag2,...>            # 可空
TS <iso8601>
---
<恰好 SIZE 字节的 content>
```

记录之间无分隔符；解析端按 `KEY` 行识别下一条；读完 SIZE 字节后回到等待 `KEY` 状态。

> 文件名/key 已禁止换行与 NUL（§4.1.5），content 中的换行也不会破坏解析（依赖字节计数）。

### 3.7 辅助子命令

```
agent-memory [--root <memory_root>] init          # 初始化目录（写 .meta.json/.log.jsonl/.lock/index/）
agent-memory [--root <memory_root>] get <key>     # 直接打印一个 key 的 content（仅 stdout）
agent-memory [--root <memory_root>] list <prefix> # 列出某逻辑前缀下的全部 key（每行一个）
agent-memory [--root <memory_root>] verify        # 校验/修复 index/ ↔ db/ 一致性
agent-memory [--root <memory_root>] compact       # 触发 compaction（见 §7）
```

`get` / `list` 纯按 key/前缀工作，便于 bash 流水线调试。

---

## 4. 数据模型与存储布局

### 4.1 初始化与目录布局（跨语言兼容契约）

#### 4.1.1 初始化签名

Memory 模块以**单一目录路径** `memory_root` 进行初始化：

```
AgentMemory(memory_root: string)
```

* `memory_root` 必须是绝对路径，指向一个存在或可创建的目录。
* 模块负责在该目录下维护下文规定的布局；**该布局是跨语言实现的兼容契约**——任何语言/进程读到一个由其它实现写入的 `memory_root` 时，都能正确重建在线状态。
* 同一 `memory_root` **只允许一个写者**（见 §4.1.6）；只读访问无并发限制。

#### 4.1.2 目录树（规范）

```
<memory_root>/
├── .meta.json                   # 必选：模块元信息（版本、编码方案）
├── .log.jsonl                   # 必选：隐藏审计日志（追加写）
├── .lock                        # 必选：写者进程文件锁
├── index/                       # 必选：本地索引文件（唯一真相源）
│   └── <ns>/<...>/<filename>@<hash>.{txt,json}
├── db/                          # 推荐：索引数据库（派生缓存）
│   └── memory.sqlite            #   schema 见 4.1.4
├── .archive/                    # 可选：审计日志归档
│   └── log_YYYYMMDD.jsonl
└── .state.jsonl                 # 可选：启动加速快照
```

约束：
* `.meta.json`、`.log.jsonl`、`.lock`、`index/` 必须存在；其余按实现可选。
* 所有以 `.` 开头的路径**默认隐藏**，`ls` 不展示；默认读取与 `agent-memory load` 不扫描隐藏路径。
* 跨语言读取 `memory_root` 时，**只读 `.meta.json` + `index/` 即可恢复完整在线状态**；`db/`、`.state.jsonl` 均为派生缓存，可任意丢弃重建。

#### 4.1.3 .meta.json 契约（自描述）

`.meta.json` 在第一次初始化时写入，描述本 `memory_root` 的版本与编码方案。**版本/编码不兼容时挂载方必须拒绝写入**（可降级为只读）：

```json
{
  "schema_version": "2.2",
  "writer": {
    "lang": "rust",
    "impl": "agent-memory-rs",
    "version": "0.3.1"
  },
  "encoding": {
    "key_to_path": "percent",        // key→path 编码方案
    "long_segment": "trunc+hash",    // 长段处理
    "hash_algo": "blake3-8",         // hash 类型与字节数（hex 长度=2N）
    "filename_suffix": "@<hash>",    // 物理文件名格式
    "max_segment_bytes": 200         // 单段截断阈值（UTF-8 字节）
  },
  "created_at": "2026-05-08T10:00:00Z"
}
```

* `schema_version` 采用 SemVer-lite（`major.minor`）；major 不一致 → 拒绝挂载，minor 不一致 → 允许只读。
* `encoding` 不被支持时同样拒绝写入，但允许只读模式。

#### 4.1.4 db/memory.sqlite 推荐 schema

数据库是派生缓存，schema **不是**强约束（实现可用其它嵌入式 KV，需在 `.meta.json` 中扩展声明）。推荐 SQLite schema 如下，作为跨语言对齐的参考：

```sql
CREATE TABLE memory (
  key            TEXT PRIMARY KEY,
  file_path      TEXT NOT NULL,         -- 相对 memory_root/index/ 的路径
  content_type   TEXT NOT NULL,         -- 'text' | 'json'
  ts             TEXT NOT NULL,         -- ISO8601
  valid          INTEGER NOT NULL,      -- 0/1
  importance     INTEGER,               -- 可空
  expired_at     TEXT,                  -- 可空
  source_summary TEXT,
  content_size   INTEGER NOT NULL
);

CREATE TABLE memory_tag (
  tag TEXT NOT NULL,
  key TEXT NOT NULL,
  PRIMARY KEY (tag, key)
);

CREATE INDEX idx_memory_tag_key ON memory_tag(key);
CREATE INDEX idx_memory_ts      ON memory(ts);
CREATE INDEX idx_memory_imp     ON memory(importance);
```

#### 4.1.5 key → 物理文件名（默认编码方案）

`.meta.json.encoding` 约束下的默认方案（其它实现可替换，但必须在 `.meta.json` 中声明并自洽）：

1. `key` 必须以 `/` 开头，按 `/` 切分为 segments；空 segment 与 `..` 段直接拒绝。
2. 每个 segment 做 RFC3986-style **percent-encoding**：保留 unreserved + 中日韩等可显示 UTF-8 字节；编码 `/`、NUL（`%00`）、换行（`%0A`）、控制字符等。
3. 末尾 segment（filename）若 UTF-8 字节数 > `max_segment_bytes`（默认 200），截断到前 N 字节并附 `@<hash>`。
4. 文件名后缀：
   * `.txt`：content 为字符串
   * `.json`：content 为 JSON object/array/number/bool/null（注意 null 表示 tombstone，通常不落盘）
5. `hash` = `key` 全文的 `blake3` 前 8 字节十六进制（共 16 个 hex 字符），**对同一 key 必须可重现**。
6. 物理路径 = `<memory_root>/index/<encoded_seg1>/<encoded_seg2>/.../<encoded_filename>@<hash>.<ext>`。

示例（key = `/user/calendar/2026-02-23_10-00_牙科复诊`）：

```
<memory_root>/index/user/calendar/2026-02-23_10-00_%E7%89%99%E7%A7%91%E5%A4%8D%E8%AF%8A@a1b2c3d4e5f60718.txt
```

#### 4.1.6 原子写入与并发约束

* `index/<...>.<ext>`：写到同目录的 `<file>.tmp.<rand>` 后 `rename`（POSIX 原子）。
* `db/memory.sqlite`：使用事务；整库重建时写到 `db/memory.sqlite.new` 后 `rename`。
* `.log.jsonl`：`O_APPEND` + 单条 envelope `fsync`；并发安全（在 `.lock` 持有期间为单写者，无锁竞争）。
* `.meta.json`：仅初始化时写入；后续升级使用 `<file>.tmp + rename`。
* **写者锁**：写入端在 `<memory_root>/.lock` 持有 POSIX `flock` / Windows `LockFileEx`；同一 `memory_root` 同时只允许一个写者。
* **只读端**（包括其它语言实现的 `load`/纯浏览）：可不加锁，但必须容忍瞬时不一致——遇到 `index/` 与 `db/` 不一致时**一律以 `index/` 为准**（见 §4.4.3）。

#### 4.1.7 跨语言互操作步骤

任何语言实现接管一个已存在的 `memory_root` 时，规范操作：

1. 读取 `.meta.json`，校验 `schema_version` 与 `encoding`；不兼容则报错或降级为只读。
2. 获取 `.lock`（写者）或跳过加锁（只读）。
3. （可选）扫描 `index/` 重建 `db/`；或直接信任既有 `db/` 但保证启动期做一次轻量校验。
4. 后续 `agent-memory set` / `agent-memory remove` 严格按 §4.1.5 / §4.1.6 写入。

> 这套契约的目标：让任何语言（Rust / Python / TS / Go ……）只要遵循 §4.1.2–§4.1.6，就能在同一 `memory_root` 上互操作而不破坏数据。

---

### 4.2 审计 JSONL：标准 envelope（隐藏文件）

系统内部每次写入（包括 tombstone）都以 envelope 形式追加到隐藏 JSONL：

* 路径：`memory_root/.log.jsonl`（**默认以 `.` 开头隐藏**）
* 仅用于审计/回放，不参与在线读取的正常路径。
* 当索引数据库或本地文件丢失/损坏时，可作为辅助回放手段，但**真相源仍是本地索引文件**。

envelope 结构：

```json
{
  "key": "/user/preference/style",
  "ts": "2026-02-22T10:00:00Z",
  "valid": true,
  "source": { ... },
  "content": "用户喜欢中文、偏好简洁",
  "tags": ["language","style"]
}
```

* `key`：身份主键（LWW 单位）
* `ts`：系统写入时间（用于新鲜度排序与 LWW 判定）
* `valid`：当 `content == null` 时写入 `false`
* `source`：来源/证据链
* `content`：**优先字符串**；亦允许 object
* `tags`：可选；提取自 content（若为 object 含 `tags` 字段）或 Agent 显式传入

---

### 4.3 key：逻辑路径 vs 物理路径（安全映射必须明确）

#### 4.3.1 key 定义

* key 是逻辑路径，形式类似 URL path：`/dir1/dir2/.../filename`。
* **命名规则不变**：依然是 `dir1/dir2/filename` 的层级形式。
* 推荐命名空间示例：

  * `/user/...`：用户相关（偏好、日程、长期事实）
  * `/kb/...`：外部知识沉淀（强制 provenance）
  * `/agent/...`：Agent 自身状态（可选）

#### 4.3.2 物理落盘规则（必须）

系统必须保证所有目录索引文件落在 `memory_root/index/` 下，并按 §4.1.5 的编码方案规范化：

* 禁止 `..`、禁止 NUL、禁止换行等危险字符
* 连续 `/` 规范化为单 `/`
* 对不可安全落盘的字符做可逆编码（默认 percent-encoding，见 §4.1.5）
* 对过长 segment 做截断 + hash（默认 `blake3-8`）
* 物理路径示例：
  `memory_root/index/user/calendar/2026-02-23_10-00_牙科复诊@a1b2c3d4e5f60718.txt`（content 为字符串时）
  `memory_root/index/user/calendar/2026-02-23_10-00_牙科复诊@a1b2c3d4e5f60718.json`（content 为 object 时）

> 注：`@<hash>` 后缀用于避免同名冲突并提高稳定性；编码方案在 `.meta.json.encoding` 中声明，必须：**可预测、可重复、安全、不会逃逸 memory_root**。

---

### 4.4 三层存储结构（本地文件 + 索引数据库 + 隐藏审计日志）

#### 4.4.1 必选：本地索引文件（**唯一真相源**）

* 路径：`memory_root/index/...`
* 每个 key 对应一个最新有效内容文件：
  * content 为字符串：直接以纯文本存储（推荐 `.txt`），首选 UTF-8 无 BOM。
  * content 为 JSON object：以 `.json` 存储。
* tombstone 后**删除该文件**（或写入 tombstone 标记，由实现决定）。
* `ls memory_root/index/...` 必须能直接浏览所有 key 的最新有效内容。

#### 4.4.2 必选：索引数据库（派生缓存）

轻量数据库（SQLite/嵌入式 KV 即可），承载：

* 主表：`key → {file_path, ts, valid, importance, expired_at, source_summary, tags, content_size}`。
* **倒排索引**：`tag → [key, ...]`，用于 `agent-memory load` 的高效查询。
* 可选辅助索引：`(namespace, ts)`、`(importance, ts)` 等。

> 数据库**仅是缓存**，可随时由 `memory_root/index/` 重建；启动/compaction 阶段必须做"文件→数据库"的一致性校验。

#### 4.4.3 冲突解决（强约束）

* **本地文件 vs 索引数据库不一致**：以**本地文件**为准。删除数据库中多余条目、补齐缺失条目、修正陈旧字段。
* **本地文件 vs 审计日志（`.log.jsonl`）不一致**：以**本地文件**为准；审计日志只用于人工排查与最终回放。
* **多份本地文件冲突**（理论上不应发生）：按 mtime 最新者为准，并在审计日志中记录冲突事件。

#### 4.4.4 必选：隐藏审计日志

* 路径：`memory_root/.log.jsonl`
* 默认隐藏（`.` 前缀）。
* 所有 `agent-memory set` / `agent-memory remove` 调用追加 envelope；并发安全（fsync + 锁）。
* compaction 阶段可归档为 `memory_root/.archive/log_YYYYMMDD.jsonl`。

#### 4.4.5 推荐：状态快照

为大规模场景兼顾启动速度，可选生成 `memory_root/.state.jsonl`（隐藏快照）：

* 每个 key 一行，最新有效 envelope。
* 仅作为加速首次启动的可选缓存，**不影响"本地文件为真相源"的约束**。

---

## 5. 默认读取（Passive Retrieval）

### 5.1 功能描述

每次 Agent 被唤醒（用户消息或 Timer Event），系统自动从 Memory 中构造一段"可嵌入 prompt 的记忆片段"。

默认读取是 **best-effort**：受 token_limit 限制，不承诺覆盖全量记忆。

### 5.2 输入参数

| 参数             | 类型        | 说明                                |
| -------------- | --------- | --------------------------------- |
| token_limit    | number    | 允许嵌入提示词的最大 token                  |
| tags           | string[]  | 与上下文相关的 tag 列表（由调用方提供，可选）         |
| current_time   | timestamp | 当前时间，用于 expired 过滤等               |

### 5.3 生效规则（过滤 + LWW）

* 同 key 多次写入：以最后一条为准（按 `ts` 或写入顺序）
* `valid=false` 的 key 不进入默认读取
* 若 content 中存在 `expired_at` 且 `current_time > expired_at`：过滤掉（视为过期）
* 若 content 中存在 `importance`、`tags`：用于排序加权；缺省则按默认值处理

### 5.4 排序策略

默认读取与 `agent-memory load` 共享同一套排序优先级：

| 优先级 | 维度          | 说明                              |
| --- | ----------- | ------------------------------- |
| P0  | 新记忆优先       | `ts` 越近优先                       |
| P1  | 重要记忆优先      | `content.importance`（若存在）越高优先   |
| P2  | tag 命中数     | 候选 key 的 tag 与输入 `tags` 的交集大小   |

> tags 在默认读取中是"辅助信号"；**强检索/批量召回**请使用 `agent-memory load`。

### 5.5 输出格式（建议）

系统应输出紧凑、可追溯的片段，建议包含：

* key（用于 grep/ls 定位）
* 可选 type/importance（若 content.type/importance 存在）
* summary（content 为字符串时取前若干字符；为 object 时优先取 `content.summary`）

示例：

```
[Agent Memory]
- /user/preference/style 用户喜欢中文、偏好简洁
- /user/calendar/2026-02-23_10-00_牙科复诊 [reminder] 明天10点牙科复诊
```

---

## 6. 主动查询（Active Query）

### 6.1 功能描述

Agent 可使用两种方式主动查询：

1. **`agent-memory load <tag1,tag2,tag3>`**：CLI 子命令，结构化、走倒排索引（推荐）。
2. **bash 通用工具**对 `memory_root/index/` 做查询（兜底，自由）。

### 6.2 实现要求

* `agent-memory load` 由倒排索引驱动，必须满足 §9 的性能 NFR。
* bash 路径不新增专用查询工具，复用 `ls/find/grep/cat`。
* `memory_root/index/` 必须可被 `ls` 直接浏览（按 key 层级展开）。
* 隐藏文件 `.log.jsonl` 仅作审计用途，不建议作为常规查询入口（必要时使用 `ls -a` 与 `grep` 配合）。

### 6.3 典型用法（CLI + bash）

```bash
# —— CLI 路径（推荐） ——

# 按 tag 召回
agent-memory --root "$MEMROOT" load calendar,提醒

# 列出某前缀下全部 key
agent-memory --root "$MEMROOT" list /user/calendar

# 直接打印一个 key 的 content
agent-memory --root "$MEMROOT" get /user/preference/style

# —— bash 兜底路径 ——

# 浏览所有日程条目（目录索引）
ls "$MEMROOT/index/user/calendar/"

# 直接查看某条记忆（content 默认是纯文本）
cat "$MEMROOT/index/user/preference/style@xxxx.txt"

# 审计日志（隐藏文件，需要 -a / 显式路径）
grep "牙科" "$MEMROOT/.log.jsonl"
```

---

## 7. 整理淘汰（Memory Compaction）

### 7.1 目标

* 控制 `.log.jsonl` 增长、修复索引数据库一致性。
* 同时满足审计/回放与在线状态读取的需求。

### 7.2 系统侧确定性处理（必须）

| 操作                | 说明                                                                |
| ----------------- | ----------------------------------------------------------------- |
| 文件→数据库重建          | 以 `memory_root/index/` 为唯一真相源，扫描重建主表与倒排索引                         |
| LWW 归并            | 多条 envelope 历史归并为最新状态（仅作为审计辅助，不影响真相源）                             |
| tombstone 生效      | 已 tombstone 的 key 在数据库与倒排索引中移除                                    |
| expired 处理        | 若 content.expired_at 过期：从数据库与倒排索引中过滤；本地文件按策略保留或删除（见 7.3）         |
| 一致性修复             | 校验文件 ↔ 数据库；冲突一律以文件为准                                              |
| 原子重建              | 采用"写新 db 文件 + rename"方式替换，并刷新倒排索引                                 |
| 审计日志归档            | 将 `.log.jsonl` 移到 `.archive/log_YYYYMMDD.jsonl`，新建空 `.log.jsonl` |

### 7.3 历史策略（二选一，必须明确）

* **方案 A（推荐）**：审计日志永久保留 + 索引数据库作为快照

  * `.log.jsonl` 不删除（按周期归档但不丢）
  * 索引数据库与本地文件构成在线状态
* **方案 B**：审计日志归档轮转

  * compaction 前将旧 `.log.jsonl` 移到 `.archive/log_YYYYMMDD.jsonl`
  * 在线仅保留最新 log

---

## 8. 记忆生命周期

| 阶段     | 触发方式                                  | 说明                                      |
| ------ | ------------------------------------- | --------------------------------------- |
| 创建/更新  | `agent-memory set <key> <content> <reason>`      | 同 key Upsert，文件+数据库双写                   |
| 活跃     | 默认读取 / `agent-memory load` / 主动查询              | 被纳入 prompt 或被 Agent 查询使用                |
| 触发（可选） | Timer Event                                      | 日程类记忆到点后 Agent 执行提醒（准点依赖 set_timer）     |
| 失效     | `agent-memory remove <key>`                      | tombstone 生效，本地文件删除，倒排索引清理              |
| 整理     | compaction/self-improve               | 系统从文件重建数据库；Agent 做语义合并/降级（通过写回）         |

---

## 9. 非功能性需求（NFR）

| 项目           | 要求                                                                          |
| ------------ | --------------------------------------------------------------------------- |
| 写入可靠性        | JSONL 追加必须原子化；并发写入必须加锁或等效机制，保证不产生半行 JSON                                    |
| 双写一致性        | 文件写入与数据库更新允许暂态不一致；但启动/compaction 必须能从文件确定性重建数据库                            |
| 真相源约束        | 文件 ↔ 数据库冲突一律以**本地文件**为准                                                     |
| 默认读取性能       | ≤ 500ms                                                                     |
| load 性能         | `agent-memory load` 在万级 key 下 ≤ 100ms（含 stdout 写出）；返回截断后单条 ≤ 4KB              |
| 浏览性能         | `ls memory_root/index/...` 近似即时                                             |
| 审计 grep 性能   | 万级记录 grep `.log.jsonl` ≤ 1s（视硬件可调）                                          |
| 可扩展性         | content 既允许字符串也允许 object；系统只依赖 envelope 与少量可选字段做排序/过滤                       |
| 可观测性         | set、remove、load、compaction 子命令的执行过程记录日志（key、ts、tag 命中、来源摘要）                  |
| 无外部依赖        | 仅依赖本地文件系统、轻量嵌入式数据库与标准 Unix 工具                                                |

---

## 10. 约束与边界

* 本模块不提供向量检索或语义搜索；相关性来自 tags（倒排索引）与目录结构（key）。
* 本模块不实现 set_timer / Timer 调度，只服务于被唤醒后的记忆读取与写入。
* 本模块不包含 SendMsg 等业务动作。
* `.log.jsonl` 单文件建议上限（如 100,000 行）触发 compaction 或归档轮转（具体阈值可配）。
* CLI 不持有任何会话级状态；调用方负责把当前需要的 tag 列表传入 `agent-memory load`。跨会话语义请通过 `agent-memory set` 持久化。
* Agent 的 key 设计、content 质量、importance/tags 的标注质量依赖上层 prompt 策略与 LLM 能力。

---

## 附录 A：示例（CLI 调用层）

> 以下示例假设 `export AGENT_MEMORY_ROOT=/path/to/memory_root` 已设置；省略 `--root` flag。

### A.1 初始化（一次性）

```bash
agent-memory init
```

### A.2 写入用户偏好（content 为纯文本）

```bash
agent-memory set /user/preference/style \
    "用户喜欢中文、偏好简洁" \
    "用户对话；conversation=c1,message=m9"
```

### A.3 写入日程（content 与 reason 都是纯文本）

```bash
agent-memory set "/user/calendar/2026-02-23_10-00_牙科复诊" \
    "明天10点牙科复诊；若未完成，2026-02-24 后可忽略" \
    "用户对话；conversation=c1,message=m10"

# 若用户要求准点：另外调用 set_timer ——不属于 Memory 模块
```

### A.4 失效（remove，推荐）

```bash
agent-memory remove "/user/calendar/2026-02-23_10-00_牙科复诊"
```

### A.5 写入 KB（外部来源强制 provenance）

```bash
agent-memory set /kb/product/iphone16/spec \
    "iPhone 16 规格摘要..." \
    "web 来源；source=apple.com；url=https://apple.com/iphone16/spec"
```

### A.6 按 tag 召回

```bash
# 上层（用户/LLM/session 模块）给出一组 tag，调用 load：
agent-memory load calendar,牙科,提醒

# 输出（默认 text 格式，按 §3.6.1 解析）：
# KEY /user/calendar/2026-02-23_10-00_牙科复诊
# SIZE 24
# TRUNCATED 0
# TAGS health,calendar,牙科
# TS 2026-02-22T10:01:00Z
# ---
# 明天10点牙科复诊
# KEY /user/preference/style
# SIZE 27
# TRUNCATED 0
# TAGS language,style
# TS 2026-02-22T10:00:00Z
# ---
# 用户喜欢中文、偏好简洁
```

### A.7 显式 tag 召回 + 截断阈值

```bash
agent-memory load product,iphone16

# 单条 content 超过 4KB 时，body 末尾会出现：
# ...[truncated, total=18234B]
# 同时头部 TRUNCATED 1
```
