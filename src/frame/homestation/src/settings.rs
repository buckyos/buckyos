//! Private user settings (Feed Preferences, Topics, collectors, profile).

use crate::audience::AudienceSpec;
use crate::db::{get_setting, set_setting};
use crate::error::HsResult;
use crate::Station;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MuteRule {
    Person { did: String, name: String },
    Group {
        #[serde(rename = "groupId")]
        group_id: String,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterRule {
    pub id: String,
    pub enabled: bool,
    /// `ai_full` / `ai_assisted` / `low_quality`; all must hold.
    pub conditions: Vec<String>,
    pub accept_inferred: bool,
    pub min_confidence: f64,
    /// `show` / `hide` when the condition is unknown (not classified).
    pub unknown: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub subscribed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectorRef {
    pub did: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Profile {
    pub name: String,
    pub bio: String,
    #[serde(default)]
    pub featured: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserSettings {
    // 待确认（TODO §13）：正文、评论、转发的默认受众，临时为公开
    pub default_audience: AudienceSpec,
    pub like_notice_shown: bool,
    pub mute_rules: Vec<MuteRule>,
    pub filter_rules: Vec<FilterRule>,
    pub topics: Vec<Topic>,
    /// Preset, replaceable collectors (§14.3): submission targets and cold-start sources.
    pub collectors: Vec<CollectorRef>,
    pub comments_open: bool,
    pub profile: Profile,
    /// `latest` or `last_position` (§9.2).
    pub reading_start: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            default_audience: AudienceSpec::Public,
            like_notice_shown: false,
            mute_rules: Vec::new(),
            filter_rules: vec![
                FilterRule {
                    id: "rule-ai-full".into(),
                    enabled: true,
                    conditions: vec!["ai_full".into()],
                    accept_inferred: true,
                    min_confidence: 0.8,
                    unknown: "show".into(),
                },
                FilterRule {
                    id: "rule-ai-assisted-low".into(),
                    enabled: false,
                    conditions: vec!["ai_assisted".into(), "low_quality".into()],
                    accept_inferred: true,
                    min_confidence: 0.6,
                    unknown: "show".into(),
                },
            ],
            topics: Vec::new(),
            collectors: Vec::new(),
            comments_open: true,
            profile: Profile::default(),
            reading_start: "latest".into(),
        }
    }
}

const KEY: &str = "user_settings";

pub fn load(conn: &Connection) -> HsResult<UserSettings> {
    Ok(match get_setting(conn, KEY)? {
        Some(value) => serde_json::from_str(&value).unwrap_or_default(),
        None => UserSettings::default(),
    })
}

pub fn save(conn: &Connection, settings: &UserSettings) -> HsResult<()> {
    set_setting(conn, KEY, &serde_json::to_string(settings)?)
}

impl Station {
    pub async fn settings(&self) -> HsResult<UserSettings> {
        self.db.call(|c| load(c)).await
    }

    pub async fn update_settings(&self, f: impl FnOnce(&mut UserSettings) + Send + 'static) -> HsResult<UserSettings> {
        let result = self
            .db
            .call(move |c| {
                let mut settings = load(c)?;
                f(&mut settings);
                save(c, &settings)?;
                Ok(settings)
            })
            .await?;
        self.bump(&["prefs"]);
        Ok(result)
    }

    pub async fn profile_name(&self) -> String {
        match self.settings().await {
            Ok(s) if !s.profile.name.is_empty() => s.profile.name,
            _ => self.cfg.owner_name.clone(),
        }
    }
}
