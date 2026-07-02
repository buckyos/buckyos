/*!
 * Durable local storage for AICC usage events.
 *
 * Mirrors the pattern used by `msg_center/src/msg_box_db.rs`: one `AnyPool`
 * per process, backend (sqlite / postgres) and schema come from the service
 * spec via `get_rdb_instance`. The pool is `Send + Sync + Clone`, so an
 * `Arc<AiccUsageLogDb>` can be shared without an outer lock.
 *
 * Only one table is managed in v1: `aicc_usage_event`. Aggregation happens in
 * Rust after selecting rows; SQL aggregation can come later when query volume
 * justifies it.
 */

use std::collections::HashMap;
use std::sync::{Arc, Once};

use buckyos_api::{
    aicc_usage_log_default_rdb_instance_config, get_rdb_instance, AiccRouteTraceEvent,
    AiccUsageEvent, QueryRouteTraceRequest, QueryRouteTraceResponse, QueryUsageRequest,
    QueryUsageResponse, RdbBackend, UsageAggregate, UsageBucketedRow, UsageGroupedRow,
    UsageQueryBucket, UsageQueryGroup, UsageQueryOutputMode, UsageQueryTimeRange,
    AICC_SERVICE_SERVICE_NAME, AICC_USAGE_LOG_RDB_INSTANCE_ID, AICC_USAGE_LOG_RDB_SCHEMA_POSTGRES,
    AICC_USAGE_LOG_RDB_SCHEMA_SQLITE,
};
use kRPC::RPCErrors;
use log::info;
use serde_json::Value;
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

#[derive(Clone, Debug)]
pub struct AiccUsageLogDb {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    pool: AnyPool,
    backend: RdbBackend,
}

