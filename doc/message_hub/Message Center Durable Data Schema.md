# MessageCenter Durable Data Schema

## 1. Overview

Service: `msg-center`

MessageCenter persists mailbox projections, delivery state, contacts, friend
requests, group subscriptions, idempotency results, tunnel recovery cursors, and
UI session state in the platform-provided RDB instance. Message objects and
attachments are content-addressed in the named store and retained by
msg-center's buckets (§3). The corresponding behavior and RPC model are described in
`doc/message_hub/Message Center.md` and
`doc/message_hub/Message Tunnel Design.md`.

## 2. Data Classification

### Durable Data

| Data item | Reason |
|---|---|
| `mailbox_records`, `msg_refs` | User mailbox truth and object references |
| `delivery_records` | Delivery queue state and diagnostic history |
| `msg_idempotency` | Prevents replay after restart |
| `msg_tunnel_cursors` | External ingress recovery position |
| `contacts`, `contact_metadata`, `group_subscribers` | Contacts and external-group subscriptions |
| `friend_requests` | Each side's record of a friend request; not reconstructible from messages |
| `group_object_purges` | Redacted messages whose bucket keys and named-store object still have to be removed |
| Stored messages (`messages` bucket) | Message bodies referenced by mailbox and delivery records |
| Attachments (`attachments` bucket) | Files referenced by messages; another Zone's sender may no longer have them |
| Collected attachments (`collected` bucket) | A user's own copy, kept after the message is recalled |

### Disposable Data

| Data item | Reason |
|---|---|
| Runtime keys in `ui_session_states` | Typing, active, and status_line are disposable even though the current implementation uses RDB KV |
| In-memory message/receipt caches | Named store or authoritative records remain available |
| kevent notifications | Acceleration signals only |

Persisted personal preferences such as a custom display title or draft are not
reconstructible from messages; do not classify the entire mixed-use
`ui_session_states` table as disposable. Shared/member business state and Action
Log data follow the target contract below, independently of these runtime keys.

## 3. Storage Strategy

All structured durable data uses the platform RDB instance and supports both
SQLite and PostgreSQL DDL. Hosted group messages keep their bodies in RDB
(`group_objects`); every other message body and every attachment is a
named-store object retained by a bucket binding (`buckets.rs`). No business
state is stored in service-private filesystem paths; the one exception is the
Telegram client's session files.

### Buckets

Declared in `system_config_builder.rs::add_msg_center` from
`buckyos_api::msg_center_bucket_decls()`. The app pool of `messages` +
`attachments` (`msg-center@_`) is `storage_quota_mib = 102400` (100 GiB): it
holds every user's chat attachments, so it is sized for years of a family or
small team; over it, a send with a new attachment fails with `quota_exceeded`
and an admin raises it with `bucket.set_pool_quota`. The two zone buckets set
`max_objects = 100_000_000`, since a chat Zone outgrows the default million keys.

| Bucket | kind / data_policy | Key | Written (who pins) | Unbound |
|---|---|---|---|---|
| `zone/msg-center/_/messages` | `object` / `app_data` | `<msg_id>` | `store_message` (`put_object_str`), before the records of a dispatch, `post_send`, friend message or notice commit | Group redaction (`purge_message`), followed by `remove_object` |
| `zone/msg-center/_/attachments` | `data` / `app_data` | `<session>/<msg_id>/<n>` | With the message, before its records commit: `store_message`, and `bind_attachments` for hosted group messages (body in RDB) | Group redaction: every key under `<session>/<msg_id>/`; a hosted group message that is not accepted gives up its keys |
| `zone/msg-center/_/attachments` (display files) | `data` / `app_data` | `<group address>/_display/<obj_id>` | When a group change makes a hosted group's display (EXT-04) show a file: `bind_display_files` before `group.create` / any `group.*` mutation commits, a file not in this Zone fails the change | The change after which no Session's display shows it (config edit, Session deleted, group deleted): `unbind_display_files` after the commit |
| `user/msg-center/<user>/collected` | `data` / `collected` | `<msg_id>/<n>/<label>` | `msg.collect_attachment` (`bucket.copy`, `origin = {author: sender DID, source: cyfs://<msg_id>}`) | The user, in Files or with `bucket.delete` |

