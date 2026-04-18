/*!
 * Unified msg-box + msg-ref storage for the msg-center.
 *
 * Uses an `sqlx::AnyPool` so the backend (sqlite / postgres) is driven by the
 * zone's rdb instance config; the pool itself is `Send + Sync + Clone` and
 * manages its own locking, so an `Arc<MsgBoxDbMgr>` is safe to share without
 * an outer Rust-level lock. Every row carries an `owner` column so a single
 * database holds mailboxes for every zone user.
 */

use buckyos_api::{
    get_rdb_instance, msg_center_default_rdb_instance_config, BoxKind, DeliveryInfo, MsgRecord,
    MsgState, RdbBackend, RouteInfo, MSG_CENTER_RDB_INSTANCE_ID, MSG_CENTER_RDB_SCHEMA_POSTGRES,
    MSG_CENTER_RDB_SCHEMA_SQLITE, MSG_CENTER_SERVICE_NAME,
};
use kRPC::RPCErrors;
use log::info;
use name_lib::DID;
use ndn_lib::{MsgObject, ObjId};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};
use std::str::FromStr;
use std::sync::{Arc, Once};

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

#[derive(Clone, Debug)]
pub struct MsgBoxDbMgr {
    inner: Arc<MsgBoxDbInner>,
}

#[derive(Debug)]
struct MsgBoxDbInner {
    pool: AnyPool,
    backend: RdbBackend,
}

#[derive(Debug)]
struct MsgRecordRow {
    record_id: String,
    box_kind: String,
    msg_id: String,
    msg_kind: Option<String>,
    msg_from: Option<String>,
    msg_to: Option<String>,
    state: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    thread_key: Option<String>,
    session_id: Option<String>,
    sort_key: i64,
    tags_json: String,
    route_tunnel_did: Option<String>,
    route_json: Option<String>,
    delivery_json: Option<String>,
}

