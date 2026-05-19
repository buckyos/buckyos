# Agent Memory Module — 核心组件规格 v2.8（优化版）

> 本文档定义 Agent Memory 核心组件的稳定契约：CLI、数据模型、存储布局、一致性、检索与恢复策略。  
> 目标是让实现者能快速、准确地实现同一个 `memory_root` 协议；理念、长场景和 ADR 仅保留必要摘要。

---

## 0. Core Contract

- `agent-memory` 是唯一核心入口；上层 Agent / Skill 通过 shell 命令调用，不向 CLI 传 JSON。
- `memory_root` 是一个本地目录；所有持久化状态都在该目录内。
- 业务内容以纯 UTF-8 文本文件保存；每个 logical key 对应一个业务内容文件。
- `.meta/log.jsonl` 是 append-only 审计日志，也是在线状态真相源的一部分。
- `memory.sqlite` 是派生缓存，可删除、重建；不能作为唯一真相源。
- 在线状态由“按日志顺序 replay 后的最新 envelope”与“业务内容文件”联合决定。
- LWW 以日志顺序为准；`ts` 只用于展示、排序和审计，不用于判定最后写入。
- 同一 `memory_root` 同时只允许一个写者；只读端允许并发，但必须容忍瞬时不一致。
- `set` 写入最新内容；`remove` 写入 tombstone；`load` 接收有序英语 tags 并返回相关有效记忆。
- CLI 不维护 session 状态，不构造 tags，不翻译，不做语言检测。
- v2.8 仅支持 `primary_language = "en"`；写入英语 content 是上层 Agent 的责任。

---

## 1. 范围与非目标

### 1.1 组件做什么

Agent Memory 为 Agent 提供跨会话持久化记忆能力，包括：

- 写入、更新、失效一条记忆。
- 按 key 直接读取或列举。
- 按 ordered tags 召回相关记忆。
- 维护本地内容文件、审计日志与 FTS5 索引。
- 在 crash、索引损坏、文件残留等场景下提供确定性恢复规则。

### 1.2 组件不做什么

- 不实现向量检索或语义搜索。
- 不实现 Timer、SendMsg、任务调度等业务动作。
- 不维护当前会话 tags、滑窗或上下文状态。
- 不判断一条信息是否“值得记”；这由上层 Agent 决定。
- 不翻译，不检测 content 语言。
- 不保证 `load` 是事务快照；浮现式读取允许 best-effort。

---

## 2. 关键概念

| 概念 | 定义 |
|---|---|
| `memory_root` | 本地根目录，包含业务文件、`.meta/` 与 `memory.sqlite`。 |
| key | 逻辑路径，形如 `/user/preference/style`。 |
| content | 纯 UTF-8 文本，可选系统前言 + 主体。 |
| reason | 写入原因和来源说明，通过 `--reason` 传入；不参与全文索引。 |
| envelope | `.meta/log.jsonl` 中的一行 JSON，记录一次 set/remove。 |
| tombstone | `valid=false` 的 envelope，表示该 key 已失效。 |
| FTS5 index | 从 key + content 派生的 SQLite FTS5 索引；仅是缓存。 |
| ordered tags | `load` 查询词列表，顺序表示优先级。 |

---

## 3. CLI 契约

### 3.1 全局形态

```bash
agent-memory [--root <memory_root>] [--quiet] <verb> [...]
```

- `--root`：覆盖默认 memory root。不传时从 `AGENT_MEMORY_ROOT` 或实现约定的本地默认目录推导。
- `--quiet`：抑制非错误日志，不改变退出码。
- 默认输入不接受 JSON；默认输出为纯文本。

### 3.2 退出码

| 退出码 | 含义 |
|---:|---|
| `0` | 成功，包括幂等成功。 |
| `1` | 参数、校验或普通运行错误。 |
| `2` | 写者锁冲突或等待超时。 |
| `3` | 真相源损坏，无法自动修复或读取。 |
| `64–78` | 可选使用 `<sysexits.h>` 语义。 |

---

## 4. 子命令

### 4.1 `init`

```bash
agent-memory [--root <memory_root>] init
```

初始化目录，创建 `.meta/`、`.meta/meta.json`、`.meta/log.jsonl`、`.meta/lock` 和 `memory.sqlite`。

