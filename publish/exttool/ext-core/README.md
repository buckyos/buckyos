# ext-core redistribution materials

The payload contains separately licensed third-party programs. The BuckyOS
repository license does not replace any tool's license. BuckyOS-authored
launchers, smoke tests, Windows shims, and packaging scripts use AGPL-3.0-only;
their LICENSE and NOTICE are retained under `LICENSES/buckyos-packaging/`.
A publishable payload
must be accompanied by its matching corresponding-source archive.

Run the builder with the existing workspace Python environment; PyYAML is
already provided by `buckyos-devkit`:

```bash
cd /path/to/buckyos
PYTHON="$(uv run python -c 'import sys; print(sys.executable)')" \
  publish/exttool/ext-core/build.sh --platform linux-x86_64 --archive
```

Once all source requirements are complete, every build prepares the
corresponding-source archive in `dist/`, including builds without `--archive`
that will subsequently be packed as PIKG. The binary
tarball and release `manifest.json` are produced with `--archive`. Use `--out`
and `--dist` for disposable output, `EXT_CORE_SOURCE_OUT` for the source staging
directory, and `EXT_CORE_CACHE` for downloads and the micromamba package cache.
Nothing is published by the build.

The builder collects Conda package metadata **before pruning**. It preserves
the exact package identity and binary URL/hash, actual `info/licenses` texts,
rendered recipes, patches, build scripts, build configuration, and installed
package records. Source URLs are read from rendered conda-build recipes and
rattler staging caches; they are never confused with Conda binary package URLs.
Each source archive is checked against the recipe's SHA256. Exact-content
upstream mirrors are listed in `source-mirrors.json`; their bytes must have the
same digest as the original recipe. Source templates, VCS-only recipes, missing
patches, and unpinned downloads fail collection.

The source archive also contains registry crate sources pinned by top-level
Cargo.lock files, including dependencies compiled into Rust libraries. A
non-registry Cargo dependency is recorded as unresolved until its exact source
is provided. The original recipes and locks remain available for rebuilding.
When a package omits standalone license files, the collector retains actual
license files or leading license/copyright comments from its matching source,
with their original paths. It does not synthesize a license from an SPDX label.

`official-sources.json` separately pins the official binaries' source materials
and release recipes. Changes to build.sh version constants or sources.sha256
must be matched by reviewed provenance updates. `official_sources.py` also
collects the pinned native dependencies listed by Python standalone's build
recipe and LibreOffice's `download.lst`, and font license texts. This collection
does not by itself prove an arbitrary upstream binary's complete static source
closure: explicit unresolved requirements block finalization and publishing.

Current release blockers are recorded in each official component's
`unresolved_source_requirements`, rather than silently ignored. In particular,
Pandoc's exact binary-specific Haskell dependency/build plan must be obtained;
the manifest records the current status of every other component separately.
A project source tarball alone is insufficient to clear these entries. Do not
remove a blocker merely because a source download succeeded.

The current release decision is to retain the official binaries and block
distribution while their required corresponding source remains incomplete.
Therefore a current Pandoc build is expected to stop at finalization; this is
not a reason to substitute another binary or bypass the checks. The collected
materials remain in the source staging directory, with a `collection-report.json`
explaining the blocker. They are review material, not a publishable source
release. Existing deployed tools are unaffected.

To inspect or repair collection for an existing cached Linux build, write to
separate directories. Omitting `--prefix` deliberately uses the checked-in
Conda lock and the exact cached package records, useful after the original
prefix's metadata has already been pruned. This checks the lock/cache source
materials; it does not prove that an arbitrary existing payload's Conda
binaries came from that lock. Official collection additionally checks its
selected files against the pinned upstream binary archive. Start with a copy
of the actual built payload, not an empty directory:

```bash
review_dir="$(mktemp -d)"
cp -a publish/exttool/ext-core/payload/linux-x86_64 "$review_dir/payload"
rm -rf "${review_dir:?}/payload/store/ext-core/LICENSES"
uv run python publish/exttool/ext-core/compliance.py conda \
  --platform linux-x86_64 \
  --package-cache publish/exttool/ext-core/.cache/mamba/pkgs \
  --download-cache publish/exttool/ext-core/.cache/corresponding-source \
  --source-dir "$review_dir/sources" --payload "$review_dir/payload"
uv run python publish/exttool/ext-core/official_sources.py \
  --platform linux-x86_64 \
  --cache publish/exttool/ext-core/.cache/corresponding-source \
  --source-root "$review_dir/sources" --payload "$review_dir/payload"
```

These commands only collect source and notices; they do not reconstruct or
change the installed tools. `--offline` requires all pinned source artifacts
to be present in the cache. Missing materials produce a nonzero exit status.
The normal build's persistent staging directory is
`publish/exttool/ext-core/corresponding-source/<platform>/` (Git-ignored).

Finalization verifies all recorded source hashes and license directories,
rejects unresolved source requirements, then writes:

- `ext-core-<version>-<platform>-corresponding-source.tar.gz`: exact upstream
  sources, locked dependencies, recipes, patches, configuration, notices, and
  BuckyOS repackaging scripts.
- Payload `store/ext-core/LICENSES/source-manifest.json`: source inventory and
  an inventory of the binary payload files.
- Payload `store/ext-core/LICENSES/corresponding-source.json`: matching source
  archive filename, SHA256, size, source-manifest SHA256, and release path.
- Payload `store/ext-core/LICENSES/SOURCE_CODE.md`: direct source download URL
  and redistribution instructions.

`release_manifest.py` refuses a missing or changed source archive, changed
payload files, missing or changed payload license texts, and mismatched source
manifests. The release manifest's per-platform `corresponding_source` entry
must be published beside the binary and preserved by downstream packaging.
The source archive is offered at
`https://buckyos-installer.tbudr.top/<corresponding_source.path>`; a PIKG
redistributor must also keep that exact source archive available.

Run the collection and tampering checks without rebuilding tools:

```bash
uv run python -m unittest discover -s publish/exttool/ext-core -p 'test_*.py' -v
bash -n publish/exttool/ext-core/build.sh
```
