# klog_daemon TODO

## Security / Admin API
- [ ] Integrate BuckyOS node token mechanism for `/klog/admin/*` authentication.
- [ ] Add role-based authorization:
  - write admin APIs: `add-learner`, `remove-learner`, `change-membership`
  - read admin APIs: `cluster-state`
- [ ] Add token rotation/reload support without daemon restart.

## Current Temporary Policy
- [x] Restrict admin APIs to loopback source by default (`admin.local_only = true`).
- [ ] For multi-machine cluster management, define secure deployment guideline:
  - set `admin.local_only = false` only on trusted internal network
  - enforce firewall/ACL for admin endpoints

## Cluster Follow-ups
- [x] Add cluster identity fields in `cluster-state` (e.g. cluster name/id) and verify during auto-join.
- [ ] Add integration test for wrong-cluster seed target rejection.
- [ ] Add dual-bootstrap conflict test (2 nodes with `auto_bootstrap=true`).

## Read Consistency Strategy
- [x] Keep Scheme 1 as default for `strong_read=true`:
  - read must be served by leader after linearizable barrier
  - follower/learner auto-forward to leader via raft network path
  - keep eventual read as default for normal query (`strong_read=false`)
- [ ] Consider Scheme 2 optimization later: follower performs ReadIndex/barrier and returns local read.
  - apply only when follower read traffic is high and leader read path becomes bottleneck
  - require strict timeout/retry/error mapping design to avoid hidden stale-read risk
  - require additional observability: barrier latency, fallback-to-leader count, stale-read guard metrics
  - keep Scheme 1 as fallback path during rollout/canary

## Notes
- Current design target:
  - reliability and correctness first
  - performance optimization (Scheme 2) second
- Strong read forwarding loop safety:
  - keep and validate hop limit headers in every forward
  - reject/record abnormal loops with clear warn/error logs
