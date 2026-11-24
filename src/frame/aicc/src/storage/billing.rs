use super::*;
use crate::execution::{BillingStatus, ProviderBillingEvent};

impl AiccStorage {
    pub(crate) async fn pending_billings(
        &self,
        now_ms: u64,
        limit: u32,
    ) -> StorageResult<Vec<ExecutionRecord>> {
        let json = |path: &str| match self.backend {
            RdbBackend::Sqlite => format!("json_extract(record_json,'$.{path}')"),
            RdbBackend::Postgres => {
                format!("record_json::jsonb #>> '{{{}}}'", path.replace('.', ","))
            }
        };
        let source = match self.backend {
            RdbBackend::Sqlite => "aicc_execution_record INDEXED BY idx_aicc_billing_due",
            RdbBackend::Postgres => "aicc_execution_record",
        };
        let sql = self.sql(&format!("SELECT state,record_json FROM {source} WHERE state='succeeded' AND {} IN ('pending','local_pending') AND CAST({} AS BIGINT)<=? ORDER BY CAST({} AS BIGINT),created_at_ms,task_id LIMIT ?",json("binding.billing.status"),json("binding.billing.next_attempt_ms"),json("binding.billing.next_attempt_ms")));
        sqlx::query(&sql)
            .bind(to_i64(now_ms)?)
            .bind(i64::from(limit.min(100)))
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(execution_from_row)
            .collect()
    }

    pub(crate) async fn claim_billing(
        &self,
        record: &ExecutionRecord,
        now_ms: u64,
    ) -> StorageResult<Option<ExecutionRecord>> {
        let mut claimed = record.clone();
        let Some(binding) = &mut claimed.binding else {
            return Ok(None);
        };
        let Some(billing) = &mut binding.billing else {
            return Ok(None);
        };
        if claimed.state != ExecutionState::Succeeded
            || billing.schema_version != 1
            || !matches!(
                billing.status,
                BillingStatus::Pending | BillingStatus::LocalPending
            )
            || billing.next_attempt_ms > now_ms
            || binding.protocol_adapter_id != crate::protocol::FAL_QUEUE_ADAPTER_ID
            || binding
                .remote_task_id
                .as_ref()
                .is_none_or(|id| id.is_empty())
        {
            return Ok(None);
        }
        billing.attempts = billing.attempts.saturating_add(1);
        billing.next_attempt_ms = now_ms.saturating_add(60_000);
        let sql = self.sql("UPDATE aicc_execution_record SET record_json=? WHERE task_id=? AND state='succeeded' AND record_json=?");
        let changed = sqlx::query(&sql)
            .bind(serde_json::to_string(&claimed)?)
            .bind(&record.task_id)
            .bind(serde_json::to_string(record)?)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1;
        Ok(changed.then_some(claimed))
    }

