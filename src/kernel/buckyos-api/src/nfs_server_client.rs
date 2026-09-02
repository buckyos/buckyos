// nfs-server (NFSP v0 file service) service constants + settings schema.
// The service speaks plain HTTP/REST (see cyfs-ndn NamedFileSystem_Protocol_v0),
// not kRPC, so unlike repo/msg-center there is no RPC client wrapper here —
// clients use the TS NfspClient against /kapi/nfs-server.

use serde::{Deserialize, Serialize};

pub const NFS_SERVER_UNIQUE_ID: &str = "nfs-server";
pub const NFS_SERVER_SERVICE_NAME: &str = "nfs-server";
pub const NFS_SERVER_SERVICE_PORT: u16 = 4110;

/// `services/nfs-server/settings`. Seeded by the scheduler boot builder with
/// defaults (insert_json_if_absent, so user edits survive re-boot).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NfsServerSettings {
    /// Reconciler scan interval in seconds; 0 disables the background loop.
    pub scan_interval_secs: u64,
    /// Enables `POST /nfs/v1/debug/*` (dev/test only).
    pub debug_api: bool,
}

impl Default for NfsServerSettings {
    fn default() -> Self {
        NfsServerSettings {
            scan_interval_secs: 30,
            debug_api: false,
        }
    }
}