规则：

- 幂等；已初始化时退出 `0`。
- `primary_language` 固定写入 `"en"`。
- 已初始化目录的 `primary_language`、`schema_version.major`、`encoding` 不兼容时，拒绝写入。

### 4.2 `set`

```bash
# 形态 A：短 content，经 argv 传入
agent-memory [--root <memory_root>] set <key> <content> --reason <reason>

# 形态 B：长 content，经 stdin 传入
agent-memory [--root <memory_root>] set <key> --reason <reason>
```

消歧规则只看 positional 数量：

| positional 数 | 行为 |
|---:|---|
| `2` | `<content>` 来自 argv；忽略 stdin。 |
| `1` | content 必须从 stdin 读取；stdin 是 tty 或 0 字节则退出 `1`。 |
| `0` 或 `>=3` | 退出 `1`。 |

校验规则：

- key 必须符合 §6。
- content 必须非空、UTF-8、无 BOM。
- `--reason` 必填且非空。
- content 超过 256KB 时，CLI 应打印 warning；是否设置硬上限由实现决定。

#### 4.2.1 `set` 原子写入顺序

`set` 的提交点是“`valid=true` envelope 成功 fsync”。由于 envelope 不包含 content，必须先确保业务文件已落盘，再追加 envelope。

规范顺序：

1. 获取 `.meta/lock`。
2. 校验 key、reason、content。
3. 将 content 写入同目录临时文件 `<file>.tmp.<rand>`。
4. `fsync` 临时文件。
5. `rename(temp, final)`。
6. `fsync` 父目录。
7. 追加 `valid=true` envelope 到 `.meta/log.jsonl`，使用 `O_APPEND` 并 `fsync`。
8. 在 SQLite 事务中更新 `memory` 与 `memory_fts`。
9. 释放锁。

失败语义：

- 步骤 7 成功后，写入视为已提交。
- 步骤 8 失败不影响真相源；后续 `verify` / `compact` 可重建索引。
- 若步骤 3–6 成功但步骤 7 失败，文件只是未提交的 orphan；`verify` 应报告或隔离，不应让它自动进入在线状态。

### 4.3 `remove`

```bash
agent-memory [--root <memory_root>] remove <key> [--reason <reason>]
```

`remove` 写入 tombstone，不要求 key 当前存在。删除不存在的 key 退出 `0`。

规范顺序：

1. 获取 `.meta/lock`。
2. 校验 key；reason 可选。
3. 追加 `valid=false` envelope 到 `.meta/log.jsonl`，`O_APPEND` + `fsync`。
4. 删除业务内容文件；若文件不存在，不报错。
5. 在 SQLite 事务中标记 `valid=0` 或删除索引行。
6. 释放锁。

失败语义：

- tombstone envelope 成功后，该 key 即失效。
- 若 crash 发生在 tombstone 后、文件删除前，恢复时以 tombstone 为准，残留文件必须被忽略或删除。

### 4.4 `load`

```bash
agent-memory [--root <memory_root>] load <tag1,tag2,tag3> [--max-bytes N] [--max-records N]
```

行为：

1. 按逗号拆分 tags，trim 空白。
2. 每个 tag 必须符合 §8.3。
3. tags 之间是并集召回：任一 tag 命中即可成为候选。
4. tag 顺序只影响排序 boost，不影响召回范围。
5. 过滤 tombstone、过期项和不可读项。
6. 按 §8.5 排序。
7. 同时应用 `--max-records` 与 `--max-bytes`。

默认：

- `--max-records` 默认 `50`。
- `--max-bytes` 默认 `65536`。
- 单条输出 body 超过 4KB 时可截断，头部 `TRUNCATED 1`。
- 不传 tags 或传入 `*` 表示全量候选，按 `ts desc, importance desc, key asc` 排序。

### 4.5 `get`

```bash
agent-memory [--root <memory_root>] get <key>
```

直接输出 key 对应的完整 content，不加头部。若最新 envelope 为 tombstone 或文件不可读，退出 `1` 或 `3`。

### 4.6 `list`

```bash
agent-memory [--root <memory_root>] list [/prefix]
```

列出某逻辑前缀下的有效 key；无参数等价于 `list /`。每行一个绝对 key。

### 4.7 `verify`

