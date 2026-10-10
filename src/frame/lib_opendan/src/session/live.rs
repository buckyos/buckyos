//! A session loaded under its lease: the single writer of
//! `.opendan_agent_session/`. `commit_state` is the commit point.

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::Lease;
use crate::protocol::*;

use super::SessionDir;

pub struct Session {
    pub dir: SessionDir,
    pub config: SessionConfig,
    pub state: SessionState,
    writer: Option<WriterInfo>,
    next_seq: u64,
    worklog_end: u64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("sid", &self.dir.sid())
            .field("rev", &self.state.rev)
            .field("run_state", &self.state.run_state)
            .finish()
    }
}

impl Session {
    pub(super) fn load(dir: SessionDir) -> Result<Self> {
        let config = dir.config()?;
        let state = dir.state()?;
        if state.schema != SESSION_STATE_SCHEMA {
            return Err(OpenDanError::blocked(
                format!("unsupported state.json schema `{}`", state.schema),
                None,
            ));
        }
        if config.schema != SESSION_CONFIG_SCHEMA {
            return Err(OpenDanError::blocked(
                format!("unsupported session_config.json schema `{}`", config.schema),
                None,
            ));
        }
        let next_seq = state.worklog.committed_seq + 1;
        let worklog_end = state.worklog.committed_bytes;
        Ok(Self {
            dir,
            config,
            state,
            writer: None,
            next_seq,
            worklog_end,
        })
    }

    pub fn sid(&self) -> &str {
        self.dir.sid()
    }

    /// Drop every uncommitted in-memory change: reload config and state
    /// from disk and truncate the worklog to its committed boundary.
    pub fn reset_to_committed(&mut self, lease: &Lease) -> Result<()> {
        self.config = self.dir.config()?;
        self.state = self.dir.state()?;
        self.truncate_uncommitted(lease)?;
        Ok(())
    }

    pub fn set_writer(&mut self, writer: WriterInfo) {
        self.writer = Some(writer);
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn worklog_end(&self) -> u64 {
        self.worklog_end
    }

    /// Recovery step: the worklog must contain at least the committed bytes;
    /// anything after them is an uncommitted tail and is truncated.
    pub fn truncate_uncommitted(&mut self, lease: &Lease) -> Result<bool> {
        let wl = self.dir.worklog();
        let len = wl.len()?;
        if len < self.state.worklog.committed_bytes {
            return Err(OpenDanError::blocked(
                format!(
                    "worklog is shorter ({len} bytes) than its committed boundary ({} bytes)",
                    self.state.worklog.committed_bytes
                ),
                None,
            ));
        }
        let truncated = wl.truncate_to(lease, self.state.worklog.committed_bytes)?;
        self.next_seq = self.state.worklog.committed_seq + 1;
        self.worklog_end = self.state.worklog.committed_bytes;
        Ok(truncated)
    }

    /// Append entries (visible to readers only after the next commit).
    pub fn append_worklog(&mut self, lease: &Lease, bodies: Vec<WorklogBody>) -> Result<()> {
        if bodies.is_empty() {
            return Ok(());
        }
        let n = bodies.len() as u64;
        let (_, end) = self.dir.worklog().append(lease, self.next_seq, bodies)?;
        self.next_seq += n;
        self.worklog_end = end;
        Ok(())
    }

    /// Commit point: `state.json` replaced atomically with `rev + 1` and the
    /// worklog boundary moved to the current end.
    pub fn commit_state(&mut self, lease: &Lease) -> Result<()> {
        lease.check()?;
        let mut next = self.state.clone();
        next.rev = self.state.rev + 1;
        next.worklog = WorklogBoundary {
            committed_seq: self.next_seq - 1,
            committed_bytes: self.worklog_end,
        };
        if let Some(w) = &self.writer {
            let mut w = w.clone();
            w.lock_epoch = lease.epoch();
            next.writer = Some(w);
        }
        next.updated_at_ms = crate::now_ms();
        fsutil::atomic_replace_json(&self.dir.file(STATE_FILE), &next)?;
        self.state = next;
        Ok(())
    }

    /// Replace `session_config.json` (dynamic subscriptions only).
    pub fn write_config(&mut self, lease: &Lease) -> Result<()> {
        lease.check()?;
        let original = self.dir.config()?;
        if original.workspace != self.config.workspace
            || original.workspace_binding != self.config.workspace_binding
            || original.runtime != self.config.runtime
            || original.prompt.llm_context.get("runtime") != self.config.prompt.llm_context.get("runtime")
        {
            return Err(OpenDanError::WorkspaceBindingInvalid(
                "session workspace is immutable".into(),
            ));
        }
        self.config.config_rev += 1;
        fsutil::atomic_replace_json(&self.dir.file(SESSION_CONFIG_FILE), &self.config)
    }

    pub fn summary(&self) -> Result<SessionSummary> {
        Ok(self
            .dir
            .summary_opt()?
            .unwrap_or_else(|| SessionSummary::initial(self.config.prompt.mechanical_compress.clone())))
    }

    pub fn write_summary(&self, lease: &Lease, summary: &SessionSummary) -> Result<()> {
        lease.fenced(|| fsutil::atomic_replace_json(&self.dir.file(SUMMARY_FILE), summary))
    }

    pub fn update_static(&self, lease: &Lease, f: impl FnOnce(&mut SessionStatic)) -> Result<()> {
        let mut s = self.dir.statistics()?;
        f(&mut s);
        s.updated_at_ms = crate::now_ms();
        lease.fenced(|| fsutil::atomic_replace_json(&self.dir.file(STATIC_FILE), &s))
    }

    pub fn write_report(&self, lease: &Lease, text: &str) -> Result<()> {
        lease.fenced(|| fsutil::atomic_replace(&self.dir.path().join(REPORT_FILE), text.as_bytes()))
    }

    /// Registry status derived from the committed state.
    pub fn status(&self, lease_epoch: u64) -> SessionStatus {
        let report_brief: String = self
            .dir
            .report()
            .map(|r| r.chars().take(500).collect())
            .unwrap_or_default();
        SessionStatus {
            rev: self.state.rev,
            run_state: self.state.run_state,
            outcome: self.state.outcome,
            acceptance: self.state.acceptance,
            one_line_status: self.state.one_line_status.clone(),
            report_brief,
            pending_decision: self.state.pending_decision.clone(),
            waiting_for: self
                .state
                .waiting_for
                .as_ref()
                .filter(|_| self.state.run_state == crate::protocol::RunState::Waiting)
                .map(|w| w.kind),
            turn_open: self.state.open_turn.is_some(),
            activity: self.state.activity.clone(),
            last_runner: self.writer.as_ref().map(|w| crate::protocol::LastRunner {
                runner_id: w.runner_id.clone(),
                host: w.host.clone(),
                pid: w.pid,
                lock_epoch: lease_epoch,
                at_ms: crate::now_ms(),
            }),
            last_error: self.state.last_error.clone(),
            updated_at_ms: self.state.updated_at_ms,
        }
    }
}