- `<session>` is the message's own address: `session_key(group, to_session)`
  for a group message, the author's SENT mailbox address
  (`<from>/<derive_session_id(SENT)>`) otherwise. `<n>` is the position of the
  object among the message's distinct `DataObj` refs; only FileObject /
  DirObject refs are bound (a data bucket binds nothing else), other referenced
  objects are not attachments msg-center keeps.
- Display files: the bound set is the files of every live Session's effective
  display (`rules(sid).display.over(group display)`), each once per group
  (`_display` cannot be a Session id: `_`-prefixed ids are reserved). A group
  mutation compares that set before and after; only when it changed does it
  list `<group address>/_display/` and put the missing keys (before commit) and
  delete the extra ones (after commit; a failed delete is retried by the next
  change). They are read with `context_path=<session address>/_display`: the
  caller reads that Session (membership, guest participation, read hooks), its
  effective display lists the file, and the key holds it.
- Zone buckets grant no user anything. Attachments are read through
  `GET /kapi/msg-center/objects/{obj_id}[/content]?context_path=[<session address>/]<msg_id>`:
  the caller must be able to read that message (mailbox record or hosted group
  Session), and `bucket.get` of its key must hold `obj_id`; then msg-center reads
  the named store. Without `context_path`, the message is the newest one of the
  caller's own mailboxes (`msg_refs`, an Agent's first for the App the Agent
  runs as) that references `obj_id` and that the caller may read; restricted
  sessions must name it. The `collected` bucket is the user's: it is read directly at
  `/bucket/v1/user/msg-center/<user>/collected/<key>`.
- Private messages have no server-side delete: their keys are never released
  and no lifecycle rule is declared. A session deletion releases only the
  messages it redacts (those no mailbox record still references).
- A local message whose attachment is not in this Zone fails the send
  (`content_not_available`). A message from another Zone (signed ingress, or a
  remote member of a hosted group) binds what this Zone has and skips the
  rest with a warning: its sender's Zone keeps it.
- Uploads from MessageHub go through the NDM gateway, whose finalize holds the
  file 24h for the uploader (`ndm-hold:<user>|<app>`); `post_send` binds it
  into `attachments` within that window or fails. Telegram ingress puts the
  downloaded file with a 24h Lease (`msg-center-ingress:<obj_id>`) until its
  message binds it.
- A redaction queues `group_object_purges(obj_id, attachment_prefix)` in its
  transaction, the prefix read from the body before it is cleared; the group
  sync loop (every 30 s) deletes the keys, the `messages` key and the
  named-store object, then the row.

### Filesystem

| Path | Reason |
|---|---|
| `$BUCKYOS_ROOT/data/var/msg-center/tg_sessions` | grammers opens its SQLite session (bot credentials) by path |

## 4. Schema Definitions

### Table: `msg_idempotency`

| Column | Type | Nullable | Description |
|---|---|---|---|
| `scope` | TEXT | No | Operation: `dispatch` or `post_send` |
| `owner_scope` | TEXT | No | Stable recipient owner for dispatch; canonical sender mailbox address for post-send |
| `idempotency_key` | TEXT | No | Caller/platform replay key |
| `msg_id` | TEXT | Yes | Content-addressed message id |
| `retention_key` | TEXT | Yes | Stable external conversation or local sender bucket |
| `state` | TEXT | No | `pending` or `completed` |
| `result_json` | TEXT | Yes | Serialized RPC result |
| `created_at_ms` | INTEGER/BIGINT | No | Creation time |
| `updated_at_ms` | INTEGER/BIGINT | No | Last update time |
| `expires_at_ms` | INTEGER/BIGINT | Yes | Physical cleanup eligibility time |

Primary key: `(scope, owner_scope, idempotency_key)`.

Indexes:

- `idx_msg_idempotency_expire(expires_at_ms)` supports global expiry cleanup.
- `idx_msg_idempotency_retention_expire(retention_key, expires_at_ms)` supports conversation-bucket cleanup.

### Table: `msg_tunnel_cursors`

