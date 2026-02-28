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
- [ ] Add cluster identity fields in `cluster-state` (e.g. cluster name/id) and verify during auto-join.
- [ ] Add integration test for wrong-cluster seed target rejection.
- [ ] Add dual-bootstrap conflict test (2 nodes with `auto_bootstrap=true`).
