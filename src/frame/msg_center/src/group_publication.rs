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
            let base = format!("resolver/cache/{}/group", did.replace('%', "%25").replace('/', "%2F"));
            let mut state = json!({"document_status":status,"authority_seq":generation,"updated_by":"msg-center","updated_at":Self::now_ms()/1000});
            let mut actions = HashMap::new();
            if let Some(doc) = doc {
                let parsed: Value = serde_json::from_str(&doc).map_err(db_error)?;
                state["document_version"] = parsed["iat"].clone();
                state["effective_owner"] = parsed["controller"].clone();
                actions.insert(format!("{base}/doc"), KVAction::Update(doc));
            } else {
                actions.insert(format!("{base}/doc"), KVAction::Remove);
            }
            actions.insert(format!("{base}/state"), KVAction::Update(state.to_string()));
            tokio::time::timeout(std::time::Duration::from_secs(10), client.exec_tx(actions, None)).await.map_err(db_error)?.map_err(db_error)?;
            let sql = self.msg_box_db.render_sql("UPDATE group_doc_publications SET published_generation=? WHERE group_did=? AND generation=?");
            sqlx::query(&sql).bind(generation).bind(&did).bind(generation).execute(self.msg_box_db.pool()).await.map_err(db_error)?;
            if let Some(names) = name_client::get_name_client() {names.invalidate_did_cache(name_lib::DID::from_str(&did).map_err(invalid)?, Some(name_lib::DidDocType::custom("group")));}
        }
        Ok(())
    }
}