```bash
agent-memory [--root <memory_root>] verify [--repair]
```

检查 `.meta/log.jsonl`、业务文件与 `memory.sqlite` 的一致性。

- 无 `--repair`：只报告问题，不修改真相源；发现不可读有效 key 时退出 `3`。
- 有 `--repair`：可重建 SQLite、删除 tombstone 残留文件、隔离 orphan 文件。是否“采纳外部修改文件”为新 envelope 由实现选项控制，不应作为默认静默行为。

### 4.8 `compact`

```bash
agent-memory [--root <memory_root>] compact
```

执行日志归档、状态快照和索引重建。见 §9。

---

## 5. 输出格式

### 5.1 `load` 默认文本格式

每条记录使用长度前缀，并以 `END` 明确结束：

```text
KEY <key>
SIZE <n>
TRUNCATED <0|1>
MATCHED <comma-separated-tags>
TS <iso8601>
---
<exactly n bytes of UTF-8 content>
END
```

解析规则：

1. 读取 header，直到 `---\n`。
2. 读取恰好 `SIZE` 字节作为 content。
3. 再读取一个换行和 `END\n`；这部分不计入 `SIZE`。
4. 重复读取下一条 `KEY`。

说明：

- `SIZE` 是输出 body 的 UTF-8 字节数；若截断，则是截断后 body 的字节数。
- `MATCHED` 可为空。
- content 可以包含任意换行和特殊字符；解析只依赖 `SIZE`。
- key 禁止换行和 NUL，因此 header 可按行解析。

---

## 6. key 与物理路径

### 6.1 key 规则

- key 是逻辑路径，必须以 `/` 开头。
- 连续 `/` 规范化为单 `/`。
- 空 segment、`.`、`..`、NUL、换行、控制字符直接拒绝。
- 第一段不得为 `.meta` 或 `memory.sqlite`。
- 每个 segment 的 UTF-8 字节数必须 `<= 200`。
- key 推荐使用小写英语、数字、`-`、`_`。

推荐命名空间：

| namespace | 用途 |
|---|---|
| `/user/...` | 用户偏好、长期事实、日程等。 |
| `/kb/...` | 外部知识沉淀；要求 provenance。 |
| `/agent/...` | Agent 自身状态或运行笔记。 |
| `/glossary/...` | 项目术语和规范翻译。 |

### 6.2 key → path 编码

默认编码方案写在 `.meta/meta.json.encoding` 中：

```json
{
  "key_to_path": "percent",
  "max_segment_bytes": 200,
  "filename_format": "bare"
}
```

映射规则：

1. 去掉 key 开头的 `/`，按 `/` 切分 segments。
2. 每个 segment 做 RFC3986-style percent-encoding。
3. unreserved 字符保持原样：`A-Z a-z 0-9 - . _ ~`。
4. `/`、NUL、换行、控制字符等必须编码或拒绝。
5. 物理路径为 `<memory_root>/<encoded_seg1>/.../<encoded_last_seg>`。
6. 文件名无扩展名、无 hash 后缀。

示例：

```text
key:  /user/calendar/2026-02-23_10-00_dental_followup
path: <memory_root>/user/calendar/2026-02-23_10-00_dental_followup
```

---

## 7. content、reason 与 envelope

### 7.1 content

content 是纯 UTF-8 文本，无 BOM，必须非空。

content 可以包含可选系统前言：

```text
Importance: 3
Expired-At: 2026-02-24T00:00:00Z

Dental follow-up appointment on 2026-02-23 at 10:00.
```

前言规则：

- 前言必须位于文件开头。
- 每行形如 `Key: Value`，匹配 `^[A-Z][A-Za-z0-9-]*: `。
- 前言以一个空行结束。
- 第一行不匹配该模式时，视为没有前言。
- 已识别字段：
  - `Importance`：整数，默认 `0`。
  - `Expired-At`：ISO8601 时间，过期后不参与 `load`。
- 未识别字段忽略但保留。
- `get` 和 `load` 输出完整 content，包括前言。
- FTS5 的 `content_text` 只索引主体，不索引前言。

### 7.2 reason / provenance

`--reason` 是必填写入原因，不参与 FTS5 索引。

CLI 必须强制：

