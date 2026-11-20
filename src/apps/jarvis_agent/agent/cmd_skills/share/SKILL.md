---
name: share
description: Give someone a file or folder from the Zone as a link — snapshot, optional password and expiry — then list your shares, see views and downloads, change or stop them.
requires: the buckyos command (agent container: /opt/buckyos/bin/agent-tools/buckyos; host: $BUCKYOS_ROOT/bin/buckyos); the file must be readable by your owner; creating or changing shares requires read_write Agent permission
baseline: standard
scope: ui, work
---
Share only what the user asked to send. Confirm before sharing anything else. For `/srv/publish`, give its existing public URL, `https://public.<zone>/<path>`. For MessageHub attachments or HomeStation posts, the user chooses Files → Share… in Desktop.

#### Create a link

1. Use the path in Zone Files (e.g. `/home/<owner>/Documents/report.pdf` or `cyfs:///home/<owner>/…`). For files created in your AgentRoot, use `/home/<owner>/agents/<agent_id>/…`. Copy temporary or runtime-only files into AgentRoot first.
2. Run the command below. Default expiry is 7 days. Over 64 MiB, creation can return `state: running` while the snapshot is prepared (Task `nfs.share_snapshot/v1` when available). Use `--wait` or check `get` later.
3. Give the returned `url` to the user and send any password separately. Keep `share_id` for management (`operation_id` tracks creation). The link serves a snapshot. If `source_changed: true` or the user needs subsequent edits, share again.

```text
buckyos share create <path> [--expires 1d|7d|30d|none|<RFC3339>] [--password <p>] [--title <t>] [--no-download] [--wait]
buckyos share list [--kind link|app_delivery] [--status active|stopped|expired] [--q <text>] [--limit 50]
buckyos share get <share_id>
buckyos share update <share_id> [--expires …] [--password <p> | --no-password] [--title <t>] [--download false]
buckyos share stop <share_id> [--reason <text>]
buckyos share stats [--share-id <share_id>] [--days 30] [--granularity day|hour]
```

`get` shows policy, snapshot obj_id, delivery, audit, totals and `excluded` items. Folders exclude unreadable entries, symlinks, `.git/`, `node_modules/`, `.env*`, `*.key`, `*.pem` and `.DS_Store`. Limits are 64 GiB and 20,000 items. `stats` counts link opens, saved downloads, bytes and visitors (once per share/day). Previews do not count as downloads. Public folder, MessageHub and HomeStation readership is outside link stats.

The owner can see and stop links in Desktop's My Content. Agent shares record `actor_app_id` and `source: agent`. On the node host, `buckyos --yes share …` uses device identity and records the zone owner and `source: cli`.

#### Errors

| Reason | Action |
| --- | --- |
| `SHARE_AGENT_READ_ONLY` (`PERMISSION_DENIED`, `agent_read_only`) | You may list/read shares and write AgentRoot. Give the file path for the owner's Desktop share dialog. Changes and app handoffs require read_write. |
| `SHARE_SOURCE_NOT_READABLE` / `SHARE_SOURCE_UNSUPPORTED` | Check owner access and that the path is a file or folder. |
| `SHARE_TOO_LARGE` / `SHARE_TOO_MANY_ITEMS` | Share a smaller folder. |
| `SHARE_PLAN_STALE` | Source changed since planning. Run create again. |
| `SHARE_REVISION_CONFLICT` | Retry update/stop once to re-read the revision. |
| `SHARE_SHARE_NOT_ACTIVE` | Link stopped or expired. Create a new share. |
| `SHARE_NAMED_STORE_UNAVAILABLE` / `SHARE_SHARE_HOST_UNAVAILABLE` / `SHARE_UNSUPPORTED` | Report Zone file service failure to the owner. |

Schema: `buckyos command describe share <verb>` (websdk `cli/modules/share.ts`). BuckyOS 0.7.0, 2026-10-19: all six verbs checked with an injected user session and host device identity. Agent-container token untested.
