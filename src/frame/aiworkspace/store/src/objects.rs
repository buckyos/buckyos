//! Content-addressed object and chunk storage with CYFS identities.
//!
//! Layout: `objects/<hex>.<obj_type>` (canonical NamedObject text) and
//! `chunks/<hex>.mix256` (chunk bytes). Every write verifies that the id
//! matches the content; every asset read re-verifies it.
//!
//! This is the storage boundary the named store plugs into (see the service
//! README): ids, encodings and file names are those of ndn-lib.

use aiworkspace_core::canonical::{
    self, chunk_id, chunk_len, file_object, obj_id_from_filename, obj_id_to_filename, parse_strict, ObjId, MAX_CHUNK_SIZE,
};
use aiworkspace_core::materialize::{ObjectSink, ObjectSource};
use aiworkspace_core::{Code, WsError, WsResult};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct FsObjectStore {
    root: PathBuf,
}

fn io(e: std::io::Error) -> WsError {
    let code = if e.raw_os_error() == Some(28) { Code::StorageFull } else { Code::StorageIoError };
    WsError::new(code, format!("object store: {e}"))
}

/// Write-then-rename so a crash never leaves a half-written object under its id.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> WsResult<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", rand::random::<u64>()));
    let mut f = fs::File::create(&tmp).map_err(io)?;
    f.write_all(bytes).map_err(io)?;
    f.sync_all().map_err(io)?;
    fs::rename(&tmp, path).map_err(io)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Available,
    Missing,
    Corrupt,
}

impl Availability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Availability::Available => "available",
            Availability::Missing => "missing",
            Availability::Corrupt => "corrupt",
        }
    }
}

impl FsObjectStore {
    pub fn open(root: &Path) -> WsResult<FsObjectStore> {
        fs::create_dir_all(root.join("objects")).map_err(io)?;
        fs::create_dir_all(root.join("chunks")).map_err(io)?;
        Ok(FsObjectStore { root: root.to_path_buf() })
    }

    pub fn object_path(&self, id: &str) -> WsResult<PathBuf> {
        let name = obj_id_to_filename(id).filter(|_| canonical::is_obj_id(id)).ok_or_else(|| WsError::invalid_schema(format!("bad object id {id}")))?;
        Ok(self.root.join("objects").join(name))
    }

    pub fn chunk_path(&self, id: &str) -> WsResult<PathBuf> {
        let name = obj_id_to_filename(id).filter(|_| chunk_len(id).is_some()).ok_or_else(|| WsError::invalid_schema(format!("bad chunk id {id}")))?;
        Ok(self.root.join("chunks").join(name))
    }

    pub fn has_object(&self, id: &str) -> bool {
        self.object_path(id).map(|p| p.exists()).unwrap_or(false)
    }

    /// Store canonical text under `id`, refusing content that does not hash to it.
    pub fn put_verified(&self, id: &str, text: &str) -> WsResult<()> {
        canonical::verify_named_object(id, text)?;
        let path = self.object_path(id)?;
        if !path.exists() {
            write_atomic(&path, text.as_bytes())?;
        }
        Ok(())
    }

    pub fn put_chunk(&self, data: &[u8]) -> WsResult<ObjId> {
        if data.len() as u64 > MAX_CHUNK_SIZE {
            return Err(WsError::limit("files above 32 MiB need a chunk list, which this phase does not write"));
        }
        let id = chunk_id(data);
        let path = self.chunk_path(&id)?;
        if !path.exists() {
            write_atomic(&path, data)?;
        }
        Ok(id)
    }

    /// Store a chunk received under a claimed id (package import).
    pub fn put_chunk_verified(&self, id: &str, data: &[u8]) -> WsResult<()> {
        if chunk_id(data) != id {
            return Err(WsError::invalid_schema(format!("chunk {id} content hash mismatch")));
        }
        self.put_chunk(data).map(|_| ())
    }