- reason 非空。
- key 位于 `/kb/...` 时，reason 至少包含一种可回溯字段，例如 `source=`、`url=`、`file=`、`tool=`、`retrieved_at=`。

CLI 无法可靠判断 content 是否来自 web/tool/file；这类 provenance 规则由上层 Agent prompt / skill 继续强制。

推荐格式：

```text
user conversation;conversation=c1,message=m9;original=用户喜欢中文、偏好简洁
web source;site=example.com;url=https://example.com/x;retrieved_at=2026-05-09T10:00:00Z
file source;file=/docs/spec.md;digest=blake3:abcd...
tool source;tool=calendar.lookup;args_hash=blake3:abcd...;result_id=r42
```

### 7.3 envelope

`.meta/log.jsonl` 中每一行是一个 envelope。`set` 和 `remove` 都必须追加 envelope。

`set` envelope：

```json
{
  "schema_version": "2.8",
  "key": "/user/preference/style",
  "ts": "2026-05-09T10:00:00Z",
  "valid": true,
  "reason": "user conversation;conversation=c1,message=m9",
  "content_digest": "blake3:abcd1234...",
  "content_size": 43
}
```

`remove` envelope：

```json
{
  "schema_version": "2.8",
  "key": "/user/preference/style",
  "ts": "2026-05-09T10:10:00Z",
  "valid": false,
  "reason": "user requested deletion",
  "content_digest": null,
  "content_size": 0
}
```

规则：

- `ts` 由系统写入，使用 UTC ISO8601。
- LWW 不按 `ts`，而按 `.meta/log.jsonl` replay 顺序。
- `content_digest` 必须使用 BLAKE3，格式为 `blake3:<hex>`。
- envelope 不保存 content。
- 单条 envelope 必须完整写入一行 JSON，不允许半行提交。

---

## 8. 检索与索引

### 8.1 SQLite schema

`memory.sqlite` 是派生缓存，位于 `<memory_root>/memory.sqlite`。

实现必须使用 SQLite 3.34+ 且启用 FTS5。

```sql
CREATE TABLE memory (
  key            TEXT PRIMARY KEY,
  file_path      TEXT NOT NULL,
  ts             TEXT NOT NULL,
  valid          INTEGER NOT NULL,
  importance     INTEGER,
  expired_at     TEXT,
  reason_summary TEXT,
  content_size   INTEGER NOT NULL
);

CREATE INDEX idx_memory_ts  ON memory(ts);
CREATE INDEX idx_memory_imp ON memory(importance);

CREATE VIRTUAL TABLE memory_fts USING fts5(
  key UNINDEXED,
  key_text,
  content_text,
  tokenize = 'unicode61 remove_diacritics 2'
);
```

### 8.2 索引文本来源

每个有效 key 对应一行 FTS5：

- `key`：原始 logical key，不参与全文检索。
- `key_text`：完整 logical key，而不是仅文件名。namespace 对召回有价值。
- `content_text`：content 主体，不含系统前言。

说明：

- v2.8 的检索契约只保证英语 key/content 与英语 tags 的行为。
- `unicode61` 会按 Unicode 字符类别分词；实现不得依赖“CJK 会被丢弃”这一假设。
- 如果未来需要保证非英语原文不进入索引，必须定义显式 index-normalization，而不能依赖 tokenizer 偶然行为。

### 8.3 tag 校验

`load` 的 tag 是查询词项，不是 memory 元数据。

每个 tag 经 trim 后必须满足：

- UTF-8 字节长度 `2–32`。
- 只允许 ASCII 字母、数字、空格、连字符：`[A-Za-z0-9 -]`。
- 必须至少包含一个字母或数字。
- 不允许双引号、单引号、冒号、括号、星号、控制字符。
- 连续空格规范化为单个空格。
- 大小写不敏感。

允许 phrase tag：

```text
phone case
calendar reminder
```

拒绝更复杂的 FTS query 语法，是为了避免 LLM 生成坏 query，也避免 MATCH 表达式注入。

### 8.4 MATCH 表达式

每个 tag 被构造为 FTS5 phrase；多个 tag 用 `OR` 连接。

```text
tags = ["dental", "appointment", "phone case"]
MATCH = "\"dental\" OR \"appointment\" OR \"phone case\""
```