impl AiccUsageLogDb {
    /// Open a pool against `connection`. An empty / None `schema` means "use
    /// the compile-time default for `backend`".
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> Result<Self, RPCErrors> {
        ensure_any_drivers_installed();
        let mut opts = AnyPoolOptions::new().max_connections(4);
        if backend == RdbBackend::Sqlite {
            opts = opts.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA journal_mode = WAL;").await?;
                    Ok(())
                })
            });
        }
        let pool = opts.connect(connection).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "open aicc usage-log db at {} failed: {}",
                connection, error
            ))
        })?;
        let mgr = Self {
            inner: Arc::new(Inner { pool, backend }),
        };
        mgr.apply_schema(schema).await?;
        Ok(mgr)
    }

    /// Resolve the usage-log rdb instance from the aicc service spec and open
    /// a pool against it. This is the production entry point.
    pub async fn open_from_service_spec() -> Result<Self, RPCErrors> {
        let instance = get_rdb_instance(
            AICC_SERVICE_SERVICE_NAME,
            None,
            AICC_USAGE_LOG_RDB_INSTANCE_ID,
        )
        .await
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "resolve aicc usage-log rdb instance failed: {}",
                error
            ))
        })?;
        info!("aicc_usage_log_db.open {}", instance.connection);
        Self::open(
            &instance.connection,
            instance.backend,
            instance.schema.as_deref(),
        )
        .await
    }

    /// Test / fallback entry: build a default instance config (sqlite) against
    /// the given connection string.
    pub async fn open_default_sqlite(connection: &str) -> Result<Self, RPCErrors> {
        let cfg = aicc_usage_log_default_rdb_instance_config();
        let schema = cfg.schema.get(&RdbBackend::Sqlite).cloned();
        Self::open(connection, RdbBackend::Sqlite, schema.as_deref()).await
    }

    fn pool(&self) -> &AnyPool {
        &self.inner.pool
    }

    fn backend(&self) -> RdbBackend {
        self.inner.backend
    }

    async fn apply_schema(&self, override_ddl: Option<&str>) -> Result<(), RPCErrors> {
        let default_ddl = match self.backend() {
            RdbBackend::Sqlite => AICC_USAGE_LOG_RDB_SCHEMA_SQLITE,
            RdbBackend::Postgres => AICC_USAGE_LOG_RDB_SCHEMA_POSTGRES,
        };
        let ddl = override_ddl
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(default_ddl);
        for statement in split_sql_statements(ddl)
            .into_iter()
            .chain(split_sql_statements(default_ddl).into_iter())
        {
            self.pool().execute(statement.as_str()).await.map_err(|e| {
                RPCErrors::ReasonError(format!("apply aicc usage-log schema failed: {}", e))
            })?;
        }
        Ok(())
    }

    fn render_sql(&self, sql: &str) -> String {
        match self.backend() {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    /// Insert a usage event, ignoring duplicates on `event_id`,
    /// `(tenant_id, task_id)` and `(tenant_id, idempotency_key)`.
    ///
    /// Returns `true` if a new row was written, `false` if the insert was
    /// skipped because an event for the same task / idempotency key already
    /// exists. Idempotency is the key acceptance criterion — the AICC
    /// complete path can retry without creating duplicate rows.
    pub async fn insert_usage_event(&self, event: &AiccUsageEvent) -> Result<bool, RPCErrors> {
        let usage_json = serde_json::to_string(&event.usage_json).map_err(|err| {
            RPCErrors::ReasonError(format!(
                "serialize usage_json for event {} failed: {}",
                event.event_id, err
            ))
        })?;
        let finance_snapshot_json = match event.finance_snapshot_json.as_ref() {
            Some(value) => Some(serde_json::to_string(value).map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "serialize finance_snapshot_json for event {} failed: {}",
                    event.event_id, err
                ))
            })?),
            None => None,
        };

        // `ON CONFLICT DO NOTHING` works for both sqlite (3.24+) and postgres,
        // and also covers the two unique indexes declared in the schema.
        let sql = self.render_sql(
            r#"
INSERT INTO aicc_usage_event (
    event_id,
    tenant_id,
    caller_app_id,
    task_id,
    idempotency_key,
    capability,
    request_model,
    provider_model,
    input_tokens,
    output_tokens,
    total_tokens,
    request_units,
    usage_json,
    finance_snapshot_json,
    created_at_ms
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT DO NOTHING
"#,
        );

        let result = sqlx::query(&sql)
            .bind(event.event_id.clone())
            .bind(event.tenant_id.clone())
            .bind(event.caller_app_id.clone())
            .bind(event.task_id.clone())
            .bind(event.idempotency_key.clone())
            .bind(event.capability.clone())
            .bind(event.request_model.clone())
            .bind(event.provider_model.clone())
            .bind(event.input_tokens.map(to_sql_i64))
            .bind(event.output_tokens.map(to_sql_i64))
            .bind(event.total_tokens.map(to_sql_i64))
            .bind(event.request_units.map(to_sql_i64))
            .bind(usage_json)
            .bind(finance_snapshot_json)
            .bind(event.created_at_ms)
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to insert aicc usage event {}: {}",
                    event.event_id, error
                ))
            })?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn insert_route_trace_event(
        &self,
        event: &AiccRouteTraceEvent,
    ) -> Result<bool, RPCErrors> {
        let route_trace_json = serde_json::to_string(&event.route_trace_json).map_err(|err| {
            RPCErrors::ReasonError(format!(
                "serialize route_trace_json for event {} failed: {}",
                event.trace_id, err
            ))
        })?;
        let sql = self.render_sql(
            r#"
INSERT INTO aicc_route_trace (
    trace_id,
    tenant_id,
    caller_app_id,
    task_id,
    request_model,
    selected_exact_model,
    provider_instance_name,
    api_type,
    route_trace_json,
    created_at_ms
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT DO NOTHING
"#,
        );

        let result = sqlx::query(&sql)
            .bind(event.trace_id.clone())
            .bind(event.tenant_id.clone())
            .bind(event.caller_app_id.clone())
            .bind(event.task_id.clone())
            .bind(event.request_model.clone())
            .bind(event.selected_exact_model.clone())
            .bind(event.provider_instance_name.clone())
            .bind(event.api_type.clone())
            .bind(route_trace_json)
            .bind(event.created_at_ms)
            .execute(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "failed to insert aicc route trace event {}: {}",
                    event.trace_id, error
                ))
            })?;

        Ok(result.rows_affected() > 0)
    }

    /// Flexible query entry — pulls rows matching the filter window, then
    /// aggregates in Rust according to the request. See the requirements doc
    /// (section 7) for the semantics.
    pub async fn query_usage(
        &self,
        req: &QueryUsageRequest,
    ) -> Result<QueryUsageResponse, RPCErrors> {
        let now_ms = current_time_ms();
        let (start_ms, end_ms) = resolve_time_window(&req.time_range, now_ms);

        let mut sql = String::from(
            r#"
SELECT
    event_id,
    tenant_id,
    caller_app_id,
    task_id,
    idempotency_key,
    capability,
    request_model,
    provider_model,
    input_tokens,
    output_tokens,
    total_tokens,
    request_units,
    usage_json,
    finance_snapshot_json,
    created_at_ms
FROM aicc_usage_event
WHERE created_at_ms >= ? AND created_at_ms < ?
"#,
        );
        let mut string_binds: Vec<String> = vec![];

        push_in_filter(
            &mut sql,
            &mut string_binds,
            "tenant_id",
            &req.filters.tenant_ids,
        );
        push_in_filter(
            &mut sql,
            &mut string_binds,
            "caller_app_id",
            &req.filters.caller_app_ids,
        );
        push_like_filter(
            &mut sql,
            &mut string_binds,
            "caller_app_id",
            &req.filters.caller_app_query,
        );
        push_in_filter(
            &mut sql,
            &mut string_binds,
            "request_model",
            &req.filters.request_models,
        );
        push_in_filter(
            &mut sql,
            &mut string_binds,
            "provider_model",
            &req.filters.provider_models,
        );
        push_like_filter(
            &mut sql,
            &mut string_binds,
            "provider_model",
            &req.filters.provider_model_query,
        );
        push_provider_instance_in_filter(
            &mut sql,
            &mut string_binds,
            &req.filters.provider_instance_names,
        );
        push_provider_instance_query_filter(
            &mut sql,
            &mut string_binds,
            &req.filters.provider_instance_query,
        );
        push_in_filter(
            &mut sql,
            &mut string_binds,
            "capability",
            &req.filters.capabilities,
        );
        push_in_filter(&mut sql, &mut string_binds, "task_id", &req.filters.task_ids);
        push_in_filter(
            &mut sql,
            &mut string_binds,
            "idempotency_key",
            &req.filters.idempotency_keys,
        );

        sql.push_str("\nORDER BY created_at_ms DESC, event_id DESC\n");

        let sql = self.render_sql(&sql);

        let mut query = sqlx::query(&sql).bind(start_ms).bind(end_ms);
        for value in string_binds {
            query = query.bind(value);
        }

        let rows = query.fetch_all(self.pool()).await.map_err(|error| {
            RPCErrors::ReasonError(format!("failed to query aicc usage events: {}", error))
        })?;

        let events = rows
            .iter()
            .map(decode_row)
            .collect::<Result<Vec<AiccUsageEvent>, RPCErrors>>()?;

        let mut response = QueryUsageResponse {
            total: aggregate(events.iter()),
            ..Default::default()
        };

        if !req.group_by.is_empty() {
            response.grouped = group_events(events.as_slice(), &req.group_by);
        }

        if let Some(bucket) = req.time_bucket {
            response.buckets = bucket_events(events.as_slice(), bucket, &req.group_by);
        }

        match req.output_mode {
            UsageQueryOutputMode::Summary => {}
            UsageQueryOutputMode::Events | UsageQueryOutputMode::SummaryAndEvents => {
                let offset = parse_cursor(req.cursor.as_deref())?;
                let limit = req.limit.map(|v| v as usize).unwrap_or(events.len());
                let end_index = offset.saturating_add(limit).min(events.len());
                if offset < events.len() {
                    response.events = events[offset..end_index].to_vec();
                }
                if end_index < events.len() {
                    response.next_cursor = Some(end_index.to_string());
                }
            }
        }

        Ok(response)
    }

    pub async fn query_route_traces(
        &self,
        req: &QueryRouteTraceRequest,
    ) -> Result<QueryRouteTraceResponse, RPCErrors> {
        let limit = req.limit.unwrap_or(20).clamp(1, 128) as usize;
        let offset = parse_cursor(req.cursor.as_deref())?;
        let mut sql = String::from(
            r#"
SELECT route_trace_json
FROM aicc_route_trace
WHERE 1 = 1
"#,
        );
        let mut string_binds: Vec<String> = vec![];
        push_trace_id_filter(&mut sql, &mut string_binds, req);
        sql.push_str(
            r#"
ORDER BY created_at_ms DESC, trace_id DESC
LIMIT ? OFFSET ?
"#,
        );
        let sql = self.render_sql(&sql);
        let mut query = sqlx::query(&sql);
        for value in string_binds {
            query = query.bind(value);
        }
        let rows = query
            .bind(to_sql_i64(limit as u64))
            .bind(to_sql_i64(offset as u64))
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("failed to query aicc route traces: {}", error))
            })?;

        let traces = rows
            .iter()
            .map(decode_route_trace_row)
            .collect::<Result<Vec<Value>, RPCErrors>>()?;
        let next_cursor = if traces.len() == limit {
            Some(offset.saturating_add(traces.len()).to_string())
        } else {
            None
        };
        Ok(QueryRouteTraceResponse {
            traces,
            next_cursor,
        })
    }
}

