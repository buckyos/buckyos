use super::store::{KLogStateSnapshot, KLogStateStore};
use crate::{KLogEntry, KLogError, KResult};
use rocksdb::{DB, IteratorMode, Options, WriteBatch};
use std::path::Path;
use std::sync::Arc;

const KEY_PREFIX_ENTRY: u8 = b'e';

fn entry_key(id: u64) -> [u8; 9] {
    let mut key = [0u8; 9];
    key[0] = KEY_PREFIX_ENTRY;
    key[1..].copy_from_slice(&id.to_be_bytes());
    key
}

fn decode_entry_key(key: &[u8]) -> Option<u64> {
    if key.len() != 9 || key[0] != KEY_PREFIX_ENTRY {
        return None;
    }

    let mut id = [0u8; 8];
    id.copy_from_slice(&key[1..]);
    Some(u64::from_be_bytes(id))
}

/// RocksDB-backed state store for high-write kernel logs.
#[derive(Clone)]
pub struct RocksDbStateStore {
    db: Arc<DB>,
}

impl std::fmt::Debug for RocksDbStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocksDbStateStore").finish()
    }
}

impl RocksDbStateStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create rocksdb parent dir {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_atomic_flush(true);

        let db = DB::open(&opts, path.as_ref()).map_err(|e| {
            format!(
                "Failed to open rocksdb at {}: {}",
                path.as_ref().display(),
                e
            )
        })?;

        Ok(Self { db: Arc::new(db) })
    }

    fn read_all_entries(&self) -> KResult<Vec<KLogEntry>> {
        let mut entries = Vec::new();
        for item in self.db.iterator(IteratorMode::Start) {
            let (k, v) = item.map_err(|e| {
                let msg = format!("Failed to iterate rocksdb entry: {}", e);
                KLogError::InvalidFormat(msg)
            })?;

            if decode_entry_key(&k).is_none() {
                continue;
            }

            let (entry, _): (KLogEntry, usize) =
                bincode::serde::decode_from_slice(v.as_ref(), bincode::config::legacy()).map_err(
                    |e| {
                        let msg = format!("Failed to deserialize state entry from rocksdb: {}", e);
                        KLogError::InvalidFormat(msg)
                    },
                )?;
            entries.push(entry);
        }

        Ok(entries)
    }
}

#[async_trait::async_trait]
impl KLogStateStore for RocksDbStateStore {
    async fn append(&self, entries: Vec<KLogEntry>) -> KResult<()> {
        let mut batch = WriteBatch::default();
        for entry in entries {
            let key = entry_key(entry.id);
            let value =
                bincode::serde::encode_to_vec(&entry, bincode::config::legacy()).map_err(|e| {
                    let msg = format!("Failed to serialize state entry for rocksdb: {}", e);
                    KLogError::InvalidFormat(msg)
                })?;
            batch.put(key, value);
        }

        self.db.write(batch).map_err(|e| {
            let msg = format!("Failed to write state entries to rocksdb: {}", e);
            KLogError::InvalidFormat(msg)
        })?;
        Ok(())
    }

    async fn build_snapshot(&self) -> KResult<KLogStateSnapshot> {
        let entries = self.read_all_entries()?;
        let data =
            bincode::serde::encode_to_vec(&entries, bincode::config::legacy()).map_err(|e| {
                let msg = format!("Failed to serialize rocksdb state snapshot: {}", e);
                KLogError::InvalidFormat(msg)
            })?;
        Ok(KLogStateSnapshot { data })
    }

    async fn install_snapshot(&self, snapshot: KLogStateSnapshot) -> KResult<()> {
        let (entries, _): (Vec<KLogEntry>, usize) =
            bincode::serde::decode_from_slice(&snapshot.data, bincode::config::legacy()).map_err(
                |e| {
                    let msg = format!("Failed to decode state snapshot for rocksdb: {}", e);
                    KLogError::InvalidFormat(msg)
                },
            )?;

        let mut batch = WriteBatch::default();
        for item in self.db.iterator(IteratorMode::Start) {
            let (k, _) = item.map_err(|e| {
                let msg = format!("Failed to iterate rocksdb during snapshot install: {}", e);
                KLogError::InvalidFormat(msg)
            })?;
            if decode_entry_key(&k).is_some() {
                batch.delete(k);
            }
        }

        for entry in entries {
            let key = entry_key(entry.id);
            let value =
                bincode::serde::encode_to_vec(&entry, bincode::config::legacy()).map_err(|e| {
                    let msg = format!("Failed to serialize state entry for install: {}", e);
                    KLogError::InvalidFormat(msg)
                })?;
            batch.put(key, value);
        }

        self.db.write(batch).map_err(|e| {
            let msg = format!("Failed to install state snapshot into rocksdb: {}", e);
            KLogError::InvalidFormat(msg)
        })?;
        Ok(())
    }
}
