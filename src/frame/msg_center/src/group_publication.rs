use crate::group_store::db_error;
use crate::group_types::*;
use crate::msg_center::MessageCenter;
use buckyos_api::SystemConfigClient;
use buckyos_kit::KVAction;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;

impl MessageCenter {
    pub(crate) async fn publish_group_documents(&self) -> Result<()> {
        let Ok(runtime) = buckyos_api::get_buckyos_api_runtime() else {
            return Ok(());
        };
        let client = runtime.get_system_config_client().await?;
        self.publish_group_documents_with(client.as_ref()).await
    }
    pub(crate) async fn publish_group_documents_with(
        &self,
        client: &SystemConfigClient,
    ) -> Result<()> {
        let _guard = self.groups.sync_lock.lock().await;
        for row in sqlx::query("SELECT group_did,generation,document_status,document_json FROM group_doc_publications WHERE published_generation<generation ORDER BY group_did LIMIT 100").fetch_all(self.msg_box_db.pool()).await.map_err(db_error)? {
            let did: String = row.try_get("group_did").map_err(db_error)?;
            let generation: i64 = row.try_get("generation").map_err(db_error)?;
            let status: String = row.try_get("document_status").map_err(db_error)?;
            let doc: Option<String> = row.try_get("document_json").map_err(db_error)?;
            // A group publishes only its group document: the did.json slot holds it.
            let base = format!("resolver/cache/{}/did", did.replace('%', "%25").replace('/', "%2F"));
            let mut state = json!({"document_status":status,"doc_type":"group","authority_seq":generation,"updated_by":"msg-center","updated_at":Self::now_ms()/1000});
            let mut actions = HashMap::new();
            if let Some(doc) = doc {
                let mut parsed: Value = serde_json::from_str(&doc).map_err(db_error)?;
                state["document_version"] = parsed["iat"].clone();
                state["effective_owner"] = parsed["controller"].clone();
                // A child name of this Zone: the OOD signs its document (Zone custody).
                let signer = self
                    .zone_signer
                    .get()
                    .ok_or_else(|| kRPC::RPCErrors::ReasonError("zone-signer-unavailable".into()))?;
                parsed["exp"] = json!(parsed["iat"].as_u64().unwrap_or_default() + name_lib::DEFAULT_EXPIRE_TIME);
                actions.insert(format!("{base}/doc"), KVAction::Update(signer.sign(&parsed)?));
            } else {
                actions.insert(format!("{base}/doc"), KVAction::Remove);
            }
            actions.insert(format!("{base}/state"), KVAction::Update(state.to_string()));
            tokio::time::timeout(std::time::Duration::from_secs(10), client.exec_tx(actions, None)).await.map_err(db_error)?.map_err(db_error)?;
            let sql = self.msg_box_db.render_sql("UPDATE group_doc_publications SET published_generation=? WHERE group_did=? AND generation=?");
            sqlx::query(&sql).bind(generation).bind(&did).bind(generation).execute(self.msg_box_db.pool()).await.map_err(db_error)?;
            if let Some(names) = name_client::get_name_client() {
                let did = name_lib::DID::from_str(&did).map_err(invalid)?;
                for doc_type in [name_lib::DEFAULT_DID_DOC_TYPE, name_lib::DidDocType::custom("group")] {
                    let _ = names.invalidate_did_cache(&did, &doc_type).await;
                }
            }
        }
        Ok(())
    }
}
