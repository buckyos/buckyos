# klog

`klog` is the replicated log and metadata KV library used by `klog_daemon`.
It provides the OpenRaft type definitions, log/state-store implementations,
cluster HTTP transport, local JSON-RPC service, and client APIs.

Deployment, process lifecycle, daemon config, gateway exposure, and BuckyOS
integration policy are documented in `src/kernel/klog_daemon/readme.md`.

## Scope

`klog` is responsible for:

- replicated append-only log entries;
- strongly consistent metadata KV writes;
- optional linearizable reads through an OpenRaft read barrier;
- follower-to-leader forwarding for writes and strong reads;
- cluster membership admin primitives;
- local client-facing JSON-RPC helpers.

`klog` is not responsible for:

- starting/stopping daemon processes;
- generating BuckyOS scheduler or gateway config;
- user/session/RBAC authorization;
- public route exposure policy.

In BuckyOS deployment, identity, route exposure, and RBAC belong to the
BuckyOS/gateway layer. `klog` only enforces protocol and consistency rules.

## Data Model

### Log Entries

`KLogEntry` is the replicated append-only record:

- `id`: global log id. Clients normally send `0`; the leader assigns the next id.
- `timestamp`: milliseconds since Unix epoch. The service fills it when omitted by the request.
- `node_name`: BuckyOS node name that created the entry.
- `request_id`: optional idempotency key. Recent ids are deduplicated.
- `level`: `TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`, or `FATAL`.
- `source`: optional logical source.
- `attrs`: optional string attributes.
- `message`: log body.

`KLogAppendRequest` limits:

- `message` maximum: 64 KiB.
- `request_id` maximum: 128 bytes.
- recent `request_id` dedup window: 5 minutes, capped at 10,000 cached ids.

### Metadata KV

`KLogMetaEntry` stores system metadata:

- `key`: metadata key, maximum 256 bytes.
- `value`: metadata value, maximum 256 KiB.
- `updated_at`: milliseconds since Unix epoch.
- `updated_by_node_name`: writer node name.
- `revision`: globally increasing metadata `mod_revision`, used as an opaque
  CAS token.
- `create_revision`: global revision at which the current live generation was
  created.
- `mod_revision`: global revision at which the current live value was last
  modified.
- `version`: number of live updates since `create_revision`; recreate after
  delete starts a new generation with version 1.

Current metadata revision semantics are intentionally MVCC-compatible:

- Every committed metadata mutation allocates a global `mod_revision`.
- All actions in the same `KLogMetaTxRequest` share one `mod_revision`.
- Deleting an existing key writes a tombstone state. The live value disappears
  from `get` and `prefix` queries, but the key's latest revision remains
  available for CAS conflict checks.
- Recreating a deleted key with `expected_revision=Some(0)` allocates a new
  `mod_revision`; stale CAS using the pre-delete revision fails with the
  tombstone revision.
- `KLogMetaPutResponse`, `KLogMetaQueryResponse`, `KLogMetaTxResponse`, and
  `KLogMetaDeleteResponse` expose explicit MVCC metadata. `revision` is kept as
  a compatibility alias for `mod_revision`.
- Historical key and prefix queries can pass `revision` to read the visible
  live value set as of that global revision. Tombstones hide deleted keys at
  their delete revision until a later recreate becomes visible.
- The storage layer also maintains a revision-major metadata change index
  ordered by `(mod_revision, key)`. It supports bounded change-feed scans with
  an exclusive cursor and optional key/prefix filters, which is the data-layer
  prerequisite for etcd-style watch APIs.
- `meta-changes` exposes the first watch-compatible API as active polling:
  callers can issue one-shot scans or set `wait_timeout_ms` for short
  long-poll behavior. Streaming push APIs are not implemented yet.
