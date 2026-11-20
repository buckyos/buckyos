---
name: buckyos_dev_release
description: Shared BuckyOS develop-and-release loop — where to work, system build vs App build, PIKG build/pack/info, test install (first install, local new version, catalog upgrade) with versions/digests/task evidence, what App publish really does, and how system/SDK/image releases are made.
requires: buckyos command (agent container: /opt/buckyos/bin/agent-tools/buckyos with the host Tool mounted; host: $BUCKYOS_ROOT/bin/buckyos); installs need an identity the control-panel accepts; system builds, Docker images and releases need the host repo and the owner
baseline: extended
commands: git, uv, node, npm, npx, deno, python3
---
## Build a package

Check the running CLI and identity with `buckyos --version --verbose` and `buckyos auth whoami`. Discover actual syntax and access with `buckyos command describe <module> <verb>`. Work under the session/project directory. Build App output with its own toolchain before packaging. Keep `dapp_meta/` and copy any PIKG you deliver out of regenerated `dapp_dist/`.

```sh
buckyos --non-interactive pikg init . --name <app> --owner did:bns:<owner> --kind static-web --source ./web/dist
buckyos pikg build ./dapp_meta && buckyos pikg pack ./dapp_dist
buckyos pikg info ./dapp_dist/<app>-<ver>.pikg
```

Use a one-label owner DID. Set the init kind/source for your package; templates have a separate layout (`opendan_agent_dev`). Record the version, `pikg_digest`, `app_doc_object_id` and subpackage IDs from package metadata. A valid object graph alone proves neither signature nor publication.

## Pick the install path

1. **First install, unsigned local package.** Save then use the same plan:

```sh
buckyos --non-interactive app fetch <file.pikg> --plan plan.json --policy local-developer
buckyos --non-interactive --yes --timeout 10m app install <file.pikg> --plan plan.json --policy local-developer
```

2. **Existing App, local new version.** Increase `app.json.version`, update `pikg_file`, build/pack. Preview with `buckyos app install <new.pikg> --policy local-developer --dry-run` (expect `plan_use UPGRADE`), then install without the old plan, using `--yes --timeout 10m`. Same version may already be satisfied.
3. **Catalog upgrade.** `buckyos app upgrade [<app_did>] [--dry-run]` resolves the DID through BNS. Local-only packages lack a catalog record.
4. Record task ID, wait for Succeeded, then `buckyos app status <full_app_did_or_instance>`. Compare installed `app_version`, `desired_deployment.pikg_digest`, actual served version and data created before upgrade. A timeout may leave a running task; query it before retry. An `aiws` install returns the library result immediately without a task.

For PIKG reproducibility, identity, install errors, library packages, removal and Zone Repo publish, read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release/references/packages.md`. `app publish` stages an unsigned Zone candidate; report digest and handle, and hand over the locally built PIKG when requested.

## System, SDK and images

- For host build/apply, worker images, standalone Agent Daemon, Desktop pkg, source/license catalog or credentials migration, read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release/references/host-build.md`. Owner authorization is needed to restart the Zone.
- For system release workflows, image tags, R2, SDK/npm, installer artifacts, licensing and Desktop Node Updater checks, read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release/references/releases.md`. Maintainers run release steps using their own push/CI rights.

Use `buckyos_app_dev` to write/debug Apps, `opendan_agent_dev` for Agent templates/skills, and `buckyos_admin` to inspect the running Zone.