fn push_trace_id_filter(sql: &mut String, binds: &mut Vec<String>, req: &QueryRouteTraceRequest) {
    let task_ids = normalized_values(&req.task_ids);
    let request_ids = normalized_values(&req.request_ids);
    if task_ids.is_empty() && request_ids.is_empty() {
        return;
    }

    let mut clauses = Vec::new();
    if !task_ids.is_empty() {
        clauses.push(format!("task_id IN ({})", placeholders(task_ids.len())));
        binds.extend(task_ids);
    }
    if !request_ids.is_empty() {
        clauses.push(format!("trace_id IN ({})", placeholders(request_ids.len())));
        binds.extend(request_ids);
    }
    sql.push_str("\nAND (");
    sql.push_str(&clauses.join(" OR "));
    sql.push_str(")\n");
}

fn normalized_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_time_window(range: &UsageQueryTimeRange, now_ms: i64) -> (i64, i64) {
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    match range {
        UsageQueryTimeRange::Last1d => (now_ms - DAY_MS, now_ms),
        UsageQueryTimeRange::Last7d => (now_ms - 7 * DAY_MS, now_ms),
        UsageQueryTimeRange::Last30d => (now_ms - 30 * DAY_MS, now_ms),
        UsageQueryTimeRange::Explicit {
            start_time_ms,
            end_time_ms,
        } => (*start_time_ms, *end_time_ms),
    }
}

