# klog_daemon TODO

This file tracks current implementation gaps after the BuckyOS integration work.

## P0: Integration Validation

- [ ] Validate true gateway-to-gateway klog cluster transport in the real multi-node/cascade DV setup.
  - Current local DV coverage validates source gateway -> target gateway -> local klog roundtrips with `KLOG_CLUSTER_DV_ROUTE_MODE=target-gateway`.
  - The earlier target gateway `invalid authority` failure is fixed by the latest cyfs-gateway origin-form URI forwarding behavior.
  - Keep this open until the same path is verified in a real multi-node/cascade DV topology instead of the local multi-process harness.
- [ ] Validate real system_config service on the klog backend in multi-OOD mode.
  - The first high-level klog meta KV semantics DV is covered locally.
  - The real `/kapi/system_config` service path is covered locally by
    `test/klog_system_config_service_dv.sh`, which starts a 3-node klog cluster
    and an isolated `system_config` process with `BUCKYOS_SYSTEM_CONFIG_STORE=klog`.
  - The klog crate now has an atomic meta transaction primitive for multi-key
    CAS/write semantics, which is the storage prerequisite for
    `sys_config_exec_tx`.
  - `system_config` now has an opt-in klog provider and an explicit
    `BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED=true` seed helper that
    writes the first rollout seed as one klog meta transaction.
  - The local multi-OOD rollout rule is covered by
    `test/klog_system_config_rollout_dv.sh`: one OOD bootstraps from sled, a
    second OOD starts without the bootstrap flag and reads/writes the same klog
    backend without copying its local sled state.
  - Keep this open until the same system_config service path is verified in a
    true multi-OOD/cascade DV topology instead of the local multi-process harness.

## P1: Security / Admin API

- [ ] Integrate BuckyOS node/session token validation for `/klog/admin/*`.
  - Current state: admin APIs only enforce `admin.local_only` loopback checks.
  - The BuckyOS session token loaded by `klog_daemon` is used for runtime integration, not admin API authentication.
  - Start this only after the P0 admin exposure boundary is decided.
- [ ] Add role-based authorization for admin APIs.
  - Write/admin operations: `add-learner`, `remove-learner`, `change-membership`.
  - Read/admin operations: `cluster-state`.
- [ ] Add token/key rotation and reload support without daemon restart.

## P2: MVCC / Watch API

- [ ] Add automatic MVCC compaction policy.
  - Manual compaction and compacted-revision errors are implemented.
  - Long-running deployments still need an explicit retention policy and
    trigger strategy before watch APIs are treated as production interfaces.
- [ ] Add production watch lifecycle semantics.
  - The first change-feed API is active polling with cursor resume.
  - A complete MVCC watch model still needs client resume rules, watch
    cancellation behavior, backpressure limits, observability, and clear
    handling for watches that fall behind the compacted revision.
- [ ] Consider push/stream watch APIs only if active polling becomes insufficient.
  - The first watch-compatible interface is short long-poll over
    `/klog/data/meta-changes` and JSON-RPC `klog.meta.changes`.
  - SSE/WebSocket/gRPC streaming remains deferred until gateway behavior,
    cancellation, backpressure, and operational demand are clear.
## P3: Deferred / Not Planned Now

- [ ] Consider Scheme 2 read optimization only if strong-read traffic shows leader bottleneck.
  - Scheme 1 remains the default: leader performs the linearizable barrier and follower/learner requests forward to the leader.
  - Scheme 2 would let followers perform ReadIndex/barrier and return local reads.
  - This needs strict timeout/retry/error mapping and observability before it is worth implementing.
- [ ] Add non-OOD follower deployment mode only if a future BuckyOS requirement needs it.
  - Current BuckyOS design runs klog/System Config on OOD nodes as voters.
  - Ordinary nodes read/write System Config through any OOD, so they do not require local klog followers.

## Completed / Verified

