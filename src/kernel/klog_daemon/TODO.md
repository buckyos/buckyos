# klog_daemon TODO

This file tracks current implementation gaps after the BuckyOS integration work.

## P0: Integration Validation

- [ ] Validate true gateway-to-gateway klog cluster transport in the real multi-node/cascade DV setup.
  - Current local DV coverage validates source gateway -> target gateway -> local klog roundtrips with `KLOG_CLUSTER_DV_ROUTE_MODE=target-gateway`.
  - The earlier target gateway `invalid authority` failure is fixed by the latest cyfs-gateway origin-form URI forwarding behavior.
  - Keep this open until the same path is verified in a real multi-node/cascade DV topology instead of the local multi-process harness.
- [ ] Finalize the BuckyOS production policy for exposing the klog admin plane.
  - Current docs cover the basic `admin.local_only` and gateway ACL model.
  - Still need the final rule for how OOD voter nodes expose admin routes and which gateway/RBAC layer enforces access control.
- [ ] Validate real system_config service on the klog backend in multi-OOD mode.
  - The first high-level klog meta KV semantics DV is covered locally.
  - Keep this open until `sys_config_get/set/create/delete/list/exec_tx` run through the actual system_config service backed by klog instead of the local sled provider.

## P1: Security / Admin API

- [ ] Integrate BuckyOS node/session token validation for `/klog/admin/*`.
  - Current state: admin APIs only enforce `admin.local_only` loopback checks.
  - The BuckyOS session token loaded by `klog_daemon` is used for runtime integration, not admin API authentication.
  - Start this only after the P0 admin exposure boundary is decided.
- [ ] Add role-based authorization for admin APIs.
  - Write/admin operations: `add-learner`, `remove-learner`, `change-membership`.
  - Read/admin operations: `cluster-state`.
- [ ] Add token/key rotation and reload support without daemon restart.

## P2: Documentation

- [ ] Replace the placeholder `src/kernel/klog/readme.md` with protocol/API documentation or link it to the daemon deployment docs.

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
- [x] Validate full restart recovery for logs, meta revision, and membership.
  - Covered by `test/klog_restart_recovery_dv.sh`.
- [x] Validate klog meta KV semantics needed by system_config replacement.
  - Covered by `test/klog_system_config_kv_dv.sh`.
  - Validates create-as-CAS, stale revision conflict, strong read, prefix listing, and delete through the target-gateway route.
- [x] Integrate BuckyOS OOD-voter deployment source.
  - Scheduler derives klog voters from `boot/config.oods` when `deployment.mode = "ood_voters"`.
