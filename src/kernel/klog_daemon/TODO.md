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

- [ ] Make Raft snapshot file retention configurable.
  - Current state: klog keeps the newest 3 snapshot files to avoid deleting a
    snapshot that is still being streamed by `install-snapshot`.
  - This slightly increases disk usage versus retaining only the latest
    snapshot. If production deployments need tighter disk control, expose the
    retain count as a klog daemon/storage config field.
- [ ] Make the local-leader write quorum freshness window configurable if needed.
  - Current state: data/meta writes submitted to the local leader require a
    fresh quorum ack before creating a Raft proposal, so failed writes during
    quorum loss do not later apply after quorum recovery.
  - The first implementation uses a fixed conservative freshness window. Keep
    it fixed unless production telemetry shows it needs per-deployment tuning.
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
- [x] Validate membership shrink and removed voter rejoin persistence.
  - Covered by `test/klog_raft_membership_change_rejoin_dv.sh`.
  - Validates `3 voters -> stopped non-leader voter -> change-membership shrink
    to remaining 2 voters -> removed node restarts with old local state` does
    not alter the active cluster membership, then add learner/promote catches up
    and restores the 3-voter set.
- [x] Validate concurrent membership admin mutation conflict handling.
  - Covered by `test/klog_raft_concurrent_membership_dv.sh`.
  - Validates two concurrent add-learner requests on the same leader produce
    exactly one success and one `409 Conflict`, then promotes the accepted
    learner and verifies gateway log/meta consistency.
- [x] Validate auto-join retry idempotency after add-learner client timeout.
  - Covered by `test/klog_raft_join_retry_idempotency_dv.sh`.
  - Validates blocking add-learner can commit after the auto-join HTTP client
    times out, the retry observes the node as an existing learner, does not
    duplicate membership, and does not promote when `target_role=learner`.
- [x] Validate learner snapshot install crash/restart recovery.
  - Covered by `test/klog_raft_snapshot_install_crash_dv.sh`.
  - Validates a learner killed after receiving snapshot chunks through
    `snapshot.temp` can restart, reinstall snapshot state, catch up joining-era
    log/meta data, and continue gateway log/meta writes.
- [x] Validate full restart recovery for logs, meta revision, and membership.
  - Covered by `test/klog_restart_recovery_dv.sh`.
- [x] Validate klog meta KV semantics needed by system_config replacement.
  - Covered by `test/klog_system_config_kv_dv.sh`.
  - Validates create-as-CAS, stale revision conflict, strong read, prefix listing, and delete through the target-gateway route.
- [x] Validate real system_config kRPC methods on the klog backend locally.
  - Covered by `test/klog_system_config_service_dv.sh`.
  - Validates `create/get/set/set_by_json_path/append/list/delete/exec_tx` and
    `dump_configs_for_scheduler` through `/kapi/system_config`.
- [x] Validate real system_config klog backend across klog leader failover.
  - Covered by `test/klog_system_config_leader_failover_dv.sh`.
  - Points `system_config` at a non-leader klog RPC endpoint, kills the current
    klog leader during a write, validates the transient kRPC error path, then
    retries reads/writes after new leader election and verifies old-leader
    rejoin catches up the klog-backed keys.
- [x] Validate gateway abnormal routing diagnostics and no-miswrite behavior.
  - Covered by `test/klog_gateway_abnormal_dv.sh`.
  - Stops the target gateway and patches a source gateway route to a stale
    address, then verifies data-route writes fail without landing in klog and
    admin-route failures include diagnosable route/status context.
- [x] Validate system_config stale klog config after OOD shrink.
  - Covered by `test/klog_system_config_stale_config_rejoin_dv.sh`.
  - Shrinks a 3-voter cluster to 2 voters, restarts the removed OOD's
    `klog-service` with its old config, then starts a real `system_config`
    instance against that stale local klog endpoint and verifies failed writes
    do not land in active klog or re-add the removed OOD.
- [x] Validate duplicate klog node id replacement is rejected.
  - Covered by `test/klog_node_id_reuse_dv.sh`.
  - Starts a replacement `klog-service` with an existing `node_id` but different
    data directory, ports, node name, and device id, then verifies admin
    add-learner and auto-join both fail with explicit node identity diagnostics.
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
- [x] Validate metadata change-feed long-poll semantics through gateway DV.
  - Covered by `test/klog_mvcc_change_feed_dv.sh`.
  - Validates empty `wait_timeout_ms` waits, wait-until-write wakeup,
    cursor resume, compacted cursor/start revision returning `COMPACTED`, and
    post-compaction change-feed continuation.
