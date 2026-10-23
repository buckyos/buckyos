use super::{allowed, denied, hold_checked, may_read, ndm_error, principal, store, RES_UPLOADS};
use crate::state::SharedState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ndn_lib::{ChunkId, ChunkType, ObjId};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;

#[derive(Deserialize)]
struct Object {
    obj_id: String,
    obj_data: String,
}

#[derive(Deserialize)]
struct QcidLink {
    qcid: String,
    content_id: String,
    size: u64,
}

#[derive(Deserialize)]
struct Manifest {
    objects: Vec<Object>,
    roots: Vec<String>,
    #[serde(default)]
    qcid_links: Vec<QcidLink>,
}

pub async fn finalize(State(state): State<SharedState>, req: Request) -> Response {
    let p = match principal(&state, req.headers()).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !allowed(&state, &p, RES_UPLOADS, "write").await {
        return denied("import objects");
    }
    let bytes = match axum::body::to_bytes(req.into_body(), 8 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return ndm_error(StatusCode::BAD_REQUEST, "invalid_data", e),
    };
    let manifest: Manifest = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(e) => return ndm_error(StatusCode::BAD_REQUEST, "invalid_data", e),
    };
    if manifest.roots.is_empty() || manifest.objects.len() > 10000 || manifest.roots.len() > 10000 {
        return ndm_error(StatusCode::BAD_REQUEST, "invalid_data", "a bounded, nonempty list of roots is required");
    }
    let store = match store(&state).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let mut pending = Vec::new();
    let mut seen = HashSet::new();
    for object in manifest.objects {
        let id = match ObjId::new(&object.obj_id) {
            Ok(id) if id.is_file_object() || id.is_dir_object() || id.is_chunk_list() => id,
            _ => return ndm_error(StatusCode::BAD_REQUEST, "invalid_id", "only files, directories and chunk lists can be imported"),
        };
        let children = match super::forward::verified_children(&id, &object.obj_data) {
            Ok(c) => c,
            Err(e) => return ndm_error(StatusCode::BAD_REQUEST, "invalid_data", e),
        };
        if seen.insert(id.to_string()) {
            pending.push((id, object.obj_data, children));
        }
    }
    while !pending.is_empty() {
        let before = pending.len();
        let mut next = Vec::new();
        for (id, body, children) in pending {
            let mut missing = false;
            for child in &children {
                if !may_read(&state, &p, &child.to_string()).await || !store.is_object_stored(child, None).await.unwrap_or(false) {
                    missing = true;
                    break;
                }
            }
            if missing {
                next.push((id, body, children));
                continue;
            }
            if let Err(e) = store.put_object(&id, &body).await {
                return ndm_error(StatusCode::INTERNAL_SERVER_ERROR, "registration_failed", e);
            }
            if !store.is_object_stored(&id, None).await.unwrap_or(false) {
                return ndm_error(StatusCode::CONFLICT, "incomplete_object", id);
            }
            if let Err(r) = hold_checked(&state, store, &p, &id, "import").await { return r; }
        }
        if next.len() == before {
            return ndm_error(StatusCode::CONFLICT, "missing_dependency", "object dependencies are incomplete or are not authorized for this session");
        }
        pending = next;
    }
    let mut roots = Vec::new();
    for root in manifest.roots {
        let id = match ObjId::new(&root) {
            Ok(id) if id.is_file_object() || id.is_dir_object() => id,
            _ => return ndm_error(StatusCode::BAD_REQUEST, "invalid_id", "an import root must be a file or directory"),
        };
        if !may_read(&state, &p, &id.to_string()).await {
            return denied("use this import root");
        }
        if !store.is_object_stored(&id, None).await.unwrap_or(false) {
            return ndm_error(StatusCode::CONFLICT, "incomplete_object", id);
        }
        if let Err(r) = hold_checked(&state, store, &p, &id, "import").await { return r; }
        roots.push(json!({"obj_id": id.to_string(), "ready": true}));
    }
    let mut warnings = Vec::new();
    for link in manifest.qcid_links {
        let (Ok(qcid), Ok(content)) = (ChunkId::new(&link.qcid), ObjId::new(&link.content_id)) else {
            warnings.push(format!("invalid QCID mapping: {}", link.qcid));
            continue;
        };
        if qcid.chunk_type != ChunkType::QCID || !(content.is_chunk() || content.is_chunk_list()) || !may_read(&state, &p, &content.to_string()).await || !store.is_object_stored(&content, None).await.unwrap_or(false) {
            warnings.push(format!("QCID content is not complete or authorized: {}", link.content_id));
            continue;
        }
        if let Err(e) = verify_qcid(store, &qcid, &content, link.size).await {
            warnings.push(format!("QCID verification failed: {e}"));
            continue;
        }
        if content.is_chunk_list() {
            if let Err(e) = store.add_chunk_by_same_as(&qcid, link.size, &content).await {
                warnings.push(format!("QCID registration failed: {e}"));
                continue;
            }
        }
        if let Err(e) = state.db.lock().execute(
            "INSERT INTO ndm_import_qcids(principal,qcid,content_id,size) VALUES (?1,?2,?3,?4) ON CONFLICT(principal,qcid) DO UPDATE SET content_id=excluded.content_id,size=excluded.size",
            rusqlite::params![p.key(), qcid.to_string(), content.to_string(), link.size],
        ) {
            warnings.push(format!("QCID index failed: {e}"));
        }
    }
    Json(json!({"ready": true, "roots": roots, "warnings": warnings})).into_response()
}

async fn verify_qcid(store: &named_store::NamedDataMgr, qcid: &ChunkId, content: &ObjId, size: u64) -> Result<(), String> {
    let piece = ndn_lib::QCID_HASH_PIECE_SIZE;
    let samples = if size < ndn_lib::QCID_SAMPLE_THRESHOLD { vec![(0, size)] } else { vec![(0, piece), ((size - piece) / 2, piece), (size - piece, piece)] };
    let mut hash = Sha256::new();
    for (offset, length) in samples {
        let (mut reader, actual_size) = if content.is_chunk_list() {
            store.open_chunklist_reader(content, offset).await
        } else {
            store.open_chunk_reader(&ChunkId::from_obj_id(content), offset).await
        }.map_err(|e| e.to_string())?;
        if actual_size != size { return Err("content size does not match".into()); }
        let mut bytes = vec![0; length as usize];
        reader.read_exact(&mut bytes).await.map_err(|e| e.to_string())?;
        hash.update(bytes);
    }
    let expected = ChunkId::from_mix_hash_result(size, &hash.finalize(), ChunkType::QCID);
    if expected != *qcid { return Err("quick hash does not match content".into()); }
    Ok(())
}