由于 §8.3 已禁止引号和 FTS5 控制字符，实现只需对 tag 做 trim、空格归一化和双引号包裹。

基础查询 SQL：

```sql
SELECT
  m.key,
  m.ts,
  COALESCE(m.importance, 0) AS importance,
  m.expired_at,
  m.content_size,
  bm25(memory_fts, 4.0, 1.0) AS bm25_score
FROM memory_fts
JOIN memory AS m ON m.key = memory_fts.key
WHERE memory_fts MATCH ?
  AND m.valid = 1
  AND (m.expired_at IS NULL OR m.expired_at > ?)
ORDER BY bm25_score ASC, m.ts DESC, importance DESC, m.key ASC;
```

当 tags 为 `*` 或空列表时，不使用 `MATCH`，直接从 `memory` 表取有效未过期项。

### 8.5 排序

排序分两层。

第一层：FTS5 给出候选和 `bm25_score`。`bm25_score` 越小越相关。

第二层：根据输入 tag 顺序计算 boost。

| tag 位置 | 命中加分 |
|---:|---:|
| 0 | `+8` |
| 1 | `+4` |
| 2 | `+2` |
| >=3 | `+1` |

候选是否命中某 tag，必须用同一个 FTS5 phrase 规则判断；不要用未规范化的原始字符串子串匹配。

最终排序键：

```text
boost DESC,
bm25_score ASC,
ts DESC,
importance DESC,
key ASC
```

如果 tags 为 `*` 或空列表：

```text
ts DESC,
importance DESC,
key ASC
```

---

## 9. 真相源、恢复与 compaction

### 9.1 真相源定义

在线状态由以下两部分联合决定：

1. replay `.meta/state.jsonl`（若存在）和 `.meta/log.jsonl` 后，每个 key 的最新 envelope；
2. key 对应的业务内容文件。

replay 规则：

- 后读到的 envelope 覆盖先读到的 envelope。
- `ts` 不参与 LWW 判定。
- 当前 `log.jsonl` 的顺序由文件行顺序决定。
- 若使用归档，归档顺序必须由 compaction 策略明确保存。

### 9.2 冲突处理

| 情况 | 处理 |
|---|---|
| 最新 envelope 为 `valid=false`，文件存在 | tombstone 生效；文件无效。`verify --repair` 应删除残留文件。 |
| 最新 envelope 为 `valid=true`，文件存在且 digest 匹配 | key 有效，可读，可索引。 |
| 最新 envelope 为 `valid=true`，文件不存在 | key 不可读；`verify` 报告并退出 `3`。无法从 envelope 恢复 content。 |
| 最新 envelope 为 `valid=true`，文件 digest 不匹配 | 报告异常。默认不静默采纳；可通过显式 repair/adopt 追加新 envelope。 |
| 文件存在但无 envelope | orphan；默认不进入在线状态。`verify --repair` 可隔离或显式采纳。 |
| SQLite 与真相源不一致 | 删除并重建 SQLite。 |

### 9.3 `compact`

compaction 目标：

- 归档长日志。
- 保留每个 key 的最新状态，尤其是 tombstone。
- 重建 `memory.sqlite`。
- 清理 tombstone 残留文件和 orphan 文件。

支持两种策略，写入 `.meta/meta.json.compaction_strategy`：

#### 策略 A：`snapshot`（推荐）

- 生成 `.meta/state.jsonl`：每个已知 key 一行最新 envelope，包括 `valid=true` 和 `valid=false`。
- 将旧 `.meta/log.jsonl` 移入 `.meta/archive/`。
- 新建空 `.meta/log.jsonl` 接收后续增量。
- 重建时先读 `state.jsonl`，再读当前 `log.jsonl`。

#### 策略 B：`log_only`

- 不生成 state 快照。
- 归档文件保持完整 replay 链。
- 重建时按归档顺序 + 当前 log 顺序 replay 全部 envelope。

跨语言实现必须识别这两种策略；不支持的策略只能只读挂载或拒绝挂载。

---

## 10. 存储布局与并发

### 10.1 目录树

```text
<memory_root>/
├── user/
├── kb/
├── agent/
├── glossary/
├── memory.sqlite
└── .meta/
    ├── meta.json
    ├── log.jsonl
    ├── lock
    ├── state.jsonl
    └── archive/
        └── log_YYYYMMDD.jsonl
```