| Column | Type | Nullable | Description |
|---|---|---|---|
| `tunnel_key` | TEXT | No | Platform, owner, tunnel account, and tunnel instance identity |
| `cursor_key` | TEXT | No | Cursor kind, such as `bot_api_update_offset` |
| `value_json` | TEXT | No | Opaque validated cursor value |
| `updated_at_ms` | INTEGER/BIGINT | No | Last successful persistence time |

Primary key: `(tunnel_key, cursor_key)`. This table is internal to msg-center;
the public UI SessionState RPC cannot read or modify it.

### Table: `friend_requests` (version 13)

One row per friend request as seen by one owner; the other side keeps its own
row in its own msg-center. Written by `friend_service.rs`; behavior in
`Contact Mgr.md` §3.4.

| Column | Type | Nullable | Description |
|---|---|---|---|
| `owner_key` | TEXT | No | Owner DID whose side this row is |
| `request_id` | TEXT | No | Id carried by every `buckyos.friend` message of the request; idempotency key |
| `peer_did` | TEXT | No | Canonical DID of the other side |
| `direction` | TEXT | No | `incoming` or `outgoing` |
| `state` | TEXT | No | `pending`, `accepted`, `rejected`, `cancelled`, `expired` or `ignored` |
| `message` | TEXT | Yes | Note attached to the request (at most 500 characters) |
| `msg_id` | TEXT | Yes | MsgObject that carried the request |
| `peer_name` | TEXT | Yes | Incoming: the sender's `profile_hint.name`; outgoing: the local contact name |
| `peer_avatar` | TEXT | Yes | Same, for the avatar |
| `delivery_json` | TEXT | Yes | Outgoing only: last known `FriendDelivery` (`pending` / `delivered` / `failed`, `reason`); a pending value is re-derived from `delivery_records` when read |
| `created_at_ms` | INTEGER/BIGINT | No | Creation time |
| `expires_at_ms` | INTEGER/BIGINT | No | Creation + 7 days (incoming: capped at receipt + 7 days) |
| `updated_at_ms` | INTEGER/BIGINT | No | Last state change |

Primary key: `(owner_key, peer_did, request_id)`.

Indexes:

- `idx_friend_requests_owner(owner_key, state, updated_at_ms)` supports listing an owner's requests.
- `idx_friend_requests_expire(state, expires_at_ms)` supports the expiry sweep (every 10 minutes).

Rows are kept when the contact is removed.

### Contact payload (`contacts.payload`)

`contacts(owner_key, did, payload)` stores one `buckyos_api::Contact` as JSON:
`did`, `name`, `avatar?`, `note?`, `source`, `is_verified`, `bindings?`,
`access_level` (`block` / `stranger` / `temporary` / `allow`), `temp_grants?`,
`friend_state` (`none` / `outgoing_pending` / `incoming_pending` / `friend`),
`tags?`, `created_at`, `updated_at`. The `groups` field no longer exists.

Only the current payload format is read. A payload that still carries `groups`
or an access level outside the list above fails the owner store load, so every
operation on that owner returns an error; the stored rows are neither rewritten
nor deleted.

### Table: `group_object_purges`

| Column | Type | Nullable | Description |
|---|---|---|---|
| `obj_id` | TEXT | No | Redacted message (primary key) |
| `attachment_prefix` | TEXT | Yes | `<session>/<msg_id>/` of its `attachments` keys, from the body before it was cleared; absent when no body was held, then derived from the stored message if there is one |
| `schema_version` | BIGINT | No | `1` |

Created by `GroupStore::open`. The attachment retention tables
(`msg_attachment_retentions`, `msg_attachment_issues`, `msg_attachment_meta`)
and the `msg-center:<msg_id>` pins are no longer created; an installation that
has them is reinstalled.

### Existing durable tables

`mailbox_records`, `delivery_records`, `msg_refs`, contact tables, and
`group_subscribers` retain their schema defined by `MSG_CENTER_RDB_SCHEMA_SQLITE` and
`MSG_CENTER_RDB_SCHEMA_POSTGRES`. Message objects retain their existing named
store object schema.

The v1 self-host tables (`groups`, `group_members`, `group_member_proofs`,
`group_subgroups`, `group_events`, `group_expansion_snapshots`) are no longer
created or used. Self-host Group v2 has no implementation yet.

### Target logical records: Session state and Action Log (2026-09-06, not implemented)