impl MsgBoxDbMgr {
    /// Open a pool against `connection`. `schema` is the DDL to apply (usually
    /// what the service spec carried for the chosen backend); an empty / None
    /// value means "use the compile-time default for `backend`".
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> std::result::Result<Self, RPCErrors> {
        ensure_any_drivers_installed();
        let mut opts = AnyPoolOptions::new().max_connections(8);
        if backend == RdbBackend::Sqlite {
            opts = opts.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON;").await?;
                    conn.execute("PRAGMA journal_mode = WAL;").await?;
                    Ok(())
                })
            });
        }
        let pool = opts.connect(connection).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "open msg-center db at {} failed: {}",
                connection, error
            ))
        })?;
        let inner = Arc::new(MsgBoxDbInner { pool, backend });
        let mgr = Self { inner };
        mgr.apply_schema(schema).await?;
        Ok(mgr)
    }

    /// Resolve the msg-center rdb instance from the service spec and open a
    /// pool against it. This is the production entry point.
    pub async fn open_from_service_spec() -> std::result::Result<Self, RPCErrors> {
        let instance =
            get_rdb_instance(MSG_CENTER_SERVICE_NAME, None, MSG_CENTER_RDB_INSTANCE_ID)
                .await
                .map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "resolve msg-center rdb instance failed: {}",
                        error
                    ))
                })?;
        info!("msg_box_db.open {}", instance.connection);
        Self::open(
            &instance.connection,
            instance.backend,
            instance.schema.as_deref(),
        )
        .await
    }

    /// Test / fallback entry: build a default instance config (sqlite) against
    /// the given connection string. The schema DDL comes from the compiled-in
    /// default for the chosen backend.
    pub async fn open_default_sqlite(connection: &str) -> std::result::Result<Self, RPCErrors> {
        let cfg = msg_center_default_rdb_instance_config();
        let schema = cfg.schema.get(&RdbBackend::Sqlite).cloned();
        Self::open(connection, RdbBackend::Sqlite, schema.as_deref()).await
    }

    pub(crate) fn pool(&self) -> &AnyPool {
        &self.inner.pool
    }

    pub(crate) fn backend(&self) -> RdbBackend {
        self.inner.backend
    }

    async fn apply_schema(&self, override_ddl: Option<&str>) -> std::result::Result<(), RPCErrors> {
        let ddl: &str =
            override_ddl
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(match self.backend() {
                    RdbBackend::Sqlite => MSG_CENTER_RDB_SCHEMA_SQLITE,
                    RdbBackend::Postgres => MSG_CENTER_RDB_SCHEMA_POSTGRES,
                });
        for statement in split_sql_statements(ddl) {
            self.pool().execute(statement.as_str()).await.map_err(|e| {
                RPCErrors::ReasonError(format!("apply msg-center schema failed: {}", e))
            })?;
        }
        Ok(())
    }

    /// Translate `?` placeholders into `$N` form for postgres.
    fn render_sql(&self, sql: &str) -> String {
        match self.backend() {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    pub async fn upsert_record(&self, record: &MsgRecord) -> std::result::Result<(), RPCErrors> {
        self.upsert_record_with_msg(record, None).await
    }

    pub async fn upsert_record_with_msg(
        &self,
        record: &MsgRecord,
        msg: Option<&MsgObject>,
    ) -> std::result::Result<(), RPCErrors> {
        let owner = owner_from_record_id(&record.record_id)?;
        self.upsert_record_with_owner(&owner, record, msg).await?;
        self.touch_message(&owner, &record.msg_id, record.created_at_ms)
            .await?;
        Ok(())
    }

    async fn upsert_record_with_owner(
        &self,
        owner: &DID,
        record: &MsgRecord,
        msg: Option<&MsgObject>,
    ) -> std::result::Result<(), RPCErrors> {
        let tags_json = serde_json::to_string(&record.tags).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode tags of msg record {}: {}",
                record.record_id, error
            ))
        })?;

        let route_tunnel_did = record
            .route
            .as_ref()
            .and_then(|route| route.tunnel_did.as_ref().map(|did| did.to_string()));
        let route_json = encode_optional_json(record.route.as_ref(), &record.record_id, "route")?;
        let delivery_json =
            encode_optional_json(record.delivery.as_ref(), &record.record_id, "delivery")?;
        let msg_from = Some(
            msg.map(|obj| obj.from.clone())
                .unwrap_or_else(|| record.from.clone())
                .to_string(),
        );
        let msg_to = Some(
            msg.and_then(|obj| obj.to.first().cloned())
                .unwrap_or_else(|| record.to.clone())
                .to_string(),
        );
        let msg_kind = msg.map(|obj| obj.kind).unwrap_or(record.msg_kind);

        let sql = self.render_sql(
            r#"
INSERT INTO msg_records (
    owner,
    record_id,
    box_kind,
    msg_id,
    msg_from,
    msg_to,
    msg_kind,
    state,
    created_at_ms,
    updated_at_ms,
    thread_key,
    session_id,
    sort_key,
    tags_json,
    route_tunnel_did,
    route_json,
    delivery_json
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(owner, record_id) DO UPDATE SET
    box_kind = excluded.box_kind,
    msg_id = excluded.msg_id,
    msg_from = COALESCE(excluded.msg_from, msg_records.msg_from),
    msg_to = COALESCE(excluded.msg_to, msg_records.msg_to),
    msg_kind = COALESCE(excluded.msg_kind, msg_records.msg_kind),
    state = excluded.state,
    created_at_ms = excluded.created_at_ms,
    updated_at_ms = excluded.updated_at_ms,
    thread_key = excluded.thread_key,
    session_id = COALESCE(excluded.session_id, msg_records.session_id),
    sort_key = excluded.sort_key,
    tags_json = excluded.tags_json,
    route_tunnel_did = excluded.route_tunnel_did,
    route_json = excluded.route_json,
    delivery_json = excluded.delivery_json
"#,
        );

        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(record.record_id.clone())
            .bind(box_kind_name(&record.box_kind).to_string())
            .bind(record.msg_id.to_string())
            .bind(msg_from)
            .bind(msg_to)
            .bind(Some(msg_obj_kind_name(&msg_kind).to_string()))
            .bind(msg_state_name(&record.state).to_string())
            .bind(to_sql_i64(record.created_at_ms))
            .bind(to_sql_i64(record.updated_at_ms))
            .bind(record.ui_session_id.clone())
            .bind(record.ui_session_id.clone())
            .bind(to_sql_i64(record.sort_key))
            .bind(tags_json)
            .bind(route_tunnel_did)
            .bind(route_json)
            .bind(delivery_json)
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to upsert msg record {}: {}",
                    record.record_id, error
                ))
            })?;
        Ok(())
    }

    pub async fn get_record(
        &self,
        owner: &DID,
        record_id: &str,
    ) -> std::result::Result<Option<MsgRecord>, RPCErrors> {
        let sql = self.render_sql(
            r#"
SELECT
    record_id,
    box_kind,
    msg_id,
    msg_kind,
    msg_from,
    msg_to,
    state,
    created_at_ms,
    updated_at_ms,
    thread_key,
    session_id,
    sort_key,
    tags_json,
    route_tunnel_did,
    route_json,
    delivery_json
FROM msg_records
WHERE owner = ? AND record_id = ?
"#,
        );

        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(record_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query msg record {}: {}",
                    record_id, error
                ))
            })?;

        match row {
            Some(row) => {
                let decoded = decode_record_row(&row, record_id)?;
                Ok(Some(row_to_record(owner, decoded)?))
            }
            None => Ok(None),
        }
    }

    pub async fn list_records(
        &self,
        owner: &DID,
        box_kind: &BoxKind,
        state_filter: Option<&[MsgState]>,
        descending: bool,
    ) -> std::result::Result<Vec<MsgRecord>, RPCErrors> {
        let order_clause = if descending {
            "ORDER BY sort_key DESC, record_id DESC"
        } else {
            "ORDER BY sort_key ASC, record_id ASC"
        };
        let sql = format!(
            r#"
SELECT
    record_id,
    box_kind,
    msg_id,
    msg_kind,
    msg_from,
    msg_to,
    state,
    created_at_ms,
    updated_at_ms,
    thread_key,
    session_id,
    sort_key,
    tags_json,
    route_tunnel_did,
    route_json,
    delivery_json
FROM msg_records
WHERE owner = ? AND box_kind = ?
{}
"#,
            order_clause
        );
        let sql = self.render_sql(&sql);

        let rows = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(box_kind_name(box_kind).to_string())
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to list msg records: {}", error))
            })?;

        let mut records = Vec::with_capacity(rows.len());
        for row in &rows {
            let decoded = decode_record_row(row, "")?;
            if !state_matches_filter(state_filter, &decoded.state) {
                continue;
            }
            records.push(row_to_record(owner, decoded)?);
        }
        Ok(records)
    }

    pub async fn touch_message(
        &self,
        owner: &DID,
        msg_id: &ObjId,
        created_at_ms: u64,
    ) -> std::result::Result<(), RPCErrors> {
        let sql = self.render_sql(
            "INSERT INTO msg_refs(owner, msg_id, created_at_ms) VALUES(?, ?, ?)
             ON CONFLICT(owner, msg_id) DO NOTHING",
        );
        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(msg_id.to_string())
            .bind(to_sql_i64(created_at_ms))
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to persist message ref {}: {}",
                    msg_id.to_string(),
                    error
                ))
            })?;
        Ok(())
    }

    pub async fn has_message(
        &self,
        owner: &DID,
        msg_id: &ObjId,
    ) -> std::result::Result<bool, RPCErrors> {
        let sql =
            self.render_sql("SELECT 1 FROM msg_refs WHERE owner = ? AND msg_id = ? LIMIT 1");
        let row = sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(msg_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query message ref {}: {}",
                    msg_id.to_string(),
                    error
                ))
            })?;
        Ok(row.is_some())
    }
}

