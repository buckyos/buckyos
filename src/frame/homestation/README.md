# homestation — personal publication stream, delivery inbox and reading pipeline

Backend of BuckyOS HomeStation. One service serves every user of the zone (like Message Center): each user has
an own home `cyfs://<zone>/home/<user>` (stream, inbox, reading pipeline, private state: its own rows of the
service's one database and its own buckets), and the zone keeps a zone feed list at `/home/~zone`. Design: [架构设计](<../../../doc/homestation/BuckyOS HomeStation 架构设计.md>) v0.7;
wire formats, endpoints, kRPC methods and implementation choices:
[协议与实现](<../../../doc/homestation/HomeStation 协议与实现.md>); tables, buckets, keys and who binds what:
[Durable Data Schema](<../../../doc/homestation/HomeStation Durable Data Schema.md>). The Desktop app is `src/frame/desktop/src/app/homestation`.

```text
src/host.rs         the zone service: users → stations (opened on use), DID lookup, zone feed reads, portal kRPC
src/users.rs        zone users (system config `users/*` or standalone config)   src/zone.rs  zone feed list store
src/protocol.rs     Feed Object / Head / follow / proof types, homes and entry URLs, interaction keys, structural rules
src/objects.rs      local object table, JWT verification, entry namespace check, `Storage` (named store + user buckets)
src/db.rs           the `homestation-main` database; `Db::for_owner` scopes a station to its rows
src/sign.rs         signer, JWT decode/verify, key generation       src/directory.rs  DID → home → origin, signer authorization
src/publish.rs      entries, serial Heads, stream changes, read grants, publish tasks
src/stream.rs       display / change reads, entry Heads, objects, chunks, author comment view, profile, collector index
src/ingress.rs      CYFS dispatch inbox, admission, idempotency, Head merge, ingest for Push and Pull
src/delivery.rs     outbox, dispatch PUT, result layers, retries  src/pull.rs  follow sync, fetch by ObjId, cold start
src/sources.rs      follow / URL / natural-language sources, follow declarations, friend-derived follows
src/spider.rs       RSS/Atom and web link cards (private captures), snapshots
src/evaluation.rs   tag evaluation service (identity, ObjId, entry path, root + InnerPath), overrides, model profile
src/selection.rs    effective tags, filter and mute rules, scoring, admission, reading list, followed candidates
src/resources.rs    preparation by ObjId with hash checks        src/comments.rs  views, stats, discussion tracking
src/interact.rs     comments, likes, bookmarks, reposts, quotes   src/feedback.rs  behaviour events, consumption proofs
src/projection.rs   UI card model                                 src/api.rs  a user's kRPC   src/http.rs  routes
src/standalone.rs   standalone bootstrap (config JSON)            src/main.rs  binary (standalone / BuckyOS service)
examples/devnet.rs  seeded multi-node network for UI work and real-backend e2e
tests/              multi-node scenarios over real HTTP (network, interactions, reading, services, sources, zone,
                    object_import); tests/common/nfs.rs is a zone's in-process nfs-server (also used by devnet)
```

## Run

BuckyOS service mode (no arguments): kernel service `homestation` on 127.0.0.1:4130; kRPC through the generic
gateway route `/kapi/homestation` (each caller's own HomeStation; `portal.*` for anyone), protocol paths `/home/*`
forwarded by `rootfs/etc/boot_gateway.yaml`. Users come from system config `users/*`; it signs for all of them
with the OOD device key (zone custody), reads each user's contacts from Message Center, keeps its records in the
rdb_mgr instance `homestation-main` (rows carry the `owner` username; the zone feed is in the same database) and
its objects and files in the zone named store, retained by the users' buckets `user/homestation/<user>/{published,
entries,following,following-entries,scratch,cache}` (declared by scheduler `add_homestation`, served by nfs-server).

Standalone (a zone from a JSON config — owner, more `users`, zone key, the zone's `nfs_server` — see the doc §7.2;
the database is `<data-dir>/homestation-main.db`, objects and buckets live in that nfs-server):

```bash
cargo run -p homestation -- --keygen
cargo run -p homestation -- --bucket-specs > hs-buckets.json
cargo run -p nfs_server -- --listen 127.0.0.1:3260 --data-dir /tmp/hs-a-nfs --export tmp=/tmp/hs-a-nfs-export \
  --named-store /tmp/hs-a-nfs/store --bucket-specs hs-buckets.json
cargo run -p homestation -- --data-dir /tmp/hs-a --config hs-a.json   # with "nfs_server": "http://127.0.0.1:3260"
```

Development network (zone of `me` with token `tok-me` and a second user `kai`/`tok-kai` on 4131; alice, bob, sarah,
collector `index` and an RSS fixture site on the next ports; each zone runs an in-process nfs-server):

```bash
cargo run -p homestation --example devnet -- --port 4131 --data-dir /tmp/hs-devnet --fresh
```

## Tests

```bash
cargo test -p homestation
```

Unit tests cover the object rules, signing, audience, rule evaluation, feed parsing and intent keywords; the
integration tests start several real zones (HomeStation, database, in-process nfs-server, listener each) and check
the acceptance scenarios of architecture §20 (mapping in the doc §8).

File and folder attachments: `object.upload_target` names a fresh key in the user's `scratch` bucket, the WebSDK
NDM upload binds the root there at finalize (`auto_bind`), then `object.register {objId, key, metadata}` checks
that binding (registrations without a key — Agent artifacts — check the caller's own NDM read access instead)
and the complete object graph, and keeps presentation metadata separately. `publish.create` binds the version in
`entries` and each attachment in `published` before the database commit; a withdrawal unbinds them. `files` posts
carry FileObject/DirObject references; audience grants extend through directories and ChunkLists. Content reads
currently buffer at most 64 MiB per file. `tests/object_import.rs` covers the upload proof, the bindings of
publication / edit / withdrawal, directory/empty-file/ChunkList readback, audience denial and the ambient copies
of other publishers' posts.
