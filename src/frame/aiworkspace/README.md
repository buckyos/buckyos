# aiworkspace — AI Workspace artifact management service

Backend of the BuckyOS AI Workspace (phase one). Design:
[第一期核心架构设计与验证](<../../../doc/workspace/BuckyOS AI Workspace 第一期核心架构设计与验证.md>) and
[第一期内置对象详细设计](<../../../doc/workspace/BuckyOS AI Workspace 第一期内置对象详细设计.md>) (called "the design" below).

```text
core/      aiworkspace-core   pure logic, builds for wasm32-unknown-unknown (no tokio/fs/sqlite/clock/random);
                              anchor.rs: annotation anchors (target / range / quote) and per-type anchor adapters
store/     aiworkspace-store  SQLite storage, object store, packages, Mock runs, URL sources
server/    aiworkspace        process entry, kRPC dispatch, upload/download routes, auth
wasm/      aiworkspace-wasm   wasm-bindgen facade of core for the browser replica (`wasm/build.sh` writes it into the Desktop app)
schemas/   richtext.basic.v1.json (single source for the Rust validator and the ProseMirror schema)
fixtures/  project-workspace/commits.json (the shared sample as a commit sequence), vectors/
```

## Run

Standalone (independent testing; data in an isolated folder, static test identities):

```bash
cargo build -p aiworkspace
echo '{"tokens":{"tok-alice":{"principal":"alice"},"tok-bob":{"principal":"bob"}}}' > /tmp/aiws-tokens.json
aiworkspace --data-dir /tmp/aiws-data --listen 127.0.0.1:4120 --auth-file /tmp/aiws-tokens.json --fixture-sources
```

BuckyOS service mode (no `--data-dir`): logs in through buckyos-api as kernel service `aiworkspace`,
keeps Workspace folders under the service data folder, listens on port 4120, is reached through the
generic gateway route `/kapi/aiworkspace`. Every request is authenticated by the service itself
(verify-hub session token); the gateway does not do it.

`AIWS_FAILPOINT=before_txn|in_txn|after_commit[:abort][:N]` injects a fault into the N-th commit.

## Tests

```bash
cargo test -p aiworkspace-core      # values, canonical ids, filters, rich text codec, planner, annotation anchors
cargo test -p aiworkspace-store     # V01–V22, V24, write locks, Mock — against real SQLite files;
                                    # tests/replica.rs: the offline engine incl. the incremental rows a replica persists
cargo test -p aiworkspace           # V23 + crash recovery — against the real process over HTTP
```

## Storage

```text
<data>/workspaces/<workspace_id>/doc.sqlite     the portable document (design §4.2)
<data>/workspaces/<workspace_id>/local.sqlite   grants, locks, runs, staged uploads — never exported
<data>/workspaces/<workspace_id>/staging/       export packages, replica databases
<data>/objects/{objects,chunks}/                content-addressed NamedObjects and chunks (CYFS ids)
<data>/trash/                                   deleted / replaced Workspaces
```

One writer per Workspace (a mutex around its connections); a Commit is planned on an overlay,
checked, and written in one SQLite transaction together with its history row, which is also the
idempotency record and the change stream.

## Protocol

kRPC: `POST /kapi/aiworkspace` with `{ "method", "params", "sys": [seq, session_token] }`.
**Business results are always in `result`**: `{ "ok": true, ... }`, `{ "ok": false, "error": { code, retryable, detail, sub_code?, data? } }`,
or the three-state commit result (`status: accepted | conflict | rejected`). The kRPC `error`
string is used only for an invalid token, an unknown method or an unparsable request.

