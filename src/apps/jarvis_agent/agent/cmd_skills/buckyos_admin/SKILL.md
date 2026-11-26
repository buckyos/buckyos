---
name: buckyos_admin
description: Inspect and manage a BuckyOS Zone — identify runtime, identity and versions, read or change settings and apps, check tasks, query logs and diagnostic bundles, and trace a fault into the source of the running version.
requires: the buckyos command (agent container: /opt/buckyos/bin/agent-tools/buckyos with the host Tool mounted at /opt/buckyos/libexec/buckyos-tool; host: $BUCKYOS_ROOT/bin/buckyos); writes depend on the agent's and its owner's RBAC
baseline: standard
---
## Inspect a Zone

```sh
command -v buckyos && buckyos --version --verbose
buckyos auth whoami
buckyos system info
buckyos system status
```

Record the runtime, principal, appid, Zone and running build. An Agent acts with its own token and its owner's grants. A missing Tool mount or refused operation calls for the exact host/owner procedure. Never switch identity to bypass a refusal. Keep credentials, invite secrets, private keys and password hashes out of evidence.

Discover available verbs with `buckyos command list` and `buckyos command describe <module> <verb>` (syntax, access, async mode). Read `data` or `error` and keep `meta.trace_id` from stdout; progress goes to stderr. Narrow a fault by time, App/instance, task and service:

```text
buckyos app status <app_instance_id_or_app_did>
buckyos task list --state running --limit 20
buckyos task get <task_id>
buckyos log services
buckyos log query --service <id> --since <RFC3339> --limit 100
```

## Change a setting or App

1. Use the owning App, Desktop page or control-panel RPC. Read the settings guide before using raw system-config. Scheduler outputs, runtime reports, signed documents and RBAC keys have their own writers.
2. For an editable key, save the current value and version from `buckyos system-config get <key>`. Extract the original `data.value` string for rollback. Edit one field, recheck the version, then write the full value with `buckyos system-config set-file <key> --file new.json`; the CLI has no compare-and-set.
3. For App install, upgrade or uninstall, check `--dry-run` before an authorized `--yes` write. Start, stop, restart and config writes take effect without prompting. Query an existing task before retrying and reuse its idempotency key where supported.
4. Read back value/version and service reload logs, or wait with `buckyos --timeout 10m task wait <task_id>` and check desired, scheduled and running App state plus the endpoint. Give before/after evidence and a revert. Restore the saved value only while the current value is still yours.

## Read for the affected component

- Host/container paths and identity, startup, Agent runtime assignment, native mode, Lite or macOS permissions: read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_admin/references/runtimes.md`.
- Node Executor profiles, grants or remote desktop/daemon operations: read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_admin/references/executors.md`.
- Accounts, groups, SSO, invites, Apps, devices, network or writable keys: read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_admin/references/settings.md`.
- Buckets, previews, missing attachments, object URLs or extension tool packages: read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_admin/references/storage-tools.md`.
- AI Center, provider models, rules, quota, usage or gateway timeouts: read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_admin/references/providers.md`.
- Logs, diagnostic bundles, expired sessions, Agent recovery, node upgrades or tracing a fault into the running source: read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_admin/references/diagnosis.md`.

For App development or release use `buckyos_app_dev` or `buckyos_dev_release`; for model calls use `xllm` or the relevant `aigc_*` skill.

For AICC failures, distinguish `no_candidate_model` (check Provider inventory and requested capabilities) from `resource_not_permitted` (check the caller’s model grants). Dynamic-login authentication rejection without usage triggers one re-login and retry on the same Provider and model; persistent rejection requires fixing that Provider. Keep the Provider, model and upstream error in diagnostic evidence without exposing credentials.
