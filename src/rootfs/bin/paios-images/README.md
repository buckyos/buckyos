# paios-images

Release packages put `paios_images.json` and the `docker save` tarballs of the
paios images (`paios/aios`, `paios/exttool`) for the target architecture here.
With the manifest present, node_daemon uses exactly the pinned images and loads
a missing one from its tarball; it never pulls.

Development installs leave this directory without a manifest, so node_daemon
uses the local `<repo>:latest-<arch>` images (`etc/devenv.json` can override the
repo). Get them with `uv run build.py`, `docker pull`, or `./build_aios --local-test`.

Format and pipeline: `notepads/paios-image-release-todo.md` §3.2.