规则：

- `.meta/`、`.meta/meta.json`、`.meta/log.jsonl`、`.meta/lock` 必须存在。
- `memory.sqlite` 推荐存在，但可删除重建。
- `.meta/` 不参与 `load`、`list`、业务 grep。
- 业务目录直接位于 `memory_root` 根目录。

### 10.2 `.meta/meta.json`

示例：

```json
{
  "schema_version": "2.8",
  "primary_language": "en",
  "writer": {
    "lang": "rust",
    "impl": "agent-memory-rs",
    "version": "0.8.0"
  },
  "encoding": {
    "key_to_path": "percent",
    "max_segment_bytes": 200,
    "filename_format": "bare"
  },
  "index": {
    "engine": "sqlite-fts5",
    "tokenizer": "unicode61 remove_diacritics 2",
    "key_text": "full_logical_key",
    "content_text": "content_body_without_preamble"
  },
  "compaction_strategy": "snapshot",
  "created_at": "2026-05-09T10:00:00Z"
}
```

兼容规则：

- `schema_version.major` 不一致：拒绝写入。
- `schema_version.minor` 不一致：可只读挂载；写入需实现明确支持。
- `primary_language` 不是 `"en"`：v2.8 拒绝挂载并提示 `unsupported primary_language; v2.8 only supports en`。
- 不支持的 `encoding` 或 `compaction_strategy`：拒绝写入。

### 10.3 写者锁

- 写入端必须持有 `.meta/lock`。
- POSIX 使用 `flock`；Windows 使用 `LockFileEx` 或等价机制。
- 默认锁等待 5 秒，超时退出 `2`。
- 只读端可不加锁，但遇到不一致时按 §9 判定。

### 10.4 可选 daemon

实现可以提供 `agent-memory daemon` 作为性能优化，由 daemon 持有锁并接受子命令转发。

约束：

- daemon 不能改变 CLI 语义。
- daemon 不能改变 `memory_root` 布局。
- daemon 崩溃后，普通 CLI 必须能接管。
- daemon 是优化，不是协议要求。

---

## 11. 默认读取与浮现语义

Memory 的核心读取模式是 surfacing：上层 session 在每轮推理前维护 ordered tags，并在合适的时候调用 `load` 把相关记忆放入 UserMessage

职责边界：

| 角色 | 做什么 | 不做什么 |
|---|---|---|
| LLM / prompt | 抽取英语 tags；决定何时 set/remove。 | 不维护持久索引，不直接管理文件。 |
| Session 合并器 | 累积、衰减、淘汰 tags，输出有序列表。 | 不写 Memory，不决定真相源。 |
| Memory 模块 | 按 tags 召回、排序、截断、返回记忆。 | 不构造 tags，不维护 session。 |

默认读取是 best-effort：

- 不保证召回所有相关记忆。
- 不保证读取过程中的强一致快照。
- 不处理 token limit；CLI 只处理 byte limit。
- token 预算由上层根据模型 tokenizer 折算。

---

## 12. 非功能性要求

| 项目 | 要求 |
|---|---|
| 本地优先 | 只依赖本地文件系统、SQLite 3.34+ 与 FTS5。 |
| 写入可靠性 | JSONL append 使用 `O_APPEND` + `fsync`；单写者锁防止半行交错。 |
| 原子文件写 | 临时文件 + `fsync` + `rename` + 父目录 `fsync`。 |
| 索引可重建 | `memory.sqlite` 任何时候都可从真相源重建。 |
| segment 上限 | 每个 key segment `<= 200` UTF-8 字节。 |
| tag 上限 | 每个 tag `2–32` UTF-8 字节。 |
| content 建议上限 | 单条建议 `<= 256KB`；长文档应写摘要 + 外部引用。 |
| 可观测性 | set/remove/load/verify/compact 应记录 key、ts、命中 tags、错误摘要。 |
| 跨语言互操作 | 以 `.meta/meta.json`、`.meta/log.jsonl`、业务文件为兼容契约。 |

---

## 附录 A：最小示例

### A.1 初始化

```bash
export AGENT_MEMORY_ROOT=/path/to/memory_root
agent-memory init
```

### A.2 写入用户偏好