fn decode_record_row(row: &AnyRow, fallback_id: &str) -> std::result::Result<MsgRecordRow, RPCErrors> {
    let decode = |field: &str, err: sqlx::Error| {
        RPCErrors::ReasonError(format!(
            "failed to decode msg_records.{} (record {}): {}",
            field, fallback_id, err
        ))
    };
    Ok(MsgRecordRow {
        record_id: row.try_get("record_id").map_err(|e| decode("record_id", e))?,
        box_kind: row.try_get("box_kind").map_err(|e| decode("box_kind", e))?,
        msg_id: row.try_get("msg_id").map_err(|e| decode("msg_id", e))?,
        msg_kind: row.try_get("msg_kind").map_err(|e| decode("msg_kind", e))?,
        msg_from: row.try_get("msg_from").map_err(|e| decode("msg_from", e))?,
        msg_to: row.try_get("msg_to").map_err(|e| decode("msg_to", e))?,
        state: row.try_get("state").map_err(|e| decode("state", e))?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|e| decode("created_at_ms", e))?,
        updated_at_ms: row
            .try_get("updated_at_ms")
            .map_err(|e| decode("updated_at_ms", e))?,
        thread_key: row.try_get("thread_key").map_err(|e| decode("thread_key", e))?,
        session_id: row.try_get("session_id").map_err(|e| decode("session_id", e))?,
        sort_key: row.try_get("sort_key").map_err(|e| decode("sort_key", e))?,
        tags_json: row.try_get("tags_json").map_err(|e| decode("tags_json", e))?,
        route_tunnel_did: row
            .try_get("route_tunnel_did")
            .map_err(|e| decode("route_tunnel_did", e))?,
        route_json: row.try_get("route_json").map_err(|e| decode("route_json", e))?,
        delivery_json: row
            .try_get("delivery_json")
            .map_err(|e| decode("delivery_json", e))?,
    })
}

