//! The product profile this installation was shipped with
//! (`$BUCKYOS_ROOT/etc/product.json`), read once. node_daemon enforces it
//! locally, before activation and whatever system-config says: BuckyOS
//! Desktop Lite never probes, loads or runs a container.

use buckyos_api::ProductProfile;
use buckyos_kit::get_buckyos_root_dir;
use std::sync::OnceLock;

static INSTALLED: OnceLock<Result<ProductProfile, String>> = OnceLock::new();

pub(crate) fn installed() -> Result<&'static ProductProfile, String> {
    INSTALLED
        .get_or_init(|| {
            // Unit tests do not depend on what is installed on the machine.
            if cfg!(test) {
                return Ok(ProductProfile::full());
            }
            ProductProfile::load_installed(&get_buckyos_root_dir())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Whether this installation may use containers at all. An unreadable
/// profile is no permission.
pub(crate) fn containers_enabled() -> bool {
    installed().is_ok_and(|product| product.container_enabled())
}
