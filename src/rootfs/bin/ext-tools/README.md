# ext-tools

The seed of the extension tool packages: `ext_tools.json` (`buckyos.ext-tools/v1`)
and the `ext-core-<version>-<platform>.tar.gz` it names. On a node with no install
record, node_daemon unpacks it into `$BUCKYOS_ROOT/tools`.

Development installs get it from `src/tools/prepare_ext_tools.py` (called by
`buckyos-build.py` and `start.py --all`). A release payload has only the manifest,
pinned by CI; the online installer and the offline package fetch the tar it names.
With nothing here the system runs, without the extension tools. This file keeps the
directory present, since `buckyos-install` copies it whether or not it was filled.

Format and pipeline: `notepads/opendan-exttools-cross-platform-todo.md` §3.10.
