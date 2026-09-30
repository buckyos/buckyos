//! Agent-level locks: `self_improve`, `artifact:<aid>` (§5.1).

use crate::error::Result;
use crate::lock::{Acquire, FileLock, Lease};
use crate::protocol::HolderInfo;

use super::fs_client::AgentLayout;
use super::LockManager;

pub(super) struct FsLocks {
    layout: AgentLayout,
}

impl FsLocks {
    pub(super) fn new(layout: AgentLayout) -> Self {
        Self { layout }
    }
}

impl LockManager for FsLocks {
    fn acquire(&self, resource: &str, holder: HolderInfo) -> Result<Acquire> {
        let path = self.layout.lock_path(resource)?;
        Lease::acquire(resource, &path, holder)
    }

    fn is_held(&self, resource: &str) -> Result<bool> {
        let path = self.layout.lock_path(resource)?;
        Ok(FileLock::try_acquire(&path)?.is_none())
    }
}
