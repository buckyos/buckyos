//! Round statistics (D6): one Round is one inference attempt the runner makes
//! through the run's `LlmClient::infer`, whatever its result. Counted at the
//! host boundary, not from `WorkEvent::LLMStarted` (once per `run()` call)
//! nor from outcomes. Retries inside a provider adapter are part of one
//! Round; history summarization uses its own client and is not counted.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::AiResponse;
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::LLMComputeError;

use crate::fsutil;
use crate::protocol::UsageRecord;

/// Rounds since the last [`RoundCounter::take`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoundCounts {
    /// Every `infer` call (successful, failed and interrupted).
    pub attempts: u64,
    /// Calls that returned an error.
    pub failed: u64,
    /// Calls abandoned in flight (future dropped by an interrupt, or the
    /// provider reported the cancellation).
    pub interrupted: u64,
}

#[derive(Debug, Default)]
pub struct RoundCounter {
    attempts: AtomicU64,
    failed: AtomicU64,
    interrupted: AtomicU64,
}

impl RoundCounter {
    /// Counts since the previous call; resets them.
    pub fn take(&self) -> RoundCounts {
        RoundCounts {
            attempts: self.attempts.swap(0, Ordering::SeqCst),
            failed: self.failed.swap(0, Ordering::SeqCst),
            interrupted: self.interrupted.swap(0, Ordering::SeqCst),
        }
    }
}

/// Where the usage of the run's Rounds is appended (the session's
/// `usage.jsonl`).
#[derive(Debug, Clone)]
pub struct UsageLog {
    pub path: PathBuf,
    pub run_id: String,
}

impl UsageLog {
    /// Best effort: statistics never fail a Round.
    fn append(&self, alias: &str, response: &AiResponse) {
        let Some(usage) = &response.usage else { return };
        let input = usage.input_tokens.unwrap_or(0);
        let output = usage.output_tokens.unwrap_or(0);
        let total = usage.total_tokens.unwrap_or(input + output);
        if total == 0 {
            return;
        }
        let model = response
            .extra
            .as_ref()
            .and_then(|e| e["model"].as_str())
            .filter(|m| !m.is_empty())
            .unwrap_or(alias);
        let record = UsageRecord {
            at_ms: crate::now_ms(),
            model: model.to_string(),
            run_id: Some(self.run_id.clone()),
            input_tokens: input,
            output_tokens: output,
            total_tokens: total,
        };
        let result = fsutil::to_json_lines(&[record]).and_then(|l| fsutil::append_batch(&self.path, &l));
        if let Err(e) = result {
            log::warn!("usage of run {}: {e}", self.run_id);
        }
    }
}

/// Wraps the run's client; every call is one Round.
pub struct CountingLlm {
    inner: Arc<dyn LlmClient>,
    counter: Arc<RoundCounter>,
    usage: Option<UsageLog>,
}

impl CountingLlm {
    pub fn new(inner: Arc<dyn LlmClient>, counter: Arc<RoundCounter>) -> Self {
        Self {
            inner,
            counter,
            usage: None,
        }
    }

    pub fn with_usage(mut self, usage: UsageLog) -> Self {
        self.usage = Some(usage);
        self
    }
}

/// Marks the call interrupted unless it completed (the waist drops the
/// inference future when it is preempted).
struct InFlight<'a> {
    counter: &'a RoundCounter,
    done: bool,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.counter.interrupted.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[async_trait]
impl LlmClient for CountingLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.counter.attempts.fetch_add(1, Ordering::SeqCst);
        let mut guard = InFlight {
            counter: &self.counter,
            done: false,
        };
        let aborted = req.abort.clone();
        let alias = self.usage.as_ref().map(|_| req.model_alias.clone());
        let result = self.inner.infer(req).await;
        guard.done = true;
        if let (Some(log), Some(alias), Ok(response)) = (&self.usage, &alias, &result) {
            log.append(alias, response);
        }
        if let Err(e) = &result {
            if matches!(e, LLMComputeError::Cancelled) || aborted.is_aborted() {
                self.counter.interrupted.fetch_add(1, Ordering::SeqCst);
            } else {
                self.counter.failed.fetch_add(1, Ordering::SeqCst);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use llm_context::InferenceAbortToken;

    use super::*;

    struct Slow;

    #[async_trait]
    impl LlmClient for Slow {
        async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
            match req.model_alias.as_str() {
                "fail" => Err(LLMComputeError::Internal("boom".into())),
                "slow" => {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(AiResponse::default())
                }
                _ => Ok(AiResponse::default()),
            }
        }
    }

    fn req(model: &str) -> LlmInferenceRequest {
        LlmInferenceRequest {
            trace_id: None,
            messages: Vec::new(),
            model_alias: model.to_string(),
            fallbacks: Vec::new(),
            temperature: None,
            max_completion_tokens: None,
            force_json: false,
            json_schema: None,
            provider_options: None,
            disable_capabilities: Vec::new(),
            tool_specs: Vec::new(),
            allow_tool_calls: false,
            abort: InferenceAbortToken::noop(),
        }
    }

    #[tokio::test]
    async fn every_attempt_is_a_round_and_failures_are_classified() {
        let counter = Arc::new(RoundCounter::default());
        let llm = CountingLlm::new(Arc::new(Slow), counter.clone());
        assert!(llm.infer(req("ok")).await.is_ok());
        assert!(llm.infer(req("fail")).await.is_err());
        // An inference dropped in flight (the waist's interrupt path).
        let dropped = tokio::time::timeout(Duration::from_millis(20), llm.infer(req("slow"))).await;
        assert!(dropped.is_err());
        assert_eq!(
            counter.take(),
            RoundCounts {
                attempts: 3,
                failed: 1,
                interrupted: 1
            }
        );
        assert_eq!(counter.take(), RoundCounts::default(), "take resets");
    }
}
