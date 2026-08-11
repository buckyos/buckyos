# MessageCenter Durable Data Schema

## 1. Overview

Service: `msg-center`

MessageCenter persists mailbox projections, delivery state, contacts, groups,
idempotency results, tunnel recovery cursors, and UI session state in the
platform-provided RDB instance. Message objects remain content-addressed in
named store. The corresponding behavior and RPC model are described in
`doc/message_hub/Message Center.md` and
`doc/message_hub/Message Tunnel Design.md`.

## 2. Data Classification

### Durable Data

| Data item | Reason |
|---|---|
| `mailbox_records`, `msg_refs` | User mailbox truth and object references |
| `delivery_records` | Delivery queue state and diagnostic history |
| `msg_idempotency` | Prevents replay after restart |
| `msg_tunnel_cursors` | External ingress recovery position |
| Contact and group tables | User and group configuration/state |

### Disposable Data

| Data item | Reason |
|---|---|
| `ui_session_states` | Typing, active state, and UI presentation hints may be rebuilt |
| In-memory message/receipt caches | Named store or authoritative records remain available |
| kevent notifications | Acceleration signals only |

## 3. Storage Strategy

All structured durable data uses the platform RDB instance and supports both
SQLite and PostgreSQL DDL. Message bodies and attachments use named store
objects. No durable state is stored in service-private filesystem paths.

## 4. Schema Definitions

### Table: `msg_idempotency`

| Column | Type | Nullable | Description |
|---|---|---|---|
| `scope` | TEXT | No | Operation: `dispatch` or `post_send` |
| `owner_scope` | TEXT | No | Stable recipient owner for dispatch or message author for post-send |
| `idempotency_key` | TEXT | No | Caller/platform replay key |
| `msg_id` | TEXT | Yes | Content-addressed message id |
| `retention_key` | TEXT | Yes | Stable external conversation or local sender bucket |
| `state` | TEXT | No | `pending` or `completed` |
| `result_json` | TEXT | Yes | Serialized RPC result |
| `created_at_ms` | INTEGER/BIGINT | No | Creation time |
| `updated_at_ms` | INTEGER/BIGINT | No | Last update time |
| `expires_at_ms` | INTEGER/BIGINT | Yes | Physical cleanup eligibility time |

Primary key: `(scope, owner_scope, idempotency_key)`.

Indexes:

- `idx_msg_idempotency_expire(expires_at_ms)` supports global expiry cleanup.
- `idx_msg_idempotency_retention_expire(retention_key, expires_at_ms)` supports conversation-bucket cleanup.

### Table: `msg_tunnel_cursors`

| Column | Type | Nullable | Description |
|---|---|---|---|
| `tunnel_key` | TEXT | No | Platform, owner, tunnel account, and tunnel instance identity |
| `cursor_key` | TEXT | No | Cursor kind, such as `bot_api_update_offset` |
| `value_json` | TEXT | No | Opaque validated cursor value |
| `updated_at_ms` | INTEGER/BIGINT | No | Last successful persistence time |

Primary key: `(tunnel_key, cursor_key)`. This table is internal to msg-center;
the public UI SessionState RPC cannot read or modify it.

### Existing durable tables

`mailbox_records`, `delivery_records`, `msg_refs`, contact tables, and group
tables retain their schema defined by `MSG_CENTER_RDB_SCHEMA_SQLITE` and
`MSG_CENTER_RDB_SCHEMA_POSTGRES`. Message objects retain their existing named
store object schema.

## 5. Schema Version

The shared msg-center RDB instance schema version is `8`, stored in the
scheduler-provided RDB instance spec. Every DDL or frozen-key semantic change
increments this version.

## 6. Upgrade Compatibility Strategy

Beta 2.2 is in no-compat mode. Version 7 instances are rebuilt as version 8;
there is no in-place migration. Durable version 8 fields and key semantics are
frozen until the next explicit schema version.

## 7. Extensibility Rules

- Frozen: idempotency primary-key meaning, state transitions, cursor primary key,
  timestamp units, and stored RPC result schema.
- Extensible: new operation scopes, cursor keys, tables, and additive columns
  with a schema-version bump.
- Cursor JSON is opaque to storage but must be validated by its tunnel before use.

## 8. Query Patterns

| Query | Support | Cost |
|---|---|---|
| Resolve idempotency result | `msg_idempotency` primary key | Constant lookup |
| Load/store one tunnel cursor | `msg_tunnel_cursors` primary key | Constant lookup/upsert |
| Clean one conversation bucket | retention/expiry index | Background maintenance |
| Clean global expired rows | expiry index, 10,000-row batches | Background maintenance |
| Enumerate oversized buckets | retention/expiry index scan | Hourly background maintenance |

Idempotency maintenance never runs in `dispatch` or `post_send` request paths.
The background worker runs immediately at service startup and then hourly.