| Method | Params (besides `workspace_id`) |
| --- | --- |
| `ws.create` | `title`, `workspace_id?` |
| `ws.list`, `ws.get_info`, `ws.delete`, `ws.fork` | |
| `ws.grant` / `ws.revoke` / `ws.list_grants` | `subject`, `scope_entity_id?`, `capabilities[]` |
| `ws.begin_import` → PUT → `ws.import` | `upload_id`, `semantics: "restore" \| "new"`, `replace?` |
| `doc.outline` | — all readable entity envelopes in tree order |
| `doc.resolve` | `reference` or `path` |
| `doc.read` | `entity_id`, `selector?` — or `targets: [{ entity_id, selector? }]` |
| `doc.list_children` | `entity_id`, `include_deleted?` |
| `doc.list_annotations` | `target_ids?`, `parent_id?` — annotations anchored to those entities and/or placed under that parent, each with its resolved `anchor` |
| `doc.query` | `view_id` or `source_id`; `filter?`, `sorts?`, `fields?`, `group?`, `limit`, `cursor?`, `consistency?`, `source_revision?` |
| `doc.source_capabilities` | `source_id` (URL query tables) |
| `doc.get_collab_state` | `entity_id` |
| `doc.prepare` / `doc.commit` | the Commit request (design §2.6) |
| `doc.get_submission` | `epoch`, `idempotency_key` |
| `doc.undo` | `epoch`, `commit_id`, `idempotency_key`, `mode?`, `plan_digest?`, `session_id?` |
| `doc.get_changes` | `epoch`, `after_seq`, `limit?`, `filter: { detail: "touched" }?` |
| `doc.wait_changes` | `epoch?`, `after_seq`, `timeout_ms?` (long poll) |
| `doc.checkpoint`, `doc.export` | `mode: "share" \| "personal_backup"`, `self_contained` |
| `asset.begin_upload` → PUT → `asset.finish_upload` | `size`, `upload_id` |
| `replica.bootstrap` | — then `GET /replica/<workspace_id>/<replica_id>` |
| `proc.start` / `proc.get` / `proc.apply` / `proc.cancel` | `program`, `params`, `idempotency_key`; `run_id`, `session_id?` |
| `lock.acquire` / `lock.renew` / `lock.release` / `lock.break` / `lock.list` | `entity_ids[]`, `session_id`; `lock_ids[]`; `entity_id` |
| `diag.list_unretained`, `diag.verify_refs` | |

HTTP routes (same port, `Authorization: Bearer <session token>`):

| Route | Use |
| --- | --- |
| `PUT /kapi/aiworkspace/upload/<upload_id>` | asset bytes or an import package |
| `GET /kapi/aiworkspace/asset/<workspace_id>/<object_id>` | verified asset bytes |
| `GET /kapi/aiworkspace/export/<workspace_id>/<export_id>` | export package (zip) |
| `GET /kapi/aiworkspace/replica/<workspace_id>/<replica_id>` | replica database built by `replica.bootstrap` |
| `GET /kapi/aiworkspace/schemas/richtext.basic.v1.json` | rich text schema definition (no auth) |

Operations (`operations[]` of a Commit): `entity.create|delete|restore|rename|set_keys|unset_keys|set_write_policy`,
`tree.move|place`, `table.insert_records|delete_records|set_values|unset_values|set_body|add_field|update_field|delete_field|add_option|update_option|delete_option|migrate_field`,
`richtext.apply_update|insert_blocks|replace_block|delete_blocks|move_block`. Parameters are exactly those of the design §3;
`core/tests/planner.rs` and `store/tests/*.rs` are executable examples of every one of them.

## Where this implementation differs from the design text

- **Export/replica download routes carry the `workspace_id`** (`/export/<workspace_id>/<export_id>`): there is no service-level database to look an export id up in.
- **Object storage is a content-addressed folder with CYFS ids and file names**, verified against ndn-lib in `store/tests/kernel.rs` (V01). It is not yet backed by the zone named store, so nothing is pinned there and assets are not fetchable through NDN. `FsObjectStore` is the boundary to replace.
- **Files are single `mix256` chunks (≤ 32 MiB)**; ChunkList for larger assets and record files is not written.
- **Undo `partial` mode** determines the applicable part by probing each inverse operation on its own.
- **URL query sources** only reach adapters registered in the service (`fixture://` generated source for tests). There is no general HTTP fetcher: a URL in a document cannot make the service issue arbitrary requests.
- Extra methods: `doc.outline`, `doc.source_capabilities`, `diag.verify_refs`.
