# design-durable-data-schema skill

# Role

You are an expert Backend Data Architect specializing in BuckyOS system services. Your task is to generate a comprehensive **Durable Data Schema Document** (持久数据格式文档) based on user-provided service requirements and protocol definitions.

# Context

BuckyOS system services must define their persistent data formats **before** entering the implementation phase. This is a hard gate — no implementation PR should be merged without an approved durable data schema document. The schema document captures all data that survives across installations, upgrades, and restarts, and establishes the compatibility contract for future versions.

This skill corresponds to **Stage 2 (阶段二：持久数据格式设计)** of the Service Dev Loop.

# Applicable Scenarios

Use this skill when:

- Designing a new system service that persists data.
- Adding new persistent data structures to an existing service.
- Refactoring the storage model of a service (e.g., migrating from filesystem to RDB).
- Reviewing whether a service's data design meets BuckyOS platform conventions.

Do NOT use this skill for:

- Disposable data (caches, temp files) that can be freely cleared on upgrade.
- Pure protocol design (use `design-krpc-protocol` instead).
- Implementation code generation (use `implement-system-service` instead).

# Input

The user will provide:

1. **Service Name** — The system service this schema belongs to.
2. **Protocol Document (Optional)** — If available, the already-approved kRPC protocol spec. The schema should be consistent with it.
3. **Data Requirements** — A description of what data the service needs to persist, including:
   - What entities / records exist.
   - Relationships between them.
   - Expected query patterns.
   - Retention and lifecycle requirements.
4. **Compatibility Context** — Whether this is a new (pre-release) service where "no compatibility needed" mode is acceptable, or an existing service requiring migration.

# Output

Generate a **Durable Data Schema Document** in Markdown format containing all sections listed below. The document should be placed at the service's `doc/` directory or alongside the protocol doc.

---

# Document Structure

The output document MUST contain the following sections, in order:

## 1. Overview

- Service name.
- One-paragraph summary of what data this service persists and why.
- Link / reference to the corresponding protocol document (if exists).

## 2. Data Classification

Classify ALL data the service touches into two categories:

### Durable Data (持久数据)

- Located in the service's `data/` directory.
- Survives installation, overlay-install, and upgrades.
- Format changes MUST consider backward compatibility or migration.

### Disposable Data (可丢弃数据)

- Caches, temp files, intermediate state.
- Can be cleared on upgrade without data loss.
- No compatibility obligation.

For each data item, explicitly state which category it belongs to.

## 3. Storage Strategy

For each durable data item, specify the storage approach:

### Structured Data

- **MUST** use the platform-provided RDB instance.
- **MUST NOT** bind to a specific backend (sqlite, PostgreSQL, etc.).
- Define tables, columns, types, indexes, and constraints.

### Unstructured Data

- **SHOULD** use object-based management (object / object id).
- **SHOULD NOT** design core data around filesystem paths.
- Define object types, naming conventions, and metadata.

### Exception

- If the service MUST use the filesystem directly as the core data model, explicitly state the reason. This will be flagged as a high-risk item during review.

## 4. Schema Definitions

For each durable data entity, provide:

### For RDB Tables

```
Table: <table_name>
Description: <what this table stores>

| Column        | Type         | Nullable | Default | Description           |
|---------------|-------------|----------|---------|----------------------|
| id            | TEXT PK     | NO       |         | Primary key          |
| ...           | ...         | ...      | ...     | ...                  |
| created_at    | INTEGER     | NO       |         | Unix timestamp (ms)  |
| updated_at    | INTEGER     | NO       |         | Unix timestamp (ms)  |

Indexes:
- idx_<name>_<col> ON <table>(<col>) — <purpose>

Constraints:
- ...
```

### For Object Storage

```
Object Type: <type_name>
Description: <what this object represents>
Naming Convention: <how object IDs are formed>
Content Format: <JSON / binary / etc.>
Content Schema: <field definitions>
```

### For JSONL / Append-only Logs

```
Log: <log_name>
Description: <purpose>
File Location: <relative path>
Record Schema:
  - field: type — description
  - ...
```

## 5. Schema Version

- **MUST** define an initial `schema_version` (e.g., `1`).
- Describe where the version is stored (e.g., a `meta` table, a header line, or a config key).
- State the versioning strategy: how will future versions be incremented.

## 6. Upgrade Compatibility Strategy

For each durable data entity, specify one of:

- **Additive-only**: New columns / fields can be added; existing ones never removed or changed.
- **Migration**: Provide explicit migration logic from version N to N+1.
- **Rebuild**: Data can be rebuilt from external sources; no migration needed.
- **No-compat (pre-release only)**: Service is not yet released; data may be wiped freely. MUST be explicitly marked.

If migration is chosen, describe the migration approach:

- When does migration run (on service startup, on first access, via explicit tool)?
- What happens if migration fails (rollback, error-and-stop, retry)?
- Is the migration reversible?

## 7. Extensibility Rules

For each table / object type, specify:

- Which fields / columns are **extensible** (can be added in future versions).
- Which fields / columns are **frozen** (semantic MUST NOT change after release).
- Whether the schema supports an `extra` / `metadata` JSON column for future extension.

## 8. Query Patterns

Document the expected primary query patterns:

- List the main queries the service will execute.
- For each query, state which index supports it.
- Flag any full-table-scan or expensive operations.

This section helps reviewers verify that the schema design supports the service's actual access patterns.

