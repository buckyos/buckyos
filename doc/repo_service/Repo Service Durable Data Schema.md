# RepoService Durable Data Schema

## 1. Overview

Service: `repo-service`

RepoService keeps this node's view of the content network: which contents it
knows (`objects`), the proofs around them (`proofs`), purchase receipts
(`receipts`) and the announce requests waiting for BNS publishing
(`pending_announces`). All of them live in the RDB instance
`repo-service-main` (`buckyos_api::REPO_SERVICE_RDB_INSTANCE_ID`). The bytes of a
pinned content stay in the node's named store and are retained by a binding in
the zone bucket `zone/repo-service/_/packages`. Behavior and RPC model:
`doc/repo_service/Repo v2.md`; bucket model: `doc/arch/应用对象桶 Bucket 设计.md`.

## 2. Data Classification

### Durable Data

| Data item | Reason |
|---|---|
| `objects` | Node-local record of every collected / pinned content; the bucket key of a pinned one |
| `proofs` | Propagation edges; kept after unpin / uncollect for settlement |
| `receipts` | Receipts validated while serving paid content |
| `pending_announces` | Announce requests not yet published to BNS |
| Bucket `zone/repo-service/_/packages` | Retention of pinned contents (the named store keeps the bytes) |

### Disposable Data

None. The service has no data directory: no files under `data/`, no private
named store.

## 3. Storage Strategy

- Structured data: RDB instance `repo-service-main` (SQLite and PostgreSQL DDL in
  `src/kernel/buckyos-api/src/repo_client.rs`), `user_data` partition, applied with
  `CREATE TABLE IF NOT EXISTS` on every start.
- Content: named store (put by the publisher, the downloader or the caller
  before `store` / `pin`). RepoService never copies bytes.
- Retention: bucket `zone/repo-service/_/packages`, declared in
  `system_config_builder.rs::add_repo_service` (`spec_config.buckets`, with
  `storage_quota_mib`):

  | bucket | kind | data_policy | key | value | who pins |
  |---|---|---|---|---|---|
  | `packages` | `data` | `app_data` | `<content_name>/<version>`; content without a `version` in its meta uses `<content_name>` (the content id when it has no name either) | the content itself when it is a `cyfile` / `cydir`; a `cyfile` named `<content_name>` wrapping it when the content is a chunk or chunk list | the bucket (`bucket:zone/repo-service/_/packages`) |

  `objects.bucket_key` records the key of a pinned row. A key already bound to a
  different content is a conflict (`if_none_match: "*"`): the same package name
  and version is never silently replaced; unpin the old content first.

### Consistency between `objects` and the bucket

Design §15.1 "bind first, then write RDB", with the business id in the row:

| Operation | Order | Failure |
|---|---|---|
| `store` / `pin` | 1. content must be complete in the named store; 2. `bucket.put(key, if_none_match="*")` (a binding of the same object is accepted, so a retry replays); 3. upsert `objects` with `status='pinned'`, `bucket_key=key`; 4. a previous `bucket_key` of the row that differs is unbound | 2 fails → nothing written. 3 fails → the binding this call created is deleted again; if that fails too, startup reconciliation removes it. 4 fails → reconciliation |
| `unpin` / `uncollect` | 1. update `objects` (`status='collected'`, `bucket_key=NULL`) or delete the row; 2. `bucket.delete(key)` | 2 fails → an extra binding (retention without a record), removed by reconciliation; never a pinned row without retention |

Reconciliation runs once after start (retried every 60 s until the bucket service
answers): every key in `packages` not referenced by a pinned row and bound more
than ten minutes ago is unbound (a younger one may be a pin still writing its
row); a pinned row whose key is missing is bound again when its content is still
in the named store, otherwise logged.

## 4. Schema Definitions

### Table: `objects`

| Column | Type | Nullable | Description |
|---|---|---|---|
| `content_id` | TEXT PK | No | Content ObjId |
| `content_name` | TEXT | Yes | Name from meta (`content_name` or `name`) |
| `status` | TEXT | No | `collected` or `pinned` |
| `origin` | TEXT | No | `local` or `remote` |
| `meta` | TEXT | No | Meta JSON |
| `owner_did`, `author`, `access_policy`, `price` | TEXT | — | Index fields extracted from meta |
| `content_size` | INTEGER/BIGINT | Yes | Content bytes |
| `bucket_key` | TEXT | Yes | Key in `zone/repo-service/_/packages` while `status='pinned'` |
| `collected_at`, `pinned_at`, `updated_at` | INTEGER/BIGINT | Yes | Unix seconds |

### Table: `proofs`, `receipts`

Unchanged; see `doc/repo_service/Repo v2.md` §4.2–4.3.

### Table: `pending_announces`

| Column | Type | Nullable | Description |
|---|---|---|---|
| `content_id` | TEXT PK | No | Content to announce; a repeated request replaces the previous one |
| `content_name` | TEXT | Yes | Name to publish |
| `meta` | TEXT | No | Meta JSON at request time |
| `requested_at` | INTEGER/BIGINT | No | Unix seconds |

## 5. Schema Version

`REPO_SERVICE_RDB_SCHEMA_VERSION = 2` (carried in the rdb instance config of the
service spec). Version 2 adds `objects.bucket_key` and `pending_announces`.

## 6. Upgrade Compatibility Strategy

No-compat (pre-release): version 1 databases and the old `pending_announces/*.json`
and `ready_check_store/` directories are not read; DV reinstalls
(`start.py --all`). Later versions are additive-only.

## 7. Extensibility Rules

- `objects.meta` is the extension point for content fields; index columns are
  added only for queried fields.
- `bucket_key` semantics are frozen: the key of the current binding of a pinned row.

## 8. Query Patterns

| Query | Index |
|---|---|
| Record by content id (`get`, `pin`, `unpin`, `serve`) | PK |
| List / resolve / reconciliation (all rows, filtered in memory) | full scan; node-local catalog, small |
| Proofs of a content | `idx_proof_content_id` |
| Announce by content id | PK |
