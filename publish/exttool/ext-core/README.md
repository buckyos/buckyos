# ext-core redistribution materials

The payload contains separately licensed third-party programs. The BuckyOS
repository license does not replace any tool's license. BuckyOS-authored
launchers, smoke tests, Windows shims, browser commands (`browser/`), and
packaging scripts use AGPL-3.0-only;
their LICENSE and NOTICE are retained under `LICENSES/buckyos-packaging/`.
The sample files that `smoke/THIRD_PARTY.md` lists (a Visio drawing from
LibreOffice's tests, MPL-2.0) are third-party and keep their own license.
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
upstream mirrors are listed in `source-mirrors.json` under that pin, tried after
the original addresses; their bytes must have the same digest as the original
recipe. `build.sh` uses the same file for what `sources.sha256` pins: when a
download from its address fails or has other bytes, the addresses listed for its
SHA256 are tried in turn and checked the same way. Source templates, VCS-only
recipes, missing patches, and unpinned downloads fail collection.

A Cargo.lock inside a source archive brings its crates.io dependencies, each
pinned by the lock's checksum. A git dependency of such a lock is included only
when `cargo-git-sources.json` pins an archive of its locked commit (an https
address, the file name, the archive's SHA256); GitHub's archive of a commit
(`codeload.github.com/<owner>/<repo>/tar.gz/<commit>`) is byte-stable. An
unpinned git dependency stays a release blocker.

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

Every source item is pinned by the SHA256 of its bytes (`sha256`) except a
gitiles archive of a commit (`https://*.googlesource.com/…/+archive/<commit>.tar.gz`),
which gitiles makes anew on each request: the same files, other bytes (member
times and the like). Such an item is pinned by its content instead,
`tree_sha256` (`compliance.tree_sha256`): one line per regular file,
`<path>\t<x or ->\t<sha256 of its contents>` (`x`: any executable bit), and per
symbolic link, `<path>\tl\t<target>`; directories are left out, paths are the
archive's without a leading `./`, and the lines are sorted, each ending in a
newline, the SHA256 taken of their UTF-8. No other address is pinned this way.
The download, its cache entry (`tree-<tree_sha256>` in the download cache, with
whichever bytes it got), `--offline` and `release_manifest.py` check such an
archive by its content. The source archive carries the bytes that were
downloaded, and its manifest records their SHA256 beside the `tree_sha256`.

Chrome for Testing is not part of the payload. It is an online item, declared
in `ext-manifest.json` (`online`) with Google's own address, SHA256 and size
per platform: a node downloads it into `<tools>/online/chromium/` once the
package is installed. Nothing we publish carries it, so it has no entry in
`official-sources.json` and no license material here (see
`OFFICIAL_SOURCES.md`). The build fetches the archive pinned in
`sources.sha256`, lays it out as a node would with `place_online.py` and checks
it against the payload's libraries, then discards it; the smoke jobs of CI use
the same script to give an unpacked payload its browser before the network is
cut.

The manifest fields of an online item (`online[]`: `id`, `version`, `desc`,
`license`, `auto`, and per platform `url`, `sha256`, `size_bytes`, `format`,
`into`, `rename`, `overlay`) and `needs` on the commands that cannot run
without it are specified in `doc/pikg/format.md` §5.1; what a node does with
them (layout, record, the one automatic download, asking again) is in
`doc/app_service/paios容器需求.md` §6.4. A command with `needs` has its probe
and smoke skipped while its item is not on the node. Once it is, the smoke runs
with `ext-tools check --full`, and also when a later version of the package is
checked in `<tools>/staging/<name>/`: a launcher has to find
`<tools>/online/<item>` from there and from `<tools>/versions/ext-core/<dir>/`
as well as from `<tools>/bin`. The payload's overlay (`online/<item>/`) is laid
over the item again each time the package is activated, without the archive
being fetched again, so a changed script in it reaches a node with the package.

Release blockers are recorded in each official component's
`unresolved_source_requirements`, rather than silently ignored. A gap the owner
accepts in writing moves to `accepted_source_gaps` (`gap`, `accepted_by`,
`accepted_at`): it no longer blocks a release and stays recorded in the source
manifest. Pandoc's binary-specific Haskell dependency/build plan is such a gap
(accepted 2026-11-15, see `OFFICIAL_SOURCES.md`). A project source tarball alone
is insufficient to clear an entry. Do not remove a blocker merely because a
source download succeeded.

The current release decision is to retain the official binaries and block
distribution while their required corresponding source remains incomplete.
Therefore a current build is expected to stop at finalization, with the payload
complete in `payload/<platform>/` but without its source manifest; this is
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