- Metadata compaction records a persisted compacted revision, keeps one
  key-major baseline record per key at or before that revision, and drops old
  revision-major change-feed index entries. Historical reads and
  `meta-changes` resumes at compacted revisions fail with
  `KLogErrorCode::Compacted` / HTTP `410`. The admin plane exposes explicit
  compaction; `klog_daemon` can also trigger automatic revision-count
  compaction when enabled in daemon config.

`KLogMetaPutRequest.expected_revision` controls CAS semantics:

- `None`: unconditional put.
- `Some(0)`: create only; fails if the key already exists.
- `Some(n)`: update only when the current live or tombstone revision is `n`.

Version conflicts are returned as `KLogErrorCode::VersionConflict` with HTTP
`409` on HTTP APIs.

`KLogMetaTxRequest` applies multiple metadata mutations atomically through the
Raft state machine:

- `actions`: keyed `put` or `delete` operations. The map key must match the
  action key.
- `expected_revision` on an action has the same CAS semantics as
  `KLogMetaPutRequest.expected_revision`.
- `guard`: optional optimistic transaction guard. When present, the transaction
  first checks the guard key revision and rejects the whole transaction on
  mismatch. If the guard key is not otherwise updated by the transaction, the
  implementation bumps the existing guard key revision after all actions pass.

On any conflict, none of the transaction actions are applied.

## API Planes

`klog` has four logical planes. `klog_daemon` maps them to separate listen
addresses in production.

| Plane | Routes | Caller |
| --- | --- | --- |
| Raft control | `/klog/append-entries`, `/klog/install-snapshot`, `/klog/vote` | OpenRaft peers |
| Inter-node data | `/klog/data/*` | peer forwarding and local services |
| Admin | `/klog/admin/*` | membership/admin tooling |
| JSON-RPC | `/klog/rpc`, `/kapi/klog-service` | local BuckyOS services |

Raft control requests use the internal bincode frame defined in
`network/request.rs`. Data, admin, and JSON-RPC APIs use JSON.

## Data HTTP APIs

These routes are exposed by both the network server inter-node plane and the
local RPC server.

| Method | Path | Request | Response |
| --- | --- | --- | --- |
| `POST` | `/klog/data/append` | `KLogAppendRequest` JSON body | `KLogAppendResponse` |
| `GET` | `/klog/data/query` | `KLogQueryRequest` query params | `KLogQueryResponse` |
| `POST` | `/klog/data/meta-put` | `KLogMetaPutRequest` JSON body | `KLogMetaPutResponse` |
| `POST` | `/klog/data/meta-delete` | `KLogMetaDeleteRequest` JSON body | `KLogMetaDeleteResponse` |
| `POST` | `/klog/data/meta-tx` | `KLogMetaTxRequest` JSON body | `KLogMetaTxResponse` |
| `GET` | `/klog/data/meta-query` | `KLogMetaQueryRequest` query params | `KLogMetaQueryResponse` |
| `GET` | `/klog/data/meta-changes` | `KLogMetaChangesRequest` query params | `KLogMetaChangesResponse` |

Query defaults and constraints:

- log query default limit: 200.
- meta query default limit: 200.
- maximum query limit: 2,000.
- `strong_read=true` forces a linearizable OpenRaft read barrier.
- log queries support `start_id`, `end_id`, `desc`, `level`, `source`,
  `attr_key`, and `attr_value`.
- meta queries support either `key` or `prefix`, but not both.
- meta queries can pass `revision` to return the visible value set at that
  historical global revision. Omit `revision` for the current live view.
  Querying a compacted revision returns HTTP `410`.
- meta changes support `start_revision`, optional `end_revision`, optional
  `key` or `prefix`, flat cursor fields `cursor_revision` and `cursor_key`,
  `include_deleted`, and `wait_timeout_ms`. `wait_timeout_ms` is capped by the
  server and is intended for short long-poll loops, not permanent streams.
  Resuming from a compacted revision returns HTTP `410`.

Default reads are local reads. Use `strong_read=true` when callers need
linearizable reads, for example system-config read-after-write validation.

