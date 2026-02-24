use buckyos_api::{BoxKind, DeliveryInfo, MsgRecord, MsgState, RouteInfo, MSG_CENTER_SERVICE_NAME};
use buckyos_kit::get_buckyos_service_data_dir;
use kRPC::RPCErrors;
use name_lib::DID;
use ndn_lib::{MsgObject, ObjId};
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MSG_BOX_DB_ROOT_ENV_KEY: &str = "BUCKYOS_MSG_CENTER_MSG_BOX_DIR";

#[derive(Clone, Debug)]
pub struct MsgBoxDbMgr {
    root_dir: Arc<PathBuf>,
}

#[derive(Debug)]
struct MsgRecordRow {
    record_id: String,
    box_kind: String,
    msg_id: String,
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
    pub fn new() -> std::result::Result<Self, RPCErrors> {
        let root_dir = std::env::var(MSG_BOX_DB_ROOT_ENV_KEY)
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_msg_box_root_dir());
        Self::new_with_root(root_dir)
    }

    pub fn new_with_root<P: AsRef<Path>>(root_dir: P) -> std::result::Result<Self, RPCErrors> {
        let root_dir = root_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&root_dir).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to create msg box root dir {}: {}",
                root_dir.display(),
                error
            ))
        })?;

        Ok(Self {
            root_dir: Arc::new(root_dir),
        })
    }

    pub fn upsert_record(&self, record: &MsgRecord) -> std::result::Result<(), RPCErrors> {
        self.upsert_record_with_msg(record, None)
    }

    pub fn upsert_record_with_msg(
        &self,
        record: &MsgRecord,
        msg: Option<&MsgObject>,
    ) -> std::result::Result<(), RPCErrors> {
        let conn = self.connect_owner(&record.owner)?;
        upsert_record_with_conn(&conn, record, msg)?;
        self.touch_message(&record.owner, &record.msg_id, record.created_at_ms)?;
        Ok(())
    }

    pub fn get_record(
        &self,
        owner: &DID,
        record_id: &str,
    ) -> std::result::Result<Option<MsgRecord>, RPCErrors> {
        let conn = self.connect_owner(owner)?;
        let row = conn
            .query_row(
                r#"
SELECT
    record_id,
    box_kind,
    msg_id,
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
WHERE record_id = ?1
"#,
                params![record_id],
                |row| {
                    Ok(MsgRecordRow {
                        record_id: row.get(0)?,
                        box_kind: row.get(1)?,
                        msg_id: row.get(2)?,
                        state: row.get(3)?,
                        created_at_ms: row.get(4)?,
                        updated_at_ms: row.get(5)?,
                        thread_key: row.get(6)?,
                        session_id: row.get(7)?,
                        sort_key: row.get(8)?,
                        tags_json: row.get(9)?,
                        route_tunnel_did: row.get(10)?,
                        route_json: row.get(11)?,
                        delivery_json: row.get(12)?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query msg record {}: {}",
                    record_id, error
                ))
            })?;

        match row {
            Some(row) => Ok(Some(row_to_record(owner, row)?)),
            None => Ok(None),
        }
    }

    pub fn list_records(
        &self,
        owner: &DID,
        box_kind: &BoxKind,
        state_filter: Option<&[MsgState]>,
        descending: bool,
    ) -> std::result::Result<Vec<MsgRecord>, RPCErrors> {
        let conn = self.connect_owner(owner)?;
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
WHERE box_kind = ?1
{}
"#,
            order_clause
        );

        let mut stmt = conn.prepare(&sql).map_err(|error| {
            RPCErrors::ReasonError(format!("failed to prepare list records query: {}", error))
        })?;
        let rows = stmt
            .query_map(params![box_kind_name(box_kind)], |row| {
                Ok(MsgRecordRow {
                    record_id: row.get(0)?,
                    box_kind: row.get(1)?,
                    msg_id: row.get(2)?,
                    state: row.get(3)?,
                    created_at_ms: row.get(4)?,
                    updated_at_ms: row.get(5)?,
                    thread_key: row.get(6)?,
                    session_id: row.get(7)?,
                    sort_key: row.get(8)?,
                    tags_json: row.get(9)?,
                    route_tunnel_did: row.get(10)?,
                    route_json: row.get(11)?,
                    delivery_json: row.get(12)?,
                })
            })
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to list msg records: {}", error))
            })?;

        let mut records = Vec::new();
        for row in rows {
            let row = row.map_err(|error| {
                RPCErrors::ReasonError(format!("failed to decode msg record row: {}", error))
            })?;

            if !state_matches_filter(state_filter, &row.state) {
                continue;
            }

            records.push(row_to_record(owner, row)?);
        }

        Ok(records)
    }

    pub fn touch_message(
        &self,
        owner: &DID,
        msg_id: &ObjId,
        created_at_ms: u64,
    ) -> std::result::Result<(), RPCErrors> {
        let conn = self.connect_owner(owner)?;
        conn.execute(
            "INSERT OR IGNORE INTO msg_refs(msg_id, created_at_ms) VALUES(?1, ?2)",
            params![msg_id.to_string(), to_sql_i64(created_at_ms)],
        )
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to persist message ref {}: {}",
                msg_id.to_string(),
                error
            ))
        })?;

        Ok(())
    }

    pub fn has_message(&self, owner: &DID, msg_id: &ObjId) -> std::result::Result<bool, RPCErrors> {
        let conn = self.connect_owner(owner)?;
        let exists: Option<u8> = conn
            .query_row(
                "SELECT 1 FROM msg_refs WHERE msg_id = ?1 LIMIT 1",
                params![msg_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to query message ref {}: {}",
                    msg_id.to_string(),
                    error
                ))
            })?;
        Ok(exists.is_some())
    }

    fn connect_owner(&self, owner: &DID) -> std::result::Result<Connection, RPCErrors> {
        let db_path = self.owner_db_path(owner);
        let conn = Connection::open(&db_path).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to open msg box db {}: {}",
                db_path.display(),
                error
            ))
        })?;

        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to configure msg box db {}: {}",
                    db_path.display(),
                    error
                ))
            })?;

        self.init_owner_db(&conn, &db_path)?;
        Ok(conn)
    }

    fn init_owner_db(
        &self,
        conn: &Connection,
        db_path: &Path,
    ) -> std::result::Result<(), RPCErrors> {
        conn.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS msg_records (
    record_id TEXT PRIMARY KEY,
    box_kind TEXT NOT NULL,
    msg_id TEXT NOT NULL,
    msg_from TEXT,
    msg_to TEXT,
    msg_kind TEXT,
    state TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    thread_key TEXT,
    session_id TEXT,
    sort_key INTEGER NOT NULL,
    tags_json TEXT NOT NULL,
    route_tunnel_did TEXT,
    route_json TEXT,
    delivery_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_msg_records_box_sort
    ON msg_records(box_kind, sort_key DESC, record_id DESC);
CREATE INDEX IF NOT EXISTS idx_msg_records_box_state_sort
    ON msg_records(box_kind, state, sort_key DESC, record_id DESC);
CREATE INDEX IF NOT EXISTS idx_msg_records_tunnel_state_sort
    ON msg_records(route_tunnel_did, state, sort_key DESC, record_id DESC);
CREATE INDEX IF NOT EXISTS idx_msg_records_box_kind_sort
    ON msg_records(box_kind, msg_kind, sort_key DESC, record_id DESC);
CREATE INDEX IF NOT EXISTS idx_msg_records_box_from_sort
    ON msg_records(box_kind, msg_from, sort_key DESC, record_id DESC);

CREATE TABLE IF NOT EXISTS msg_refs (
    msg_id TEXT PRIMARY KEY,
    created_at_ms INTEGER NOT NULL
);
"#,
        )
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to initialize msg box db {}: {}",
                db_path.display(),
                error
            ))
        })?;

        self.ensure_msg_records_session_id_column(conn, db_path)?;

        Ok(())
    }

    fn ensure_msg_records_session_id_column(
        &self,
        conn: &Connection,
        db_path: &Path,
    ) -> std::result::Result<(), RPCErrors> {
        let exists: Option<u8> = conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('msg_records') WHERE name = ?1 LIMIT 1",
                params!["session_id"],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to inspect msg_records schema {}: {}",
                    db_path.display(),
                    error
                ))
            })?;
        if exists.is_some() {
            return Ok(());
        }

        conn.execute("ALTER TABLE msg_records ADD COLUMN session_id TEXT", [])
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to add session_id column for msg_records {}: {}",
                    db_path.display(),
                    error
                ))
            })?;
        Ok(())
    }

    fn owner_db_path(&self, owner: &DID) -> PathBuf {
        self.root_dir.join(format!(
            "{}.sqlite3",
            sanitize_for_filename(&owner.to_string())
        ))
    }
}

