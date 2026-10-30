---
name: buckyos_dev_release
description: Shared BuckyOS develop-and-release loop — where to work, system build vs App build, PIKG build/pack/info, test install (first install, local new version, catalog upgrade) with versions/digests/task evidence, what App publish really does, and how system/SDK/image releases are made.
requires: buckyos command (agent container: /usr/local/bin/buckyos with the host Tool mounted; host: $BUCKYOS_ROOT/bin/buckyos); installs need an identity the control-panel accepts; system builds, Docker images and releases need the host repo and the owner
baseline: extended
---
#### When to use
Building, packaging, test-installing, upgrading or publishing anything for BuckyOS, and questions on how system or image releases happen. Writing the App itself is `buckyos_app_dev`; Agent templates, behaviors and skill packages are `opendan_agent_dev`; running-zone diagnosis is `buckyos_admin`. Those manuals point here for the shared steps.

#### Before you start
```
command -v buckyos && buckyos --version --verbose   # tool_version, buckyos_build_id, runtime
buckyos auth whoami                                 # identity + zone actually used
```
- Agent container: identity = your agent token (`BUCKYOS_APPCLIENT_SESSION_TOKEN`, refreshed per shell call), endpoint `BUCKYOS_TOOL_ENDPOINT`. The image has node/npm/npx, deno, python3/uv, git; it has no docker, cargo, pnpm and no BuckyOS repo. System builds, `start.py`, `build_aios` and Docker images are host work: give the owner exact commands.
- Host (OOD): `buckyos --yes <cmd>` uses the device identity (`ood1`; checked for reads and `app fetch`, installs were tested with a user token). To act as a user, pass `--session-token-file <f>` with a token from control-panel `auth.login` (target `{kind:"system",service_id:"control-panel"}`, recipe in the repo `doc/sdk/app-dev-quickstart.md` §4); the CLI's interactive password login asks verify-hub for app `buckycli` and the DV zone answers `AppAccessDenied`. Tokens last about 15 min.
- The system CLI reads and writes under the cwd and `~/.buckyos_tool`; a source outside the cwd fails with exit 9 `read access is outside system-default` — add `--allow-read <dir>` or work inside the project dir.

#### Find the real commands
- `buckyos command list`, `buckyos command describe <module> <verb>` (options, `access`, `async_mode`). The newer verbs (`app publish`, `skill`, `pikg init --kind agent-skills`) are missing from older installs; trust this list over any manual.
- Source of the running CLI: `/opt/buckyos/libexec/buckyos-tool/cli/modules/{pikg,pikg_protocol,pikg_archive,app,skill,task}.ts` (host: `$BUCKYOS_ROOT/libexec/buckyos-tool/cli/`); exit codes in `cli/core/errors.ts`. Output is one JSON envelope `{ok, data|error{code,message,retryable}, meta{trace_id}}`; task progress lines go to stderr.

#### Where to work
- Your project (App or template): a directory in the session dir or bound workspace. Commit/keep `dapp_meta/`; `dapp_dist/` is regenerated and `pikg build` deletes the previous `.pikg` in it, so copy each PIKG you hand over (e.g. `releases/<name>-<ver>.pikg`).
- BuckyOS itself (host): repo `buckyos/` with siblings `buckyos-websdk`, `buckyos-devkit`, `cyfs-gateway`; Cargo and `uv run` scripts run in `buckyos/src`; read `AGENTS.md` there first. Runtime root `$BUCKYOS_ROOT` (default `C:\buckyos` on Windows, `/opt/buckyos` on macOS/Linux).

#### System build vs App build
- Independent Linux/macOS Agent Daemon: from `buckyos/src`, `uv run buckyos-build.py --agent-daemon` builds only
  the self-contained executable into `rootfs/bin/buckyos-agent-daemon/`, without SDK/Web preparation.
  `uv run src/tools/package_agent_daemon.py --target <target> --output /tmp/agent-daemon-dist` from the repo
  root produces the portable package, manifest and checksum; targets include x86_64-unknown-linux-musl,
  aarch64-unknown-linux-gnu, x86_64-apple-darwin and aarch64-apple-darwin. macOS requires Xcode Command
  Line Tools to build and macOS 14+ to run; the archive is not Developer ID signed or notarized.
  Stop the daemon before replacing a package, retain
  its per-executor ledger, then rerun the valid URL. Default launch installs no service. Do not claim an
  architecture is validated until its package has run there; see `doc/arch/agent_daemon.md`.