- [x] Restrict admin APIs to loopback source by default (`admin.local_only = true`).
- [x] Finalize the BuckyOS production policy for exposing the klog admin plane.
  - OOD voter admin calls use node gateway internal cluster routes only.
  - ZoneGateway/public business routes must not expose `/klog/admin/*`.
  - Covered by `src/kernel/klog_daemon/readme.md`.
- [x] Add cluster identity fields in `cluster-state` and verify them during auto-join.
- [x] Add integration test for wrong-cluster seed target rejection.
  - Covered by `src/kernel/klog_daemon/tests/cluster_identity.rs`.
- [x] Add dual-bootstrap conflict test with two `auto_bootstrap=true` nodes.
  - Covered by `src/kernel/klog_daemon/tests/bootstrap_conflict.rs`.
- [x] Keep Scheme 1 as the default strong-read strategy.
  - Covered by follower forwarding and multi-node read/write tests.
- [x] Validate forwarding hop-limit safety for forwarded data/meta requests.
- [x] Validate gateway cluster transport and leader failover smoke path.
  - Covered by `test/klog_gateway_cluster_failover_smoke.sh`.
  - Local DV now defaults to the real target-gateway second-hop route; set `KLOG_CLUSTER_DV_ROUTE_MODE=direct-plane` only for legacy diagnostics.
- [x] Validate membership admin semantics through gateway roundtrip.
  - Covered by `test/klog_membership_dv.sh`.
- [x] Validate local OOD voter membership changes through gateway roundtrip.
  - Covered by `test/klog_ood_membership_dv.sh`.
  - Validates `3 voters <-> 4 voters`, `2 voters <-> 3 voters`, and
    `1 voter <-> 2 voters` flows, including add as learner, promote to voter,
    demote, remove learner, and log/meta roundtrips after each topology change.
- [x] Validate local OOD snapshot catch-up during membership changes.
  - Covered by `test/klog_ood_snapshot_membership_dv.sh`.
  - Forces low snapshot thresholds, writes bulk log/meta data, adds a new OOD
    as learner, verifies the learner receives a persisted snapshot and sees the
    pre-existing data, promotes it to voter, demotes/removes that added OOD, and
    checks data consistency on the remaining voters.
- [x] Define safe admin semantics for demoting/removing the current leader.
  - `change-membership` now rejects direct current-leader removal with HTTP 409
    instead of committing a membership that can leave remaining voters with
    `current_leader=None`.
  - Covered by `test_admin_change_membership_rejects_current_leader_demotion`.
- [x] Validate local OOD leader passive failover followed by 3-to-2 voter shrink.
  - Covered by `test/klog_ood_leader_failover_shrink_dv.sh`.
  - Validates `3 voters -> stopped leader -> 2 voters elect new leader ->
    gateway log/meta reads and writes -> change-membership to 2 voters ->
    continued log/meta reads and writes`.
- [x] Validate local OOD single-to-two voter expansion.
  - Covered by `test/klog_ood_single_to_two_dv.sh`.
  - Validates `1 voter -> add learner -> promote to 2 voters` with gateway
    log/meta witness checks before learner join, after learner sync, and after
    voter promotion.
- [x] Validate local OOD two-voter quorum loss boundary.
  - Covered by `test/klog_ood_two_voter_loss_dv.sh`.
  - Validates that `2 voters -> stopped leader -> 1 survivor` does not elect a
    replacement leader and rejects strong reads/writes without quorum.
- [x] Validate full restart recovery for logs, meta revision, and membership.
  - Covered by `test/klog_restart_recovery_dv.sh`.
- [x] Validate klog meta KV semantics needed by system_config replacement.
  - Covered by `test/klog_system_config_kv_dv.sh`.
  - Validates create-as-CAS, stale revision conflict, strong read, prefix listing, and delete through the target-gateway route.