fn row_to_record(owner: &DID, row: MsgRecordRow) -> std::result::Result<MsgRecord, RPCErrors> {
    let box_kind = box_kind_from_name(&row.box_kind)?;
    let state = msg_state_from_name(&row.state)?;
    let msg_kind = parse_msg_obj_kind(row.msg_kind.as_deref(), &box_kind);
    let msg_id = parse_obj_id(&row.msg_id, &row.record_id)?;
    let created_at_ms = from_sql_i64(row.created_at_ms, "created_at_ms", &row.record_id)?;
    let updated_at_ms = from_sql_i64(row.updated_at_ms, "updated_at_ms", &row.record_id)?;
    let sort_key = from_sql_i64(row.sort_key, "sort_key", &row.record_id)?;
    let tags: Vec<String> = parse_json(&row.tags_json, &row.record_id, "tags_json")?;
    let msg_from = parse_record_did(row.msg_from.as_deref(), owner);
    let to_fallback = match box_kind {
        BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => owner.clone(),
        BoxKind::Outbox | BoxKind::TunnelOutbox => msg_from.clone(),
    };
    let msg_to = parse_record_did(row.msg_to.as_deref(), &to_fallback);

    let mut route: Option<RouteInfo> =
        parse_optional_json(row.route_json.as_deref(), &row.record_id, "route_json")?;

    if let Some(tunnel_did_raw) = row.route_tunnel_did.as_deref() {
        let tunnel_did = DID::from_str(tunnel_did_raw).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "invalid route_tunnel_did for record {}: {}",
                row.record_id, error
            ))
        })?;
        if let Some(route_obj) = route.as_mut() {
            if route_obj.tunnel_did.is_none() {
                route_obj.tunnel_did = Some(tunnel_did);
            }
        } else {
            route = Some(RouteInfo {
                tunnel_did: Some(tunnel_did),
                ..Default::default()
            });
        }
    }

    let delivery: Option<DeliveryInfo> = parse_optional_json(
        row.delivery_json.as_deref(),
        &row.record_id,
        "delivery_json",
    )?;

    Ok(MsgRecord {
        record_id: row.record_id,
        box_kind,
        msg_id,
        msg_kind,
        state,
        from: msg_from,
        from_name: None,
        to: msg_to,
        created_at_ms,
        updated_at_ms,
        route,
        delivery,
        ui_session_id: row.thread_key.or(row.session_id),
        sort_key,
        tags,
    })
}

