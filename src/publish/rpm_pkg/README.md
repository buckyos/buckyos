# BuckyOS RPM Package Hooks

This directory contains the RPM-side Linux component hooks used by
`make_local_rpm.py`.

`buckyos.spec` is the visible RPM spec template. `make_local_rpm.py` fills
the `{{...}}` placeholders at build time and writes the rendered spec into
the temporary rpmbuild `SPECS` directory.

The `buckyos_preinstall` and `buckyos_postinstall` hooks intentionally mirror
the current `deb_pkg` hooks so that Fedora rpm installs follow the same
preinstall/postinstall behavior as Debian packages. `make_local_rpm.py`
renders the same generated `modules` and `data_paths` blocks into these files
while producing the rpm spec scriptlets.
