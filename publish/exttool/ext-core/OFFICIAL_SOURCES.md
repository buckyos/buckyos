# Official ext-core source materials

`official-sources.json` binds each official binary's existing SHA256 pin to
versioned upstream source archives and license texts. `official_sources.py`
checks those pins against `build.sh` and `sources.sha256`, verifies every
download, and writes `official-components.json` for the release collector.
A failed collection invalidates the previous inventory. Upstream projects and
embedded third-party components retain their own licenses.

Each selected component must actually exist in the supplied payload. The
collector verifies the original cached binary archive's SHA256 and compares
the selected executable or font with its archive member. LibreOffice uses
platform-specific installers: its actual executable and original installer
hash are recorded, but this independent collector does not prove that an old
payload was unpacked from that installer. The normal build supplies that
provenance through its verified download and platform unpacking steps.

Run collection after installing the official tools into a payload:

```sh
python3 official_sources.py --payload payload/linux-x86_64 \
  --source-root corresponding-source/linux-x86_64 --platform linux-x86_64 \
  --cache .cache/corresponding-source
```

The matching source archive must be distributed beside the binary payload.
`compliance.py finalize` rejects declared incomplete source coverage; merely
downloading a project's top-level source archive does not clear that check.

## Included upstream materials

- Pandoc source and upstream build recipes.
- CPython source, the exact python-build-standalone release recipe, and its
  checksum-pinned native-library sources from `pythonbuild/downloads.json`.
  Other Python versions and binary-only build tools are not labeled as source.
- uv, Node.js, Deno, and jq versioned source archives and notices. uv and Deno
  also include every checksum-pinned registry crate from their release
  `Cargo.lock`, with actual dependency license texts. Non-registry Cargo
  dependencies become explicit release blockers rather than being omitted.
- Deno's locked `v8` crate excludes most upstream notices, so the collection
  also includes the exact Rust binding repository and all 20 native
  submodules, plus the Chromium root license corresponding to its split
  repositories. `v8-sources.json` records the revisions and comparison with
  the published crate: 14,321 shared files match; the crate retains its
  generated bindings, normalized manifest and pruned lockfile. Both the
  original crate and the full source notices are preserved.
- LibreOffice source, patches and build recipes, plus every SHA256-pinned
  external input in that release's `download.lst`. This is a superset of the
  headless payload's dependencies. A NuGet build tool is labeled as a build
  input, not as source code.
- MinGit's 65 exact package versions mapped to 50 official source packages,
  with embedded source, recipes and patches. `mingit-sources.json` records the
  mapping, inner-source verification, split-package evidence and two verified
  Git source repositories. The collector checks the original ZIP's package
  list against this mapping and preserves its actual notices. The GPL
  `docx2txt-rs` package also includes its locked Rust dependency sources.
  Full `build-extra` assembly scripts from the recorded `git-extra` revision
  accompany those packages. The package records do not separately identify
  the release assembly checkout; this limitation does not mean that known
  compilation or installation source inputs are missing.
- The matching BusyBox-w32 release source archive.
- Original Noto Sans/Serif CJK font files and each family's actual OFL text.
  The redistributed font files are not described as editable design sources.

## Remaining release blockers

**Pandoc 3.12.1:** the official static binaries have no verified, binary-specific
frozen Cabal package plan in the collected source tree. Extracting some package
versions with `strings`, or using the repository's Nix flake lock, does not
prove the complete linked source closure. Obtain the publisher's matching
build plan and all linked sources, or rebuild the binary from a captured Cabal
plan and package its dependency sources, patches and build settings. The
current project decision is to keep the official binary and block incomplete
releases; this change does not introduce a replacement compiler or binary.
Verify every supported platform before clearing `unresolved_source_requirements`.

MinGit's 65 package-source mappings and 426 embedded inputs have been
verified. Its `git-extra` package identifies `build-extra` commit
`85f886165bf9c422ffed7aebcded27c0b7a55630`, whose assembly scripts are included.
This provides the known corresponding source and packaging materials without
claiming a bit-for-bit reconstruction of the publisher's archive. No MinGit
source blocker remains recorded for this version.

Collection success means the declared materials were fetched and hashed.
Release eligibility additionally requires all recorded source gaps to be
resolved and the finalizer to accept the source inventory. No published
release or remote source hosting is performed by this script.