fn push_in_filter(sql: &mut String, binds: &mut Vec<String>, column: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let placeholders = std::iter::repeat("?")
        .take(values.len())
        .collect::<Vec<_>>()
        .join(", ");
    sql.push_str(&format!(" AND {} IN ({})", column, placeholders));
    binds.extend(values.iter().cloned());
}

fn push_like_filter(sql: &mut String, binds: &mut Vec<String>, column: &str, value: &Option<String>) {
    if let Some(val) = value.as_deref().map(str::trim).filter(|val| !val.is_empty()) {
        sql.push_str(&format!(" AND LOWER(COALESCE({}, '')) LIKE LOWER(?) ESCAPE '\\'", column));
        binds.push(format!("%{}%", escape_like_value(val)));
    }
}

fn push_provider_instance_in_filter(sql: &mut String, binds: &mut Vec<String>, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let clauses = std::iter::repeat("provider_model LIKE ? ESCAPE '\\'")
        .take(values.len())
        .collect::<Vec<_>>()
        .join(" OR ");
    sql.push_str(&format!(" AND ({})", clauses));
    binds.extend(values.iter().map(|value| format!("%@{}", escape_like_value(value))));
}

fn push_provider_instance_query_filter(sql: &mut String, binds: &mut Vec<String>, value: &Option<String>) {
    if let Some(val) = value.as_deref().map(str::trim).filter(|val| !val.is_empty()) {
        sql.push_str(" AND LOWER(provider_model) LIKE LOWER(?) ESCAPE '\\'");
        binds.push(format!("%{}%", escape_like_value(val)));
    }
}