- An App builds with its own toolchain (vite/npm, cargo, `docker build`, python); `pikg` packages existing output and runs no build.
- Built-in `text_editor` and `sys_test` refresh their Git `#main` SDK dependency before every `pnpm build`; Text Editor prefers a configured or sibling local SDK, which must already be built.
- Build options: `uv run buckyos-build.py --help` (or `-h`) lists wrapper/devkit options and SDK/CLI environment inputs without preparing or building artifacts.
- System, in `buckyos/src`: `uv run buckyos-build.py [-s <module>] [--skip-web]` (modules from `bucky_project.yaml`). It first prepares the SDK/Tool (sibling `buckyos-websdk` source, else npm `buckyos@latest`; prebuilt mode needs all four `BUCKYOS_SDK_TOOL_*`) and copies `tools/buckyos-agent` into `rootfs/libexec/buckyos-agent` with its SDK import pointing to the adjacent Tool, then runs devkit, which also packs the built-in PIKGs (`jarvis_agent`, `text_editor`, `buckyos_systest` → `rootfs/data/cache/<app_id>-<ver>.pikg`). On macOS it then moves node_daemon into `rootfs/bin/node-daemon/BuckyOS Runtime.app` (`tools/macos_runtime_bundle.py assemble`, ad-hoc signed unless `BUCKYOS_MACOS_CODESIGN_IDENTITY` is set). Use this wrapper for system builds so the installed payload includes these generated files.
- Apply, all restart the zone, ask the owner first: `uv run start.py` (stop, update installed files, start), `--skip-update` (restart), `--all` (fresh dev install devtest/test.buckyos.io, keeps `data/home`, `data/srv`, `storage`), `--reinstall release`. On macOS in the owner's GUI session start.py binds the root to that user and starts the `com.buckyos.runtime` LaunchAgent (same path as the pkg); under sudo or SSH it starts node_daemon directly without desktop control. Then `uv run check.py`: "Activation Ready" is not running.
- macOS pkg: `make_local_pkg.py build-pkg` then `verify-pkg` (one signed Runtime, non-relocatable bundles, LaunchAgent scripts). Release signing needs a Developer ID identity in `BUCKYOS_MACOS_CODESIGN_IDENTITY` before the build; ad-hoc builds lose TCC grants on every rebuild, so they cannot show upgrade continuity.
- Container code (opendan, xagent, agent_tool, buckyos-agent, entrypoint): repo root `./build_aios --local-test` (writes `$BUCKYOS_ROOT/etc/devenv.json` `{"aios":"local/aios-test"}`), then `uv run src/start.py --skip-update` (or wait: node-daemon recreates worker containers whose image id differs from the current one within about 30 s). node-daemon never pulls paios images: a dev install uses local `<repo>:latest-<arch>` and fails deploy when it is missing (`docker pull paios/aios:latest-<arch>` or `uv run build.py`); a release install loads the pinned tarballs in `bin/paios-images/`.

- Runtime startup credentials now live in `security/start_config.json`; development CA keys and private boot
  templates also stay in `security/`. Containers receive a read-only host `etc/` mount. Before applying this layout
  to an existing install, move its sensitive files out of `etc/` and restore the public boot template; see
  `doc/path_usage.md` and `buckyos_admin`. Use the current backup script with the new layout.
- Refresh-race fixes span the aios image (OpenDAN/runtime) and host binaries (verify-hub/node-daemon). Record the
  source commit/build id, actual installed binary hashes and running image id for both; publishing only the host or
  only the image is insufficient. Preserve verify-hub `sessions.db` and AgentRootFS. Use the existing authorized App
  recreate flow for an invalidated instance, then verify its pending-message reply and later renewals (`buckyos_admin`).