fn owner_from_record_id(record_id: &str) -> std::result::Result<DID, RPCErrors> {
    let owner = record_id.split('|').next().ok_or_else(|| {
        RPCErrors::ReasonError(format!("invalid record id '{}': missing owner", record_id))
    })?;
    DID::from_str(owner).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "invalid record id '{}': owner DID parse failed: {}",
            record_id, error
        ))
    })
}

fn parse_record_did(raw: Option<&str>, fallback: &DID) -> DID {
    let parsed = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            DID::from_str(value).ok().or_else(|| {
                serde_json::from_str::<Vec<String>>(value)
                    .ok()
                    .and_then(|values| {
                        values.into_iter().find_map(|entry| {
                            let normalized = entry.trim();
                            if normalized.is_empty() {
                                None
                            } else {
                                DID::from_str(normalized).ok()
                            }
                        })
                    })
            })
        });

    parsed.unwrap_or_else(|| fallback.clone())
}

fn parse_obj_id(raw: &str, record_id: &str) -> std::result::Result<ObjId, RPCErrors> {
    serde_json::from_value::<ObjId>(Value::String(raw.to_string())).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "invalid msg_id for record {}: {}",
            record_id, error
        ))
    })
}

fn msg_obj_kind_name(kind: &ndn_lib::MsgObjKind) -> &'static str {
    match kind {
        ndn_lib::MsgObjKind::Chat => "chat",
        ndn_lib::MsgObjKind::GroupMsg => "group_msg",
        ndn_lib::MsgObjKind::Deliver => "deliver",
        ndn_lib::MsgObjKind::Notify => "notify",
        ndn_lib::MsgObjKind::Event => "event",
        ndn_lib::MsgObjKind::Operation => "operation",
    }
}

fn parse_msg_obj_kind(raw: Option<&str>, box_kind: &BoxKind) -> ndn_lib::MsgObjKind {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    match normalized.as_deref() {
        Some("chat") => ndn_lib::MsgObjKind::Chat,
        Some("group_msg") => ndn_lib::MsgObjKind::GroupMsg,
        Some("deliver") => ndn_lib::MsgObjKind::Deliver,
        Some("notify") => ndn_lib::MsgObjKind::Notify,
        Some("event") => ndn_lib::MsgObjKind::Event,
        Some("operation") => ndn_lib::MsgObjKind::Operation,
        _ => infer_msg_obj_kind_from_box(box_kind),
    }
}

fn infer_msg_obj_kind_from_box(box_kind: &BoxKind) -> ndn_lib::MsgObjKind {
    match box_kind {
        BoxKind::GroupInbox => ndn_lib::MsgObjKind::GroupMsg,
        _ => ndn_lib::MsgObjKind::Chat,
    }
}

fn parse_json<T: DeserializeOwned>(
    raw: &str,
    record_id: &str,
    field: &str,
) -> std::result::Result<T, RPCErrors> {
    serde_json::from_str(raw).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "failed to parse {} for record {}: {}",
            field, record_id, error
        ))
    })
}