    pub fn get_chunk(&self, id: &str) -> WsResult<Vec<u8>> {
        let data = fs::read(self.chunk_path(id)?).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                WsError::new(Code::DependencyUnavailable, format!("chunk {id} is missing"))
            } else {
                io(e)
            }
        })?;
        if chunk_id(&data) != id {
            return Err(WsError::new(Code::DependencyUnavailable, format!("chunk {id} is corrupt")));
        }
        Ok(data)
    }

    /// The chunk id a FileObject points at.
    pub fn file_chunk(&self, file_object_id: &str) -> WsResult<ObjId> {
        let v = parse_strict(&self.get_object(file_object_id)?)?;
        v["content"].as_str().map(str::to_string).ok_or_else(|| WsError::invalid_schema("file object without content"))
    }

    /// Read-time diagnosis of an asset; it is not a persisted field.
    pub fn availability(&self, file_object_id: &str) -> Availability {
        let chunk = match self.file_chunk(file_object_id) {
            Ok(c) => c,
            Err(e) if e.code == Code::DependencyUnavailable => return Availability::Missing,
            Err(_) => return Availability::Corrupt,
        };
        match self.get_chunk(&chunk) {
            Ok(_) => Availability::Available,
            Err(e) if e.detail.contains("missing") => Availability::Missing,
            Err(_) => Availability::Corrupt,
        }
    }

    pub fn list_objects(&self) -> WsResult<Vec<ObjId>> {
        let mut out = Vec::new();
        for dir in ["objects", "chunks"] {
            for entry in fs::read_dir(self.root.join(dir)).map_err(io)? {
                let name = entry.map_err(io)?.file_name().to_string_lossy().to_string();
                if let Some(id) = obj_id_from_filename(&name) {
                    out.push(id);
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

impl ObjectSource for FsObjectStore {
    fn get_object(&self, id: &str) -> WsResult<String> {
        let text = fs::read_to_string(self.object_path(id)?).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                WsError::new(Code::DependencyUnavailable, format!("object {id} is missing"))
            } else {
                io(e)
            }
        })?;
        canonical::verify_named_object(id, &text)?;
        Ok(text)
    }
    fn get_file(&self, file_object_id: &str) -> WsResult<Vec<u8>> {
        self.get_chunk(&self.file_chunk(file_object_id)?)
    }
}

/// A sink writing into the store and remembering what it wrote (the closure of a snapshot).
pub struct StoreSink<'a> {
    pub store: &'a FsObjectStore,
    pub written: Vec<ObjId>,
    pub chunks: Vec<ObjId>,
}

impl<'a> StoreSink<'a> {
    pub fn new(store: &'a FsObjectStore) -> Self {
        StoreSink { store, written: Vec::new(), chunks: Vec::new() }
    }
}

impl ObjectSink for StoreSink<'_> {
    fn put_object(&mut self, obj_type: &str, canonical_text: &str) -> WsResult<ObjId> {
        let id = canonical::build_obj_id(obj_type, canonical_text);
        let path = self.store.object_path(&id)?;
        if !path.exists() {
            write_atomic(&path, canonical_text.as_bytes())?;
        }
        self.written.push(id.clone());
        Ok(id)
    }
    fn put_file(&mut self, bytes: &[u8]) -> WsResult<ObjId> {
        let chunk = self.store.put_chunk(bytes)?;
        let (id, text) = file_object(bytes.len() as u64, &chunk)?;
        let stored = self.put_object(canonical::OBJ_TYPE_FILE, &text)?;
        debug_assert_eq!(id, stored);
        self.chunks.push(chunk);
        Ok(stored)
    }
}

/// Media type from magic bytes — never from what the client claims.
pub fn sniff_media_type(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'%', b'P', b'D', b'F', ..] => "application/pdf",
        [b'P', b'K', 3, 4, ..] => "application/zip",
        _ if std::str::from_utf8(data).is_ok() => {
            if data.iter().take(256).any(|b| *b == b'<') && String::from_utf8_lossy(&data[..data.len().min(256)]).contains("<svg") {
                "image/svg+xml"
            } else {
                "text/plain"
            }
        }
        _ => "application/octet-stream",
    }
}
