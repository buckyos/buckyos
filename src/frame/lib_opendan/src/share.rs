//! "Agent shares its results": after a user task (a Work created by intent
//! analysis under a UI session) ends with a final report, the agent posts a
//! privacy-free note about how it was done to its own HomeStation, so other
//! agents can learn from it (notepads/agent-homestation-auto-share-todo.md
//! §5).
//!
//! A host gives a [`ShareHost`]: in a zone it is the HomeStation's
//! `publish.create` as the agent ([`ZoneShare`]); [`LocalShare`] keeps the
//! posts as files of the AgentRoot for `--dev` and tests. Without one
//! nothing is shared.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::InferenceAbortToken;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};

/// Marker at the head of the compose prompt (log and test recognition).
pub const SHARE_MARKER: &str = "[libopendan:share]";
/// Tag every shared post carries.
pub const SHARE_TAG: &str = "agent-experience";
/// HomeStation posts are capped at 2000 characters; the note stays shorter.
pub const MAX_TEXT_CHARS: usize = 1200;
pub const MAX_TAGS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareArtifact {
    pub name: String,
    pub obj_id: String,
}

/// Where the deliverable can be visited (a published page, a demo): the
/// only place a URL appears in a shared post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareLink {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharePost {
    /// Publish key, idempotent on the HomeStation: `share:<session_id>`.
    pub key: String,
    pub session_id: String,
    pub text: String,
    pub tags: Vec<String>,
    /// Report artifacts that are objects (`cyfile:…`).
    pub artifacts: Vec<ShareArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<ShareLink>,
}

/// Media type of an artifact by its file name; `application/octet-stream`
/// when unknown.
pub fn artifact_mime(name: &str) -> &'static str {
    match Path::new(name).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("m4a") => "audio/mp4",
        Some("ogg") => "audio/ogg",
        Some("pdf") => "application/pdf",
        Some("html" | "htm") => "text/html",
        Some("md") => "text/markdown",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// HomeStation attachment kind of a media type: `image`, `video`, `audio`
/// or `file`.
pub fn attachment_kind(mime: &str) -> &'static str {
    match mime.split('/').next() {
        Some("image") => "image",
        Some("video") => "video",
        Some("audio") => "audio",
        _ => "file",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareReceipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj_id: Option<String>,
}

/// Where the agent's shared posts go.
#[async_trait]
pub trait ShareHost: Send + Sync {
    /// The owner's switch (`AgentSettings.shares_results()` in a zone).
    async fn enabled(&self) -> bool;
    async fn publish(&self, post: &SharePost) -> Result<ShareReceipt>;
}

/// What the note is written from.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ShareInput {
    pub session_id: String,
    pub objective: String,
    pub report: String,
    pub artifacts: Vec<String>,
    pub cmd_skills: Vec<String>,
    pub turns: u64,
}

pub fn share_key(session_id: &str) -> String {
    format!("share:{session_id}")
}

const COMPOSE_SYSTEM: &str = "You write short experience notes that an AI agent publishes on its public home page for other agents to learn from. Reply with one JSON object only.";

