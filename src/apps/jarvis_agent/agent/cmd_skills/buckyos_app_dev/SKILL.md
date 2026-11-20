---
name: buckyos_app_dev
description: Develop a traditional BuckyOS App — static web page, Python/TypeScript script service or Docker service — with SDK identity, AppDoc permissions/endpoints/data dirs, PIKG metadata, local debugging, page tests with Playwright on the system's Chromium, test install and an upgrade that keeps the App's data.
requires: buckyos command with pikg (agent container or host); node/deno/python for the App's own build; Docker Apps need a host with Docker; installs need an identity the control-panel accepts
baseline: extended
commands: node, npm, npx, deno, uv, python3, curl, agent-browser
---
## Choose the App

- `static-web`: gateway serves built files at the App's web host. Use the browser SDK for user identity and storage.
- `script`: package Python (`main.py`, uv dependencies) or TypeScript (`main.ts`, Deno) with `buckyos_script.json` naming the entry. Listen on `0.0.0.0:$BUCKYOS_SERVICE_PORT`. Declare the service endpoint.
- `docker`: build the image on a host with Docker, match its architecture to the node and declare the service endpoint. Desktop Lite supports user-installed static web Apps only.
- For Agent templates and skill packages use `opendan_agent_dev`. For shared commands, package `ext-tools` and have the admin enable them. For AI Workspace controls use `aiworkspace_template_dev` and export with `buckyos aiws ext-export --pikg`.

## Build and test install

1. In the project run `buckyos pikg doctor` and `buckyos auth whoami`. Build the App with its own toolchain, e.g. build a page into `web/dist`.
2. Initialize once with `buckyos --non-interactive pikg init . --name myapp --owner did:bns:<owner> --kind static-web --source ./web/dist`. For script or Docker change kind and source. Keep `dapp_meta/` under version control.
3. Read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_app_dev/references/package.md` for AppDoc permissions, endpoints/ports, package selectors, SDK compatibility, local debugging and schema errors.
4. Read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_app_dev/references/sdk.md` for service login, browser SSO, desktop file dialogs, preview and embedded conversations. Use the user's own identity for user actions. For user files, uploads, durable storage and uninstall retention, read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_app_dev/references/storage.md`.
5. Build/pack/inspect, then test-install using `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release/SKILL.md`. For a local upgrade, write a sample record first. Keep DID/name, increase `app.json.version`, update `pikg.json.pikg_file` and Docker image/tag if applicable. Install the new PIKG and compare old/new content.
6. Report App DID, version, PIKG digest, successful task, instance ID and actual served URL/API response. Verify pre-upgrade data survived. If the user asked for the package, attach the saved PIKG through `report.artifacts`.

## Page tests

Run `agent-browser version`, match `@playwright/test` to the reported `playwright-core` version and set `use: { channel: 'chromium' }` for Playwright tests on the extended runtime. Read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_app_dev/references/page-tests.md` for the commands, missing browser and pinned-version cases. For a user's live browser session use `browser`.
