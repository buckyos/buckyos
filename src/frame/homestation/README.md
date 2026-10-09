# homestation — personal publication stream, delivery inbox and reading pipeline

Backend of BuckyOS HomeStation. One service serves every user of the zone (like Message Center): each user has
an own home `cyfs://<zone>/home/<user>` (stream, inbox, reading pipeline, private state in an own database), and
the zone keeps a zone feed list at `/home/~zone`. Design: [架构设计](<../../../doc/homestation/BuckyOS HomeStation 架构设计.md>) v0.7;
wire formats, endpoints, kRPC methods, schema and implementation choices:
[协议与实现](<../../../doc/homestation/HomeStation 协议与实现.md>). The Desktop app is `src/frame/desktop/src/app/homestation`.

```text
src/host.rs         the zone service: users → stations (opened on use), DID lookup, zone feed reads, portal kRPC
src/users.rs        zone users (system config `users/*` or standalone config)   src/zone.rs  zone feed list store
src/protocol.rs     Feed Object / Head / follow / proof types, homes and entry URLs, interaction keys, structural rules
src/objects.rs      local object store, JWT verification, entry namespace check, chunk stores (folder / NDM)
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
tests/              multi-node scenarios over real HTTP (network, interactions, reading, services, sources, zone)
```

## Run

BuckyOS service mode (no arguments): kernel service `homestation` on 127.0.0.1:4130; kRPC through the generic
gateway route `/kapi/homestation` (each caller's own HomeStation; `portal.*` for anyone), protocol paths `/home/*`
forwarded by `rootfs/etc/boot_gateway.yaml`. Users come from system config `users/*`; it signs for all of them
with the OOD device key (zone custody), reads each user's contacts from Message Center, stores files in the zone
named store and keeps per-user databases under `<data>/users/<user>/` plus `<data>/zone.db`.

Standalone (a zone from a JSON config — owner, more `users`, zone key — see the doc §7.2):

```bash
cargo run -p homestation -- --keygen
cargo run -p homestation -- --data-dir /tmp/hs-a --config hs-a.json
```

Development network (zone of `me` with token `tok-me` and a second user `kai`/`tok-kai` on 4131; alice, bob, sarah,
collector `index` and an RSS fixture site on the next ports):

```bash
cargo run -p homestation --example devnet -- --port 4131 --data-dir /tmp/hs-devnet --fresh
```

## Tests

```bash
cargo test -p homestation
```

Unit tests cover the object rules, signing, audience, rule evaluation, feed parsing and intent keywords; the
integration tests start several real HomeStations with their own keys, databases and listeners and check the
acceptance scenarios of architecture §20 (mapping in the doc §8).
