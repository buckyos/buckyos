use crate::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const NFS_COPY_SCHEMA_ID: &str = "nfs.copy/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NfsCopyChoice {
    Ask,
    KeepBoth,
    Skip,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NfsCopySource {
    pub source_ref: Value,
    pub name: String,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NfsCopyInput {
    pub sources: Vec<NfsCopySource>,
    pub destination_ref: Value,
    pub conflict: NfsCopyChoice,
    #[serde(default)]
    pub retry_of: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NfsCopySummary {
    pub success: u64,
    pub failed: u64,
    pub skipped: u64,
    pub cancelled: u64,
    pub pending: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NfsCopyItemResult {
    pub id: i64,
    pub source_index: usize,
    pub source_path: String,
    pub target_path: String,
    pub kind: String,
    pub size: Option<u64>,
    pub mtime: Option<f64>,
    pub status: String,
    pub error: Option<Value>,
    pub identity: Option<Value>,
}

pub fn nfs_copy_task_schema() -> TaskSchemaDefinition {
    TaskSchemaDefinition {
        schema_id: NFS_COPY_SCHEMA_ID.into(),
        schema_version: 1,
        input_schema: json!({
            "type": "object", "additionalProperties": false,
            "required": ["sources", "destination_ref", "conflict"],
            "properties": {
                "sources": {"type": "array", "minItems": 1, "maxItems": 256,
                    "items": {"type": "object", "required": ["source_ref", "name", "source_path"],
                        "properties": {"source_ref": {"type": "object"}, "source_path": {"type": "string"}, "name": {"type": "string", "minLength": 1}}}},
                "destination_ref": {"type": "object"},
                "conflict": {"enum": ["ask", "keep-both", "skip", "cancel"]},
                "retry_of": {"type": ["string", "null"]}
            }
        }),
        output_schema: json!({"type": "object", "required": ["summary"], "properties": {"summary": {"type": "object"}}}),
        presentation_schema: None,
        allowed_executor_kinds: vec![TaskExecutorKind::App],
        user_creatable: false,
        default_storage_domain: StorageDomain::System,
        publisher_app_id: NFS_SERVER_SERVICE_NAME.into(),
        enabled: true,
        created_at: 0,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NfsCopyTestConfig {
    pub task_mgr_url: String,
    pub service_token: String,
    pub public_key: String,
}

impl NfsCopyTestConfig {
    pub fn task_client(&self) -> TaskManagerClient {
        TaskManagerClient::new(::kRPC::kRPC::new(
            &self.task_mgr_url,
            Some(self.service_token.clone()),
        ))
    }
}

pub async fn authenticate_nfs_copy_user(
    token: &str,
    test: Option<&NfsCopyTestConfig>,
) -> ::kRPC::Result<ActorRef> {
    let parsed = if let Some(test) = test {
        let key = jsonwebtoken::DecodingKey::from_ed_components(&test.public_key)
            .map_err(|e| ::kRPC::RPCErrors::InvalidToken(e.to_string()))?;
        let mut parsed = ::kRPC::RPCSessionToken::from_string(token)?;
        parsed.verify_by_key(&key)?;
        parsed
    } else {
        get_buckyos_api_runtime()?
            .verify_trusted_session_token(token)
            .await?
    };
    let claims = validate_verify_hub_token_claims(&parsed, TokenUse::Session)?;
    if claims.principal_kind != TokenPrincipalKind::User {
        return Err(::kRPC::RPCErrors::NoPermission(
            "copy requires a user session".into(),
        ));
    }
    let user = parsed
        .sub
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ::kRPC::RPCErrors::InvalidToken("missing subject".into()))?;
    Ok(ActorRef::new(user, claims.target.canonical_key()))
}