fn upsert_record_with_conn(
    conn: &Connection,
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
    let msg_from = msg.map(|obj| obj.from.to_string());
    let msg_to = match msg {
        Some(obj) => Some(serde_json::to_string(&obj.to).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "failed to encode msg_to for record {}: {}",
                record.record_id, error
            ))
        })?),
        None => None,
    };
    let msg_kind = match msg {
        Some(obj) => {
            let kind_value = serde_json::to_value(obj.kind).map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to encode msg_kind for record {}: {}",
                    record.record_id, error
                ))
            })?;
            let kind_name = kind_value.as_str().ok_or_else(|| {
                RPCErrors::ReasonError(format!(
                    "failed to encode msg_kind for record {}: non-string value",
                    record.record_id
                ))
            })?;
            Some(kind_name.to_string())
        }
        None => None,
    };

    conn.execute(
        r#"
INSERT INTO msg_records (
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
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
ON CONFLICT(record_id) DO UPDATE SET
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
        params![
            record.record_id,
            box_kind_name(&record.box_kind),
            record.msg_id.to_string(),
            msg_from,
            msg_to,
            msg_kind,
            msg_state_name(&record.state),
            to_sql_i64(record.created_at_ms),
            to_sql_i64(record.updated_at_ms),
            record.thread_key,
            record.session_id,
            to_sql_i64(record.sort_key),
            tags_json,
            route_tunnel_did,
            route_json,
            delivery_json,
        ],
    )
    .map_err(|error| {
        RPCErrors::ReasonError(format!(
            "failed to upsert msg record {}: {}",
            record.record_id, error
        ))
    })?;

    Ok(())
}

