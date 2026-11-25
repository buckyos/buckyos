use super::*;
use crate::execution::{ExecutionRecord, ProviderBillingEvent};
use crate::protocol::HttpRequest;
mod probe;

const BILLING_OPERATION: &str = "billing.events";
const BILLING_LIMIT: usize = 1024 * 1024;

#[derive(Debug)]
struct BillingLookupError {
    kind: &'static str,
    retry_after_ms: Option<u64>,
}

impl From<&'static str> for BillingLookupError {
    fn from(kind: &'static str) -> Self {
        Self {
            kind,
            retry_after_ms: None,
        }
    }
}

fn billing_request(
    record: &ExecutionRecord,
    now: u64,
    base_url: &str,
) -> Result<HttpRequest, BillingLookupError> {
    let binding = record.binding.as_ref().ok_or("missing_binding")?;
    let request_id = binding
        .remote_task_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or("missing_request_id")?;
    let mut url = reqwest::Url::parse(&format!(
        "{}/models/billing-events",
        base_url.trim_end_matches('/')
    ))
    .map_err(|_| "invalid_billing_url")?;
    let day = 86_400_000;
    let start = record.created_at_ms / day;
    let start = start.saturating_sub(1);
    let end = (now / day + 1).min(start + 90).max(start + 1);
    url.query_pairs_mut()
        .append_pair("request_id", request_id)
        .append_pair("endpoint_id", &binding.provider_model_id)
        .append_pair("source", "billed")
        .append_pair("limit", "100")
        .append_pair("start", &utc_date(start))
        .append_pair("end", &utc_date(end));
    let mut request = HttpRequest::new(reqwest::Method::GET, url.to_string());
    request.timeout = Some(Duration::from_secs(15));
    request.max_response_bytes = Some(BILLING_LIMIT);
    Ok(request)
}

fn utc_date(days: u64) -> String {
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn parse_event(
    body: &Value,
    request_id: &str,
    endpoint_id: &str,
) -> Result<Option<ProviderBillingEvent>, BillingLookupError> {
    if body.get("has_more").and_then(Value::as_bool) == Some(true) {
        return Err("incomplete_page".into());
    }
    let rows = body
        .get("billing_events")
        .and_then(Value::as_array)
        .ok_or("invalid_response")?;
    let mut matched = rows.iter().filter(|row| {
        row.get("request_id").and_then(Value::as_str) == Some(request_id)
            && row.get("endpoint_id").and_then(Value::as_str) == Some(endpoint_id)
    });
    let Some(row) = matched.next() else {
        return Ok(None);
    };
    if matched.next().is_some() {
        return Err("duplicate_event".into());
    }
    let amount = row
        .get("cost_total")
        .and_then(Value::as_f64)
        .filter(|amount| amount.is_finite() && *amount >= 0.0)
        .ok_or("invalid_amount")?;
    if row
        .get("currency")
        .and_then(Value::as_str)
        .is_some_and(|currency| currency != "USD")
    {
        return Err("invalid_currency".into());
    }
    let quantity = match row.get("quantity") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|q| q.is_finite() && *q >= 0.0)
                .ok_or("invalid_quantity")?,
        ),
    };
    let unit = match row.get("unit") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_str().ok_or("invalid_unit")?.to_owned()),
    };
    Ok(Some(ProviderBillingEvent {
        request_id: request_id.into(),
        endpoint_id: endpoint_id.into(),
        quantity,
        unit,
        cost: buckyos_api::AiCost::new(amount, "USD"),
    }))
}