    pub(crate) async fn finish_billing(
        &self,
        claimed: &ExecutionRecord,
        event: Option<ProviderBillingEvent>,
        local: Option<(AiUsage, Option<Value>)>,
        error: Option<&str>,
        retry_after_ms: Option<u64>,
        now_ms: u64,
    ) -> StorageResult<bool> {
        let mut record = claimed.clone();
        let Some(binding) = record.binding.as_mut() else {
            return Err(StorageError::InvalidRecord("billing has no binding".into()));
        };
        let Some(billing) = binding.billing.as_mut() else {
            return Err(StorageError::InvalidRecord("billing has no state".into()));
        };
        if !matches!(
            billing.status,
            BillingStatus::Pending | BillingStatus::LocalPending
        ) {
            return Ok(false);
        }
        let sql = self.sql("SELECT usage_json FROM aicc_usage_event WHERE event_id=? AND tenant_id=? AND task_id=? AND provider_instance_name=? AND provider_model=?");
        let mut transaction = match self.backend {
            RdbBackend::Sqlite => self.pool.begin_with("BEGIN IMMEDIATE").await?,
            RdbBackend::Postgres => self.pool.begin().await?,
        };
        let Some(row) = sqlx::query(&sql)
            .bind(&record.usage_event_id)
            .bind(&record.scope.tenant_id)
            .bind(&record.task_id)
            .bind(&binding.provider_instance_name)
            .bind(&binding.exact_model)
            .fetch_optional(&mut *transaction)
            .await?
        else {
            return Err(StorageError::InvalidRecord(
                "billing usage event is missing".into(),
            ));
        };
        let old_usage: String = row.try_get("usage_json")?;
        let mut usage: AiUsage = serde_json::from_str(&old_usage)?;
        let protocol_cost = usage.cost_source.as_deref() == Some("provider_reported")
            && usage.cost.as_ref().is_some_and(|cost| {
                cost.amount.is_finite() && cost.amount >= 0.0 && !cost.currency.trim().is_empty()
            });
        let mut replace_usage = false;
        if protocol_cost {
            billing.status = BillingStatus::ProtocolCost;
            billing.last_error = None;
        } else if let Some(event) = event {
            if event.request_id != binding.remote_task_id.as_deref().unwrap_or_default()
                || event.endpoint_id != binding.provider_model_id
                || event.cost.currency != "USD"
                || !event.cost.amount.is_finite()
                || event.cost.amount < 0.0
                || event
                    .quantity
                    .is_some_and(|value| !value.is_finite() || value < 0.0)
            {
                return Err(StorageError::InvalidRecord(
                    "billing event does not match execution".into(),
                ));
            }
            usage.cost = Some(event.cost.clone());
            usage.reported_cost = Some(event.cost.amount);
            usage.cost_source = Some("provider_billing".into());
            usage.cost_unknown_reason = None;
            if let Some(quantity) = event.quantity {
                match event.unit.as_deref() {
                    Some("megapixel") => usage.megapixels = usage.megapixels.or(Some(quantity)),
                    Some("audio_second") => {
                        usage.audio_seconds = usage.audio_seconds.or(Some(quantity))
                    }
                    Some("video_second") => {
                        usage.video_seconds = usage.video_seconds.or(Some(quantity))
                    }
                    Some("second") => {
                        let unit = match binding.pricing.as_ref().map(|p| &p.basis) {
                            Some(crate::execution::PinnedPricingBasis::Units { unit, .. }) => {
                                Some(*unit)
                            }
                            _ => None,
                        };
                        match unit {
                            Some(crate::catalog::PricingUnit::AudioSecond) => {
                                usage.audio_seconds = usage.audio_seconds.or(Some(quantity))
                            }
                            Some(crate::catalog::PricingUnit::VideoSecond) => {
                                usage.video_seconds = usage.video_seconds.or(Some(quantity))
                            }
                            Some(crate::catalog::PricingUnit::Second) => {
                                usage.compute_seconds = usage.compute_seconds.or(Some(quantity))
                            }
                            _ => {}
                        }
                    }
                    Some("image") if quantity.fract() == 0.0 && quantity < u64::MAX as f64 => {
                        usage.image_units = usage.image_units.or(Some(quantity as u64))
                    }
                    _ => {}
                }
            }
            if let Some(output) = &mut record.output {
                output.usage = usage.clone();
                output.cost = Some(event.cost.clone());
            }
            replace_usage = true;
            billing.event = Some(event);
            billing.status = BillingStatus::Applied;
            billing.last_error = None;
        } else if let Some((mut local_usage, measurement)) = local {
            let reason = local_usage.cost_unknown_reason.take();
            local_usage.cost = binding
                .pricing
                .as_ref()
                .and_then(|p| p.completion_cost(&local_usage));
            local_usage.cost_source = Some("static_pricing".into());
            if local_usage.cost.is_none() {
                local_usage.cost_unknown_reason =
                    reason.or_else(|| Some("missing_usage_or_pricing".into()));
            }
            usage = local_usage;
            if let Some(output) = &mut record.output {
                output.usage = usage.clone();
                output.cost = usage.cost.clone();
            }
            billing.measurement = measurement;
            billing.status = BillingStatus::Local;
            replace_usage = true;
        } else {
            billing.last_error = Some(error.unwrap_or("not_ready").to_owned());
            if billing.exhausted(record.created_at_ms, now_ms) {
                billing.status = BillingStatus::Unresolved;
                usage.cost_source = Some(
                    if usage.cost.is_some() {
                        "static_pricing"
                    } else {
                        "billing_unresolved"
                    }
                    .into(),
                );
                if usage.cost.is_none() {
                    usage.cost_unknown_reason = Some(format!(
                        "billing_retries_exhausted:{}",
                        billing.last_error.as_deref().unwrap()
                    ));
                }
                if let Some(output) = &mut record.output {
                    output.usage = usage.clone();
                    output.cost = usage.cost.clone();
                }
                replace_usage = true;
            }
            let delay = if matches!(error, Some("permission_denied" | "credential_unavailable")) {
                3_600_000
            } else {
                (10_000u64.saturating_mul(1u64 << billing.attempts.min(9))).min(3_600_000)
            };
            billing.next_attempt_ms = now_ms.saturating_add(delay.max(retry_after_ms.unwrap_or(0)));
        }
        let sql = self.sql("UPDATE aicc_execution_record SET record_json=? WHERE task_id=? AND state='succeeded' AND record_json=?");
        let changed = sqlx::query(&sql)
            .bind(serde_json::to_string(&record)?)
            .bind(&record.task_id)
            .bind(serde_json::to_string(claimed)?)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            == 1;
        if !changed {
            transaction.rollback().await?;
            return Ok(false);
        }
        if replace_usage {
            let cost = usage.cost.as_ref();
            let sql=self.sql("UPDATE aicc_usage_event SET usage_json=?,finance_snapshot_json=?,finance_amount=?,finance_currency=?,finance_valid=? WHERE event_id=? AND tenant_id=? AND usage_json=?");
            if sqlx::query(&sql)
                .bind(serde_json::to_string(&usage)?)
                .bind(cost.map(serde_json::to_string).transpose()?)
                .bind(cost.map(|cost| cost.amount))
                .bind(cost.map(|cost| cost.currency.as_str()))
                .bind(i64::from(cost.is_some()))
                .bind(&record.usage_event_id)
                .bind(&record.scope.tenant_id)
                .bind(old_usage)
                .execute(&mut *transaction)
                .await?
                .rows_affected()
                != 1
            {
                return Err(StorageError::InvalidRecord(
                    "billing usage changed concurrently".into(),
                ));
            }
        }
        transaction.commit().await?;
        Ok(true)
    }

    pub(crate) async fn begin_local_billing(
        &self,
        claimed: &ExecutionRecord,
        reason: &str,
    ) -> StorageResult<Option<ExecutionRecord>> {
        let mut record = claimed.clone();
        let Some(billing) = record
            .binding
            .as_mut()
            .and_then(|binding| binding.billing.as_mut())
        else {
            return Ok(None);
        };
        if billing.status != BillingStatus::Pending {
            return Ok(None);
        }
        billing.status = BillingStatus::LocalPending;
        billing.last_error = Some(reason.into());
        let sql = self.sql("UPDATE aicc_execution_record SET record_json=? WHERE task_id=? AND state='succeeded' AND record_json=?");
        let changed = sqlx::query(&sql)
            .bind(serde_json::to_string(&record)?)
            .bind(&record.task_id)
            .bind(serde_json::to_string(claimed)?)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1;
        Ok(changed.then_some(record))
    }
}