fn row_to_record(owner: &DID, row: MsgRecordRow) -> std::result::Result<MsgRecord, RPCErrors> {
    let box_kind = box_kind_from_name(&row.box_kind)?;
    let state = msg_state_from_name(&row.state)?;
    let msg_id = parse_obj_id(&row.msg_id, &row.record_id)?;
    let created_at_ms = from_sql_i64(row.created_at_ms, "created_at_ms", &row.record_id)?;
    let updated_at_ms = from_sql_i64(row.updated_at_ms, "updated_at_ms", &row.record_id)?;
    let sort_key = from_sql_i64(row.sort_key, "sort_key", &row.record_id)?;
    let tags: Vec<String> = parse_json(&row.tags_json, &row.record_id, "tags_json")?;

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
        owner: owner.clone(),
        box_kind,
        msg_id,
        state,
        created_at_ms,
        updated_at_ms,
        route,
        delivery,
        thread_key: row.thread_key,
        session_id: row.session_id,
        sort_key,
        tags,
    })
}

fn parse_obj_id(raw: &str, record_id: &str) -> std::result::Result<ObjId, RPCErrors> {
    serde_json::from_value::<ObjId>(Value::String(raw.to_string())).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "invalid msg_id for record {}: {}",
            record_id, error
        ))
    })
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

fn sanitize_for_filename(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut prev_sep = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            output.push('_');
            prev_sep = true;
        }
    }

    let trimmed = output.trim_matches('_');
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    let suffix = format!("{:016x}", hasher.finish());
    let prefix = if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.chars().take(120).collect()
    };
    format!("{}_{}", prefix, suffix)
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

fn default_msg_box_root_dir() -> PathBuf {
    get_buckyos_service_data_dir(MSG_CENTER_SERVICE_NAME).join("msg_boxes")
}
