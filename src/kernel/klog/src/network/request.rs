use crate::{KLogEntry, KLogError, KLogMetaEntry, KNode, KNodeId, KResult, KTypeConfig};
use openraft::error::PayloadTooLarge;
use openraft::error::{InstallSnapshotError, RaftError};
use openraft::network::RPCTypes;
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const NETWORK_MAGIC: &[u8; 8] = b"KLOGRPC1";
const NETWORK_VERSION_V1: u16 = 1;
const NETWORK_CODEC_BINCODE_LEGACY: u8 = 1;
const NETWORK_HEADER_LEN: usize = 8 + 2 + 1 + 1 + 1 + 4;
pub const KLOG_FORWARD_HOPS_HEADER: &str = "x-klog-forward-hops";
pub const KLOG_FORWARDED_BY_HEADER: &str = "x-klog-forwarded-by";
pub const KLOG_TRACE_ID_HEADER: &str = "x-klog-trace-id";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaftRequestType {
    AppendEntries,
    InstallSnapshot,
    Vote,
}

impl RaftRequestType {
    fn to_code(self) -> u8 {
        match self {
            RaftRequestType::AppendEntries => 1,
            RaftRequestType::InstallSnapshot => 2,
            RaftRequestType::Vote => 3,
        }
    }

    fn from_code(v: u8) -> Option<Self> {
        match v {
            1 => Some(RaftRequestType::AppendEntries),
            2 => Some(RaftRequestType::InstallSnapshot),
            3 => Some(RaftRequestType::Vote),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            RaftRequestType::AppendEntries => "append-entries",
            RaftRequestType::InstallSnapshot => "install-snapshot",
            RaftRequestType::Vote => "vote",
        }
    }

