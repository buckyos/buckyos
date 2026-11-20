# Store App data

Keep bytes/immutable objects in declared buckets. Keep records with ObjIds or (bucket_id,key,version) in `rdb_instances` (sqlite/postgres). Only use `$BUCKYOS_DATA_DIR` for actual path semantics (external programs/executables/working directory); explain the need. It maps to `$BUCKYOS_ROOT/data/home/<owner>/.local/share/<app_id>` and survives upgrades. /tmp is cache; script Apps also have /opt/buckyos/instance for venv/cache.

| data_policy | Use and retention |
| --- | --- |
| documents | User-created or requested documents; visible read-only in Files → Apps/<App>/<bucket>/. |
| collected | Other people's content the user keeps; attach origin. Both collected and documents remain visible as orphans on uninstall; user may delete in Storage. |
| app_data | App-generated, costly to rebuild; uninstall --data retain orphans, --data delete removes. |
| ambient, temp, cache | Nice-to-have, work in progress, or rebuildable respectively. Uninstall removes these buckets; cache is unretained. |

AppDoc `service_config_tips.buckets` maps names to `kind` (data/object), data_policy, `scope` (user/zone), optional versioning, quota_mib, default_ttl_days, lifecycle. kind/policy/scope fixed after install. app_data/temp require storage_quota_mib; temp needs default_ttl_days 1..30; documents/collected take no quota_mib.

- `buckyos.ndm.importFiles(files, options)` or `importDirectory(tree, options)`, then `ensureUploaded(sessionId, options)`. Supply `bind:{bucketId,key}` during importFiles with autoStartUpload:true or to ensureUploaded/startUpload so finalize returns status.binding. Treat a missing requested binding as failure. Without binding, upload is held only 24 h. Wait for `objectStatus==='ready'` before consuming root. Explicit directory trees preserve empty dirs; FileList cannot. Keep uploader's user+App/System session for business submission; let destination service extract a filesystem destination from ObjId. SDK handles hashing/TUS/finalize.
- `buckyos.getBucketClient()`: put/get/head/listObjects/delete/copy/createAccessToken. `putBytes` up to 32 MiB. Build IDs with `bucket.userBucketId(appId,owner,name)` or `zoneBucketId(appId,name)`; durable references `bucket.bucketObjUrl(bucketId,key)` (`obj://bucket/...`, parseBucketObjUrl). ObjId alone gives neither retention nor access; bind or use a processing lease ≤24 h. For sharing use grant or a single-key token ≤7 days.
- `objectUrl(bucketId,key)` → `/bucket/v1/<scope>/<appid>/<owner>/<name>/<key>` uses SSO cookie. Desktop-origin data plane displays image/audio/video/PDF/plain text inline; other content including HTML downloads as octet-stream. Serve interactive pages from App origin. For tools such as curl use `objectUrl.ndnReadUrl(origin,fileId,{token})` (`/ndn/<file_id>?hint=...`, Range supported); require read-only single-key token without max_uses/bound principal, build the URL on demand and never store it. `bucket.cachePublic(fileId, "https://<other zone>/ndn/<file_id>")` caches an already-public file locally.
- CLI: `buckyos bucket ls|stat|put|get|rm|link`. S3 clients (rclone/aws-cli/restic): `buckyos bucket s3-key-create --app <app_id>`, path-style `https://<zone>/s3`, us-east-1; S3 bucket names are <app_id>-<name> with non-alphanumerics mapped to '-'. Declare buckets via AppDoc.
- See `doc/arch/应用对象桶 Bucket 设计.md`, websdk `src/bucket_client.ts` and NDM API, `src/frame/nfs_server/README.md`, `src/apps/sys_test/README.md`.
