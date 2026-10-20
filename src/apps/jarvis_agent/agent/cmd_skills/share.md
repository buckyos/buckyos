---
name: share
description: Give someone a file or folder from the Zone as a link — snapshot, optional password and expiry — then list your shares, see views and downloads, change or stop them.
requires: the buckyos command (agent container: /usr/local/bin/buckyos; host: $BUCKYOS_ROOT/bin/buckyos); the file must be readable by your owner; creating or changing shares requires read_write Agent permission
---
#### When to use
A person needs a file you produced or one the owner keeps, outside this conversation: make a link and hand it over.
Posting to HomeStation or sending in MessageHub with the file attached is the user's step in the Desktop share dialog
(Files → right click → Share…); from here you deliver a link.

#### Commands
```
buckyos share create <path> [--expires 1d|7d|30d|none|<RFC3339>] [--password <p>] [--title <t>] [--no-download] [--wait]
buckyos share list [--kind link|app_delivery] [--status active|stopped|expired] [--q <text>] [--limit 50]
buckyos share get <share_id>          # policy, snapshot obj_id, excluded items, delivery, audit, totals
buckyos share update <share_id> [--expires …] [--password <p> | --no-password] [--title <t>] [--download false]
buckyos share stop <share_id> [--reason <text>]
buckyos share stats [--share-id <share_id>] [--days 30] [--granularity day|hour]
```
`buckyos command describe share <verb>` has the exact schema; source: `cli/modules/share.ts` in the mounted Tool.

#### Paths
- `<path>` is a Zone path as Files shows it, `/home/<owner>/Documents/report.pdf`, or `cyfs:///home/<owner>/…`.
- Docker AgentRoot is `/opt/buckyos/data/home/<owner>/.local/share/<app_id>/agents/<agent_id>/`; use the Zone path
  `/home/<owner>/.local/share/<app_id>/agents/<agent_id>/` when sharing a file there. `/tmp` and runtime/cache paths
  are not in the Zone; write the file under your AgentRoot first.
- The link carries a snapshot taken now. Later edits of the file are not in it; share again for a new version.
- Folders leave out `.git/`, `node_modules/`, `.env*`, `*.key`, `*.pem`, `.DS_Store`, symlinks and what the owner cannot
  read; `share get` lists them under `excluded`. Up to 20 000 items.

#### Results
- `create` returns `url` (`https://share.<zone>/s/<token>`), `share_id`, `operation_id`, `state`. Expiry defaults to
  7 days; `--expires none` keeps it until stopped. Give the url to the user; send the password through another channel.
- Over 64 MiB the snapshot is prepared in the background (`state: running`, a Task `nfs.share_snapshot/v1` when the
  Task Manager is reachable): pass `--wait`, or read `share get <share_id>` later.
- `source_changed: true`: the file changed while it was read; the link has what was read. Share again if that matters.
- The share is recorded with your agent id (audit `actor_app_id`, `source: agent`) and shows in the owner's
  Desktop app "My Content", where they can stop it. On a node host, `buckyos --yes share …` uses the device identity
  and creates shares owned by the zone owner (`source: cli`).
- `stats` counts link visits: views (page opens), downloads (saved copies; previews on the page are not), bytes,
  visitors (once per share per day). Not counted: the Public folder, MessageHub and HomeStation readers.

#### When it fails
Errors come as `SHARE_<REASON>`:
- `SHARE_AGENT_READ_ONLY` (`PERMISSION_DENIED`, reason `agent_read_only`): the owner selected `read_only` when
  creating the Agent. You can list/read shares and write your own AgentRoot, but cannot create/update/stop/hide
  shares or initiate/report app handoffs. Give the owner the file path and let them use the Desktop share dialog.
- `SHARE_SOURCE_NOT_READABLE`: your owner cannot read that path. `SHARE_SOURCE_UNSUPPORTED`: not a file or folder.
- `SHARE_TOO_LARGE` / `SHARE_TOO_MANY_ITEMS`: over 64 GiB or 20 000 items; share a smaller folder.
- `SHARE_PLAN_STALE`: the source changed between plan and create; run create again.
- `SHARE_REVISION_CONFLICT`: someone changed the share; `update`/`stop` re-read the revision, so just retry once.
- `SHARE_SHARE_NOT_ACTIVE`: already stopped or expired; create a new share.
- `SHARE_NAMED_STORE_UNAVAILABLE` / `SHARE_SHARE_HOST_UNAVAILABLE` / `SHARE_UNSUPPORTED`: the Zone's file service
  cannot share; report it to the owner.

#### Before you share
- Anything under `/srv/publish` is already public at `https://public.<zone>/<path>`; give that address instead.
- With `read_write` Agent permission, reading a file is enough to share it (a known risk): only share what the
  user asked to send, and confirm before sharing something outside the task.

Checked against: BuckyOS 0.7.0 + Tool/SDK from websdk source on test.buckyos.io, 2026-10-19 — create/list/get/update/
stop/stats with a user session injected like the container's, and `--yes` on the host as the device; not yet run from
inside an agent container with an Agent token.
