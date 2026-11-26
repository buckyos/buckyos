# AppDoc and PIKG

- AppDoc input is `dapp_meta/app.json`: schema_version 1, did, name, version, owner, author, show_name, categories, permissions, optional content_handlers/presentation, selector_type and service_config_tips. Use a one-label owner `did:bns:<owner>`; `categories[0]` determines App type. Use the installed CLI's validation rules. Set AppDoc `sdk_version` to the SDK semver used to build the App; Desktop installer preflights its minimum and target loader checks it, absent field uses compatibility loader.
- Permission example: `{"scope_path":"user/home","required":true,"actions":["read","write"],"exp":null}`. An App that calls `notify.send` declares `{"scope_path":"msg-center/notify","required":false,"actions":["write"],"exp":null}`; otherwise it gets `NOTIFY_NOT_DECLARED`, and `NOTIFY_DISABLED` once the user turned it off (do not retry). Optional `window` declares how the App is shown (`instance`, `default_mode`, sizes, buttons, `mobile_header`; `AppWindowDeclaration` in `app_doc.rs`). For presentation, title/summary/description/links are string maps, icons maps names to serialized ObjectIds, license is a string. Empty maps permitted. Desktop reads `links.app_icon_url` (relative to App, HTTPS or small image data URL). Both `icon.svg` and `/icon.svg` resolve on the App's web host, not the Desktop host; query strings such as `?v=2` are preserved. Package the asset and verify the resolved URL returns an image content type, not the Desktop HTML fallback. Favicon and Desktop shortcut are separate. See `src/kernel/buckyos-api/src/app_doc.rs` (`AppPresentation`).
- Add `service_config_tips.service_endpoints` after init. For example:

```json
{"www":{"protocol":"http","inner_port":3000,"required":true,"expose":{"route":{"type":"web"},"scope":"","allow_guest":false}}}
```

  Anonymous visitors receive 302 to login. Expose a dedicated port with `{"type":"port","preferred_port":3000}`. Container `BUCKYOS_SERVICE_PORT` is inner_port for www or sole service; native preinstalled script Apps on Lite receive instance port instead. All service instance ports are in `app_instance_config.service_ports_config`. See `src/apps/sys_test/main.ts` (`listenAddress()`).
- Remaining service tips include data/local_cache/external_mount_points (`mount_point_name,access,reason`), `instance_volume {mode,quota_mib,ephemeral_contents}`, rdb_instances, buckets, bash_envs, runtime_caps, container_param and start_param. See `validateServiceConfigTips` in mounted `cli/modules/pikg_protocol.ts`. Storage requirements and lifecycle are in this skill's storage reference.
- `pikg.json` example (paths relative to `dapp_meta/`):

```json
{"schema_version":1,"output_dir":"../dapp_dist","pikg_file":"myapp-0.1.0.pikg","sub_pkgs":{"web":{"required":true,"source":{"type":"path","path":"../web/dist"}}}}
```

  Sources may be build directories or finished .tar.gz. Script: `--kind script --source ./script/dist`, subpackage `script` with `{"entry":"main.py"}` in buckyos_script.json. Python requirements.txt/pyproject.toml → uv venv; deno.json/main.ts → Deno. Docker host: `docker build -t local/myapp:0.1.0-amd64 .`, `--kind docker --source local/myapp:0.1.0-amd64`. Use `amd64_docker_image` / `aarch64_docker_image` as appropriate, e.g. `{"selector":{"os":"linux","arch":"x86_64"},"source":{"type":"docker-image","image":"local/myapp:0.1.0-amd64"}}`. For multi-arch add both keys/images.
- ext-tools subpackage `ext_tools_<os>_<arch>` holds `ext-manifest.json`, `payload/<platform>/`. Publish or `ext-tools import` an official PIKG, then enable Zone-wide. `app install` refuses it. `aiws-controls` packages use `aiws` with `aiws-manifest.json` and `controls/<name>/…`; install populates user's library without a running App.

## Debug and verify

- Static: serve dist locally for layout; test SSO/Zone integration on the installed URL from `buckyos app status did:bns:myapp.<owner>`. After install, gateway reload has taken ~1 min (403); a tested upgrade served old content for >7 min. Record both responses if observed.
- Script: run a Python entry locally with `BUCKYOS_DATA_DIR=./data BUCKYOS_SERVICE_PORT=3000 python3 main.py`; host repo `src/rootfs/bin/service_debug.tsx <app_id> <owner>` starts an installed script foreground with real env (historically unverified). Check for `script runner: no entry point` and verify inner_port. Docker host: `docker run --rm -p 8080:<inner_port> -e BUCKYOS_DATA_DIR=/data -v "$PWD/data:/data" <image>`, then `docker logs buckyos-app-<web host>` and `docker port buckyos-app-<web host>` for the installed container.
- `INVALID_OWNER_DID`: check one-label DID. `SCHEMA_VALIDATION_FAILED`: inspect allowed fields. `SELECTOR_CONFLICT` / `UNSUPPORTED_ARCHITECTURE`: compare selector/key and node architecture. `INVALID_SOURCE`: check source is directory or .tar.gz.
- Built-in text_editor and sys_test refresh Git #main SDK at each pnpm build; Text Editor prioritizes a configured/sibling SDK, which must be built first. sys_test's prepare.mjs traces dist/node.mjs with Deno's module graph including literal dynamic imports, excluding source maps, declarations and unused SDK outputs; run type checks first.
- Sources: [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md), [PIKG format](https://github.com/buckyos/buckyos/blob/main/doc/pikg/format.md), [App config](https://github.com/buckyos/buckyos/blob/main/doc/sdk/app-package-config.md). Examples `test/app_installer_test/pikg_samples/{static-web,script-host,docker}`, `src/apps/text_editor`, websdk `examples/`. Private extensions go in project or `$OPENDAN_AGENT_ROOT/tools/`.
- Historical check (2026-10-17): BuckyOS 0.7.0, Tool/SDK 0.7.131, test.buckyos.io host, static/Docker install and upgrade, script install. Agent container installs were untested.
