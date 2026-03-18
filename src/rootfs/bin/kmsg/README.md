Lifecycle scripts were removed from this directory.
`node_daemon` now handles `start`/`stop`/`status` through the Rust native fallback in `ServicePkg` when scripts are absent.
