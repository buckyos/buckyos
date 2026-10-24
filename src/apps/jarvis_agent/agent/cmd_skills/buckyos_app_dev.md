---
name: buckyos_app_dev
description: Develop a traditional BuckyOS App — static web page, Python/TypeScript script service or Docker service — with SDK identity, AppDoc permissions/endpoints/data dirs, PIKG metadata, local debugging, test install and an upgrade that keeps the App's data.
requires: buckyos command with pikg (agent container or host); node/deno/python for the App's own build; Docker Apps need a host with Docker; installs need an identity the control-panel accepts
---
#### When to use
Writing or converting an App that runs in a Zone: a web page/SPA, a small HTTP service in Python or TypeScript, or an existing Docker image; then installing and upgrading it for testing. The shared steps (build vs system build, `pikg` details, the three install paths, waiting for tasks, publish limits, error codes, evidence) are in `buckyos_dev_release`; when that manual is not in this session, read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release.md`. Agents and skill packages are `opendan_agent_dev`.

#### Choose the shape
| kind (`pikg init --kind`) | runs as | sub package | good for |
| --- | --- | --- | --- |
| `static-web` | files served by the zone gateway at `http://<host>.<zone>/`, no process | `web` | pages/SPAs; user data through the SDK in the user's storage (the built-in Text Editor works this way) |
| `script` | aios worker container: Python (`requirements.txt`/`pyproject.toml` → uv venv) or TypeScript (`deno.json`/`main.ts` → `deno run --allow-all`) | `script` | small services without your own image; entry from `buckyos_script.json` `{"entry":"main.py"}` |
| `docker` | your local image, one container per instance | `amd64_docker_image` / `aarch64_docker_image` | anything with its own runtime; the image arch has to match the node |
Your agent container has no Docker: write the Dockerfile and app, and hand the owner the host commands for `docker build` and packaging.

#### Before you start
```
buckyos pikg doctor            # Tool/SDK versions, policy, metadata checks of ./dapp_meta
buckyos auth whoami            # needed for the install steps
```

#### Identity and SDK inside the App
- Script and Docker Apps get `BUCKYOS_APP_ID`, `BUCKYOS_APP_DID`, `BUCKYOS_APP_INSTANCE_ID` (`<app_id>@<owner>`), `BUCKYOS_OWNER_USER_ID`, `BUCKYOS_DATA_DIR`, `BUCKYOS_APP_TOKEN` (a device-signed login assertion the SDK trades for a session), `BUCKYOS_HOST_GATEWAY`, `BUCKYOS_ZONE_CONFIG`. TypeScript service: `await buckyos.initBuckyOS('', { appId: '', runtimeType: RuntimeType.AppService, zoneHost, defaultProtocol: 'https://' }); await buckyos.login()`, then `getAppDataDir()`, `getAppSetting()/setAppSetting()`, `getZoneServiceURL('<svc>')` (pattern of websdk `examples/app_service_local_debug.ts`; not exercised inside an installed App in the 2026-10-17 check).
- Browser App: `await buckyos.initBuckyOS('<app_id>')`, `await buckyos.getAccountInfo()` (no `user_id` → `buckyos.login()` SSO), then `getSystemConfigClient()` etc. `buckyos.getCurrentIdentity()` tells an account (`kind:"account"`, with `user_type` and `restricted`) from an External DID or an anonymous visitor; a Limited user's session is restricted: it reaches your App (target = your instance), but system services that have not opened the method refuse it with kRPC `code:"access_denied"` (`buckyos.accessDenialOf(err)?.reason`). Show that as "not available for this account"; never swap in the App's own token to act for the user (doc/sdk/SSO.md, "不要混用两种 session_token").
- Browser App that opens content (declares `content_handlers.open`): talk to the desktop only through `buckyos/app-frame`. Shell origin = `embedderOrigin(o => originInZone(o, buckyos.getZoneHostName()))`, system entry = `${location.protocol}//${buckyos.getZoneHostName()}` (never split your own hostname or assume `sys.<zone>`). "Open with…" is the system's: `frame.openContent({ source }, 'chooser', { hints: { name, mime, size } })` (embedded → shell chooser that leaves your App out; standalone tab → new tab on the system entry). Content your App cannot show itself goes in an inline `PreviewEmbed` (system `/embed/preview`, reads with its own session) instead of a new window. Reference: `src/apps/text_editor/readme.md` §7, fixture `test/app_installer_test/pikg_samples/content-open-handler/web/main.js`, source `buckyos-websdk/src/app-frame.ts`.
- SDK types: `/opt/buckyos/libexec/buckyos-tool/dist/*.d.ts` (`sdk_core.d.ts`, `runtime.d.ts`, `node.d.ts`); npm package `buckyos` for the App's own build. Put no tokens, keys or host tools into the package.
- Built-in `src/apps/text_editor` and `src/apps/sys_test` refresh the SDK dependency on every `pnpm build`, following Git `#main`; Text Editor's local SDK hook takes precedence, so build that SDK first.
- Browser file/folder imports use `buckyos.ndm.importFiles(files, options)` or `importDirectory(tree, options)`, then `ensureUploaded(sessionId, options)`. Consume roots only after `objectStatus === 'ready'`; an ObjId alone does not prove content readiness. Keep the original user + App/System session for NDM and the business submission; lookup existence never grants access. Reference consumers retain the ready object for their business lifetime; filesystem destinations submit ObjId to the destination service for local NamedStore extraction. Do not copy hashing/TUS/finalize orchestration into the App. See the source `src/apps/sys_test/README.md`, `src/frame/nfs_server/README.md` and WebSDK NDM API documentation. Directory pickers based on FileList cannot preserve empty directories; explicit trees can.

