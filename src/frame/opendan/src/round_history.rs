use std::collections::BTreeMap;
use std::fs::File as StdFile;
use std::io::{BufRead, BufReader};
use std::ops::{Range, RangeInclusive};
use std::path::{Path, PathBuf};

use buckyos_api::{AiContent, AiMessage, AiRole, AiUsage};
use chrono::{DateTime, Utc};
use llm_context::{
    state::LLMContextSnapshot, LLMBehaviorResult, LLMContextOutcome, Observation, StepRecord,
};
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub const SCHEMA_VERSION: u32 = 1;
pub const ROUND_HISTORY_DIR: &str = "round_history";
pub const META_DIR: &str = ".meta";
pub const ROUND_LOGS_FILE: &str = "round_logs.jsonl";

pub type HistoryResult<T> = Result<T, HistoryError>;

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("io: {path}: {err}")]
    Io {
        path: String,
        #[source]
        err: std::io::Error,
    },
    #[error("encode history json: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("decode history json: {path}:{line}: {err}")]
    Decode {
        path: String,
        line: usize,
        #[source]
        err: serde_json::Error,
    },
    #[error("round {0} not found")]
    RoundNotFound(u64),
    #[error("no open round")]
    NoOpenRound,
    #[error("round {0} is already open")]
    RoundAlreadyOpen(u64),
    #[error("round index must be greater than zero")]
    InvalidRoundIndex,
    #[error("history page must be -1 or a non-negative integer")]
    InvalidPage,
    #[error("history page_size must be greater than zero")]
    InvalidPageSize,
    #[error("history time range start must not be greater than end")]
    InvalidTimeRange,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    Chat,
    Behavior,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoundTrigger {
    UserMsg { preview: String },
    SystemEvent { source: String, event_kind: String },
    Mixed,
    Resume,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoundStatus {
    Open,
    Completed,
    Interrupted,
    Errored,
    WaitingTool,
}

impl RoundStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RoundStatus::Completed | RoundStatus::Interrupted | RoundStatus::Errored
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundSummary {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub round_index: u64,
    pub trigger: RoundTrigger,
    #[serde(default)]
    pub input_keys: Vec<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub status: RoundStatus,
    pub entry_count: u32,
    pub mode: ContextMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub seq: u32,
    pub ts: DateTime<Utc>,
    pub mode: ContextMode,
    pub payload: EntryPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryPayload {
    Message {
        message: AiMessage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        llm_call: Option<u64>,
    },
    Step {
        step: StepRecord,
        llm_call: u64,
    },
    Event {
        event: HistoryEvent,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HistoryEvent {
    SystemInput {
        source: String,
        payload: Value,
    },
    Outcome {
        kind: OutcomeKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        behavior_result: Option<LLMBehaviorResult>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage_delta: Option<AiUsage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Compaction {
        target: CompactionTarget,
        dropped: u32,
        kept_head: u32,
        kept_tail: u32,
        summary_preview: String,
    },
    Interrupt {
        mode: InterruptMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    SessionTopicUpdated {
        session_id: String,
        topic: String,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        tag_reasons: BTreeMap<String, String>,
        current_turn: u32,
        updated_at: DateTime<Utc>,
        topic_changed: bool,
    },
    Fork {
        child_label: String,
    },
    Join {
        child_label: String,
        outcome_kind: OutcomeKind,
    },
}

impl HistoryEvent {
    pub fn outcome_from_llm(outcome: &LLMContextOutcome) -> Self {
        let (kind, behavior_result, usage_delta, error) = match outcome {
            LLMContextOutcome::Done {
                behavior_result,
                usage,
                ..
            } => (
                OutcomeKind::Done,
                behavior_result.clone(),
                Some(usage.clone()),
                None,
            ),
            LLMContextOutcome::PendingTool { .. } => (OutcomeKind::PendingTool, None, None, None),
            LLMContextOutcome::BudgetExhausted { which, usage, .. } => (
                OutcomeKind::BudgetExhausted,
                None,
                Some(usage.clone()),
                Some(format!("budget exhausted: {which:?}")),
            ),
            LLMContextOutcome::Error { error, usage } => (
                OutcomeKind::Error,
                None,
                Some(usage.clone()),
                Some(error.to_string()),
            ),
            LLMContextOutcome::ContextLimitReached { which, usage, .. } => (
                OutcomeKind::ContextLimitReached,
                None,
                Some(usage.clone()),
                Some(format!("context limit reached: {which:?}")),
            ),
            LLMContextOutcome::Interrupted { reason, usage, .. } => (
                OutcomeKind::Interrupted,
                None,
                Some(usage.clone()),
                Some(reason.clone()),
            ),
        };
        HistoryEvent::Outcome {
            kind,
            behavior_result,
            usage_delta,
            error,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Done,
    PendingTool,
    ContextLimitReached,
    BudgetExhausted,
    Error,
    Interrupted,
}

impl OutcomeKind {
    pub fn from_llm_outcome(outcome: &LLMContextOutcome) -> Self {
        match outcome {
            LLMContextOutcome::Done { .. } => OutcomeKind::Done,
            LLMContextOutcome::PendingTool { .. } => OutcomeKind::PendingTool,
            LLMContextOutcome::BudgetExhausted { .. } => OutcomeKind::BudgetExhausted,
            LLMContextOutcome::Error { .. } => OutcomeKind::Error,
            LLMContextOutcome::ContextLimitReached { .. } => OutcomeKind::ContextLimitReached,
            LLMContextOutcome::Interrupted { .. } => OutcomeKind::Interrupted,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTarget {
    Accumulated,
    Steps,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptMode {
    Graceful,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryView {
    MsgOnly,
    Full,
    Raw,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundView {
    pub summary: RoundSummary,
    pub payload: RoundPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHistoryMessage {
    pub round_index: u64,
    pub seq: u32,
    pub ts: DateTime<Utc>,
    pub role: AiRole,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum SessionHistoryQuery {
    TimeRange {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },
    Page {
        page: i64,
        page_size: usize,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct SessionHistoryReadOptions {
    pub token_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHistoryReadResult {
    pub messages: Vec<SessionHistoryMessage>,
    pub total_candidates: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_round_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_round_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_round_index: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoundPayload {
    MsgOnly { messages: Vec<AiMessage> },
    Full(RoundFullPayload),
    Raw { entries: Vec<Entry> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RoundFullPayload {
    Chat { messages: Vec<AiMessage> },
    Behavior { steps: Vec<StepRecord> },
}

#[derive(Debug)]
struct OpenRound {
    summary: RoundSummary,
    file: File,
    path: PathBuf,
    next_seq: u32,
}

#[derive(Debug)]
pub struct SessionHistoryWriter {
    session_dir: PathBuf,
    history_dir: PathBuf,
    round_log_path: PathBuf,
    summaries: BTreeMap<u64, RoundSummary>,
    current: Option<OpenRound>,
}

impl SessionHistoryWriter {
    pub async fn open(session_dir: &Path) -> HistoryResult<Self> {
        let session_dir = session_dir.to_path_buf();
        let history_dir = session_dir.join(ROUND_HISTORY_DIR);
        let meta_dir = session_dir.join(META_DIR);
        let round_log_path = meta_dir.join(ROUND_LOGS_FILE);
        fs::create_dir_all(&history_dir)
            .await
            .map_err(|err| io_err(&history_dir, err))?;
        fs::create_dir_all(&meta_dir)
            .await
            .map_err(|err| io_err(&meta_dir, err))?;

        let mut writer = Self {
            summaries: read_round_summaries(&round_log_path)?,
            session_dir,
            history_dir,
            round_log_path,
            current: None,
        };
        writer.recover_index().await?;
        writer.reopen_waiting_round().await?;
        Ok(writer)
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn round_history_dir(&self) -> &Path {
        &self.history_dir
    }

    pub fn round_log_path(&self) -> &Path {
        &self.round_log_path
    }

    pub fn current_round(&self) -> Option<u64> {
        self.current.as_ref().map(|open| open.summary.round_index)
    }

    pub fn summaries(&self) -> &BTreeMap<u64, RoundSummary> {
        &self.summaries
    }

    pub async fn begin_round(
        &mut self,
        trigger: RoundTrigger,
        input_keys: Vec<String>,
        mode: ContextMode,
    ) -> HistoryResult<u64> {
        if let Some(index) = self.current_round() {
            return Err(HistoryError::RoundAlreadyOpen(index));
        }
        let round_index = self.summaries.keys().next_back().copied().unwrap_or(0) + 1;
        let path = round_file_path(&self.history_dir, round_index);
        let file = open_round_file(&path).await?;
        let summary = RoundSummary {
            schema_version: SCHEMA_VERSION,
            round_index,
            trigger,
            input_keys,
            started_at: Utc::now(),
            ended_at: None,
            status: RoundStatus::Open,
            entry_count: 0,
            mode,
        };
        append_json_line_to_path(&self.round_log_path, &summary, true).await?;
        self.summaries.insert(round_index, summary.clone());
        self.current = Some(OpenRound {
            summary,
            file,
            path,
            next_seq: 1,
        });
        Ok(round_index)
    }

    pub async fn append_message(
        &mut self,
        message: AiMessage,
        llm_call: Option<u64>,
    ) -> HistoryResult<()> {
        self.append_payload(EntryPayload::Message { message, llm_call })
            .await
    }

    pub async fn append_step(&mut self, step: StepRecord, llm_call: u64) -> HistoryResult<()> {
        self.append_payload(EntryPayload::Step { step, llm_call })
            .await
    }

    pub async fn append_event(&mut self, event: HistoryEvent) -> HistoryResult<()> {
        self.append_payload(EntryPayload::Event { event }).await
    }

    pub async fn finalize_round(&mut self, status: RoundStatus) -> HistoryResult<()> {
        let open = self.current.as_mut().ok_or(HistoryError::NoOpenRound)?;
        open.file
            .sync_all()
            .await
            .map_err(|err| io_err(&open.path, err))?;
        open.summary.status = status;
        if status.is_terminal() {
            open.summary.ended_at = Some(Utc::now());
        }
        append_json_line_to_path(&self.round_log_path, &open.summary, true).await?;
        self.summaries
            .insert(open.summary.round_index, open.summary.clone());
        if status.is_terminal() {
            self.current = None;
        }
        Ok(())
    }

    async fn append_payload(&mut self, payload: EntryPayload) -> HistoryResult<()> {
        let open = self.current.as_mut().ok_or(HistoryError::NoOpenRound)?;
        let entry = Entry {
            schema_version: SCHEMA_VERSION,
            seq: open.next_seq,
            ts: Utc::now(),
            mode: open.summary.mode,
            payload,
        };
        append_json_line_to_file(&mut open.file, &entry).await?;
        open.next_seq += 1;
        open.summary.entry_count += 1;
        self.summaries
            .insert(open.summary.round_index, open.summary.clone());
        Ok(())
    }

    async fn recover_index(&mut self) -> HistoryResult<()> {
        let files = scan_round_files(&self.history_dir)?;
        for (round_index, file_info) in files {
            match self.summaries.get_mut(&round_index) {
                Some(summary) => {
                    summary.entry_count = summary.entry_count.max(file_info.entry_count);
                    if summary.status == RoundStatus::Open {
                        summary.status = RoundStatus::Errored;
                        summary.ended_at = Some(Utc::now());
                        append_json_line_to_path(&self.round_log_path, summary, true).await?;
                    }
                }
                None => {
                    let summary = RoundSummary {
                        schema_version: SCHEMA_VERSION,
                        round_index,
                        trigger: RoundTrigger::Resume,
                        input_keys: Vec::new(),
                        started_at: Utc::now(),
                        ended_at: Some(Utc::now()),
                        status: RoundStatus::Errored,
                        entry_count: file_info.entry_count,
                        mode: file_info.mode.unwrap_or(ContextMode::Chat),
                    };
                    append_json_line_to_path(&self.round_log_path, &summary, true).await?;
                    self.summaries.insert(round_index, summary);
                }
            }
        }
        Ok(())
    }

    async fn reopen_waiting_round(&mut self) -> HistoryResult<()> {
        let Some((&round_index, summary)) = self
            .summaries
            .iter()
            .rev()
            .find(|(_, summary)| summary.status == RoundStatus::WaitingTool)
        else {
            return Ok(());
        };
        let path = round_file_path(&self.history_dir, round_index);
        let file = open_round_file(&path).await?;
        self.current = Some(OpenRound {
            summary: summary.clone(),
            file,
            path,
            next_seq: summary.entry_count + 1,
        });
        Ok(())
    }
}

pub struct SessionHistoryRecorder {
    session_id: String,
    session_dir: PathBuf,
    writer: Mutex<Option<SessionHistoryWriter>>,
}

impl SessionHistoryRecorder {
    pub fn new(session_id: String, session_dir: PathBuf) -> Self {
        Self {
            session_id,
            session_dir,
            writer: Mutex::new(None),
        }
    }

    async fn ensure_writer(
        &self,
    ) -> Option<tokio::sync::MutexGuard<'_, Option<SessionHistoryWriter>>> {
        let mut guard = self.writer.lock().await;
        if guard.is_none() {
            match SessionHistoryWriter::open(&self.session_dir).await {
                Ok(w) => *guard = Some(w),
                Err(err) => {
                    warn!(
                        "opendan.session[{}]: open round-history writer failed: {err}",
                        self.session_id
                    );
                    return None;
                }
            }
        }
        Some(guard)
    }

    pub async fn begin_round(
        &self,
        trigger: RoundTrigger,
        input_keys: Vec<String>,
        mode: ContextMode,
    ) -> Option<u64> {
        let mut guard = self.ensure_writer().await?;
        let writer = guard.as_mut().expect("history writer initialised");
        match writer.begin_round(trigger, input_keys, mode).await {
            Ok(idx) => Some(idx),
            Err(err) => {
                warn!(
                    "opendan.session[{}]: history begin_round failed: {err}",
                    self.session_id
                );
                None
            }
        }
    }

    pub async fn append_message(&self, message: AiMessage, llm_call: Option<u64>) {
        let Some(mut guard) = self.ensure_writer().await else {
            return;
        };
        let writer = guard.as_mut().expect("history writer initialised");
        if let Err(err) = writer.append_message(message, llm_call).await {
            warn!(
                "opendan.session[{}]: history append_message failed: {err}",
                self.session_id
            );
        }
    }

    pub async fn append_step(&self, step: StepRecord, llm_call: u64) {
        let Some(mut guard) = self.ensure_writer().await else {
            return;
        };
        let writer = guard.as_mut().expect("history writer initialised");
        if let Err(err) = writer.append_step(step, llm_call).await {
            warn!(
                "opendan.session[{}]: history append_step failed: {err}",
                self.session_id
            );
        }
    }

    pub async fn append_event(&self, event: HistoryEvent) {
        let Some(mut guard) = self.ensure_writer().await else {
            return;
        };
        let writer = guard.as_mut().expect("history writer initialised");
        if let Err(err) = writer.append_event(event).await {
            warn!(
                "opendan.session[{}]: history append_event failed: {err}",
                self.session_id
            );
        }
    }

    pub async fn finalize_round(&self, status: RoundStatus) {
        let Some(mut guard) = self.ensure_writer().await else {
            return;
        };
        let writer = guard.as_mut().expect("history writer initialised");
        if let Err(err) = writer.finalize_round(status).await {
            warn!(
                "opendan.session[{}]: history finalize_round failed: {err}",
                self.session_id
            );
        }
    }

    pub async fn current_round(&self) -> Option<u64> {
        let guard = self.ensure_writer().await?;
        guard.as_ref().and_then(|w| w.current_round())
    }

    pub async fn append_outcome(&self, outcome: &LLMContextOutcome) {
        self.append_event(HistoryEvent::outcome_from_llm(outcome))
            .await;
    }

    pub fn round_status_for(outcome: &LLMContextOutcome) -> Option<RoundStatus> {
        match outcome {
            LLMContextOutcome::Done { .. } => Some(RoundStatus::Completed),
            LLMContextOutcome::PendingTool { .. } => Some(RoundStatus::WaitingTool),
            LLMContextOutcome::BudgetExhausted { .. } | LLMContextOutcome::Error { .. } => {
                Some(RoundStatus::Errored)
            }
            LLMContextOutcome::Interrupted { .. } => Some(RoundStatus::Interrupted),
            LLMContextOutcome::ContextLimitReached { .. } => None,
        }
    }

    pub async fn record_run_diff(
        &self,
        mode: ContextMode,
        baseline_accumulated_len: usize,
        baseline_steps_len: usize,
        baseline_last_step_text: Option<String>,
        final_snapshot: &LLMContextSnapshot,
        outcome: &LLMContextOutcome,
        llm_call: u64,
    ) {
        match mode {
            ContextMode::Chat => {
                let accumulated = &final_snapshot.state.accumulated;
                let mut already_emitted: Option<String> = None;
                if accumulated.len() > baseline_accumulated_len {
                    for msg in &accumulated[baseline_accumulated_len..] {
                        already_emitted = msg.content.iter().rev().find_map(|c| match c {
                            AiContent::Text { text } => Some(text.clone()),
                            _ => None,
                        });
                        self.append_message(msg.clone(), Some(llm_call)).await;
                    }
                }
                if let LLMContextOutcome::Done { response, .. } = outcome {
                    let text = response.message.text_content();
                    let dup = already_emitted
                        .as_deref()
                        .map(|t| t == text.as_str())
                        .unwrap_or(false);
                    if !dup {
                        self.append_message(response.message.clone(), Some(llm_call))
                            .await;
                    }
                }
            }
            ContextMode::Behavior => {
                let steps = &final_snapshot.state.steps;
                if steps.len() > baseline_steps_len {
                    for step in &steps[baseline_steps_len..] {
                        self.append_step(step.clone(), llm_call).await;
                    }
                }
                if let Some(last) = final_snapshot.state.last_step.as_ref() {
                    let is_new = baseline_last_step_text
                        .as_deref()
                        .map(|prev| prev != last.assistant_text.as_str())
                        .unwrap_or(true);
                    if is_new {
                        self.append_step(last.clone(), llm_call).await;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionHistoryReader {
    session_dir: PathBuf,
}

impl SessionHistoryReader {
    pub fn open(session_dir: &Path) -> HistoryResult<Self> {
        Ok(Self {
            session_dir: session_dir.to_path_buf(),
        })
    }

    pub fn list_rounds(&self, range: Option<Range<u64>>) -> HistoryResult<Vec<RoundSummary>> {
        let summaries = self.read_summaries()?;
        Ok(summaries
            .into_iter()
            .filter(|(index, _)| match &range {
                Some(range) => range.contains(index),
                None => true,
            })
            .map(|(_, summary)| summary)
            .collect())
    }

    pub fn latest_round_index(&self) -> HistoryResult<Option<u64>> {
        Ok(self.read_summaries()?.keys().next_back().copied())
    }

    pub fn read_round(&self, round_index: u64, view: HistoryView) -> HistoryResult<RoundView> {
        if round_index == 0 {
            return Err(HistoryError::InvalidRoundIndex);
        }
        let summaries = self.read_summaries()?;
        let summary = summaries
            .get(&round_index)
            .cloned()
            .ok_or(HistoryError::RoundNotFound(round_index))?;
        let entries = read_entries(&round_file_path(&self.history_dir(), round_index))?;
        let payload = match view {
            HistoryView::Raw => RoundPayload::Raw { entries },
            HistoryView::Full => RoundPayload::Full(full_payload(summary.mode, &entries)),
            HistoryView::MsgOnly => RoundPayload::MsgOnly {
                messages: msgonly_payload(summary.mode, &entries),
            },
        };
        Ok(RoundView { summary, payload })
    }

    pub fn read_range(
        &self,
        from: u64,
        to: u64,
        view: HistoryView,
    ) -> HistoryResult<Vec<RoundView>> {
        if from == 0 || to == 0 {
            return Err(HistoryError::InvalidRoundIndex);
        }
        if from > to {
            return Ok(Vec::new());
        }
        let summaries = self.read_summaries()?;
        let mut out = Vec::new();
        for round_index in from..=to {
            if summaries.contains_key(&round_index) {
                out.push(self.read_round(round_index, view)?);
            }
        }
        Ok(out)
    }

    pub fn read_session_messages(
        &self,
        query: SessionHistoryQuery,
        options: SessionHistoryReadOptions,
    ) -> HistoryResult<SessionHistoryReadResult> {
        let mut messages = self.collect_session_messages()?;
        match query {
            SessionHistoryQuery::TimeRange { start, end } => {
                if start > end {
                    return Err(HistoryError::InvalidTimeRange);
                }
                messages.retain(|msg| msg.ts >= start && msg.ts <= end);
            }
            SessionHistoryQuery::Page { page, page_size } => {
                if page < -1 {
                    return Err(HistoryError::InvalidPage);
                }
                if page_size == 0 {
                    return Err(HistoryError::InvalidPageSize);
                }
                messages = slice_history_page(messages, page, page_size);
            }
        }

        let total_candidates = messages.len();
        let (messages, token_truncated) = apply_history_token_limit(messages, options.token_limit);
        Ok(SessionHistoryReadResult {
            truncated: token_truncated || messages.len() < total_candidates,
            first_round_index: messages.first().map(|msg| msg.round_index),
            last_round_index: messages.last().map(|msg| msg.round_index),
            latest_round_index: self.latest_round_index()?,
            messages,
            total_candidates,
        })
    }

    pub fn read_session_messages_from_round_index(
        &self,
        start_round_index: u64,
        options: SessionHistoryReadOptions,
    ) -> HistoryResult<SessionHistoryReadResult> {
        let summaries = self.read_summaries()?;
        let latest_round_index = summaries.keys().next_back().copied();
        if options.token_limit == 0 {
            let total_candidates = summaries
                .keys()
                .copied()
                .filter(|round_index| *round_index >= start_round_index)
                .try_fold(0usize, |acc, round_index| {
                    let entries = read_entries(&round_file_path(&self.history_dir(), round_index))?;
                    Ok::<usize, HistoryError>(
                        acc + entries
                            .iter()
                            .flat_map(|entry| session_messages_from_entry(round_index, entry))
                            .count(),
                    )
                })?;
            return Ok(SessionHistoryReadResult {
                messages: Vec::new(),
                total_candidates,
                truncated: total_candidates > 0,
                first_round_index: None,
                last_round_index: None,
                latest_round_index,
            });
        }
        let mut messages = Vec::new();
        let mut total_candidates = 0usize;
        let mut used_tokens = 0u32;
        let mut truncated = false;

        for round_index in summaries.keys().copied() {
            if round_index < start_round_index {
                continue;
            }
            let entries = read_entries(&round_file_path(&self.history_dir(), round_index))?;
            let round_messages = entries
                .iter()
                .flat_map(|entry| session_messages_from_entry(round_index, entry))
                .collect::<Vec<_>>();
            if round_messages.is_empty() {
                continue;
            }
            total_candidates += round_messages.len();
            let round_tokens = round_messages
                .iter()
                .map(estimate_history_message_tokens)
                .fold(0u32, u32::saturating_add);
            if !messages.is_empty()
                && used_tokens.saturating_add(round_tokens) > options.token_limit
            {
                truncated = true;
                break;
            }
            used_tokens = used_tokens.saturating_add(round_tokens);
            messages.extend(round_messages);
            if used_tokens >= options.token_limit {
                truncated = summaries.keys().any(|idx| *idx > round_index);
                break;
            }
        }

        let first_round_index = messages.first().map(|msg| msg.round_index);
        let last_round_index = messages.last().map(|msg| msg.round_index);
        Ok(SessionHistoryReadResult {
            messages,
            total_candidates,
            truncated,
            first_round_index,
            last_round_index,
            latest_round_index,
        })
    }

    fn collect_session_messages(&self) -> HistoryResult<Vec<SessionHistoryMessage>> {
        let summaries = self.read_summaries()?;
        let mut out = Vec::new();
        for round_index in summaries.keys().copied() {
            let entries = read_entries(&round_file_path(&self.history_dir(), round_index))?;
            for entry in entries {
                out.extend(session_messages_from_entry(round_index, &entry));
            }
        }
        Ok(out)
    }

    fn history_dir(&self) -> PathBuf {
        self.session_dir.join(ROUND_HISTORY_DIR)
    }

    fn round_log_path(&self) -> PathBuf {
        self.session_dir.join(META_DIR).join(ROUND_LOGS_FILE)
    }

    fn read_summaries(&self) -> HistoryResult<BTreeMap<u64, RoundSummary>> {
        read_round_summaries(&self.round_log_path())
    }
}

#[derive(Debug, Clone)]
struct RoundFileInfo {
    entry_count: u32,
    mode: Option<ContextMode>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

fn round_file_path(history_dir: &Path, round_index: u64) -> PathBuf {
    history_dir.join(format!("{round_index:06}.jsonl"))
}

async fn open_round_file(path: &Path) -> HistoryResult<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .await
        .map_err(|err| io_err(path, err))
}

async fn append_json_line_to_path<T: Serialize>(
    path: &Path,
    value: &T,
    sync: bool,
) -> HistoryResult<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|err| io_err(path, err))?;
    append_json_line_to_file(&mut file, value).await?;
    if sync {
        file.sync_all().await.map_err(|err| io_err(path, err))?;
    }
    Ok(())
}

async fn append_json_line_to_file<T: Serialize>(file: &mut File, value: &T) -> HistoryResult<()> {
    let mut line = serde_json::to_vec(value).map_err(HistoryError::Encode)?;
    line.push(b'\n');
    file.write_all(&line).await.map_err(|err| HistoryError::Io {
        path: "<open history file>".to_string(),
        err,
    })
}

fn read_round_summaries(path: &Path) -> HistoryResult<BTreeMap<u64, RoundSummary>> {
    let mut out = BTreeMap::new();
    for summary in read_jsonl_lossy::<RoundSummary>(path)? {
        out.insert(summary.round_index, summary);
    }
    Ok(out)
}

fn read_entries(path: &Path) -> HistoryResult<Vec<Entry>> {
    read_jsonl_lossy(path)
}

fn read_jsonl_lossy<T>(path: &Path) -> HistoryResult<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let file = match StdFile::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(io_err(path, err)),
    };
    let mut out = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|err| io_err(path, err))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<T>(&line) {
            out.push(value);
        }
    }
    Ok(out)
}

fn scan_round_files(history_dir: &Path) -> HistoryResult<BTreeMap<u64, RoundFileInfo>> {
    let mut out = BTreeMap::new();
    let entries = match std::fs::read_dir(history_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(err) => return Err(io_err(history_dir, err)),
    };
    for entry in entries {
        let entry = entry.map_err(|err| io_err(history_dir, err))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(round_index) = stem.parse::<u64>() else {
            continue;
        };
        let parsed = read_entries(&path)?;
        let mode = parsed.first().map(|entry| entry.mode);
        out.insert(
            round_index,
            RoundFileInfo {
                entry_count: parsed.len() as u32,
                mode,
            },
        );
    }
    Ok(out)
}

fn full_payload(mode: ContextMode, entries: &[Entry]) -> RoundFullPayload {
    match mode {
        ContextMode::Chat => RoundFullPayload::Chat {
            messages: entries
                .iter()
                .filter_map(|entry| match &entry.payload {
                    EntryPayload::Message { message, .. } => Some(message.clone()),
                    _ => None,
                })
                .collect(),
        },
        ContextMode::Behavior => RoundFullPayload::Behavior {
            steps: entries
                .iter()
                .filter_map(|entry| match &entry.payload {
                    EntryPayload::Step { step, .. } => Some(step.clone()),
                    _ => None,
                })
                .collect(),
        },
    }
}

fn msgonly_payload(mode: ContextMode, entries: &[Entry]) -> Vec<AiMessage> {
    match mode {
        ContextMode::Chat => entries
            .iter()
            .filter_map(|entry| match &entry.payload {
                EntryPayload::Message { message, .. } => chat_msgonly_message(message),
                _ => None,
            })
            .collect(),
        ContextMode::Behavior => behavior_msgonly_messages(entries),
    }
}

fn session_messages_from_entry(round_index: u64, entry: &Entry) -> Vec<SessionHistoryMessage> {
    match &entry.payload {
        EntryPayload::Message { message, .. } => chat_msgonly_message(message)
            .map(|message| session_message_from_ai(round_index, entry, message))
            .into_iter()
            .collect(),
        EntryPayload::Step { step, .. } => behavior_step_assistant_text(step)
            .map(|text| SessionHistoryMessage {
                round_index,
                seq: entry.seq,
                ts: entry.ts,
                role: AiRole::Assistant,
                text,
            })
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn session_message_from_ai(
    round_index: u64,
    entry: &Entry,
    message: AiMessage,
) -> SessionHistoryMessage {
    SessionHistoryMessage {
        round_index,
        seq: entry.seq,
        ts: entry.ts,
        role: message.role,
        text: message.text_content(),
    }
}

fn slice_history_page(
    messages: Vec<SessionHistoryMessage>,
    page: i64,
    page_size: usize,
) -> Vec<SessionHistoryMessage> {
    if messages.is_empty() {
        return Vec::new();
    }
    let range: RangeInclusive<usize> = if page == -1 {
        let start = messages.len().saturating_sub(page_size);
        start..=messages.len() - 1
    } else {
        let start = page as usize * page_size;
        if start >= messages.len() {
            return Vec::new();
        }
        let end = (start + page_size).min(messages.len()) - 1;
        start..=end
    };
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(idx, msg)| range.contains(&idx).then_some(msg))
        .collect()
}

fn apply_history_token_limit(
    messages: Vec<SessionHistoryMessage>,
    token_limit: u32,
) -> (Vec<SessionHistoryMessage>, bool) {
    if token_limit == 0 {
        return (Vec::new(), !messages.is_empty());
    }
    let mut out = Vec::new();
    let mut used = 0u32;
    let mut truncated = false;
    for msg in messages {
        let cost = estimate_history_message_tokens(&msg);
        if used.saturating_add(cost) > token_limit && !out.is_empty() {
            truncated = true;
            break;
        }
        used = used.saturating_add(cost);
        out.push(msg);
        if used >= token_limit {
            truncated = true;
            break;
        }
    }
    (out, truncated)
}

fn estimate_history_message_tokens(msg: &SessionHistoryMessage) -> u32 {
    let text_tokens = (msg.text.chars().count() as u32).saturating_add(3) / 4;
    text_tokens.max(1).saturating_add(1)
}

fn chat_msgonly_message(message: &AiMessage) -> Option<AiMessage> {
    if !matches!(message.role, AiRole::User | AiRole::Assistant) {
        return None;
    }
    let blocks: Vec<AiContent> = message
        .content
        .iter()
        .filter_map(|block| match block {
            AiContent::Text { text } => Some(AiContent::Text { text: text.clone() }),
            AiContent::ProviderState { provider, value } if message.role == AiRole::Assistant => {
                Some(AiContent::ProviderState {
                    provider: provider.clone(),
                    value: value.clone(),
                })
            }
            _ => None,
        })
        .collect();
    if blocks.is_empty() {
        None
    } else {
        Some(AiMessage::new(message.role, blocks))
    }
}

fn behavior_msgonly_messages(entries: &[Entry]) -> Vec<AiMessage> {
    let mut out = Vec::new();
    for entry in entries {
        match &entry.payload {
            EntryPayload::Message { message, .. } if message.role == AiRole::User => {
                if let Some(message) = chat_msgonly_message(message) {
                    out.push(message);
                }
            }
            EntryPayload::Step { step, .. } => {
                if let Some(message) = step.assistant_message.clone() {
                    out.push(message);
                }
                if let Some(text) = behavior_step_observation_text(step) {
                    out.push(AiMessage::text(AiRole::Tool, text));
                }
            }
            _ => {}
        }
    }
    out
}

fn behavior_step_assistant_text(step: &StepRecord) -> Option<String> {
    let thought = step
        .thought
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let assistant = strip_xml_tags(&step.assistant_text);
    let assistant = assistant.trim();
    let mut parts = Vec::new();
    if let Some(thought) = thought {
        parts.push(thought.to_string());
    }
    if !assistant.is_empty() && Some(assistant) != thought {
        parts.push(assistant.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn behavior_step_observation_text(step: &StepRecord) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(observation) = step.observation.as_deref().map(str::trim) {
        if !observation.is_empty() {
            parts.push(observation.to_string());
        }
    }
    for result in &step.action_results {
        parts.push(render_observation(result));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn render_observation(observation: &Observation) -> String {
    match observation {
        Observation::Success {
            content, truncated, ..
        } => {
            let mut rendered = content
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| content.to_string());
            if *truncated {
                rendered.push_str("\n[truncated]");
            }
            rendered
        }
        Observation::Error { message, .. } => message.clone(),
        Observation::Pending { call_id, .. } => format!("pending: {call_id}"),
        Observation::Cancelled { reason, .. } => format!("cancelled: {reason}"),
    }
}

fn strip_xml_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn io_err(path: &Path, err: std::io::Error) -> HistoryError {
    HistoryError::Io {
        path: path.display().to_string(),
        err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;

    use buckyos_api::AiToolResultContent;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn session_topic_event_serializes_stable_type() {
        let event = HistoryEvent::SessionTopicUpdated {
            session_id: "s1".to_string(),
            topic: "Implement topic history".to_string(),
            tags: vec!["history".to_string()],
            tag_reasons: BTreeMap::from([(
                "history".to_string(),
                "topic timeline implementation".to_string(),
            )]),
            current_turn: 2,
            updated_at: "2026-05-19T00:00:00Z".parse().unwrap(),
            topic_changed: true,
        };

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "session_topic_updated");
        assert_eq!(value["session_id"], "s1");
        assert_eq!(value["topic"], "Implement topic history");
        assert_eq!(value["tags"], json!(["history"]));
        assert_eq!(value["current_turn"], 2);
        assert_eq!(value["topic_changed"], true);
    }

    #[tokio::test]
    async fn writer_reader_chat_round() {
        let dir = tempdir().unwrap();
        let mut writer = SessionHistoryWriter::open(dir.path()).await.unwrap();
        let round = writer
            .begin_round(
                RoundTrigger::UserMsg {
                    preview: "hello".to_string(),
                },
                vec!["msg:1".to_string()],
                ContextMode::Chat,
            )
            .await
            .unwrap();
        assert_eq!(round, 1);
        writer
            .append_message(AiMessage::text(AiRole::User, "hello"), None)
            .await
            .unwrap();
        writer
            .append_message(
                AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::Thinking {
                            summary: None,
                            text: Some("hidden".to_string()),
                            provider_metadata: None,
                        },
                        AiContent::Text {
                            text: "visible".to_string(),
                        },
                    ],
                ),
                Some(1),
            )
            .await
            .unwrap();
        writer
            .append_message(
                AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::ToolResult {
                        call_id: "call-1".to_string(),
                        content: vec![AiToolResultContent::Text {
                            text: "tool".to_string(),
                        }],
                        is_error: false,
                    }],
                ),
                Some(1),
            )
            .await
            .unwrap();
        writer
            .append_event(HistoryEvent::Outcome {
                kind: OutcomeKind::Done,
                behavior_result: None,
                usage_delta: None,
                error: None,
            })
            .await
            .unwrap();
        writer.finalize_round(RoundStatus::Completed).await.unwrap();

        let reader = SessionHistoryReader::open(dir.path()).unwrap();
        let rounds = reader.list_rounds(None).unwrap();
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].status, RoundStatus::Completed);
        assert_eq!(rounds[0].entry_count, 4);

        let raw = reader.read_round(1, HistoryView::Raw).unwrap();
        match raw.payload {
            RoundPayload::Raw { entries } => assert_eq!(entries.len(), 4),
            _ => panic!("expected raw payload"),
        }
        let msgonly = reader.read_round(1, HistoryView::MsgOnly).unwrap();
        match msgonly.payload {
            RoundPayload::MsgOnly { messages } => {
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].text_content(), "hello");
                assert_eq!(messages[1].text_content(), "visible");
            }
            _ => panic!("expected msgonly payload"),
        }
    }

    #[tokio::test]
    async fn waiting_tool_round_reopens_and_continues() {
        fn provider_message(id: &str, text: &str) -> AiMessage {
            AiMessage::new(
                AiRole::Assistant,
                vec![
                    AiContent::text(text),
                    AiContent::ProviderState {
                        provider: "openrouter".to_string(),
                        value: serde_json::json!({
                            "type": "message",
                            "id": id,
                            "status": "completed",
                            "role": "assistant",
                            "content": [{
                                "type": "output_text",
                                "text": text,
                                "annotations": [],
                            }],
                        }),
                    },
                ],
            )
        }

        let dir = tempdir().unwrap();
        {
            let mut writer = SessionHistoryWriter::open(dir.path()).await.unwrap();
            writer
                .begin_round(RoundTrigger::Resume, Vec::new(), ContextMode::Behavior)
                .await
                .unwrap();
            writer
                .append_step(
                    StepRecord {
                        assistant_text: "<thought>call tool</thought>".to_string(),
                        assistant_message: Some(provider_message(
                            "msg_1",
                            "<thought>call tool</thought>",
                        )),
                        thought: Some("call tool".to_string()),
                        ..Default::default()
                    },
                    1,
                )
                .await
                .unwrap();
            writer
                .finalize_round(RoundStatus::WaitingTool)
                .await
                .unwrap();
            assert_eq!(writer.current_round(), Some(1));
        }

        let mut writer = SessionHistoryWriter::open(dir.path()).await.unwrap();
        assert_eq!(writer.current_round(), Some(1));
        writer
            .append_step(
                StepRecord {
                    assistant_text: "done".to_string(),
                    assistant_message: Some(provider_message("msg_2", "done")),
                    observation: Some("ok".to_string()),
                    action_results: vec![Observation::Success {
                        call_id: "call-1".to_string(),
                        content: json!("ok"),
                        bytes: 2,
                        truncated: false,
                        tool_result: None,
                    }],
                    ..Default::default()
                },
                2,
            )
            .await
            .unwrap();
        writer.finalize_round(RoundStatus::Completed).await.unwrap();

        let reader = SessionHistoryReader::open(dir.path()).unwrap();
        let view = reader.read_round(1, HistoryView::Full).unwrap();
        assert_eq!(view.summary.status, RoundStatus::Completed);
        assert_eq!(view.summary.entry_count, 2);
        match view.payload {
            RoundPayload::Full(RoundFullPayload::Behavior { steps }) => {
                assert_eq!(steps.len(), 2);
                assert_eq!(steps[1].observation.as_deref(), Some("ok"));
                assert!(steps[0]
                    .assistant_message
                    .as_ref()
                    .unwrap()
                    .content
                    .iter()
                    .any(|content| matches!(content, AiContent::ProviderState { value, .. } if value["id"] == "msg_1")));
            }
            _ => panic!("expected behavior full payload"),
        }
        let msgonly = reader.read_round(1, HistoryView::MsgOnly).unwrap();
        match msgonly.payload {
            RoundPayload::MsgOnly { messages } => {
                assert_eq!(messages.len(), 3);
                assert_eq!(messages[0].role, AiRole::Assistant);
                assert_eq!(messages[1].role, AiRole::Assistant);
                assert_eq!(messages[2].role, AiRole::Tool);
                assert!(messages[1]
                    .content
                    .iter()
                    .any(|content| matches!(content, AiContent::ProviderState { value, .. } if value["id"] == "msg_2")));
            }
            _ => panic!("expected msgonly payload"),
        }
    }

    #[tokio::test]
    async fn reader_uses_last_summary_and_ignores_bad_tail() {
        let dir = tempdir().unwrap();
        let mut writer = SessionHistoryWriter::open(dir.path()).await.unwrap();
        writer
            .begin_round(RoundTrigger::Resume, Vec::new(), ContextMode::Chat)
            .await
            .unwrap();
        writer
            .append_message(AiMessage::text(AiRole::User, "one"), None)
            .await
            .unwrap();
        writer
            .finalize_round(RoundStatus::WaitingTool)
            .await
            .unwrap();
        writer.finalize_round(RoundStatus::Completed).await.unwrap();
        drop(writer);

        let history_path = dir.path().join(ROUND_HISTORY_DIR).join("000001.jsonl");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&history_path)
            .unwrap()
            .write_all(b"{bad tail\n")
            .unwrap();

        let reader = SessionHistoryReader::open(dir.path()).unwrap();
        let latest = reader.latest_round_index().unwrap();
        assert_eq!(latest, Some(1));
        let view = reader.read_round(1, HistoryView::Raw).unwrap();
        assert_eq!(view.summary.status, RoundStatus::Completed);
        match view.payload {
            RoundPayload::Raw { entries } => assert_eq!(entries.len(), 1),
            _ => panic!("expected raw payload"),
        }
    }

    #[tokio::test]
    async fn read_session_messages_filters_pages_and_truncates() {
        let dir = tempdir().unwrap();
        let mut writer = SessionHistoryWriter::open(dir.path()).await.unwrap();
        writer
            .begin_round(
                RoundTrigger::UserMsg {
                    preview: "first".to_string(),
                },
                vec!["msg:1".to_string()],
                ContextMode::Chat,
            )
            .await
            .unwrap();
        writer
            .append_message(AiMessage::text(AiRole::User, "first"), None)
            .await
            .unwrap();
        writer
            .append_message(
                AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::Text {
                            text: "second".to_string(),
                        },
                        AiContent::tool_use("call-1", "lookup", HashMap::new()),
                    ],
                ),
                Some(1),
            )
            .await
            .unwrap();
        writer
            .append_message(
                AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::tool_result_text("call-1", "hidden", false)],
                ),
                Some(1),
            )
            .await
            .unwrap();
        writer.finalize_round(RoundStatus::Completed).await.unwrap();

        writer
            .begin_round(
                RoundTrigger::UserMsg {
                    preview: "third".to_string(),
                },
                vec!["msg:2".to_string()],
                ContextMode::Chat,
            )
            .await
            .unwrap();
        writer
            .append_message(AiMessage::text(AiRole::System, "hidden system"), None)
            .await
            .unwrap();
        writer
            .append_message(AiMessage::text(AiRole::User, "third"), None)
            .await
            .unwrap();
        writer
            .append_message(AiMessage::text(AiRole::Assistant, "fourth"), Some(2))
            .await
            .unwrap();
        writer.finalize_round(RoundStatus::Completed).await.unwrap();
        drop(writer);

        let reader = SessionHistoryReader::open(dir.path()).unwrap();
        let page0 = reader
            .read_session_messages(
                SessionHistoryQuery::Page {
                    page: 0,
                    page_size: 2,
                },
                SessionHistoryReadOptions { token_limit: 4096 },
            )
            .unwrap();
        assert_eq!(page0.total_candidates, 2);
        assert_eq!(
            page0
                .messages
                .iter()
                .map(|msg| msg.text.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );

        let latest = reader
            .read_session_messages(
                SessionHistoryQuery::Page {
                    page: -1,
                    page_size: 2,
                },
                SessionHistoryReadOptions { token_limit: 4096 },
            )
            .unwrap();
        assert_eq!(
            latest
                .messages
                .iter()
                .map(|msg| msg.text.as_str())
                .collect::<Vec<_>>(),
            vec!["third", "fourth"]
        );

        let truncated = reader
            .read_session_messages(
                SessionHistoryQuery::Page {
                    page: 0,
                    page_size: 4,
                },
                SessionHistoryReadOptions { token_limit: 2 },
            )
            .unwrap();
        assert!(truncated.truncated);
        assert_eq!(truncated.messages.len(), 1);
        assert_eq!(truncated.messages[0].text, "first");
    }

    #[tokio::test]
    async fn read_session_messages_filters_exact_time_range() {
        let dir = tempdir().unwrap();
        let history_dir = dir.path().join(ROUND_HISTORY_DIR);
        let meta_dir = dir.path().join(META_DIR);
        std::fs::create_dir_all(&history_dir).unwrap();
        std::fs::create_dir_all(&meta_dir).unwrap();
        let t0 = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = DateTime::parse_from_rfc3339("2026-01-01T00:01:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t2 = DateTime::parse_from_rfc3339("2026-01-01T00:02:00Z")
            .unwrap()
            .with_timezone(&Utc);
        append_json_line_to_path(
            &meta_dir.join(ROUND_LOGS_FILE),
            &RoundSummary {
                schema_version: SCHEMA_VERSION,
                round_index: 1,
                trigger: RoundTrigger::Resume,
                input_keys: Vec::new(),
                started_at: t0,
                ended_at: Some(t2),
                status: RoundStatus::Completed,
                entry_count: 3,
                mode: ContextMode::Chat,
            },
            false,
        )
        .await
        .unwrap();
        let round_path = round_file_path(&history_dir, 1);
        for entry in [
            Entry {
                schema_version: SCHEMA_VERSION,
                seq: 1,
                ts: t0,
                mode: ContextMode::Chat,
                payload: EntryPayload::Message {
                    message: AiMessage::text(AiRole::User, "outside-before"),
                    llm_call: None,
                },
            },
            Entry {
                schema_version: SCHEMA_VERSION,
                seq: 2,
                ts: t1,
                mode: ContextMode::Chat,
                payload: EntryPayload::Message {
                    message: AiMessage::text(AiRole::Assistant, "inside"),
                    llm_call: Some(1),
                },
            },
            Entry {
                schema_version: SCHEMA_VERSION,
                seq: 3,
                ts: t2,
                mode: ContextMode::Chat,
                payload: EntryPayload::Message {
                    message: AiMessage::text(AiRole::User, "outside-after"),
                    llm_call: None,
                },
            },
        ] {
            append_json_line_to_path(&round_path, &entry, false)
                .await
                .unwrap();
        }

        let reader = SessionHistoryReader::open(dir.path()).unwrap();
        let result = reader
            .read_session_messages(
                SessionHistoryQuery::TimeRange { start: t1, end: t1 },
                SessionHistoryReadOptions { token_limit: 4096 },
            )
            .unwrap();
        assert_eq!(result.total_candidates, 1);
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].role, AiRole::Assistant);
        assert_eq!(result.messages[0].text, "inside");
    }

    #[tokio::test]
    async fn read_session_messages_from_round_index_keeps_rounds_whole() {
        let dir = tempdir().unwrap();
        let mut writer = SessionHistoryWriter::open(dir.path()).await.unwrap();
        for idx in 1..=3 {
            writer
                .begin_round(RoundTrigger::Resume, Vec::new(), ContextMode::Chat)
                .await
                .unwrap();
            writer
                .append_message(AiMessage::text(AiRole::User, format!("round-{idx}")), None)
                .await
                .unwrap();
            writer.finalize_round(RoundStatus::Completed).await.unwrap();
        }
        drop(writer);

        let reader = SessionHistoryReader::open(dir.path()).unwrap();
        let result = reader
            .read_session_messages_from_round_index(2, SessionHistoryReadOptions { token_limit: 3 })
            .unwrap();
        assert!(result.truncated);
        assert_eq!(result.first_round_index, Some(2));
        assert_eq!(result.last_round_index, Some(2));
        assert_eq!(result.latest_round_index, Some(3));
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].text, "round-2");
    }
}