async fn fetch_event(
    transport: &HttpTransport,
    request: HttpRequest,
    request_id: &str,
    endpoint_id: &str,
) -> Result<Option<ProviderBillingEvent>, BillingLookupError> {
    let response = transport
        .send(request)
        .await
        .map_err(|_| BillingLookupError::from("network_error"))?;
    let retry_after_ms = response
        .retry_after
        .map(|delay| delay.as_millis().min(u128::from(u64::MAX)) as u64);
    if !response.status.is_success() {
        return Err(BillingLookupError {
            kind: match response.status.as_u16() {
                401 | 403 => "permission_denied",
                429 => "rate_limited",
                _ => "http_error",
            },
            retry_after_ms,
        });
    }
    let body = response
        .json(BILLING_LIMIT)
        .map_err(|_| BillingLookupError::from("invalid_response"))?;
    parse_event(&body, request_id, endpoint_id)
}

impl RuntimeProviderExecutionPort {
    pub(crate) fn spawn_billing_reconciler(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(10));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let Some(service) = weak.upgrade() else {
                    break;
                };
                if let Err(error) = service.reconcile_billing_once().await {
                    log::warn!("AICC billing reconciliation deferred: {error}");
                }
            }
        });
    }

    async fn lookup_fal_billing(
        &self,
        record: &ExecutionRecord,
    ) -> Result<Option<ProviderBillingEvent>, BillingLookupError> {
        let binding = record.binding.as_ref().ok_or("missing_binding")?;
        let expected = binding
            .resume
            .as_ref()
            .and_then(|resume| resume.credential.as_ref())
            .ok_or("credential_unavailable")?;
        let snapshot = self.runtime.capture().await;
        let provider = snapshot
            .providers
            .get(&binding.provider_instance_name)
            .ok_or("credential_unavailable")?;
        if provider.profile.provider_profile_id != "fal"
            || provider.config.credential.reference != expected.reference
            || credential_fingerprint(&provider.config.credential.reference) != expected.fingerprint
        {
            return Err("credential_unavailable".into());
        }
        let credential = provider
            .resolve_credential()
            .await
            .map_err(|_| BillingLookupError::from("credential_unavailable"))?;
        if resume_credential_kind(credential.audit().kind) != expected.kind {
            return Err("credential_unavailable".into());
        }
        let billing_base_url = provider
            .config
            .operation_base_urls
            .get(BILLING_OPERATION)
            .ok_or("billing_endpoint_unavailable")?;
        let mut request = billing_request(record, now_ms(), billing_base_url)?;
        credential
            .apply(&mut request.headers)
            .map_err(|_| BillingLookupError::from("credential_unavailable"))?;
        let transport = HttpTransport::new(HttpTransportConfig {
            request_timeout: Duration::from_secs(15),
            max_response_bytes: BILLING_LIMIT,
            ..Default::default()
        })
        .map_err(|_| BillingLookupError::from("transport_unavailable"))?;
        fetch_event(
            &transport,
            request,
            binding
                .remote_task_id
                .as_deref()
                .ok_or("missing_request_id")?,
            &binding.provider_model_id,
        )
        .await
    }

    async fn reconcile_billing_once(self: &Arc<Self>) -> crate::error::StorageResult<()> {
        let records = self.storage.pending_billings(now_ms(), 32).await?;
        stream::iter(records)
            .for_each_concurrent(4, |record| async move {
                let result = async {
                    let Some(mut claimed) = self.storage.claim_billing(&record, now_ms()).await?
                    else {
                        return Ok::<(), crate::error::StorageError>(());
                    };
                    if claimed.output.as_ref().is_some_and(|output| {
                        output.usage.cost_source.as_deref() == Some("provider_reported")
                    }) {
                        self.storage
                            .finish_billing(&claimed, None, None, None, None, now_ms())
                            .await?;
                        return Ok(());
                    }
                    let local_pending = claimed
                        .binding
                        .as_ref()
                        .and_then(|b| b.billing.as_ref())
                        .is_some_and(|b| b.status == crate::execution::BillingStatus::LocalPending);
                    let (event, error, retry_after) = if local_pending {
                        (None, None, None)
                    } else {
                        match self.lookup_fal_billing(&claimed).await {
                            Ok(event) => (event, None, None),
                            Err(error) => {
                                log::warn!(
                                    "AICC billing lookup deferred: task_id={} reason={}",
                                    claimed.task_id,
                                    error.kind
                                );
                                (None, Some(error.kind), error.retry_after_ms)
                            }
                        }
                    };
                    let exhausted = claimed
                        .binding
                        .as_ref()
                        .and_then(|b| b.billing.as_ref())
                        .is_some_and(|billing| billing.exhausted(claimed.created_at_ms, now_ms()));
                    if event.is_none()
                        && (local_pending
                            || exhausted
                            || matches!(
                                error,
                                Some(
                                    "permission_denied"
                                        | "billing_endpoint_unavailable"
                                        | "credential_unavailable"
                                )
                            ))
                    {
                        if !local_pending {
                            let Some(local_record) = self
                                .storage
                                .begin_local_billing(
                                    &claimed,
                                    error.unwrap_or(if exhausted {
                                        "billing_retries_exhausted"
                                    } else {
                                        "not_ready"
                                    }),
                                )
                                .await?
                            else {
                                return Ok(());
                            };
                            claimed = local_record;
                        }
                        let measured = tokio::time::timeout(
                            Duration::from_secs(40),
                            self.local_billing_usage(&claimed),
                        )
                        .await;
                        let (local, error) = match measured {
                            Ok(Ok(local)) => (Some(local), None),
                            Ok(Err(error)) => (None, Some(error.kind)),
                            Err(_) => (None, Some("probe_timeout")),
                        };
                        self.storage
                            .finish_billing(&claimed, None, local, error, None, now_ms())
                            .await?;
                        return Ok(());
                    }
                    self.storage
                        .finish_billing(&claimed, event, None, error, retry_after, now_ms())
                        .await?;
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    log::warn!(
                        "AICC billing retry retained: task_id={} error={error}",
                        record.task_id
                    );
                }
            })
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event() -> Value {
        json!({"request_id":"r/one","endpoint_id":"fal-ai/video-upscaler","quantity":3.2,"unit":"megapixel","cost_subtotal":0.1,"cost_discount":0.01,"cost_total":0.09})
    }

    #[test]
    fn billing_uses_final_discounted_usd_and_rejects_ambiguous_or_foreign_events() {
        let row = event();
        let body = json!({"billing_events":[row],"has_more":false});
        let parsed = parse_event(&body, "r/one", "fal-ai/video-upscaler")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.cost, buckyos_api::AiCost::new(0.09, "USD"));
        assert_eq!(parsed.quantity, Some(3.2));
        assert!(parse_event(&body, "foreign", "fal-ai/video-upscaler")
            .unwrap()
            .is_none());
        assert!(parse_event(&body, "r/one", "other/model")
            .unwrap()
            .is_none());
        assert!(parse_event(
            &json!({"billing_events":[event(),event()]}),
            "r/one",
            "fal-ai/video-upscaler"
        )
        .is_err());
        assert!(parse_event(
            &json!({"billing_events":[event()],"has_more":true}),
            "r/one",
            "fal-ai/video-upscaler"
        )
        .is_err());
        let mut bad = event();
        bad["cost_total"] = json!(-0.1);
        assert!(parse_event(
            &json!({"billing_events":[bad]}),
            "r/one",
            "fal-ai/video-upscaler"
        )
        .is_err());
        let mut free = event();
        free["cost_total"] = json!(0);
        free["quantity"] = Value::Null;
        free["unit"] = Value::Null;
        let free = parse_event(
            &json!({"billing_events":[free]}),
            "r/one",
            "fal-ai/video-upscaler",
        )
        .unwrap()
        .unwrap();
        assert_eq!(free.cost.amount, 0.0);
        assert_eq!(free.quantity, None);
    }

    #[test]
    fn utc_windows_can_address_old_requests_without_the_api_default_day_filter() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(18_321), "2020-02-29");
        assert_eq!(utc_date(20_779), "2026-11-22");
    }

    #[test]
    fn billing_request_pins_model_request_and_a_ninety_day_bounded_history_window() {
        let mut record:ExecutionRecord=serde_json::from_value(json!({
            "scope":{"tenant_id":"tenant","method":"chat.completions.create","key":"key"},
            "usage_event_id":"usage","trace_id":null,"user_id":"owner","caller_app_id":"app","request_model":"model","body_fingerprint":"fp","task_id":"task","event_ref":"events/task","state":"succeeded","output":null,"error":null,"created_at_ms":20776u64*86400000,"expires_at_ms":20777u64*86400000,
            "binding":{"runtime_generation":1,"origin_provider":"fal","exact_model":"model@fal","provider_model_id":"fal-ai/video-upscaler","provider_instance_name":"fal-main","protocol_adapter_id":"fal-queue","operation":"queue.submit","api_type":"llm","remote_task_id":"r/one","result_artifacts":{},"cancel_supported":false,"resume":null,"pricing":null,"reported_cost_currency":null}
        })).unwrap();
        let request = billing_request(&record, 20779 * 86400000, "https://api.fal.ai/v1").unwrap();
        let url = reqwest::Url::parse(&request.url).unwrap();
        assert_eq!(url.host_str(), Some("api.fal.ai"));
        let query: HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(query["request_id"], "r/one");
        assert_eq!(query["endpoint_id"], "fal-ai/video-upscaler");
        assert_eq!(query["start"], "2026-11-18");
        assert_eq!(query["end"], "2026-11-23");
        assert_eq!(query["source"], "billed");
        assert!(request.headers.is_empty());
        let mock_request =
            billing_request(&record, 20779 * 86400000, "http://127.0.0.1:18081/v1/").unwrap();
        let mock_url = reqwest::Url::parse(&mock_request.url).unwrap();
        assert_eq!(mock_url.host_str(), Some("127.0.0.1"));
        assert_eq!(mock_url.path(), "/v1/models/billing-events");
        record.binding.as_mut().unwrap().provider_model_id = "fal-ai/deepfilternet3".into();
        assert!(
            billing_request(&record, 20779 * 86400000, "https://api.fal.ai/v1")
                .unwrap()
                .url
                .contains("deepfilternet3")
        );
    }

    #[tokio::test]
    async fn billing_http_empty_then_ready_and_rate_limit_are_read_only() {
        use axum::{extract::Query, routing::get, Json, Router};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let app = Router::new()
            .route(
                "/v1/models/billing-events",
                get(move |Query(query): Query<HashMap<String, String>>| {
                    let counted = counted.clone();
                    async move {
                        assert_eq!(query["request_id"], "r/one");
                        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                            Json(json!({"billing_events":[],"has_more":false}))
                        } else {
                            Json(json!({"billing_events":[event()],"has_more":false}))
                        }
                    }
                }),
            )
            .route(
                "/limited",
                get(|| async {
                    (
                        [(axum::http::header::RETRY_AFTER, "120")],
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let transport = HttpTransport::new(Default::default()).unwrap();
        let url = format!("http://{address}/v1/models/billing-events?request_id=r%2Fone");
        assert!(fetch_event(
            &transport,
            HttpRequest::new(reqwest::Method::GET, url.clone()),
            "r/one",
            "fal-ai/video-upscaler"
        )
        .await
        .unwrap()
        .is_none());
        assert_eq!(
            fetch_event(
                &transport,
                HttpRequest::new(reqwest::Method::GET, url),
                "r/one",
                "fal-ai/video-upscaler"
            )
            .await
            .unwrap()
            .unwrap()
            .cost
            .amount,
            0.09
        );
        let error = fetch_event(
            &transport,
            HttpRequest::new(reqwest::Method::GET, format!("http://{address}/limited")),
            "r/one",
            "fal-ai/video-upscaler",
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, "rate_limited");
        assert_eq!(error.retry_after_ms, Some(120_000));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        server.abort();
    }
}