```bash
agent-memory set /user/preference/style \
  "User prefers concise responses in Chinese." \
  --reason "user conversation;conversation=c1,message=m9;original=用户喜欢中文、偏好简洁"
```

### A.3 写入日程

```bash
agent-memory set /user/calendar/2026-02-23_10-00_dental_followup \
  --reason "user conversation;conversation=c1,message=m10;original=明天10点牙科复诊" <<'EOF'
Importance: 3
Expired-At: 2026-02-24T00:00:00Z

Dental follow-up appointment on 2026-02-23 at 10:00.
Discardable after 2026-02-24 if not completed.
EOF
```

### A.4 召回

```bash
agent-memory load calendar,dental,reminder --max-records 10 --max-bytes 8192
```

示例输出：

```text
KEY /user/calendar/2026-02-23_10-00_dental_followup
SIZE 141
TRUNCATED 0
MATCHED calendar,dental
TS 2026-02-22T10:01:00Z
---
Importance: 3
Expired-At: 2026-02-24T00:00:00Z

Dental follow-up appointment on 2026-02-23 at 10:00.
Discardable after 2026-02-24 if not completed.
END
```

### A.5 删除

```bash
agent-memory remove /user/calendar/2026-02-23_10-00_dental_followup \
  --reason "reminded and confirmed completed"
```

### A.6 KB 写入

```bash
agent-memory set /kb/product/example/spec \
  --reason "web source;site=example.com;url=https://example.com/spec;retrieved_at=2026-05-09T10:00:00Z" <<'EOF'
Importance: 2

Example product specification summary.
Use the source URL for full details.
EOF
```

### A.7 Bash 兜底查询

```bash
ls "$AGENT_MEMORY_ROOT/"
cat "$AGENT_MEMORY_ROOT/user/preference/style"
grep -r "dental" "$AGENT_MEMORY_ROOT/" --exclude-dir=.meta --exclude=memory.sqlite
grep "牙科" "$AGENT_MEMORY_ROOT/.meta/log.jsonl"
```

---

## 附录 B：FTS5 fixture

v2.8 不要求实现自定义分词。CI 应验证 SQLite FTS5 `unicode61 remove_diacritics 2` 对英语和 ASCII key 的行为稳定。

| 输入 | 预期可匹配 token / 行为 |
|---|---|
| `dental` | `dental` |
| `Dental Followup` | 可被 `dental`、`followup` 命中 |
| `iPhone16` | 可被 `iphone16` 命中；不保证被 `iphone` 命中 |
| `iPhone 16 specifications` | 可被 `iphone`、`16`、`specifications` 命中 |
| `2026-02-23_10-00_dental_followup` | 可被 `2026`、`02`、`23`、`10`、`00`、`dental`、`followup` 命中 |
| `Hello, world!` | 可被 `hello`、`world` 命中 |
| `naïve résumé café` | 可被 `naive`、`resume`、`cafe` 命中 |
| 空字符串或纯标点 | 不产生有效召回 token |

CJK / 非英语文本：

- v2.8 不承诺非英语召回行为。
- 不应写 fixture 断言 CJK 被丢弃。
- 上层 Agent 应把可召回事实写成英语；原文可放在 reason 或括号中作为审计信息。

---

## 附录 C：ADR 摘要

### C.1 为什么 content 统一英语

Memory 是 Agent 的工作记忆，不是用户语言转写库。同一事实需要能被不同对话语言触发，因此进入 memory 的事实应归一为英语。CLI 不检查语言；这由上层 prompt/skill 保证。

### C.2 为什么是文件 + JSONL + SQLite

- 文件：人类可读、可 grep、易备份。
- JSONL：append-only，适合审计和 crash 恢复。
- SQLite FTS5：本地、确定、可解释、无需外部服务。

### C.3 为什么不把 tag 写进 memory

tags 是 retrieve-time 信号，不是 memory 的固有属性。上层 session 根据当前上下文动态生成 ordered tags；Memory 只负责按 tags 召回。

### C.4 为什么 `set` 先写文件再写 envelope

因为 envelope 不保存 content。如果先写 envelope，再 crash 于写文件之前，系统无法从 log 恢复 content。先写业务文件再提交 envelope，可以把 crash 造成的问题限制为 orphan 文件，而不是已提交但不可恢复的有效记录。