fn compose_prompt(input: &ShareInput) -> String {
    format!(
        "{SHARE_MARKER}\nAn agent just completed a task for its user. Write the note it publishes about it, for other agents: what kind of task it was, the key steps and decisions, which commands or skills were used, how the result was checked, and what went wrong on the way. Write it so another agent facing a similar task can reuse the approach.\n\nHard rules:\n- The note is public. Never include names of people, accounts, e-mail addresses, phone numbers, postal addresses, file paths, host names, URLs, keys, tokens, quoted conversation, or the business data and figures of the report itself. Describe the kind of thing, not the thing.\n- If the task concerns the user's personal matters (health, finances, family, schedule, relationships) or the approach cannot be described without such details, do not share: answer with \"share\": false and say why in \"reason\".\n- At most {MAX_TEXT_CHARS} characters of plain text (short paragraphs or a short list). Write in the language of the objective.\n- \"tags\": up to {MAX_TAGS} short lowercase topic tags.\n- \"link\": when the final report names a public URL where the deliverable itself can be visited (a published page, a demo, a video), give it as {{\"url\", \"title\", \"summary\"}} (title up to 120 characters, summary up to 280); this is the only place a URL may appear. Otherwise null. Never put internal, local or private addresses there.\n\nAnswer with exactly this JSON object: {{\"share\": true|false, \"reason\": \"<why shared or not>\", \"text\": \"<the note, empty when not shared>\", \"tags\": [\"<tag>\"], \"link\": {{\"url\": \"<https://…>\", \"title\": \"<title>\", \"summary\": \"<one sentence>\"}} | null}}\n\n<task>\n{}\n</task>\n\n<final_report>\n{}\n</final_report>\n\n<artifacts>\n{}\n</artifacts>\n\n<skills_used>\n{}\n</skills_used>\n\nTurns: {}",
        input.objective.trim(),
        input.report.trim(),
        if input.artifacts.is_empty() { "(none)".to_string() } else { input.artifacts.join("\n") },
        if input.cmd_skills.is_empty() { "(none)".to_string() } else { input.cmd_skills.join("\n") },
        input.turns,
    )
}

#[derive(Debug, Deserialize)]
struct Composed {
    #[serde(default)]
    share: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    link: Option<ShareLink>,
}

/// A link the note may carry: a public http(s) URL with a title.
fn public_link(link: Option<ShareLink>) -> Option<ShareLink> {
    let link = link?;
    let url = link.url.trim();
    let title: String = link.title.trim().chars().take(120).collect();
    if !(url.starts_with("https://") || url.starts_with("http://")) || title.is_empty() {
        return None;
    }
    Some(ShareLink { url: url.to_string(), title, summary: link.summary.trim().chars().take(280).collect() })
}

fn parse_composed(text: &str) -> Result<Composed> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(trimmed);
    serde_json::from_str(body).map_err(|e| OpenDanError::Llm(format!("share: compose reply is not the expected JSON: {e}")))
}

/// Ask the model for the note; `Ok(None)` when it decides not to share
/// (the reason is in the log).
pub async fn compose(
    llm: &Arc<dyn LlmClient>,
    model: &str,
    input: &ShareInput,
    artifacts: Vec<ShareArtifact>,
) -> Result<Option<SharePost>> {
    let req = LlmInferenceRequest {
        trace_id: None,
        messages: vec![
            AiMessage::text(AiRole::System, COMPOSE_SYSTEM),
            AiMessage::text(AiRole::User, compose_prompt(input)),
        ],
        model_alias: model.to_string(),
        fallbacks: Vec::new(),
        temperature: None,
        max_completion_tokens: Some(1024),
        force_json: true,
        json_schema: None,
        provider_options: None,
        disable_capabilities: vec![
            buckyos_api::features::WEB_SEARCH.to_string(),
            buckyos_api::features::TOOL_CALL.to_string(),
        ],
        tool_specs: Vec::new(),
        allow_tool_calls: false,
        abort: InferenceAbortToken::noop(),
    };
    let resp = llm
        .infer(req)
        .await
        .map_err(|e| OpenDanError::Llm(format!("share: compose: {e}")))?;
    let composed = parse_composed(&resp.message.text_content())?;
    let text: String = composed.text.trim().chars().take(MAX_TEXT_CHARS).collect();
    if !composed.share || text.is_empty() {
        log::info!(
            "opendan.share.skipped {}",
            json!({ "session_id": input.session_id, "reason": composed.reason })
        );
        return Ok(None);
    }
    let mut tags: Vec<String> = composed
        .tags
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty() && t != SHARE_TAG)
        .take(MAX_TAGS)
        .collect();
    tags.insert(0, SHARE_TAG.to_string());
    Ok(Some(SharePost {
        key: share_key(&input.session_id),
        session_id: input.session_id.clone(),
        text,
        tags,
        artifacts,
        link: public_link(composed.link),
    }))
}

// ----------------------------------------------------------------------
// ZoneShare
// ----------------------------------------------------------------------