- [x] Validate large revision change-feed pagination under stress.
  - Covered by `test/klog_mvcc_change_feed_stress_dv.sh`.
  - Configurable by key count, concurrency, update rounds, page size, and
    round delay; validates many revision-major pages, delete/recreate
    tombstones, compaction, compacted cursor resume, post-compaction resume,
    and current-state consistency across all gateway nodes.
- [x] Add explicit MVCC metadata compaction.
  - `/klog/admin/meta-compact` submits a Raft state-machine command and
    persists `meta_compacted_revision`.
  - Storage keeps one key-major baseline per compacted key for post-compaction
    historical reads, drops old revision-major change-feed index entries, and
    returns compacted-revision errors through HTTP/JSON-RPC query paths.
  - Covered by klog state-machine/RocksDB state-store tests and the
    klog_daemon meta compaction client test.
- [x] Add automatic MVCC metadata compaction policy.
  - `klog_daemon` supports optional leader-only `revision_count` auto
    compaction through `meta_compaction` config and `KLOG_META_COMPACTION_*`
    environment variables.
  - Auto compaction submits the same Raft `CompactMeta` write command used by
    explicit admin compaction, so followers and learners converge through the
    state machine.
  - Covered by `test_single_node_auto_meta_compaction_triggers`.
- [x] Validate klog MVCC behavior through a local gateway cluster DV.
  - Covered by `test/klog_mvcc_cluster_dv.sh`.
  - Validates 3-voter gateway cluster routes, atomic meta transactions,
    historical prefix reads, change-feed cursor pagination, explicit
    compaction/compacted-revision errors, and restart persistence.
- [x] Validate klog MVCC behavior across leader failover.
  - Covered by `test/klog_mvcc_failover_dv.sh`.
  - Writes MVCC history, stops the current leader, verifies the remaining voter
    quorum continues delete/recreate and transaction writes, then compacts and
    checks compacted-revision errors, post-compaction historical reads, and
    change-feed records.
- [x] Validate MVCC snapshot catch-up during OOD membership changes.
  - Covered by `test/klog_mvcc_snapshot_membership_dv.sh`.
  - Forces low snapshot thresholds, writes MVCC history with tombstones and
    delete/recreate, compacts metadata, adds a new OOD as learner, verifies the
    learner catches up through snapshot with compacted-revision semantics,
    promotes it to voter, writes post-promote MVCC data, then demotes/removes it.
- [x] Validate explicit MVCC compaction while a learner is installing snapshot.
  - Covered by `test/klog_mvcc_compact_during_snapshot_dv.sh`.
  - Adds a learner with snapshot-sized MVCC history, observes non-empty
    `snapshot.temp`, then compacts on the leader and verifies the learner
    converges on compacted-revision errors, retained historical reads,
    change-feed continuation, and post-recovery gateway writes.
- [x] Validate high-level system_config MVCC behavior on the klog backend.
  - Covered by `test/klog_system_config_mvcc_dv.sh`.
  - Validates real `/kapi/system_config` create/set/set-by-json-path/delete,
    guarded `exec_tx` conflict atomicity, delete/recreate revision semantics,
    klog historical reads, change-feed visibility, and explicit compaction
    through the local 3-voter gateway cluster harness.
- [x] Validate multi-OOD system_config MVCC behavior on the klog backend.
  - Covered by `test/klog_system_config_multi_ood_mvcc_dv.sh`.
  - Starts one `system_config` process per OOD, points each at its local klog
    RPC endpoint, validates parallel creates, cross-OOD reads, guarded CAS
    conflict atomicity, delete/recreate tombstones, compaction, and scheduler
    dump visibility.
- [x] Integrate BuckyOS OOD-voter deployment source.
  - Scheduler derives klog voters from `boot/config.oods` when `deployment.mode = "ood_voters"`.
- [x] Replace the placeholder `src/kernel/klog/readme.md` with protocol/API documentation.
  - The `klog` crate readme now documents scope, data model, HTTP/JSON-RPC/admin APIs, cluster transport, error model, storage, and validation commands.