#### dapp_meta/app.json (AppDoc input)
- Fields: `schema_version 1, did, name, version, owner, author, show_name, categories, permissions, content_handlers?, selector_type, service_config_tips`; anything else is rejected. `categories[0]` becomes `app_type` (`web`, `dapp`, `agent`, `agent_skills`).
- `permissions`: `[{"scope_path":"user/home","required":true,"actions":["read","write"],"exp":null}]`.
- Endpoint for a service (add by hand after `init`; port = what your server listens on):
`"service_config_tips":{"service_endpoints":{"www":{"protocol":"http","inner_port":3000,"required":true,"expose":{"route":{"type":"web"},"scope":"","allow_guest":false}}}}`. `allow_guest:false` sends anonymous visitors to the zone login (302); route `{"type":"port","preferred_port":N}` exposes a port instead.
- More keys: `data_mount_points`, `local_cache_mount_points`, `external_mount_points` (`{mount_point_name, access, reason}`), `instance_volume {mode, quota_mib, ephemeral_contents}`, `rdb_instances` (sqlite/postgres), `bash_envs`, `runtime_caps`, `container_param`, `start_param`. Exact checks: `validateServiceConfigTips` in `cli/modules/pikg_protocol.ts`.
- Data: `$BUCKYOS_DATA_DIR` = `/opt/buckyos/data/home/<owner>/.local/share/<app_id>` (same path under the host `$BUCKYOS_ROOT`) survives upgrades; `/tmp` is local cache; script Apps also get `/opt/buckyos/instance` (venv, caches).

#### dapp_meta/pikg.json
`{"schema_version":1,"output_dir":"../dapp_dist","pikg_file":"<name>-<ver>.pikg","sub_pkgs":{"web":{"required":true,"source":{"type":"path","path":"../web/dist"}}}}`. Paths are relative to `dapp_meta`; a source is a build-output dir or a finished `.tar.gz`; Docker: `{"selector":{"os":"linux","arch":"x86_64"},"source":{"type":"docker-image","image":"local/<name>:<ver>-amd64"}}`; add both arch keys for a multi-arch App.