fn zone_err(e: impl std::fmt::Display) -> OpenDanError {
    OpenDanError::Channel(format!("share: {e}"))
}

/// Reads the owner's switch for the agent (the zone's `AgentSettings`).
#[async_trait]
pub trait ShareSwitch: Send + Sync {
    async fn shares_results(&self) -> bool;
}

/// The agent's own HomeStation (`/kapi/homestation`, as the agent's app).
/// Clients are taken from the runtime for every call.
pub struct ZoneShare {
    pub switch: Arc<dyn ShareSwitch>,
}

impl ZoneShare {
    async fn client() -> Result<::kRPC::kRPC> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(zone_err)?
            .get_zone_service_krpc_client(buckyos_api::HOMESTATION_SERVICE_NAME)
            .await
            .map_err(zone_err)
    }
}

#[async_trait]
impl ShareHost for ZoneShare {
    async fn enabled(&self) -> bool {
        self.switch.shares_results().await
    }

    async fn publish(&self, post: &SharePost) -> Result<ShareReceipt> {
        let client = Self::client().await?;
        let mut attachments = Vec::new();
        let mut text = post.text.clone();
        let mut unattached = Vec::new();
        for a in &post.artifacts {
            let mime = artifact_mime(&a.name);
            let registered = client
                .call(
                    "object.register",
                    json!({ "objId": a.obj_id, "metadata": { "name": a.name, "mime": mime } }),
                )
                .await;
            match registered {
                Ok(_) => attachments.push(json!({
                    "id": a.obj_id,
                    "kind": attachment_kind(mime),
                    "name": a.name,
                    "status": "uploaded",
                    "object": a.obj_id,
                })),
                Err(e) => {
                    log::warn!("share: artifact {} ({}) not registered: {e}", a.name, a.obj_id);
                    unattached.push(a.name.clone());
                }
            }
        }
        if !unattached.is_empty() {
            text.push_str("\n\nArtifacts: ");
            text.push_str(&unattached.join(", "));
        }
        let result = client
            .call(
                "publish.create",
                json!({
                    "key": post.key,
                    "input": {
                        "text": text,
                        "tags": post.tags,
                        "audience": { "kind": "public" },
                        "category": "work",
                        "attachments": attachments,
                        "link": post.link.as_ref().map(|l| json!({ "url": l.url, "title": l.title, "summary": l.summary })),
                    }
                }),
            )
            .await
            .map_err(zone_err)?;
        Ok(ShareReceipt {
            entry: result.get("entry").and_then(Value::as_str).map(str::to_string),
            obj_id: result.get("objId").and_then(Value::as_str).map(str::to_string),
        })
    }
}

// ----------------------------------------------------------------------
// LocalShare
// ----------------------------------------------------------------------

/// Shared posts as files (`<agent_root>/shares/<key>.json`), for `--dev`
/// and tests. Always enabled.
#[derive(Debug, Clone)]
pub struct LocalShare {
    dir: PathBuf,
}

impl LocalShare {
    pub fn dir_of(agent_root: &Path) -> PathBuf {
        agent_root.join("shares")
    }

    pub fn enable(agent_root: &Path) -> Result<Self> {
        let dir = Self::dir_of(agent_root);
        std::fs::create_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
        Ok(Self { dir })
    }

    pub fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.json", crate::ids::sanitize_segment(key)))
    }

    pub fn all(&self) -> Result<Vec<SharePost>> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&self.dir).map_err(|e| OpenDanError::io(&self.dir, e))?;
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                if let Some(r) = crate::fsutil::read_json_opt::<SharePost>(&p)? {
                    out.push(r);
                }
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }
}

#[async_trait]
impl ShareHost for LocalShare {
    async fn enabled(&self) -> bool {
        true
    }

    async fn publish(&self, post: &SharePost) -> Result<ShareReceipt> {
        let path = self.path(&post.key);
        crate::fsutil::atomic_replace_json(&path, post)?;
        Ok(ShareReceipt {
            entry: Some(path.display().to_string()),
            obj_id: None,
        })
    }
}