## JSON-RPC API

The local client-facing JSON-RPC endpoints are:

- `/klog/rpc`
- `/kapi/klog-service`

Supported method names:

- `klog.log.append`
- `klog.log.query`
- `klog.meta.put`
- `klog.meta.delete`
- `klog.meta.tx`
- `klog.meta.query`
- `klog.meta.changes`

Legacy aliases are still accepted for log operations:

- `klog.append`
- `klog.query`

Example:

```json
{
  "jsonrpc": "2.0",
  "method": "klog.meta.put",
  "params": {
    "key": "system/config/example",
    "value": "{\"enabled\":true}",
    "expected_revision": 0
  },
  "id": 1
}
```

`KLogClient` wraps this protocol and defaults to the BuckyOS service route
`http://127.0.0.1:4080/kapi/klog-service`.

## Admin APIs

Admin APIs are cluster membership primitives:

| Method | Path | Params |
| --- | --- | --- |
| `POST` | `/klog/admin/add-learner` | `node_id`, `addr`, `port`, optional `inter_port`, `admin_port`, `rpc_port`, `node_name`, `device_id`, `blocking` |
| `POST` | `/klog/admin/remove-learner` | `node_id` |
| `POST` | `/klog/admin/change-membership` | `voters`, optional `retain` |
| `GET` | `/klog/admin/cluster-state` | none |
| `POST` | `/klog/admin/meta-compact` | `KLogMetaCompactRequest` JSON body |

`cluster-state` returns `KLogClusterStateResponse`, including cluster identity,
server state, leader id, Raft term/vote/log progress, quorum-ack age, voters,
learners, and node descriptors. It also includes the raw OpenRaft metrics
summary as `raft_metrics` for manual diagnostics. The Raft diagnostics are
intended for investigating transient leader-view differences during restart or
transport recovery.

`meta-compact` is an explicit maintenance operation. It is submitted as a Raft
write command, so all voters and learners converge on the same compacted
revision. `klog_daemon` automatic compaction uses the same Raft command path.

Production exposure is intentionally not decided in this crate. BuckyOS should
keep admin routes behind local gateway/internal ACLs; see the daemon deployment
notes for the current authority boundary.

## Cluster Transport

Peer routing is controlled by `KClusterTransportConfig`:

- `direct`: call the peer's advertised host and plane-specific port directly.
- `gateway_proxy`: call the local gateway route for the target node.
- `hybrid`: try direct and gateway candidates.

Gateway proxy paths use the configured route prefix and target node name, for
example:

```text
/.cluster/klog/{node_name}/raft/{suffix}
/.cluster/klog/{node_name}/inter/{suffix}
/.cluster/klog/{node_name}/admin/{suffix}
```

Forwarded data/meta operations carry:

- `x-klog-forward-hops`
- `x-klog-forwarded-by`
- `x-klog-trace-id`

The service rejects excessive forwarding hops to avoid loops.

## Error Model

HTTP APIs return `KLogErrorEnvelope` on failure:

- `error_code`
- `message`
- `retryable`
- `leader_hint`
- `trace_id`

The same `trace_id` is also returned in `x-klog-trace-id`. JSON-RPC errors put
the same envelope in `error.data`.

Retryable error codes are:

- `NOT_LEADER`
- `LEADER_UNAVAILABLE`
- `CONFIG_CHANGE_IN_PROGRESS`
- `TIMEOUT`
- `UNAVAILABLE`

## Storage

`logs` provides OpenRaft log storage implementations:

- memory
- RocksDB
- SQLite

`state_store` provides replicated state-machine storage for log entries and
metadata KV:

- memory
- RocksDB

Snapshots include both appended log entries and metadata KV state. RocksDB is
the production-oriented state store for daemon deployments.

## Local Validation

Run from `src/`:

```bash
cargo fmt -p klog
cargo clippy -p klog -- -D warnings
cargo test -p klog
```