fn parse_optional_json<T: DeserializeOwned>(
    raw: Option<&str>,
    record_id: &str,
    field: &str,
) -> std::result::Result<Option<T>, RPCErrors> {
    match raw {
        Some(raw) => parse_json(raw, record_id, field).map(Some),
        None => Ok(None),
    }
}

fn encode_optional_json<T: serde::Serialize>(
    value: Option<&T>,
    record_id: &str,
    field: &str,
) -> std::result::Result<Option<String>, RPCErrors> {
    match value {
        Some(value) => serde_json::to_string(value).map(Some).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode {} for record {}: {}",
                field, record_id, error
            ))
        }),
        None => Ok(None),
    }
}

fn box_kind_name(box_kind: &BoxKind) -> &'static str {
    match box_kind {
        BoxKind::Inbox => "INBOX",
        BoxKind::Outbox => "OUTBOX",
        BoxKind::GroupInbox => "GROUP_INBOX",
        BoxKind::TunnelOutbox => "TUNNEL_OUTBOX",
        BoxKind::RequestBox => "REQUEST_BOX",
    }
}

fn box_kind_from_name(raw: &str) -> std::result::Result<BoxKind, RPCErrors> {
    match raw {
        "INBOX" => Ok(BoxKind::Inbox),
        "OUTBOX" => Ok(BoxKind::Outbox),
        "GROUP_INBOX" => Ok(BoxKind::GroupInbox),
        "TUNNEL_OUTBOX" => Ok(BoxKind::TunnelOutbox),
        "REQUEST_BOX" => Ok(BoxKind::RequestBox),
        _ => Err(RPCErrors::ReasonError(format!(
            "invalid box kind '{}', expected one of INBOX/OUTBOX/GROUP_INBOX/TUNNEL_OUTBOX/REQUEST_BOX",
            raw
        ))),
    }
}

fn msg_state_name(state: &MsgState) -> &'static str {
    match state {
        MsgState::Unread => "UNREAD",
        MsgState::Reading => "READING",
        MsgState::Readed => "READED",
        MsgState::Wait => "WAIT",
        MsgState::Sending => "SENDING",
        MsgState::Sent => "SENT",
        MsgState::Failed => "FAILED",
        MsgState::Dead => "DEAD",
        MsgState::Deleted => "DELETED",
        MsgState::Archived => "ARCHIVED",
    }
}

fn msg_state_from_name(raw: &str) -> std::result::Result<MsgState, RPCErrors> {
    match raw {
        "UNREAD" => Ok(MsgState::Unread),
        "READING" => Ok(MsgState::Reading),
        "READED" => Ok(MsgState::Readed),
        "WAIT" => Ok(MsgState::Wait),
        "SENDING" => Ok(MsgState::Sending),
        "SENT" => Ok(MsgState::Sent),
        "FAILED" => Ok(MsgState::Failed),
        "DEAD" => Ok(MsgState::Dead),
        "DELETED" => Ok(MsgState::Deleted),
        "ARCHIVED" => Ok(MsgState::Archived),
        _ => Err(RPCErrors::ReasonError(format!(
            "invalid msg state '{}', expected enum value",
            raw
        ))),
    }
}

fn state_matches_filter(state_filter: Option<&[MsgState]>, state_name: &str) -> bool {
    match state_filter {
        None => true,
        Some(filters) if filters.is_empty() => true,
        Some(filters) => filters
            .iter()
            .any(|item| msg_state_name(item) == state_name),
    }
}

fn to_sql_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn from_sql_i64(value: i64, field: &str, record_id: &str) -> std::result::Result<u64, RPCErrors> {
    if value < 0 {
        return Err(RPCErrors::ReasonError(format!(
            "invalid {} for record {}: negative value {}",
            field, record_id, value
        )));
    }
    Ok(value as u64)
}

fn rewrite_placeholders_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut idx = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    for ch in sql.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '?' if !in_single && !in_double => {
                idx += 1;
                out.push('$');
                out.push_str(&idx.to_string());
            }
            _ => out.push(ch),
        }
    }
    out
}