#### Shortest path (static web; no Docker needed)
```
mkdir -p myapp/web/dist && cd myapp            # build your page into web/dist (vite build, or plain files)
buckyos --non-interactive pikg init . --name myapp --owner did:bns:<owner> --kind static-web --source ./web/dist
buckyos pikg build ./dapp_meta && buckyos pikg pack ./dapp_dist && buckyos pikg info ./dapp_dist/myapp-0.1.0.pikg
buckyos --non-interactive app fetch ./dapp_dist/myapp-0.1.0.pikg --plan plan.json --policy local-developer
buckyos --non-interactive --yes --timeout 10m app install ./dapp_dist/myapp-0.1.0.pikg --plan plan.json --policy local-developer
buckyos app status did:bns:myapp.<owner>       # web_hosts → http://myapp.<zone>/
```
Script: same with `--kind script --source ./script/dist` (dir holding `main.py` or `main.ts` and `buckyos_script.json`) plus the endpoint JSON. Docker (host): `docker build -t local/myapp:0.1.0-amd64 .` then `--kind docker --source local/myapp:0.1.0-amd64` plus the endpoint JSON.

#### Local debug
- Static: serve `web/dist` with your own dev server for layout work; test SDK login and zone calls on the installed copy at `http://<host>.<zone>/`.
- Script: run the entry directly with `BUCKYOS_DATA_DIR=./data PORT=3000` style env; on the host the repo tool `src/rootfs/bin/service_debug.tsx <app_id> <owner>` starts an installed script App in the foreground with the real env (not run in the 2026-10-17 check).
- Docker (host): `docker run --rm -p 8080:<inner_port> -e BUCKYOS_DATA_DIR=/data -v $PWD/data:/data <image>`. Installed (host): `docker logs buckyos-app-<web host>`, `docker port buckyos-app-<web host>`.

#### New version and upgrade
1. Write something into `$BUCKYOS_DATA_DIR` (or the App's user data) with the old version and note it.
2. Change code; set `app.json.version` (semver, higher), `pikg.json.pikg_file`, and for Docker a new image tag + `pikg.json` image; keep `did`/`name`.
3. Build/pack/info, then `app install ./dapp_dist/myapp-0.1.1.pikg --policy local-developer --dry-run` (expect `plan_use UPGRADE`) and the same without `--dry-run` plus `--yes --timeout 10m`.
4. Check `app status` (`app_version`, same `app_instance_id`, digest), the page/API showing 0.1.1, and that step 1's data is still there (2026-10-17: Docker counter and first-version file survived).

#### When it fails
- `INVALID_OWNER_DID`: owner is not a one-label `did:bns:x`. `SCHEMA_VALIDATION_FAILED`/unknown field: the field is not in the list above. `SELECTOR_CONFLICT`: selector disagrees with the key. `INVALID_SOURCE`: source is neither dir nor `.tar.gz`. `UNSUPPORTED_ARCHITECTURE`: image arch.
- Install accepted but nothing answers: script → container log `script runner: no entry point` (add `buckyos_script.json`) or a wrong `inner_port`; static web → 403 for about a minute after install while the gateway reloads (2026-10-17).
- Static web after an upgrade: the 2026-10-17 check kept serving the previous package for over 7 min (gateway keeps the old directory). Report it with both versions' content; it is a platform issue, not your build.

#### Check the result
Report: App DID, version, `pikg_digest`, task ids and outcome, URL, the request you made and the answer showing the version, the data check. Attach the PIKG (`report.artifacts`) when the user wants the package.

#### Source and extending
Current repository contracts: [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md), [package format](https://github.com/buckyos/buckyos/blob/main/doc/pikg/format.md), [App configuration](https://github.com/buckyos/buckyos/blob/main/doc/sdk/app-package-config.md). This manual supplies the task recipe; schemas and historical proposals are maintained in those documents.
Mounted CLI source `/opt/buckyos/libexec/buckyos-tool/cli/modules/{pikg,pikg_protocol,app}.ts`; samples in the BuckyOS repo `test/app_installer_test/pikg_samples/{static-web,script-host,docker}` and `src/apps/text_editor` (static-web App with SDK, NFSP and settings); websdk `examples/`. Extra scripts go to the project or `$OPENDAN_AGENT_ROOT/tools/`.

Checked against: BuckyOS 0.7.0, Tool/SDK 0.7.131, static-web and Docker samples installed and upgraded, script sample installed, on test.buckyos.io from the host (not yet from an agent container), 2026-10-17.
