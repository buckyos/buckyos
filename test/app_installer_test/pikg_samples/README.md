# PIKG test samples

This directory contains three complete `.pikg` files generated through the running Control Panel's `app.publish` endpoint:

- `static-web.pikg`: platform-independent static website package (`pkg_list.web`).
- `script-host.pikg`: platform-independent Python service package (`pkg_list.script`).
- `docker.pikg`: host-architecture Docker image package (`pkg_list.amd64_docker_image` or `pkg_list.aarch64_docker_image`).

`manifest.json` records each package digest, App DID, App Document object ID, package key, and byte size. Regenerate all artifacts from `test/app_installer_test` with:

```bash
pnpm run generate:pikg-samples
```

Generation requires a running BuckyOS development environment, `repo-service`, Control Panel built from the current source, and a working Docker daemon. The Docker architecture is selected from the generation host.
