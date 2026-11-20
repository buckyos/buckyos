use super::*;
use named_store::{NamedDataMgr, NamedLocalConfig, NamedStore, StoreLayout, StoreTarget};
use ndn_lib::ObjId;
use rusqlite::Connection;
use serde_json::Value;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

#[derive(Default)]
pub(super) struct TextEditorData {
    pub(super) records: Vec<(String, String, String)>,
    pub(super) stores: Vec<(String, PathBuf, PathBuf)>,
    pub(super) epoch: u64,
    pub(super) targets: Vec<StoreTarget>,
}
impl TextEditorData {
    pub(super) fn discover(&mut self, db: &Connection) -> rusqlite::Result<()> {
        let mut stmt = db.prepare(
            "SELECT b.name, r.key, r.obj_id FROM buckets b JOIN bindings r USING(bucket_id)
             WHERE b.appid='text-editor.buckyos.bns.did' AND r.is_delete_marker=0
               AND ((b.name='buffers' AND r.key LIKE '%.buf')
                 OR (b.name='recovery' AND r.key LIKE '%/entry.json'))
             ORDER BY b.bucket_id,r.key,r.version",
        )?;
        for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
            self.records.push(row?);
        }
        Ok(())
    }
}

/// Resolve content through the same store layout as the running app. Object and
/// chunk bytes stay in place. All access-time and other metadata writes are private.
pub(super) fn text_editor(ctx: &mut Context) -> Result<(), Failure> {
    if ctx.editor.records.is_empty() {
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| ctx.error("objects", e))?;
    runtime.block_on(async {
        let ndm = NamedDataMgr::new();
        for (id, dir, db) in &ctx.editor.stores {
            let cfg = NamedLocalConfig {
                db_path: Some(db.clone()),
                chunk_dir: None,
                read_only: true,
            };
            let store = NamedStore::from_config(Some(id.clone()), dir.clone(), cfg)
                .await
                .map_err(|e| ctx.error(dir.display(), e))?;
            ndm.register_store(Arc::new(tokio::sync::Mutex::new(store)))
                .await;
        }
        if ctx.editor.stores.is_empty() {
            return Err(ctx.error(
                "Text Editor buckets",
                "no available local Named Store for existing records",
            ));
        }
        ndm.add_layout(StoreLayout::new(
            ctx.editor.epoch,
            ctx.editor.targets.clone(),
            0,
            0,
        ))
        .await;
        for (bucket, key, id) in &ctx.editor.records {
            let area = format!("{bucket}/{key}");
            let id = ObjId::new(id).map_err(|e| ctx.unsupported(&area, e))?;
            let file = bucket_store::content::load_file(&ndm, &id)
                .await
                .map_err(|e| ctx.error(&area, e))?;
            // BufferStore counts UTF-16 code units in the header. RecoveryStore
            // reads at most 65,536 bytes of JSON. Neither needs a user's document.
            const LIMIT: u64 = 65536;
            let content = ObjId::new(&file.content).map_err(|e| ctx.unsupported(&area, e))?;
            let (reader, _) = ndm
                .open_reader(&content, None)
                .await
                .map_err(|e| ctx.error(&area, e))?;
            let mut bytes = Vec::new();
            let limit = if bucket == "buffers" {
                4 * LIMIT + 1
            } else {
                LIMIT
            };
            reader
                .take(limit)
                .read_to_end(&mut bytes)
                .await
                .map_err(|e| ctx.error(&area, e))?;
            let header = if bucket == "buffers" {
                let end = bytes
                    .iter()
                    .position(|&c| c == b'\n')
                    .ok_or_else(|| ctx.unsupported(&area, "missing buffer header terminator"))?;
                let text =
                    std::str::from_utf8(&bytes[..end]).map_err(|e| ctx.unsupported(&area, e))?;
                if text.encode_utf16().count() > LIMIT as usize {
                    return Err(ctx.unsupported(&area, "buffer header exceeds the supported size"));
                }
                &bytes[..end]
            } else {
                if bytes.len() as u64 != file.size {
                    return Err(ctx.unsupported(&area, "incomplete recovery metadata"));
                }
                bytes.as_slice()
            };
            let v: Value = serde_json::from_slice(header).map_err(|e| ctx.unsupported(&area, e))?;
            let valid = if bucket == "buffers" {
                v["format"] == "buckyos.text-editor.buffer/1"
                    && v["bufferId"].as_str().is_some_and(|s| !s.is_empty())
                    && v["updatedAt"].is_number()
                    && matches!(v["eol"].as_str(), Some("\r" | "\n" | "\r\n"))
            } else {
                v["format"] == "buckyos.text-editor.recovery/1"
                    && v["fileName"].is_string()
                    && v["createdAt"].is_number()
            };
            if !valid {
                return Err(
                    ctx.unsupported(&area, format!("unknown Text Editor {bucket} record schema"))
                );
            }
        }
        Ok(())
    })
}
