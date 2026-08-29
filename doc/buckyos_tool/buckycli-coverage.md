# Rust `buckycli` retirement coverage

The new `buckyos` command is not a name-compatible wrapper for Rust `buckycli`. Beta 2.2 callers
must move to the domain command or the owning build/provision API below. Rust removal remains gated
on one formal release of installer upgrade/rollback coverage.

| Rust command | Replacement / disposition | Retirement state |
| --- | --- | --- |
| `version` | `buckyos --version --verbose` | covered |
| `sys_config` | `buckyos system-config get/list/set/set-file/append` | covered |
| `install_pkg`, `pack_pkg`, `load_pkg` | `buckyos pikg build/pack/info` for App PIKG; system package environment remains installer-owned | App path covered; legacy system-pkg path not carried forward |
| `pub_app`, `pub_pkg`, `pub_index`, `update_index` | repository/Installer service APIs; no local CLI compatibility command | intentionally removed from Tool surface |
| `load` | versioned App/DV environment, then `app install/status` and `log` | covered by new workflow, DV still release-gated |
| `connect` | explicit `--zone`/`--endpoint` plus normal domain commands | covered |
| `node start/stop/restart/check/ensure-running/detect-host-control` | installer/updater and host service lifecycle APIs | not a Tool compatibility surface |
| `did genkey/create_user/create_device/create_zoneboot/create_zone`, `sign`, `create_user_env`, `create_node_configs`, `create_sn_configs`, `register_device_to_sn`, `register_user_to_sn`, `build_did_docs`, `create_chunk`, `set_pkg_meta` | typed `buckyos/provision` APIs and the `websdk_provision_todo.md` migration | retained temporarily for build/provision only |

Production build/rootfs/package entries for Rust `buckycli` may be removed only after the final row
has no callers and a formal BuckyOS release has validated the new Tool's upgrade and rollback. This
is a sequencing gate, not a reason to reintroduce the deleted TypeScript compatibility path.