The canonical field and behavior definitions are in
[Session State and Action Log.md](<./Session State and Action Log.md>).
This supplement does not change DDL or the current schema version.

| Logical record | Key / relationship | Durability |
|---|---|---|
| Session authority mapping | Local `(owner, session_id)` → `(authority_did, session_key)` | Durable session registration; not a disposable projection |
| Shared state | `(authority_did, session_key)`; own revision | Durable title / description / schema-defined extensions |
| Member state | `(authority_did, session_key, member_did)`; own revision | Durable session nickname and permitted member fields |
| State mutation result | Authority + authenticated caller + idempotency key; request identity and result | Retry returns the same committed revision / event IDs |
| Pending Action Log publication | `(producer_did, event_id)`; immutable payload and publication progress | Commit with authoritative state, recover publication after restart |
| Published Action Log | Existing MsgObject / named store + mailbox references | Ordinary durable event history; no new MsgObjKind |

Use the existing authority's transaction to commit state, revision, mutation
result, and pending event together. Named-store publication and mailbox/delivery
fan-out may occur later through idempotent recovery; they are not part of a
cross-store atomic transaction. Group event identity will be defined by the v2 implementation.
External platform writes remain pending until confirmed and cannot join a local
RDB transaction. A future implementation must define DDL, uniqueness rules,
query indexes, and a schema-version bump before enabling these records.

## 5. Schema Version

The shared msg-center RDB instance schema version is `13`, stored in the
scheduler-provided RDB instance spec. Every DDL or frozen-key semantic change
increments this version.

## 6. Upgrade Compatibility Strategy

Beta 2.2 is in no-compat mode. On startup, msg-center applies the service spec's
DDL followed by the current backend's compiled required DDL. Both are idempotent:
an older stored spec cannot suppress newly required tables or indexes, and
deployment-specific additions remain. This does not rewrite the stored spec version
or migrate existing column definitions or types.

Version 12 removes the v1 self-host group table definitions. Version 13 adds
`friend_requests` and its indexes with `IF NOT EXISTS`, and changes the contact
payload; payloads written before version 13 are rejected on load as described
above. Existing database tables
are not automatically dropped; code removal does not delete deployed data.
Session-addressed queue and sender-mailbox idempotency semantics remain unchanged.

## 7. Extensibility Rules

- Frozen: idempotency primary-key meaning, state transitions, cursor primary key,
  timestamp units, and stored RPC result schema.
- Extensible: new operation scopes, cursor keys, tables, and additive columns
  with a schema-version bump.
- Cursor JSON is opaque to storage but must be validated by its tunnel before use.

## 8. Query Patterns

| Query | Support | Cost |
|---|---|---|
| Resolve idempotency result | `msg_idempotency` primary key | Constant lookup |
| Load/store one tunnel cursor | `msg_tunnel_cursors` primary key | Constant lookup/upsert |
| Clean one conversation bucket | retention/expiry index | Background maintenance |
| Clean global expired rows | expiry index, 10,000-row batches | Background maintenance |
| Enumerate oversized buckets | retention/expiry index scan | Hourly background maintenance |
| List one owner's friend requests | `idx_friend_requests_owner` | Indexed range, newest first |
| Expire pending friend requests | `idx_friend_requests_expire` | Background sweep every 10 minutes |

Idempotency maintenance never runs in `dispatch` or `post_send` request paths.
The background worker runs immediately at service startup and then hourly.


## Session inbox partition (version 10)

The existing `mailbox_records.session_id` is both the inbox partition and the
owner-local session projection key. `(owner, session_id, box_kind)` identifies
one queue; NULL selects only the default inbox. API `mailbox` addresses are
computed from these columns and are not an independent stored routing field.
The new `idx_mailbox_address_box_state_sort` index covers
`(owner, session_id, box_kind, state, sort_key, record_id)` on SQLite and PostgreSQL.
`idx_mailbox_msg_id(msg_id)` supports message-body authorization through its mailbox references.

Queue claims and record moves compare the persisted session, state and update
version before updating. Replayed dispatches do not restore the old session,
even after a record has moved into the NULL/default partition. No owner-wide
fallback or legacy `owner` queue RPC alias is provided.
