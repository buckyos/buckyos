# nfs-server ShareMgr Durable Data Schema

## 1. Overview

Service: `nfs-server`, module `src/frame/nfs_server/src/share/` (ShareContentMgr).

A share is a snapshot of a file or folder (an immutable FileObject / DirObject
in the node named store) plus its link policy, app delivery, audit and access
statistics. The records live in nfs-server's filedb (`filedb.sqlite`, opened by
path; design `doc/arch/应用对象桶 Bucket 设计.md` §14.1 keeps it there). The
snapshot is retained by nfs-server's own zone bucket while the share is active.
Protocol: `share.*` NFSP methods (`src/frame/nfs_server/README.md`).

## 2. Data Classification

### Durable Data

| Data item | Reason |
|---|---|
| `share_operations`, `shares`, `share_versions`, `share_link_policy`, `share_deliveries`, `share_audit`, `share_access_stats`, `public_observations`, `grants` (link credentials) | Share records, history and statistics |
| `share_access_log` | Raw access events, pruned after 90 days |
| Bucket `zone/nfs-server/_/shares` | Retention of active shares' snapshots |

### Disposable Data

In-memory statistics buffer, unlock attempts, running-operation progress.

## 3. Storage Strategy

- Structured data: filedb tables (`share::db::SCHEMA`).
- Snapshot bytes: named store (`snapshot::run_snapshot`, store mode).
- Retention: nfs-server hosts the buckets, so ShareMgr calls the in-process
  `BucketStore` with `Caller::system()`:

  | bucket | kind | data_policy | key | value | who pins |
  |---|---|---|---|---|---|
  | `shares` (`scope: zone`) | `data` | `app_data` | `<share_id>` | the share's root FileObject / DirObject (`shares.obj_id`) | the bucket (`bucket:zone/nfs-server/_/shares`), once per distinct snapshot |

  Declared in `system_config_builder.rs::add_nfs_server` (`spec_config.buckets` +
  `storage_quota_mib`, from `buckyos_api::nfs_server_bucket_decls`); a standalone
  server declares it for itself (`share::declare_bucket`, merged into
  `--bucket-specs`).

### Consistency between `shares` and the bucket

| Operation | Order | Failure |
|---|---|---|
| `share.create` / `share.app.handoff` | 1. snapshot into the named store; 2. `put(<share_id>, obj_id, create-only)`; 3. insert the share rows in one filedb transaction | 2 fails → no share. 3 fails → the key is deleted again |
| `share.stop`, delivery `canceled` / `failed`, link expiry (sweeper) | 1. set the final status (`stopped` / `expired`) in filedb; 2. `delete(<share_id>)` | 2 fails → a stray key (retention without an active share) |
| Start (`sweeper_loop`, once) | `reconcile_retention`: keys without an active share and bound more than ten minutes ago are deleted (a younger key may be a share still being created), an active share without a key is bound again | logged |

There is no intermediate `stopping` status and no share-owned pin: named-store
pin and unpin (including a failed unpin) are the bucket's state machine
(design §7.2–7.3).

`share.get` returns the binding as `retention: [{bucket_id, key, obj_id, version, bound_at}]`
(empty once released); views carry `retained` (= the share is active).

## 4. Schema Definitions

See `share::db::SCHEMA`. Removed in this version: `share_retentions`.

## 5. Schema Version

filedb tables are created with `CREATE TABLE IF NOT EXISTS` at open; no version
row. Removing `share_retentions` is a no-compat change.

## 6. Upgrade Compatibility Strategy

No-compat (pre-release): old `share:<share_id>` pins and `share_retentions` rows
are not migrated; DV reinstalls (`start.py --all`).

## 7. Extensibility Rules

The bucket key is the share id and is frozen; any further per-share object
(e.g. a cover image) gets its own key prefix under the same bucket.

## 8. Query Patterns

| Query | Index |
|---|---|
| Expired active links (sweeper, every minute) | join `shares` / `grants` |
| Active shares (startup reconciliation) | `idx_shares_owner` (status) scan |
| Binding of a share (`share.get`) | bucket index primary key |
