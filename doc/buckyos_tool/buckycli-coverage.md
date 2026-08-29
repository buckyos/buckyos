# Retired Rust `buckycli` coverage

Rust `buckycli` was retired in Beta 2.2. The `buckyos` command is not a name-compatible wrapper;
callers must use the domain command or the owning build/provision API below. BuckyOS now ships the
system Tool at `$BUCKYOS_ROOT/bin/buckyos` as part of the main `buckyos` component.

| Rust command | Replacement / disposition | Retirement state |
| --- | --- | --- |
| `version` | `buckyos --version --verbose` | covered |
| `sys_config` | `buckyos system-config get/list/set/set-file/append` | covered |
| `install_pkg`, `pack_pkg`, `load_pkg` | `buckyos pikg build/pack/info` for App PIKG; system package environment remains installer-owned | App path covered; legacy system-pkg path not carried forward |
| `pub_app`, `pub_pkg`, `pub_index`, `update_index` | repository/Installer service APIs; no local CLI compatibility command | intentionally removed from Tool surface |
| `load` | versioned App/DV environment, then `app install/status` and `log` | covered by the new workflow |
| `connect` | explicit `--zone`/`--endpoint` plus normal domain commands | covered |
| `node start/stop/restart/check/ensure-running/detect-host-control` | installer/updater and host service lifecycle APIs | not a Tool compatibility surface |
| `did genkey/create_user/create_device/create_zoneboot/create_zone`, `sign`, `create_user_env`, `create_node_configs`, `create_sn_configs`, `register_device_to_sn`, `register_user_to_sn`, `build_did_docs`, `create_chunk`, `set_pkg_meta` | typed `buckyos/provision` APIs and `src/make_config.ts` | covered; no Rust CLI caller remains |

The Rust crate, workspace member, rootfs module, standalone app, host build helper, CI artifact, and
macOS/Windows installer components have been removed. The stable `buckycli` app id and historical
`.buckycli` identity/config paths are runtime contracts and are not executable compatibility entry
points.
