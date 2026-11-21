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

## 用量与成本核对

AICenter 的 Usage 消费卡统计当前筛选条件下的全部记录，不限当前分页；金额按币种分开显示。点击 token 数值可以查看已上报的缓存、模态 token、图片张数、音视频秒数、字符和请求单位等明细，不把这些不同单位相加。不存在的维度不会显示；部分上报只代表已知小计。

调用结果的 `usage.cost_source` 区分 Provider 实报和静态价格估算；`usage.cost_unknown_reason` 说明缺失用量、费率或币种。费用未知不表示免费。媒体任务已成功但费用未知时，先核对结果与计费用量，不为获取费用重复生成。历史记录不会随新价格自动改写。
