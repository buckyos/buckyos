use serde::Serialize;
use serde::de::DeserializeOwned;

pub(crate) const PERSIST_MAGIC: &[u8; 8] = b"KLOGFMT1";
const PERSIST_VERSION_V1: u16 = 1;
const CODEC_BINCODE_LEGACY: u8 = 1;
const HEADER_LEN: usize = 8 + 2 + 2 + 1 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub(crate) enum PersistPayloadType {
    SnapshotData = 1,
    SqliteLogEntry = 10,
    SqliteVote = 11,
    SqliteCommittedLogId = 12,
    SqliteLastPurgedLogId = 13,
}

impl PersistPayloadType {
    fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::SnapshotData),
            10 => Some(Self::SqliteLogEntry),
            11 => Some(Self::SqliteVote),
            12 => Some(Self::SqliteCommittedLogId),
            13 => Some(Self::SqliteLastPurgedLogId),
            _ => None,
        }
    }
}

impl std::fmt::Display for PersistPayloadType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::SnapshotData => "snapshot_data",
            Self::SqliteLogEntry => "sqlite_log_entry",
            Self::SqliteVote => "sqlite_vote",
            Self::SqliteCommittedLogId => "sqlite_committed_log_id",
            Self::SqliteLastPurgedLogId => "sqlite_last_purged_log_id",
        };
        write!(f, "{}", name)
    }
}

pub(crate) fn encode_with_header<T: Serialize>(
    payload_type: PersistPayloadType,
    value: &T,
) -> Result<Vec<u8>, String> {
    let payload = bincode::serde::encode_to_vec(value, bincode::config::legacy())
        .map_err(|e| format!("Failed to encode payload {}: {}", payload_type, e))?;

    let payload_len = u64::try_from(payload.len())
        .map_err(|_| format!("Payload too large for {}: {}", payload_type, payload.len()))?;

    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(PERSIST_MAGIC);
    out.extend_from_slice(&PERSIST_VERSION_V1.to_be_bytes());
    out.extend_from_slice(&(payload_type as u16).to_be_bytes());
    out.push(CODEC_BINCODE_LEGACY);
    out.extend_from_slice(&payload_len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

pub(crate) fn decode_with_header<T: DeserializeOwned>(
    expected_type: PersistPayloadType,
    data: &[u8],
) -> Result<T, String> {
    if data.len() < HEADER_LEN {
        return Err(format!(
            "Payload too short for header: bytes={}, expected_at_least={}",
            data.len(),
            HEADER_LEN
        ));
    }

    let magic = &data[0..8];
    if magic != PERSIST_MAGIC {
        return Err(format!("Invalid payload magic for {}", expected_type));
    }

    let version = u16::from_be_bytes([data[8], data[9]]);
    if version != PERSIST_VERSION_V1 {
        return Err(format!(
            "Unsupported payload version for {}: {}",
            expected_type, version
        ));
    }

    let payload_type_raw = u16::from_be_bytes([data[10], data[11]]);
    let payload_type = PersistPayloadType::from_u16(payload_type_raw)
        .ok_or_else(|| format!("Unknown payload type code: {}", payload_type_raw))?;
    if payload_type != expected_type {
        return Err(format!(
            "Unexpected payload type: expected={}, actual={}",
            expected_type, payload_type
        ));
    }

    let codec = data[12];
    if codec != CODEC_BINCODE_LEGACY {
        return Err(format!(
            "Unsupported payload codec for {}: {}",
            expected_type, codec
        ));
    }

    let payload_len = u64::from_be_bytes([
        data[13], data[14], data[15], data[16], data[17], data[18], data[19], data[20],
    ]);
    let payload_len = usize::try_from(payload_len)
        .map_err(|_| format!("Payload length overflow for {}", expected_type))?;
    if data.len() != HEADER_LEN + payload_len {
        return Err(format!(
            "Payload length mismatch for {}: header={}, actual={}",
            expected_type,
            payload_len,
            data.len().saturating_sub(HEADER_LEN)
        ));
    }

    let payload = &data[HEADER_LEN..];
    let (decoded, _): (T, usize) =
        bincode::serde::decode_from_slice(payload, bincode::config::legacy())
            .map_err(|e| format!("Failed to decode payload {}: {}", expected_type, e))?;
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct SampleValue {
        id: u64,
        name: String,
    }

    #[test]
    fn test_encode_decode_with_header_roundtrip() {
        let input = SampleValue {
            id: 7,
            name: "klog".to_string(),
        };
        let encoded = encode_with_header(PersistPayloadType::SnapshotData, &input).unwrap();
        assert!(encoded.starts_with(PERSIST_MAGIC));

        let decoded: SampleValue =
            decode_with_header(PersistPayloadType::SnapshotData, &encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_decode_with_header_rejects_unexpected_type() {
        let input = SampleValue {
            id: 9,
            name: "vote".to_string(),
        };
        let encoded = encode_with_header(PersistPayloadType::SqliteVote, &input).unwrap();
        let err = decode_with_header::<SampleValue>(PersistPayloadType::SnapshotData, &encoded)
            .expect_err("should reject payload type mismatch");
        assert!(err.contains("Unexpected payload type"));
    }
}
