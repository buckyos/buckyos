//! Cards of the work a UI Turn started (notepads/
//! messagehub-worksession-ui-todo.md §3.5): one card per work session the
//! Turn created, a plan's card with its TodoList, each Todo bound to a work
//! session with that session's card. Read-only, from the registry and the
//! session directories; nothing here is cached or written.

use std::collections::BTreeMap;

use agent_tool::todo_tools::{TodoMode, TodoStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::AgentStateClient;

/// Class of a plan session.
pub const PLAN_CLASS: &str = "plan_simple";
/// Behavior of a read-only investigation.
pub const EXPLORE_BEHAVIOR: &str = "explore";
/// Longest line of a card (chars).
pub const CARD_LINE_MAX: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CardShape {
    Simple,
    Plan,
    Explore,
}

/// What a card shows of the session's state (§3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CardState {
    Queued,
    Running,
    NeedsInput,
    Paused,
    Succeeded,
    Failed,
    Stopped,
}

impl CardState {
    pub fn is_terminal(self) -> bool {
        matches!(self, CardState::Succeeded | CardState::Failed | CardState::Stopped)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Tokens of the successful Rounds (`usage.jsonl`), by model in the order
/// they were first used.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CardUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub rounds: u64,
    pub models: Vec<ModelUsage>,
}

impl CardUsage {
    fn add(&mut self, model: &str, input: u64, output: u64, rounds: u64) {
        self.input_tokens += input;
        self.output_tokens += output;
        self.rounds += rounds;
        match self.models.iter_mut().find(|m| m.model == model) {
            Some(m) => {
                m.input_tokens += input;
                m.output_tokens += output;
            }
            None => self.models.push(ModelUsage {
                model: model.to_string(),
                input_tokens: input,
                output_tokens: output,
            }),
        }
    }