    pub fn klog_path(&self) -> String {
        format!("/klog/{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KLogAdminRequestType {
    AddLearner,
    RemoveLearner,
    ChangeMembership,
    ClusterState,
}

impl KLogAdminRequestType {
    pub fn as_str(&self) -> &str {
        match self {
            KLogAdminRequestType::AddLearner => "add-learner",
            KLogAdminRequestType::RemoveLearner => "remove-learner",
            KLogAdminRequestType::ChangeMembership => "change-membership",
            KLogAdminRequestType::ClusterState => "cluster-state",
        }
    }

    pub fn klog_path(&self) -> String {
        format!("/klog/admin/{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KLogDataRequestType {
    Append,
    Query,
    MetaPut,
    MetaDelete,
    MetaQuery,
}

impl KLogDataRequestType {
    pub fn as_str(&self) -> &str {
        match self {
            KLogDataRequestType::Append => "append",
            KLogDataRequestType::Query => "query",
            KLogDataRequestType::MetaPut => "meta-put",
            KLogDataRequestType::MetaDelete => "meta-delete",
            KLogDataRequestType::MetaQuery => "meta-query",
        }
    }

    pub fn klog_path(&self) -> String {
        format!("/klog/data/{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogAppendRequest {
    pub message: String,
    pub timestamp: Option<u64>,
    pub node_id: Option<KNodeId>,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogAppendResponse {
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KLogQueryRequest {
    pub start_id: Option<u64>,
    pub end_id: Option<u64>,
    pub limit: Option<usize>,
    pub desc: Option<bool>,
    /// When true, require linearizable read on leader before serving query.
    pub strong_read: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogQueryResponse {
    pub items: Vec<KLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogMetaPutRequest {
    pub key: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogMetaPutResponse {
    pub key: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogMetaDeleteRequest {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogMetaDeleteResponse {
    pub key: String,
    pub existed: bool,
    pub prev_meta: Option<KLogMetaEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KLogMetaQueryRequest {
    pub key: Option<String>,
    pub prefix: Option<String>,
    pub limit: Option<usize>,
    /// When true, require linearizable read on leader before serving query.
    pub strong_read: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogMetaQueryResponse {
    pub items: Vec<KLogMetaEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogClusterStateResponse {
    pub node_id: KNodeId,
    pub cluster_name: String,
    pub cluster_id: String,
    pub server_state: String,
    pub current_leader: Option<KNodeId>,
    pub voters: Vec<KNodeId>,
    pub learners: Vec<KNodeId>,
    pub nodes: BTreeMap<KNodeId, KNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum NetworkFrameKind {
    Request = 1,
    Response = 2,
}

impl NetworkFrameKind {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Request),
            2 => Some(Self::Response),
            _ => None,
        }
    }
}

impl std::fmt::Display for NetworkFrameKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Request => "request",
            Self::Response => "response",
        };
        write!(f, "{}", name)
    }
}

fn encode_network_frame<T: Serialize>(
    kind: NetworkFrameKind,
    rpc_type: RaftRequestType,
    value: &T,
) -> KResult<Vec<u8>> {
    let payload = bincode::serde::encode_to_vec(value, bincode::config::legacy()).map_err(|e| {
        let msg = format!(
            "Failed to bincode serialize {} frame for rpc {}: {}",
            kind,
            rpc_type.as_str(),
            e
        );
        error!("{}", msg);
        KLogError::InvalidFormat(msg)
    })?;

    let payload_len = u32::try_from(payload.len()).map_err(|_| {
        let msg = format!(
            "Network frame payload too large: kind={}, rpc={}, bytes={}",
            kind,
            rpc_type.as_str(),
            payload.len()
        );
        error!("{}", msg);
        KLogError::InvalidFormat(msg)
    })?;

    let mut out = Vec::with_capacity(NETWORK_HEADER_LEN + payload.len());
    out.extend_from_slice(NETWORK_MAGIC);
    out.extend_from_slice(&NETWORK_VERSION_V1.to_be_bytes());
    out.push(kind as u8);
    out.push(rpc_type.to_code());
    out.push(NETWORK_CODEC_BINCODE_LEGACY);
    out.extend_from_slice(&payload_len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

fn decode_network_frame<T: DeserializeOwned>(
    expected_kind: NetworkFrameKind,
    data: &[u8],
) -> KResult<(RaftRequestType, T)> {
    if data.len() < NETWORK_HEADER_LEN {
        let msg = format!(
            "Network frame too short for header: bytes={}, expected_at_least={}",
            data.len(),
            NETWORK_HEADER_LEN
        );
        error!("{}", msg);
        return Err(KLogError::InvalidFormat(msg));
    }

    let magic = &data[0..8];
    if magic != NETWORK_MAGIC {
        let msg = format!(
            "Invalid network frame magic: expected={:?}, got={:?}",
            NETWORK_MAGIC, magic
        );
        error!("{}", msg);
        return Err(KLogError::InvalidFormat(msg));
    }

    let version = u16::from_be_bytes([data[8], data[9]]);
    if version != NETWORK_VERSION_V1 {
        let msg = format!(
            "Unsupported network frame version: expected={}, got={}",
            NETWORK_VERSION_V1, version
        );
        error!("{}", msg);
        return Err(KLogError::InvalidFormat(msg));
    }

    let frame_kind = NetworkFrameKind::from_u8(data[10]).ok_or_else(|| {
        let msg = format!("Unknown network frame kind: {}", data[10]);
        error!("{}", msg);
        KLogError::InvalidFormat(msg)
    })?;
    if frame_kind != expected_kind {
        let msg = format!(
            "Unexpected network frame kind: expected={}, got={}",
            expected_kind, frame_kind
        );
        error!("{}", msg);
        return Err(KLogError::InvalidFormat(msg));
    }

    let rpc_type = RaftRequestType::from_code(data[11]).ok_or_else(|| {
        let msg = format!("Unknown network rpc type code: {}", data[11]);
        error!("{}", msg);
        KLogError::InvalidFormat(msg)
    })?;

    let codec = data[12];
    if codec != NETWORK_CODEC_BINCODE_LEGACY {
        let msg = format!(
            "Unsupported network frame codec: expected={}, got={}",
            NETWORK_CODEC_BINCODE_LEGACY, codec
        );
        error!("{}", msg);
        return Err(KLogError::InvalidFormat(msg));
    }

    let payload_len = u32::from_be_bytes([data[13], data[14], data[15], data[16]]);
    let payload_len = usize::try_from(payload_len).map_err(|_| {
        let msg = format!("Network frame payload length overflow: {}", payload_len);
        error!("{}", msg);
        KLogError::InvalidFormat(msg)
    })?;
    let actual_payload_len = data.len().saturating_sub(NETWORK_HEADER_LEN);
    if actual_payload_len != payload_len {
        let msg = format!(
            "Network frame payload length mismatch: header={}, actual={}",
            payload_len, actual_payload_len
        );
        error!("{}", msg);
        return Err(KLogError::InvalidFormat(msg));
    }

    let payload = &data[NETWORK_HEADER_LEN..];
    let (decoded, _) = bincode::serde::decode_from_slice(payload, bincode::config::legacy())
        .map_err(|e| {
            let msg = format!(
                "Failed to bincode deserialize {} frame for rpc {}: {}",
                expected_kind,
                rpc_type.as_str(),
                e
            );
            error!("{}", msg);
            KLogError::InvalidFormat(msg)
        })?;

    Ok((rpc_type, decoded))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RaftRequest {
    AppendEntries(AppendEntriesRequest<KTypeConfig>),
    InstallSnapshot(InstallSnapshotRequest<KTypeConfig>),
    Vote(VoteRequest<KNodeId>),
}

impl RaftRequest {
    pub fn request_type(&self) -> RaftRequestType {
        match self {
            RaftRequest::AppendEntries(_) => RaftRequestType::AppendEntries,
            RaftRequest::InstallSnapshot(_) => RaftRequestType::InstallSnapshot,
            RaftRequest::Vote(_) => RaftRequestType::Vote,
        }
    }

    pub fn request_path(&self) -> String {
        self.request_type().as_str().to_string()
    }

    pub fn rpc_type(&self) -> RPCTypes {
        match self {
            RaftRequest::AppendEntries(_) => RPCTypes::AppendEntries,
            RaftRequest::InstallSnapshot(_) => RPCTypes::InstallSnapshot,
            RaftRequest::Vote(_) => RPCTypes::Vote,
        }
    }

    pub fn payload_too_large(&self) -> Option<PayloadTooLarge> {
        match self {
            RaftRequest::AppendEntries(req) => {
                // openraft requires entries_hint > 0.
                let hint = std::cmp::max(1, req.entries.len() as u64 / 2);
                Some(PayloadTooLarge::new_entries_hint(hint))
            }
            RaftRequest::InstallSnapshot(_) => {
                error!("InstallSnapshotRequest is too large to send");
                None
            }
            RaftRequest::Vote(_) => {
                error!("VoteRequest is too large to send");
                None
            }
        }
    }

    // Header format: magic + version + frame_kind + rpc_type + codec + payload_len + payload.
    pub fn serialize(&self) -> KResult<Vec<u8>> {
        encode_network_frame(NetworkFrameKind::Request, self.request_type(), self)
    }

    pub fn deserialize(data: &[u8]) -> KResult<Self> {
        let (header_rpc, this) = decode_network_frame::<Self>(NetworkFrameKind::Request, data)?;
        if this.request_type() != header_rpc {
            let msg = format!(
                "RaftRequest rpc type mismatch between header and payload: header={}, payload={}",
                header_rpc.as_str(),
                this.request_type().as_str()
            );
            error!("{}", msg);
            return Err(KLogError::InvalidFormat(msg));
        }

        Ok(this)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum RaftResponse {
    AppendEntries(AppendEntriesResponse<KNodeId>),
    AppendEntriesError(RaftError<KNodeId>),
    InstallSnapshot(InstallSnapshotResponse<KNodeId>),
    InstallSnapshotError(RaftError<KNodeId, InstallSnapshotError>),
    Vote(VoteResponse<KNodeId>),
    VoteError(RaftError<KNodeId>),
}

impl RaftResponse {
    pub fn request_type(&self) -> RaftRequestType {
        match self {
            RaftResponse::AppendEntries(_) | RaftResponse::AppendEntriesError(_) => {
                RaftRequestType::AppendEntries
            }
            RaftResponse::InstallSnapshot(_) | RaftResponse::InstallSnapshotError(_) => {
                RaftRequestType::InstallSnapshot
            }
            RaftResponse::Vote(_) | RaftResponse::VoteError(_) => RaftRequestType::Vote,
        }
    }

    // Header format: magic + version + frame_kind + rpc_type + codec + payload_len + payload.
    pub fn serialize(&self) -> KResult<Vec<u8>> {
        encode_network_frame(NetworkFrameKind::Response, self.request_type(), self)
    }

    pub fn deserialize(data: &[u8]) -> KResult<Self> {
        let (header_rpc, this) = decode_network_frame::<Self>(NetworkFrameKind::Response, data)?;
        if this.request_type() != header_rpc {
            let msg = format!(
                "RaftResponse rpc type mismatch between header and payload: header={}, payload={}",
                header_rpc.as_str(),
                this.request_type().as_str()
            );
            error!("{}", msg);
            return Err(KLogError::InvalidFormat(msg));
        }

        Ok(this)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openraft::Vote;

    #[test]
    fn test_request_roundtrip_with_header() {
        let req = RaftRequest::Vote(VoteRequest::new(Vote::new(7, 3), None));

        let bytes = req.serialize().expect("serialize request");
        assert!(bytes.starts_with(NETWORK_MAGIC));
        assert_eq!(bytes[10], NetworkFrameKind::Request as u8);
        assert_eq!(bytes[11], RaftRequestType::Vote.to_code());

        let decoded = RaftRequest::deserialize(&bytes).expect("deserialize request");
        match decoded {
            RaftRequest::Vote(v) => {
                assert_eq!(v.vote, Vote::new(7, 3));
                assert!(v.last_log_id.is_none());
            }
            other => panic!("unexpected request type: {:?}", other),
        }
    }

    #[test]
    fn test_response_roundtrip_with_header() {
        let resp = RaftResponse::Vote(VoteResponse::new(Vote::new(9, 2), None, true));

        let bytes = resp.serialize().expect("serialize response");
        assert!(bytes.starts_with(NETWORK_MAGIC));
        assert_eq!(bytes[10], NetworkFrameKind::Response as u8);
        assert_eq!(bytes[11], RaftRequestType::Vote.to_code());

        let decoded = RaftResponse::deserialize(&bytes).expect("deserialize response");
        match decoded {
            RaftResponse::Vote(v) => {
                assert_eq!(v.vote, Vote::new(9, 2));
                assert!(v.vote_granted);
            }
            other => panic!("unexpected response type: {:?}", other),
        }
    }

    #[test]
    fn test_request_deserialize_rejects_response_frame_kind() {
        let resp = RaftResponse::Vote(VoteResponse::new(Vote::new(1, 1), None, true));
        let bytes = resp.serialize().expect("serialize response");

        let err = RaftRequest::deserialize(&bytes).expect_err("must reject response frame");
        assert!(matches!(err, KLogError::InvalidFormat(_)));
        let msg = err.to_string();
        assert!(msg.contains("Unexpected network frame kind"));
    }

    #[test]
    fn test_request_deserialize_rejects_header_payload_rpc_mismatch() {
        let req = RaftRequest::Vote(VoteRequest::new(Vote::new(11, 5), None));
        let mut bytes = req.serialize().expect("serialize request");
        bytes[11] = RaftRequestType::AppendEntries.to_code();

        let err = RaftRequest::deserialize(&bytes).expect_err("must reject rpc mismatch");
        assert!(matches!(err, KLogError::InvalidFormat(_)));
        let msg = err.to_string();
        assert!(msg.contains("rpc type mismatch"));
    }

    #[test]
    fn test_admin_request_paths() {
        assert_eq!(
            KLogAdminRequestType::AddLearner.klog_path(),
            "/klog/admin/add-learner"
        );
        assert_eq!(
            KLogAdminRequestType::ChangeMembership.klog_path(),
            "/klog/admin/change-membership"
        );
        assert_eq!(
            KLogAdminRequestType::RemoveLearner.klog_path(),
            "/klog/admin/remove-learner"
        );
        assert_eq!(
            KLogAdminRequestType::ClusterState.klog_path(),
            "/klog/admin/cluster-state"
        );
    }

    #[test]
    fn test_data_request_paths() {
        assert_eq!(KLogDataRequestType::Append.klog_path(), "/klog/data/append");
        assert_eq!(KLogDataRequestType::Query.klog_path(), "/klog/data/query");
        assert_eq!(
            KLogDataRequestType::MetaPut.klog_path(),
            "/klog/data/meta-put"
        );
        assert_eq!(
            KLogDataRequestType::MetaDelete.klog_path(),
            "/klog/data/meta-delete"
        );
        assert_eq!(
            KLogDataRequestType::MetaQuery.klog_path(),
            "/klog/data/meta-query"
        );
    }
}
