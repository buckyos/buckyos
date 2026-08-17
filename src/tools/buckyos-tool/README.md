# BuckyOS Tool

`buckyos-tool` is the TypeScript/Deno implementation of the production-facing `buckyos` command. The
current implementation covers PRD Phase 0 and Phase 1: command discovery, profiles, JSON protocols,
authentication, `system status`, and the shared interactive session.

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

## Configuration and online commands

```bash
./buckyos --config-dir /tmp/buckyos-tool config set zone \
  --value test.buckyos.io --profile-name dev
./buckyos --config-dir /tmp/buckyos-tool config use dev
./buckyos --profile dev --session-token-file /run/secrets/buckyos.jwt auth whoami
./buckyos --profile dev --session-token-file /run/secrets/buckyos.jwt system status
./buckyos --profile dev --identity alice --cli
```

Identity lookup uses only the PRD IdentityRoots order. It never scans `~/.buckycli` or `~/buckycli`.
Session and refresh tokens are never persisted by the tool.

## Verification

```bash
deno task check
deno lint
deno task test
```