- [x] Validate real system_config kRPC methods on the klog backend locally.
  - Covered by `test/klog_system_config_service_dv.sh`.
  - Validates `create/get/set/set_by_json_path/append/list/delete/exec_tx` and
    `dump_configs_for_scheduler` through `/kapi/system_config`.
- [x] Validate local multi-OOD system_config klog rollout rule.
  - Covered by `test/klog_system_config_rollout_dv.sh`.
  - Validates that only the bootstrap OOD copies sled data into klog and that a
    non-bootstrap OOD directly reads/writes the shared klog backend.
- [x] Add klog atomic meta transaction primitive for system_config replacement.
  - `KLogMetaTxRequest` supports multi-key put/delete actions plus an optional
    optimistic guard revision.
  - Covered by klog state-machine and JSON-RPC client tests.
- [x] Add first-stage MVCC-compatible meta revision semantics.
  - Meta writes now allocate a global `mod_revision`; delete writes tombstone
    state, and delete/recreate does not reset the CAS revision.
  - Covered by klog state-store, state-machine, and klog_daemon meta revision
    tests.
- [x] Expose explicit MVCC metadata on klog meta APIs.
  - `KLogMetaEntry`, meta put/query/delete, and meta transaction responses now
    expose `create_revision`, `mod_revision`, and `version`; `revision` remains
    a compatibility alias for `mod_revision`.
- [x] Add first historical MVCC query support for klog metadata.
  - `KLogMetaQueryRequest.revision` supports key and prefix reads against the
    visible value set at a previous global revision.
  - Covered by RocksDB state-store tests and the klog_daemon meta revision
    client test.
- [x] Add revision-major metadata change index for future watch/change-feed APIs.
  - RocksDB persists a `(mod_revision, key)` index beside the key-major history
    index; MemoryStateStore keeps the same ordering in memory.
  - Snapshot install rebuilds the index from metadata history.
  - Covered by RocksDB state-store tests for ordering, cursor pagination,
    filters, tombstones, and snapshot install.
- [x] Expose first metadata change-feed API.
  - `/klog/data/meta-changes` and JSON-RPC `klog.meta.changes` support one-shot
    scans and short long-poll active querying with `wait_timeout_ms`.
  - The API returns change records, `current_revision`, `next_start_revision`,
    and an exclusive `(revision, key)` cursor when more matching changes remain.
- [x] Add explicit MVCC metadata compaction.
  - `/klog/admin/meta-compact` submits a Raft state-machine command and
    persists `meta_compacted_revision`.
  - Storage keeps one key-major baseline per compacted key for post-compaction
    historical reads, drops old revision-major change-feed index entries, and
    returns compacted-revision errors through HTTP/JSON-RPC query paths.
  - Covered by klog state-machine/RocksDB state-store tests and the
    klog_daemon meta compaction client test.
- [x] Validate klog MVCC behavior through a local gateway cluster DV.
  - Covered by `test/klog_mvcc_cluster_dv.sh`.
  - Validates 3-voter gateway cluster routes, atomic meta transactions,
    historical prefix reads, change-feed cursor pagination, explicit
    compaction/compacted-revision errors, and restart persistence.
- [x] Validate high-level system_config MVCC behavior on the klog backend.
  - Covered by `test/klog_system_config_mvcc_dv.sh`.
  - Validates real `/kapi/system_config` create/set/set-by-json-path/delete,
    guarded `exec_tx` conflict atomicity, delete/recreate revision semantics,
    klog historical reads, change-feed visibility, and explicit compaction
    through the local 3-voter gateway cluster harness.
- [x] Integrate BuckyOS OOD-voter deployment source.
  - Scheduler derives klog voters from `boot/config.oods` when `deployment.mode = "ood_voters"`.
- [x] Replace the placeholder `src/kernel/klog/readme.md` with protocol/API documentation.
  - The `klog` crate readme now documents scope, data model, HTTP/JSON-RPC/admin APIs, cluster transport, error model, storage, and validation commands.