#### Package and inspect
```
buckyos --non-interactive pikg init . --name <app> --owner did:bns:<owner> --kind static-web|script|docker|agent-skills --source <dir | .tar.gz | local image>
buckyos pikg build ./dapp_meta && buckyos pikg pack ./dapp_dist
buckyos pikg info ./dapp_dist/<app>-<ver>.pikg
```
- Owner is a one-label `did:bns:<owner>`; App DID becomes `did:bns:<name>.<owner>`, AppId `<name>.<owner>.bns.did`. `init` writes `dapp_meta/{app.json,pikg.json}` and refuses a non-empty `dapp_meta`. Agent templates have no init kind (see `opendan_agent_dev`).
- Each build gets a new AppDoc object id (timestamps) unless `SOURCE_DATE_EPOCH` is set. `info` `valid:true` = container and object graph intact; `signature_validation not-present` and `publication_validation not-checked` mean unsigned and unpublished.
- Record per build: version, `pikg_digest`, `app_doc_object_id`, `pkg_id`s.

#### Test install: three different paths
1. First install (App not installed): a plan, then install with the same plan.
```
buckyos --non-interactive app fetch <x.pikg> --plan plan.json --policy local-developer
buckyos --non-interactive --yes --timeout 10m app install <x.pikg> --plan plan.json --policy local-developer
```
2. Local new version: bump `app.json.version` (and `pikg_file`), build/pack, then `app install <new.pikg> --policy local-developer` without `--plan`; `--dry-run` first shows `plan_use UPGRADE`. The same version is answered as satisfied and nothing reinstalls (from the code).
3. Catalog upgrade: `buckyos app upgrade [<app_did>] [--dry-run]` resolves the App DID through BNS; it takes no PIKG. A local-developer App has no BNS record → `TRUST_RESOLUTION_REQUIRED`.
- An `aiws` package (AI Workspace controls) goes through the same `app install`, but control-panel answers at once (no task): `action` `installed | satisfied | replaced`, `pkg_id`, `controls` — it is in the user's AI Workspace library (`buckyos aiws library list`), no App is deployed. A developer can skip the Installer with `buckyos aiws library install <x.pikg>`.
- `local-developer` is the policy for unsigned dev packages. The default CLI timeout is 30 s: an install then exits 8 `TIMEOUT` while the task goes on. Use `--timeout 10m`, or `--no-wait` and `buckyos --timeout 10m task wait <task_id>`; `app cancel <task_id>` stops one.
- Select installed Apps by DID or instance id: `app status did:bns:<name>.<owner>` / `<app_id>@<owner>`; a short name becomes `did:bns:<name>` → `APP_NOT_INSTALLED`.
- Remove: `app uninstall <app_did> --data retain|delete --yes`. On 2026-10-17 `--data delete` left `data/home/<owner>/.local/share/<app_id>/` and the script instance volume; report leftovers instead of claiming deletion.

#### Publish: what exists today
- `buckyos app publish <absolute dir on the control-panel host> --app-doc <AppDoc template> [--app-type dapp|web|agent]`: control-panel `app.publish` re-packs that dir into the Zone Repo/NamedStore and stages a PIKG. Result: `publish_status repo_stored_candidate`, `app_doc_signature not-signed`, `app_did_publication not-published`, `pikg_handle` (a control-panel staging handle owned by the caller: the PIKG lives in the named store, bound in `zone/control-panel/_/pikg_staging` until its TTL; `app install` has no flag for it — install through control-panel `apps.inspect` with `source: {kind: "local_pikg", staging_handle}`), `pikg_digest`. Template = `dapp_dist/APPDOC.json` without `pkg_objid`. The re-pack changes digests; the dir is a host path, so files in your container are not visible to it.
- Not implemented anywhere: Owner signature (`APPDOC.jwt`), BNS/App DID update, upload to a public server, read-back. After `app publish`, `app fetch did:bns:…` still fails `TRUST_RESOLUTION_REQUIRED`. Deliver the digest, the staging handle with its install route, and say it is a candidate, not a release; a PIKG file to hand elsewhere comes from your own local build, not from `app publish`.

