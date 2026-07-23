# kevent / kmsg tests

This directory keeps the gateway, restart, and peer test entries for the
kevent/kmsg test plan.

Subdirectories:

- `dv`: standard devtest gateway smoke for kmsg and kevent.
- `task_mgr`: TaskMgr `task_ready` gateway smoke.
- `restart`: standard devtest restart recovery smoke.
- `peer_container`: two-node peer delivery in Docker plus a container Full SDK client publishing/subscribing through a host native bridge.
- `peer_vm`: two-node peer delivery harness in QEMU/KVM VMs.
- `reports`: local per-run reports and evidence snapshots. This directory is
  ignored by git; the repository keeps only the test plan and current summary
  conclusion.

Run from the repository root:

```bash
uv run test/run.py -p kevent_kmsg/dv
uv run test/run.py -p kevent_kmsg/task_mgr
uv run test/run.py -p kevent_kmsg/restart
uv run test/run.py -p kevent_kmsg/peer_container
uv run test/run.py -p kevent_kmsg/peer_vm
```

`uv run test/run.py -p kevent_kmsg` prints this grouped entry and does not run
the heavier subcases unless `BUCKYOS_KEVENT_KMSG_CASES` is set.