    fn merge(&mut self, other: &CardUsage) {
        self.rounds += other.rounds;
        for m in &other.models {
            self.add(&m.model, m.input_tokens, m.output_tokens, 0);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkCard {
    pub session_id: String,
    pub shape: CardShape,
    /// ≤ [`TITLE_MAX_CHARS`].
    pub title: String,
    pub created_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub state: CardState,
    /// ≤ [`CARD_LINE_MAX`]; empty where the viewer says it in its own words
    /// (queued, needs input, paused or stopped without a reason).
    pub line: String,
    pub usage: CardUsage,
    /// The model of the last successful Round, as the provider reported it.
    pub model: Option<String>,
    /// A plan: its own usage and that of all its Todo sessions.
    pub subtree_usage: Option<CardUsage>,
    /// A plan: its TodoList.
    pub todos: Option<CardTodos>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CardTodos {
    pub rev: u64,
    /// Todos not cancelled.
    pub total: usize,
    pub done: usize,
    pub items: Vec<CardTodo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CardTodo {
    pub todo_id: String,
    pub seq: u64,
    pub title: String,
    pub status: TodoStatus,
    pub mode: TodoMode,
    /// Its latest attempt (0: none yet).
    pub attempt: u32,
    pub blocked_by: Vec<String>,
    pub block_reason: Option<String>,
    /// The card of the work session its latest attempt is bound to.
    pub card: Option<Box<WorkCard>>,
}

/// `session.work_cards`: what one UI Turn started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkCards {
    pub agent_did: String,
    pub session_id: String,
    pub task_id: String,
    /// Nothing shown can change any more: the Turn is closed, every card
    /// and every bound Todo card ended.
    pub complete: bool,
    pub sessions: Vec<WorkCard>,
}

fn cut(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The first non-empty line, without the marks of a Markdown heading.
fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    cut(line.trim_start_matches('#').trim_start(), CARD_LINE_MAX)
}

/// The usage of a session's successful Rounds and the model of the last one.
pub fn usage_of(sd: &SessionDir) -> (CardUsage, Option<String>) {
    let mut usage = CardUsage::default();
    let mut last = None;
    let Ok(text) = std::fs::read_to_string(sd.file(USAGE_FILE)) else {
        return (usage, last);
    };
    for line in text.lines() {
        // A line being appended is not a record yet.
        let Ok(r) = serde_json::from_str::<UsageRecord>(line) else {
            continue;
        };
        usage.add(&r.model, r.input_tokens, r.output_tokens, 1);
        last = Some(r.model);
    }
    (usage, last)
}

fn state_of(cfg: &SessionConfig, st: &SessionState) -> CardState {
    let goal = cfg.session.kind == SessionKind::Goal && cfg.session.goal.is_some();
    match (st.run_state, st.outcome) {
        (RunState::Finished, Some(Outcome::Succeeded)) => CardState::Succeeded,
        (RunState::Finished, Some(Outcome::Failed)) => CardState::Failed,
        (RunState::Finished, _) => CardState::Stopped,
        _ if st.paused.is_some() => CardState::Paused,
        _ if st.needs_input(goal) => CardState::NeedsInput,
        (RunState::Created | RunState::Ready, _) if st.turn_seq == 0 => CardState::Queued,
        _ => CardState::Running,
    }
}

/// The first line of what the session ended with: its answer, else its
/// report without the header `report.md` adds (`# Report — …`, `- session:`…).
fn report_line(sd: &SessionDir, st: &SessionState) -> String {
    let result = st.result.as_ref();
    let said = result
        .and_then(|r| r.get("answer"))
        .or_else(|| result.and_then(|r| r.pointer("/submission/report")))
        .and_then(|v| v.as_str())
        .map(first_line)
        .filter(|l| !l.is_empty());
    said.or_else(|| {
        let report = sd.report()?;
        let body = report
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .skip_while(|l| {
                l.starts_with("# Report — ")
                    || ["- session:", "- goal:", "- outcome:", "- turns:"].iter().any(|p| l.starts_with(p))
            })
            .next()?;
        Some(first_line(body))
    })
    .unwrap_or_default()
}

fn line_of(state: CardState, sd: &SessionDir, st: &SessionState) -> String {
    let report = || report_line(sd, st);
    match state {
        CardState::Stopped | CardState::Succeeded => report(),
        CardState::Failed => st
            .last_error
            .as_ref()
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .map(first_line)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(report),
        CardState::Paused => st
            .paused
            .as_ref()
            .and_then(|p| p.reason.as_deref())
            .map(first_line)
            .unwrap_or_default(),
        CardState::NeedsInput | CardState::Queued => String::new(),
        // The live line while a Turn is open; between Turns the reply of the
        // last one (a multi-Turn Work's progress reply).
        CardState::Running if st.open_turn.is_some() => cut(&st.one_line_status, CARD_LINE_MAX),
        CardState::Running => st
            .turn_tasks
            .iter()
            .filter(|b| b.closed.is_some())
            .max_by_key(|b| b.turn)
            .and_then(|b| b.summary.as_deref())
            .map(first_line)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cut(&st.one_line_status, CARD_LINE_MAX)),
    }
}

fn title_of(entry: &RegistryEntry, cfg: &SessionConfig) -> String {
    entry
        .title
        .clone()
        .or_else(|| cfg.session.title.clone())
        .or_else(|| {
            cfg.session
                .objective
                .lines()
                .find(|l| !l.trim().is_empty())
                .and_then(clip_title)
        })
        .unwrap_or_default()
}

/// The card of one work session. `children`: a plan's sub sessions (its
/// Todos' work sessions); ignored for any other session.
pub fn work_card(entry: &RegistryEntry, children: &[RegistryEntry]) -> Result<WorkCard> {
    let sd = SessionDir::open(&entry.location)?;
    let cfg = sd.config()?;
    let st = sd.state()?;
    let shape = if cfg.session.class == PLAN_CLASS {
        CardShape::Plan
    } else if cfg.prompt.behavior.as_deref() == Some(EXPLORE_BEHAVIOR) {
        CardShape::Explore
    } else {
        CardShape::Simple
    };
    let state = state_of(&cfg, &st);
    let (usage, model) = usage_of(&sd);
    let mut card = WorkCard {
        session_id: entry.session_id.clone(),
        shape,
        title: title_of(entry, &cfg),
        created_at_ms: cfg.session.created_at_ms,
        finished_at_ms: st.finished_at_ms.filter(|_| st.is_finished()),
        state,
        line: line_of(state, &sd, &st),
        usage,
        model,
        subtree_usage: None,
        todos: None,
    };
    if shape == CardShape::Plan {
        let by_sid: BTreeMap<&str, &RegistryEntry> =
            children.iter().map(|c| (c.session_id.as_str(), c)).collect();
        let mut subtree = card.usage.clone();
        for c in children {
            if let Ok(csd) = SessionDir::open(&c.location) {
                subtree.merge(&usage_of(&csd).0);
            }
        }
        let list = crate::todos::store_of(&sd).load().map_err(crate::todos::todo_error)?;
        let mut todos: Vec<_> = list.todos.iter().collect();
        todos.sort_by_key(|t| t.seq);
        let items = todos
            .into_iter()
            .map(|t| {
                let latest = t.latest();
                let bound = latest
                    .and_then(|a| a.child_sid.as_deref())
                    .and_then(|sid| by_sid.get(sid))
                    .and_then(|e| work_card(e, &[]).ok())
                    .map(|mut c| {
                        c.title = clip_title(&t.title).unwrap_or(c.title);
                        Box::new(c)
                    });
                CardTodo {
                    todo_id: t.todo_id.clone(),
                    seq: t.seq,
                    title: clip_title(&t.title).unwrap_or_default(),
                    status: t.status,
                    mode: t.mode,
                    attempt: latest.map(|a| a.attempt).unwrap_or(0),
                    blocked_by: t.blocked_by.clone(),
                    block_reason: t.block_reason.clone(),
                    card: bound,
                }
            })
            .collect::<Vec<_>>();
        card.todos = Some(CardTodos {
            rev: list.rev,
            total: items.iter().filter(|t| t.status != TodoStatus::Cancelled).count(),
            done: items.iter().filter(|t| t.status == TodoStatus::Completed).count(),
            items,
        });
        card.subtree_usage = Some(subtree);
    }
    Ok(card)
}

fn settled(card: &WorkCard) -> bool {
    card.state.is_terminal()
        && card.todos.as_ref().is_none_or(|l| {
            l.items.iter().all(|t| {
                t.card.as_ref().is_none_or(|c| c.state.is_terminal())
                    && !(t.status == TodoStatus::Running && t.card.is_none())
            })
        })
}

/// The cards of the work sessions the UI Turn `task_id` of session `sid`
/// created (`origin.parent_task`), oldest first.
pub async fn work_cards(agent: &dyn AgentStateClient, sid: &str, task_id: &str) -> Result<WorkCards> {
    let ui = agent
        .sessions()
        .lookup(sid)
        .await?
        .ok_or_else(|| crate::error::OpenDanError::NotFound(format!("session {sid}")))?;
    let turn_open = SessionDir::open(&ui.location)
        .and_then(|sd| sd.state())
        .map(|st| st.open_turn_task().is_some_and(|b| b.task_id == task_id))
        .unwrap_or(false);
    let created: Vec<RegistryEntry> = agent
        .sessions()
        .children_of(&[sid.to_string()])
        .await?
        .into_iter()
        .filter(|e| {
            e.origin
                .as_ref()
                .and_then(|o| o.parent_task.as_deref())
                == Some(task_id)
        })
        .collect();
    let mut sessions = Vec::new();
    for e in &created {
        let children = match SessionDir::open(&e.location).and_then(|sd| sd.config()) {
            Ok(cfg) if cfg.session.class == PLAN_CLASS => {
                agent.sessions().children_of(&[e.session_id.clone()]).await?
            }
            _ => Vec::new(),
        };
        match work_card(e, &children) {
            Ok(c) => sessions.push(c),
            Err(err) => log::warn!("no card for session {}: {err}", e.session_id),
        }
    }
    sessions.sort_by_key(|c| c.created_at_ms);
    Ok(WorkCards {
        agent_did: agent.agent_did().to_string(),
        session_id: sid.to_string(),
        task_id: task_id.to_string(),
        complete: !turn_open && !sessions.is_empty() && sessions.iter().all(settled),
        sessions,
    })
}
