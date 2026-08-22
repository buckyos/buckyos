# BuckyOS Tool

`buckyos-tool` is the TypeScript/Deno implementation of the production-facing `buckyos` command. The
current implementation covers PRD Phase 0 through Phase 2: command discovery, profiles, JSON
protocols, authentication, online system operations, the shared interactive session, and the fully
local `pikg` development workflow.

## Run from source

Deno 2.2 or newer is required. The repository's existing `buckyos` WebSDK installation under
`src/apps/sys_test/node_modules` supplies the SDK and kRPC implementation.

```bash
cd src/tools/buckyos-tool
./buckyos --version
./buckyos --help
./buckyos command list
./buckyos command describe system status
```

The launcher grants network access, read access to the tool, SDK, selected config/input/identity
paths, and write access only to the selected config directory. It does not grant process execution
or use Deno's `-A` permission.

`pikg` is the local-development exception. For these commands the launcher grants filesystem access
but no network access; Docker builds additionally use only `docker image inspect` and
`docker image save` through argument-vector process calls. The module never resolves a profile,
Zone, identity, or session.

## Local PIKG workflow

```bash
./buckyos --non-interactive pikg init . \
  --owner did:bns:root --kind static-web --source ./web/dist
./buckyos pikg build ./dapp_meta
./buckyos pikg pack ./dapp_dist
./buckyos pikg info ./dapp_dist/example-0.1.0.pikg
./buckyos --non-interactive --yes pikg clean ./dapp_meta
```

## Configuration and online commands

```bash
./buckyos system status
./buckyos --non-interactive --yes system status
./buckyos --config-dir /tmp/buckyos-tool config set zone \
  --value test.buckyos.io --profile-name dev
./buckyos --config-dir /tmp/buckyos-tool config use dev
./buckyos --profile dev --session-token-file /run/secrets/buckyos.jwt auth whoami
./buckyos --profile dev --session-token-file /run/secrets/buckyos.jwt system status
./buckyos --profile dev --session-token-file /run/secrets/buckyos.jwt system-config get boot/config
./buckyos --profile dev --session-token-file /run/secrets/buckyos.jwt system-config list services
./buckyos --profile dev --session-token-file /run/secrets/buckyos.jwt system-config set-file \
  services/example/config --file ./config.json
./buckyos --profile dev --identity alice --cli
```

With no token, identity, Zone, or endpoint configured, an online command reads the current device
from `$BUCKYOS_ROOT/etc/node_identity.json` and uses the local NodeGateway. Interactive terminals
must confirm this high-privilege fallback once per process. Non-interactive execution must provide
`--yes`; an explicitly selected identity or connection target never triggers the fallback.

Identity lookup uses only the PRD IdentityRoots order. It never scans `~/.buckycli` or `~/buckycli`.
Session and refresh tokens are never persisted by the tool.

## Verification

```bash
deno task check
deno lint
deno task test
```