fn split_sql_statements(ddl: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for ch in ddl.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                buf.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                buf.push(ch);
            }
            ';' if !in_single && !in_double => {
                let trimmed = buf.trim();
                if !trimmed.is_empty() {
                    stmts.push(trimmed.to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        stmts.push(trimmed.to_string());
    }
    stmts
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn setup_test_mgr() -> (MsgBoxDbMgr, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("msg-box.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
        let mgr = MsgBoxDbMgr::open_default_sqlite(&conn).await.unwrap();
        (mgr, temp_dir)
    }

    fn sample_owner() -> DID {
        DID::from_str("did:bns:alice").unwrap()
    }

    fn sample_record(owner: &DID, box_kind: BoxKind, suffix: &str) -> MsgRecord {
        let record_id = format!("{}|{}|{}", owner.to_string(), box_kind_name(&box_kind), suffix);
        let msg_id: ObjId = serde_json::from_value(Value::String(format!(
            "mobjchat:{}",
            "a".repeat(40)
        )))
        .unwrap();
        MsgRecord {
            record_id,
            box_kind,
            msg_id,
            msg_kind: ndn_lib::MsgObjKind::Chat,
            state: MsgState::Unread,
            from: owner.clone(),
            from_name: None,
            to: owner.clone(),
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
            route: None,
            delivery: None,
            ui_session_id: Some("topic-1".to_string()),
            sort_key: 1_700_000_000_000,
            tags: vec!["tag".to_string()],
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn open_and_upsert_record_roundtrip() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let owner = sample_owner();
        let record = sample_record(&owner, BoxKind::Inbox, "one");
        mgr.upsert_record(&record).await.unwrap();
        let got = mgr.get_record(&owner, &record.record_id).await.unwrap();
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.record_id, record.record_id);
        assert_eq!(got.box_kind, BoxKind::Inbox);
        assert_eq!(got.state, MsgState::Unread);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_records_and_state_filter() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let owner = sample_owner();
        let r1 = sample_record(&owner, BoxKind::Inbox, "one");
        let mut r2 = sample_record(&owner, BoxKind::Inbox, "two");
        r2.state = MsgState::Readed;
        mgr.upsert_record(&r1).await.unwrap();
        mgr.upsert_record(&r2).await.unwrap();
        let all = mgr
            .list_records(&owner, &BoxKind::Inbox, None, true)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        let unread = mgr
            .list_records(&owner, &BoxKind::Inbox, Some(&[MsgState::Unread]), true)
            .await
            .unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].state, MsgState::Unread);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn touch_message_and_has_message() {
        let (mgr, _tmp) = setup_test_mgr().await;
        let owner = sample_owner();
        let msg_id: ObjId =
            serde_json::from_value(Value::String(format!("mobjchat:{}", "b".repeat(40))))
                .unwrap();
        assert!(!mgr.has_message(&owner, &msg_id).await.unwrap());
        mgr.touch_message(&owner, &msg_id, 1_700_000_000_000)
            .await
            .unwrap();
        assert!(mgr.has_message(&owner, &msg_id).await.unwrap());
        // Idempotent.
        mgr.touch_message(&owner, &msg_id, 1_700_000_000_001)
            .await
            .unwrap();
    }

    #[test]
    fn rewrite_placeholders_handles_quotes() {
        assert_eq!(
            rewrite_placeholders_to_dollar("SELECT ? FROM t WHERE s = '?' AND x = ?"),
            "SELECT $1 FROM t WHERE s = '?' AND x = $2"
        );
    }

    #[test]
    fn split_sql_statements_basic() {
        let ddl = "CREATE TABLE a(x INT); CREATE TABLE b(y TEXT);";
        let stmts = split_sql_statements(ddl);
        assert_eq!(stmts.len(), 2);
    }
}