fn escape_like_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn decode_row(row: &AnyRow) -> Result<AiccUsageEvent, RPCErrors> {
    let decode = |field: &str, err: sqlx::Error| {
        RPCErrors::ReasonError(format!(
            "failed to decode aicc_usage_event.{}: {}",
            field, err
        ))
    };
    let event_id: String = row.try_get("event_id").map_err(|e| decode("event_id", e))?;
    let tenant_id: String = row
        .try_get("tenant_id")
        .map_err(|e| decode("tenant_id", e))?;
    let caller_app_id: Option<String> = row
        .try_get("caller_app_id")
        .map_err(|e| decode("caller_app_id", e))?;
    let task_id: String = row.try_get("task_id").map_err(|e| decode("task_id", e))?;
    let idempotency_key: Option<String> = row
        .try_get("idempotency_key")
        .map_err(|e| decode("idempotency_key", e))?;
    let capability: String = row
        .try_get("capability")
        .map_err(|e| decode("capability", e))?;
    let request_model: String = row
        .try_get("request_model")
        .map_err(|e| decode("request_model", e))?;
    let provider_model: String = row
        .try_get("provider_model")
        .map_err(|e| decode("provider_model", e))?;
    let input_tokens: Option<i64> = row
        .try_get("input_tokens")
        .map_err(|e| decode("input_tokens", e))?;
    let output_tokens: Option<i64> = row
        .try_get("output_tokens")
        .map_err(|e| decode("output_tokens", e))?;
    let total_tokens: Option<i64> = row
        .try_get("total_tokens")
        .map_err(|e| decode("total_tokens", e))?;
    let request_units: Option<i64> = row
        .try_get("request_units")
        .map_err(|e| decode("request_units", e))?;
    let usage_json_str: String = row
        .try_get("usage_json")
        .map_err(|e| decode("usage_json", e))?;
    let finance_snapshot_str: Option<String> = row
        .try_get("finance_snapshot_json")
        .map_err(|e| decode("finance_snapshot_json", e))?;
    let created_at_ms: i64 = row
        .try_get("created_at_ms")
        .map_err(|e| decode("created_at_ms", e))?;

    let usage_json: Value = serde_json::from_str(&usage_json_str).map_err(|err| {
        RPCErrors::ReasonError(format!(
            "failed to parse usage_json for event {}: {}",
            event_id, err
        ))
    })?;
    let finance_snapshot_json = match finance_snapshot_str {
        Some(raw) => Some(serde_json::from_str(&raw).map_err(|err| {
            RPCErrors::ReasonError(format!(
                "failed to parse finance_snapshot_json for event {}: {}",
                event_id, err
            ))
        })?),
        None => None,
    };

    Ok(AiccUsageEvent {
        event_id,
        tenant_id,
        caller_app_id,
        task_id,
        idempotency_key,
        capability,
        request_model,
        provider_model,
        input_tokens: input_tokens.and_then(from_sql_i64_opt),
        output_tokens: output_tokens.and_then(from_sql_i64_opt),
        total_tokens: total_tokens.and_then(from_sql_i64_opt),
        request_units: request_units.and_then(from_sql_i64_opt),
        usage_json,
        finance_snapshot_json,
        created_at_ms,
    })
}

fn decode_route_trace_row(row: &AnyRow) -> Result<Value, RPCErrors> {
    let raw: String = row.try_get("route_trace_json").map_err(|err| {
        RPCErrors::ReasonError(format!(
            "failed to decode aicc_route_trace.route_trace_json: {}",
            err
        ))
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        RPCErrors::ReasonError(format!("failed to parse route_trace_json: {}", err))
    })
}

fn aggregate<'a, I: Iterator<Item = &'a AiccUsageEvent>>(events: I) -> UsageAggregate {
    let mut agg = UsageAggregate::default();
    for event in events {
        accumulate(&mut agg, event);
    }
    agg
}

fn accumulate(agg: &mut UsageAggregate, event: &AiccUsageEvent) {
    agg.total_requests = agg.total_requests.saturating_add(1);
    agg.input_tokens = agg
        .input_tokens
        .saturating_add(event.input_tokens.unwrap_or(0));
    agg.output_tokens = agg
        .output_tokens
        .saturating_add(event.output_tokens.unwrap_or(0));
    agg.total_tokens = agg
        .total_tokens
        .saturating_add(event.total_tokens.unwrap_or(0));
    agg.request_units = agg
        .request_units
        .saturating_add(event.request_units.unwrap_or(0));
    if let Some(amount) = finance_amount(event) {
        agg.finance_amount = Some(agg.finance_amount.unwrap_or(0.0) + amount);
    }
}

