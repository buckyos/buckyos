# BuckyOS SDK / Tool distribution

`buckyos-websdk` is the only source repository for the TypeScript Tool. BuckyOS source consumers
track its latest `main` branch and do not commit dependency lockfiles. A system-image build resolves
that source once and stages one verified `buckyos-<version>.tgz`; only third-party App projects pin
the SDK through their package-manager lockfile.

## Developer distribution

Pin `buckyos` in the App project's package manager lockfile and run the project-local binary:

```bash
npm install buckyos
npx buckyos --version --verbose
npx buckyos pikg doctor
```

This distribution runs the prebuilt CLI bundle on Node and contains no Deno runtime. Do not install
it globally: an unqualified `buckyos` on `PATH` is reserved for the system installer entry.

For reproducible PIKG fixtures and release builds, set `SOURCE_DATE_EPOCH` to a non-negative Unix
timestamp. Node and Deno then produce byte-identical PIKG output for the same source tree.

## System distribution

The SDK release job builds once, packs once, and creates an integrity manifest using the pinned Deno
binary for the target platform:

```bash
pnpm run build
npm pack
node scripts/create-sbom.mjs \
  --tarball ./buckyos-<version>.tgz \
  --deno /path/to/pinned/deno \
  --output ./sbom.cdx.json
node scripts/create-release-manifest.mjs \
  --tarball ./buckyos-<version>.tgz \
  --deno /path/to/pinned/deno \
  --sbom ./sbom.cdx.json \
  --output ./release-manifest.json \
  --buckyos-version <buckyos-version> \
  --build-id <immutable-build-id>
```

The CycloneDX SBOM records the production dependency graph, npm and lockfile hashes, plus the pinned
Deno runtime. The manifest records the SBOM digest, npm SHA-256/SRI, every package file digest,
Tool/SDK/protocol versions, and the Deno version and digest. BuckyOS verifies all of them before
changing rootfs:

```bash
BUCKYOS_SDK_TOOL_ARTIFACT=/immutable/buckyos-<version>.tgz \
BUCKYOS_SDK_TOOL_RELEASE_MANIFEST=/immutable/release-manifest.json \
BUCKYOS_SDK_TOOL_DENO=/immutable/deno \
BUCKYOS_SDK_TOOL_SBOM=/immutable/sbom.cdx.json \
uv run src/buckyos-build.py <normal build arguments>
```

The staged layout is fixed:

```text
$BUCKYOS_ROOT/bin/buckyos
$BUCKYOS_ROOT/libexec/buckyos-tool/
  cli/ dist/ package.json LICENSE distribution.json sbom.cdx.json
  runtime/deno[.exe]
```

The launcher uses only the bundled runtime. It constructs the system policy once, then starts the
inner Tool with matching Deno read/write/env/run/network permissions. Installer/updater module
transactions own updates and rollback; the Tool has no self-update path.

Use `buckyos --version --verbose` to inspect the selected executable/runtime, distribution policy,
versions, artifact integrity summary, target and identity candidate order. `buckyos pikg doctor` is
the read-only project-oriented equivalent.