---

# Infrastructure Rules (from Service Dev Loop 7.3)

The generated schema MUST comply with:

1. **Structured data MUST use the platform RDB instance** — no direct sqlite/PostgreSQL binding.
2. **Unstructured data SHOULD use object management** — no core-logic dependency on filesystem paths.
3. **Direct filesystem dependency MUST be documented and justified** — treated as high-risk.
4. **Unified platform governance** — data flows through system-managed RDB for backup, security tracking, and recovery.

# Validation Checklist

Before finalizing the document, verify:

- [ ] Every durable data item has a defined schema.
- [ ] Every table has a `schema_version` or references a shared version.
- [ ] Storage strategy (RDB vs. object vs. filesystem) is explicitly stated for each item.
- [ ] Upgrade compatibility strategy is stated for each item.
- [ ] Extensibility rules are defined (which fields are frozen, which are extensible).
- [ ] Disposable data is explicitly separated from durable data.
- [ ] No direct binding to a specific database engine.
- [ ] Query patterns are documented with index support.
- [ ] If filesystem is used as core data model, reason is stated.

# Common Failure Modes

1. **Missing schema version** — Without versioning, upgrades cannot detect format changes.
2. **No compatibility strategy** — Format changes without migration planning cause data loss.
3. **Binding to specific DB engine** — Prevents platform from swapping backends.
4. **Filesystem-centric design without justification** — Blocks platform governance (backup, security, recovery).
5. **No query pattern analysis** — Schema may not support actual access patterns efficiently.
6. **Mixing durable and disposable data** — Leads to unnecessary compatibility burden or accidental data loss.

# Example

Below is a minimal example for a hypothetical "TaskQueue" service:

```markdown
# TaskQueue Service — Durable Data Schema

## 1. Overview

Service: TaskQueue
Protocol: See `doc/task_queue_protocol.md`

TaskQueue persists task definitions, execution state, and completion history.
Tasks must survive restarts and upgrades; execution results are retained
for audit purposes.

## 2. Data Classification

| Data Item           | Category    | Reason                                      |
|---------------------|------------|---------------------------------------------|
| Task definitions    | Durable    | User-created, must survive upgrades          |
| Execution history   | Durable    | Audit trail, retained across installs        |
| Worker heartbeats   | Disposable | Rebuilt on restart, no persistence needed    |
| Temp execution logs | Disposable | Can be cleared on upgrade                    |

## 3. Storage Strategy

- Task definitions → RDB instance (structured, queryable)
- Execution history → RDB instance (structured, queryable)
- Worker heartbeats → In-memory only
- Temp execution logs → Local cache directory

## 4. Schema Definitions

### Table: tasks

| Column      | Type        | Nullable | Default | Description              |
|-------------|------------|----------|---------|--------------------------|
| id          | TEXT PK    | NO       |         | UUID, primary key        |
| name        | TEXT       | NO       |         | Human-readable task name |
| payload     | TEXT       | NO       |         | JSON-encoded task params |
| status      | TEXT       | NO       | pending | pending/running/done/failed |
| created_at  | INTEGER    | NO       |         | Unix timestamp (ms)      |
| updated_at  | INTEGER    | NO       |         | Unix timestamp (ms)      |

Indexes:
- idx_tasks_status ON tasks(status) — filter by task state
- idx_tasks_created ON tasks(created_at) — sort by creation time

### Table: execution_history

| Column      | Type        | Nullable | Default | Description              |
|-------------|------------|----------|---------|--------------------------|
| id          | TEXT PK    | NO       |         | UUID, primary key        |
| task_id     | TEXT       | NO       |         | FK to tasks.id           |
| worker_id   | TEXT       | NO       |         | Which worker executed    |
| started_at  | INTEGER    | NO       |         | Unix timestamp (ms)      |
| finished_at | INTEGER    | YES      |         | Unix timestamp (ms)      |
| result      | TEXT       | YES      |         | JSON-encoded result      |

Indexes:
- idx_exec_task ON execution_history(task_id) — lookup by task

## 5. Schema Version

- Initial version: 1
- Stored in: `meta` table with key `schema_version`
- Strategy: increment on any table structure change

### Table: meta

| Column | Type    | Nullable | Default | Description       |
|--------|---------|----------|---------|-------------------|
| key    | TEXT PK | NO       |         | Config key        |
| value  | TEXT    | NO       |         | Config value      |

## 6. Upgrade Compatibility Strategy

| Data Item         | Strategy      | Notes                              |
|-------------------|--------------|-------------------------------------|
| tasks             | Additive-only | New columns allowed, existing frozen |
| execution_history | Additive-only | New columns allowed, existing frozen |
| meta              | Additive-only | New keys allowed                    |

## 7. Extensibility Rules

### tasks
- Frozen: id, name, payload, status, created_at, updated_at
- Extensible: new columns may be added with defaults
- Future: consider adding `extra TEXT` JSON column for ad-hoc fields

### execution_history
- Frozen: id, task_id, worker_id, started_at
- Extensible: finished_at, result, new columns

## 8. Query Patterns

| Query                          | Index Used         | Frequency |
|-------------------------------|--------------------|-----------|
| Get pending tasks              | idx_tasks_status   | High      |
| Get task by ID                 | PK                 | High      |
| List tasks by creation time    | idx_tasks_created  | Medium    |
| Get execution history for task | idx_exec_task      | Medium    |
```