fn finance_amount(event: &AiccUsageEvent) -> Option<f64> {
    event
        .finance_snapshot_json
        .as_ref()
        .and_then(|snapshot| snapshot.get("amount"))
        .and_then(Value::as_f64)
        .filter(|amount| amount.is_finite())
}

fn group_events(events: &[AiccUsageEvent], group_by: &[UsageQueryGroup]) -> Vec<UsageGroupedRow> {
    let mut buckets: HashMap<Vec<String>, UsageGroupedRow> = HashMap::new();
    for event in events {
        let key = build_group_key(event, group_by);
        let row = buckets
            .entry(key.clone())
            .or_insert_with(|| UsageGroupedRow {
                group: build_group_map(event, group_by),
                aggregate: UsageAggregate::default(),
            });
        accumulate(&mut row.aggregate, event);
    }
    let mut rows: Vec<UsageGroupedRow> = buckets.into_values().collect();
    rows.sort_by(|a, b| b.aggregate.total_requests.cmp(&a.aggregate.total_requests));
    rows
}

fn bucket_events(
    events: &[AiccUsageEvent],
    bucket: UsageQueryBucket,
    group_by: &[UsageQueryGroup],
) -> Vec<UsageBucketedRow> {
    let span = bucket.span_ms();
    let mut buckets: HashMap<(i64, Vec<String>), UsageBucketedRow> = HashMap::new();
    for event in events {
        let bucket_start = (event.created_at_ms / span) * span;
        let group_key = build_group_key(event, group_by);
        let row = buckets
            .entry((bucket_start, group_key.clone()))
            .or_insert_with(|| UsageBucketedRow {
                bucket_start_ms: bucket_start,
                group: build_group_map(event, group_by),
                aggregate: UsageAggregate::default(),
            });
        accumulate(&mut row.aggregate, event);
    }
    let mut rows: Vec<UsageBucketedRow> = buckets.into_values().collect();
    rows.sort_by(|a, b| a.bucket_start_ms.cmp(&b.bucket_start_ms));
    rows
}

fn build_group_key(event: &AiccUsageEvent, group_by: &[UsageQueryGroup]) -> Vec<String> {
    group_by
        .iter()
        .map(|dim| dim_value(event, *dim))
        .collect::<Vec<_>>()
}

fn build_group_map(
    event: &AiccUsageEvent,
    group_by: &[UsageQueryGroup],
) -> HashMap<String, String> {
    group_by
        .iter()
        .map(|dim| (dim.as_key().to_string(), dim_value(event, *dim)))
        .collect()
}

fn dim_value(event: &AiccUsageEvent, dim: UsageQueryGroup) -> String {
    match dim {
        UsageQueryGroup::ProviderModel => event.provider_model.clone(),
        UsageQueryGroup::RequestModel => event.request_model.clone(),
        UsageQueryGroup::Capability => event.capability.clone(),
        UsageQueryGroup::CallerAppId => event.caller_app_id.clone().unwrap_or_default(),
        UsageQueryGroup::TenantId => event.tenant_id.clone(),
    }
}

fn parse_cursor(cursor: Option<&str>) -> Result<usize, RPCErrors> {
    match cursor {
        None => Ok(0),
        Some(value) => value
            .parse::<usize>()
            .map_err(|err| RPCErrors::ReasonError(format!("invalid usage query cursor: {}", err))),
    }
}