#### System, SDK and image releases
- No single release command. System: `.github/workflows/build-all.yml` on push to `main` builds linux/macos/windows payloads through `python buckyos-build.py`, stages them with `buckyos-install --all`, writes `.install-buckyos.json`, uploads `buckyos-<os>-<arch>-<ver>.tar.gz` to R2 `buckyos-installer` (secrets `R2_*`). Release and dev-test workflows use the same system build wrapper. Version: `src/VERSION` = `src/Cargo.toml` = `bucky_project.yaml`; binaries report `<ver>+build<YYMMDD>.<branch>.<commit12>[.dirty]`.
- Agent Daemon: `.github/workflows/agent-daemon.yml` publishes on tag `agent-daemon-v<version>` or dispatch with `publish`. `src/tools/package_agent_daemon.py` reads the built binary's `--version` and extracts `<src/VERSION>+build<YYMMDD>`, as the system payload workflows do; `--version`, `doctor` and `status` use `buckyos-kit::get_version()`. Archives, `manifest.json`, the release tag and `latest.json` share that package version (e.g. `0.7.0+build261026`). Release assembly reads archive manifests and rejects mixed target versions rather than recalculating the date. A pushed tag must match the full package version. Same-day rebuilds keep the same version and cannot overwrite an existing versioned release; `agent-daemon-latest` provides the rolling links. See `doc/arch/agent_daemon.md` for the release and verification steps.
- Images: `build-aios.yml` / `build-exttool.yml` run from build-all when their paths changed since the last published image (R2 pointer `paios-images/main/<name>.json`). They push `paios/aios` and `paios/exttool` `:<ver>-<commit12>-<arch>` (immutable), `:<ver>-<arch>`, `:latest-<arch>` and manifests (secrets `DOCKERHUB_*`), then export `docker save` tarballs to R2 `paios-images/<name>/<arch>/<sha256>.tar.gz`. Every main payload carries `bin/paios-images/paios_images.json` pinning those images (`.github/scripts/paios_images.py validate <file>` checks one). Locally `./build_aios --push` uses the host's `docker login` and moves only the mutable tags.
- Installers: `make_local_pkg.py build-pkg|verify-pkg` (deb/rpm/pkg/exe, scripts in the `src/publish` submodule); dev-tests builds and verifies them, nothing uploads them.
- SDK/CLI: websdk npm `buckyos`, `publish-to-npmjs.yml` on tag `publish<version>`; the system build repacks it into `libexec/buckyos-tool` with `distribution.json` (versions, sha256 per file) and an SBOM.
- Built-in Apps and the Jarvis template ship inside the system release (`rootfs/data/cache` + `system/install_settings`). All of this needs repo push rights or CI secrets: prepare the change and commands; the maintainer runs them.

#### When it fails
- Exit 2 usage (`INVALID_OWNER_DID`, `SCHEMA_VALIDATION_FAILED`, `ARGUMENT_CONFLICT`), 3 auth, 4 permission (`PERMISSION_DENIED`, `CONFIRMATION_REQUIRED` → add `--yes`), 6 operation (`PLAN_REQUIRED`, `PLAN_NOT_APPLICABLE`, `PLAN_INPUT_REQUIRED` → give `target`/`install_params` through `--input`, `TRUST_RESOLUTION_REQUIRED`, `INVALID_PACKAGE`), 8 timeout (task may still succeed: `task get` before retrying), 9 internal/policy.
- A refused write with your agent identity is an answer: report it and hand the owner the commands; don't switch identities.

#### Check the result
Evidence chain in the report: version → `pikg_digest` → `task_id` + `task_outcome Succeeded` → `app status`: `app_version` and `desired_deployment.pikg_digest` equal yours → the page/API answers with the new version → data written before the upgrade is still there. Static web shows `readiness NOT_READY` (no instance) and the gateway served the previous package after an upgrade in the 2026-10-17 check; trust the served content, not exit 0.

#### Source and extending
Current repository contracts: [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md), [package format](https://github.com/buckyos/buckyos/blob/main/doc/pikg/format.md), [App configuration](https://github.com/buckyos/buckyos/blob/main/doc/sdk/app-package-config.md). This manual supplies the task recipe; schemas and historical proposals are maintained in those documents.
CLI source as above; SDK types `/opt/buckyos/libexec/buckyos-tool/dist/*.d.ts`. For a missing verb copy the call pattern of `cli/modules/app.ts` into `$OPENDAN_AGENT_ROOT/tools/<name>.ts` and run it with `deno run --config /opt/buckyos/libexec/buckyos-tool/cli/deno.json`; never edit the mounted copies. Repo references (host): `AGENTS.md`, `src/buckyos-build.py`, `src/bucky_project.yaml`, `build_aios`, `.github/workflows/`, websdk `doc/modules/{pikg,app}.md`.

Checked against: BuckyOS 0.7.0, Tool/SDK 0.7.131, on test.buckyos.io from the host (not yet from an agent container): static-web and Docker install+upgrade, script install, `app publish`; release flow from code and workflows, 2026-10-17.