fn current_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn to_sql_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn from_sql_i64_opt(value: i64) -> Option<u64> {
    if value < 0 {
        None
    } else {
        Some(value as u64)
    }
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
    use buckyos_api::{UsageQueryFilters, UsageQueryGroup, UsageQueryTimeRange};
    use serde_json::json;
    use tempfile::tempdir;

    async fn setup() -> (AiccUsageLogDb, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let db_path = path.to_string_lossy().replace('\\', "/");
        let conn = format!("sqlite:///{}?mode=rwc", db_path);
        let db = AiccUsageLogDb::open_default_sqlite(&conn).await.unwrap();
        (db, dir)
    }

    fn sample_event(
        suffix: &str,
        model: &str,
        now_ms: i64,
        input: u64,
        output: u64,
    ) -> AiccUsageEvent {
        AiccUsageEvent {
            event_id: format!("evt-{}", suffix),
            tenant_id: "alice".to_string(),
            caller_app_id: Some("sys_test".to_string()),
            task_id: format!("task-{}", suffix),
            idempotency_key: Some(format!("idem-{}", suffix)),
            capability: "llm_router".to_string(),
            request_model: "llm.plan.default".to_string(),
            provider_model: model.to_string(),
            input_tokens: Some(input),
            output_tokens: Some(output),
            total_tokens: Some(input + output),
            request_units: None,
            usage_json: json!({
                "input_tokens": input,
                "output_tokens": output,
                "total_tokens": input + output,
            }),
            finance_snapshot_json: None,
            created_at_ms: now_ms,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn insert_and_query_total() {
        let (db, _tmp) = setup().await;
        let now = current_time_ms();
        let one = sample_event("one", "gpt4.openai", now - 1000, 10, 20);
        let two = sample_event("two", "claude-sonnet.anthropic", now - 500, 5, 15);

        assert!(db.insert_usage_event(&one).await.unwrap());
        assert!(db.insert_usage_event(&two).await.unwrap());
        // duplicate event_id is a no-op
        assert!(!db.insert_usage_event(&one).await.unwrap());

        let resp = db
            .query_usage(&QueryUsageRequest {
                time_range: UsageQueryTimeRange::Last1d,
                filters: UsageQueryFilters::default(),
                group_by: vec![],
                time_bucket: None,
                output_mode: UsageQueryOutputMode::Summary,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap();

        assert_eq!(resp.total.total_requests, 2);
        assert_eq!(resp.total.input_tokens, 15);
        assert_eq!(resp.total.output_tokens, 35);
        assert_eq!(resp.total.total_tokens, 50);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn group_by_provider_model() {
        let (db, _tmp) = setup().await;
        let now = current_time_ms();
        db.insert_usage_event(&sample_event("a", "gpt4.openai", now - 1000, 10, 20))
            .await
            .unwrap();
        db.insert_usage_event(&sample_event("b", "gpt4.openai", now - 800, 3, 4))
            .await
            .unwrap();
        db.insert_usage_event(&sample_event(
            "c",
            "claude-sonnet.anthropic",
            now - 500,
            1,
            2,
        ))
        .await
        .unwrap();

        let resp = db
            .query_usage(&QueryUsageRequest {
                time_range: UsageQueryTimeRange::Last7d,
                filters: UsageQueryFilters::default(),
                group_by: vec![UsageQueryGroup::ProviderModel],
                time_bucket: None,
                output_mode: UsageQueryOutputMode::Summary,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap();

        assert_eq!(resp.grouped.len(), 2);
        let openai = resp
            .grouped
            .iter()
            .find(|row| row.group.get("provider_model") == Some(&"gpt4.openai".to_string()))
            .unwrap();
        assert_eq!(openai.aggregate.total_requests, 2);
        assert_eq!(openai.aggregate.input_tokens, 13);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn idempotency_guards_duplicate() {
        let (db, _tmp) = setup().await;
        let now = current_time_ms();
        let first = sample_event("dup", "gpt4.openai", now - 100, 1, 1);
        let mut second = first.clone();
        // Different event_id / task_id but same (tenant_id, idempotency_key).
        second.event_id = "evt-dup-retry".to_string();
        second.task_id = "task-dup-retry".to_string();

        assert!(db.insert_usage_event(&first).await.unwrap());
        assert!(!db.insert_usage_event(&second).await.unwrap());

        let resp = db
            .query_usage(&QueryUsageRequest {
                time_range: UsageQueryTimeRange::Last1d,
                filters: UsageQueryFilters::default(),
                group_by: vec![],
                time_bucket: None,
                output_mode: UsageQueryOutputMode::Summary,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(resp.total.total_requests, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn events_pagination_roundtrip() {
        let (db, _tmp) = setup().await;
        let now = current_time_ms();
        for i in 0..5 {
            db.insert_usage_event(&sample_event(
                &format!("p{}", i),
                "gpt4.openai",
                now - 500 + i,
                1,
                1,
            ))
            .await
            .unwrap();
        }

        let first = db
            .query_usage(&QueryUsageRequest {
                time_range: UsageQueryTimeRange::Last1d,
                filters: UsageQueryFilters::default(),
                group_by: vec![],
                time_bucket: None,
                output_mode: UsageQueryOutputMode::Events,
                limit: Some(2),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(first.events.len(), 2);
        assert!(first.events[0].created_at_ms > first.events[1].created_at_ms);
        let cursor = first.next_cursor.clone().expect("cursor after first page");

        let second = db
            .query_usage(&QueryUsageRequest {
                time_range: UsageQueryTimeRange::Last1d,
                filters: UsageQueryFilters::default(),
                group_by: vec![],
                time_bucket: None,
                output_mode: UsageQueryOutputMode::Events,
                limit: Some(2),
                cursor: Some(cursor),
            })
            .await
            .unwrap();
        assert_eq!(second.events.len(), 2);
        assert!(second.next_cursor.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn insert_and_query_route_traces() {
        let (db, _tmp) = setup().await;
        let now = current_time_ms();
        let trace = AiccRouteTraceEvent {
            trace_id: "trace-one".to_string(),
            tenant_id: "alice".to_string(),
            caller_app_id: Some("sys_test".to_string()),
            task_id: "trace-one".to_string(),
            request_model: "llm.plan.default".to_string(),
            selected_exact_model: Some("gpt-5@test".to_string()),
            provider_instance_name: Some("test".to_string()),
            api_type: "llm".to_string(),
            route_trace_json: json!({
                "request_id": "trace-one",
                "api_type": "llm",
                "requested_model": "llm.plan.default",
                "selected_exact_model": "gpt-5@test"
            }),
            created_at_ms: now,
        };

        assert!(db.insert_route_trace_event(&trace).await.unwrap());
        assert!(!db.insert_route_trace_event(&trace).await.unwrap());
        let sibling = AiccRouteTraceEvent {
            trace_id: "trace-two".to_string(),
            task_id: "trace-one".to_string(),
            route_trace_json: json!({
                "request_id": "trace-two",
                "api_type": "llm",
                "requested_model": "llm.plan.default",
                "selected_exact_model": "gpt-5-mini@test"
            }),
            created_at_ms: now + 1,
            ..trace.clone()
        };
        assert!(db.insert_route_trace_event(&sibling).await.unwrap());

        let resp = db
            .query_route_traces(&QueryRouteTraceRequest {
                limit: Some(20),
                cursor: None,
                task_ids: vec![],
                request_ids: vec![],
            })
            .await
            .unwrap();
        assert_eq!(resp.traces.len(), 2);
        assert_eq!(
            resp.traces[0].get("request_id").and_then(Value::as_str),
            Some("trace-two")
        );
        assert_eq!(
            resp.traces[1].get("request_id").and_then(Value::as_str),
            Some("trace-one")
        );

        let task_resp = db
            .query_route_traces(&QueryRouteTraceRequest {
                limit: Some(20),
                cursor: None,
                task_ids: vec!["trace-one".to_string()],
                request_ids: vec![],
            })
            .await
            .unwrap();
        assert_eq!(task_resp.traces.len(), 2);

        let request_resp = db
            .query_route_traces(&QueryRouteTraceRequest {
                limit: Some(20),
                cursor: None,
                task_ids: vec![],
                request_ids: vec!["trace-two".to_string()],
            })
            .await
            .unwrap();
        assert_eq!(request_resp.traces.len(), 1);
        assert_eq!(
            request_resp.traces[0].get("request_id").and_then(Value::as_str),
            Some("trace-two")
        );
    }

    #[test]
    fn rewrite_placeholders_handles_quotes() {
        assert_eq!(
            rewrite_placeholders_to_dollar("SELECT ? FROM t WHERE s = '?' AND x = ?"),
            "SELECT $1 FROM t WHERE s = '?' AND x = $2"
        );
    }
}
