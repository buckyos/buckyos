use std::path::PathBuf;

use serde_json::Value as Json;

const DEFAULT_ROLE_MD: &str = "role.md";
const DEFAULT_SELF_MD: &str = "self.md";
const DEFAULT_BEHAVIORS_DIR: &str = "behaviors";
const DEFAULT_WORKLOG_FILE: &str = "worklog/agent-loop.jsonl";
const DEFAULT_MEMORY_TOKEN_LIMIT: u32 = 1_500;
const DEFAULT_SELF_CHECK_TIMER_SECS: u64 = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentLocalConfigOverrides {
    pub default_ui_behavior_name: Option<String>,
    pub default_work_behavior_name: Option<String>,
    pub self_check_timer_secs: Option<u64>,
}

impl AgentLocalConfigOverrides {
    pub fn from_json(json: &Json) -> Result<Self, String> {
        let Some(map) = json.as_object() else {
            return Err("agent.json must be a JSON object".to_string());
        };

        Ok(Self {
            default_ui_behavior_name: parse_optional_string(
                map.get("default_ui_behavior_name")
                    .or_else(|| map.get("default_ui_behavior")),
                "default_ui_behavior_name",
            )?,
            default_work_behavior_name: parse_optional_string(
                map.get("default_work_behavior_name")
                    .or_else(|| map.get("default_work_behavior")),
                "default_work_behavior_name",
            )?,
            self_check_timer_secs: parse_optional_u64(
                map.get("self_check_timer_secs")
                    .or_else(|| map.get("self_check_timer")),
                "self_check_timer",
            )?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AIAgentConfig {
    pub agent_instance_id: String,
    pub agent_root: PathBuf,
    pub agent_package_root: Option<PathBuf>,
    pub agent_did: Option<String>,
    pub agent_owner_did: Option<String>,
    pub behaviors_dir_name: String,
    pub role_file_name: String,
    pub self_file_name: String,
    pub worklog_file_rel_path: PathBuf,
    pub max_steps_per_wakeup: u32,
    pub max_behavior_hops: u32,
    pub max_walltime_ms: u64,
    pub hp_max: u32,
    pub hp_floor: u32,
    pub hp_per_token: u32,
    pub hp_per_action: u32,
    pub default_sleep_ms: u64,
    pub max_sleep_ms: u64,
    pub session_worker_threads: usize,
    pub memory_token_limit: u32,
    pub default_ui_behavior_name: Option<String>,
    pub default_work_behavior_name: Option<String>,
    pub self_check_timer_secs: u64,
}

impl AIAgentConfig {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_instance_id: String::new(),
            agent_root: agent_root.into(),
            agent_package_root: None,
            agent_did: None,
            agent_owner_did: None,
            behaviors_dir_name: DEFAULT_BEHAVIORS_DIR.to_string(),
            role_file_name: DEFAULT_ROLE_MD.to_string(),
            self_file_name: DEFAULT_SELF_MD.to_string(),
            worklog_file_rel_path: PathBuf::from(DEFAULT_WORKLOG_FILE),
            max_steps_per_wakeup: 64,
            max_behavior_hops: 16,
            max_walltime_ms: 120_000,
            hp_max: 10_000,
            hp_floor: 1,
            hp_per_token: 1,
            hp_per_action: 10,
            default_sleep_ms: 2_000,
            max_sleep_ms: 120_000,
            session_worker_threads: 1,
            memory_token_limit: DEFAULT_MEMORY_TOKEN_LIMIT,
            default_ui_behavior_name: None,
            default_work_behavior_name: None,
            self_check_timer_secs: DEFAULT_SELF_CHECK_TIMER_SECS,
        }
    }

    pub fn apply_local_overrides(&mut self, overrides: AgentLocalConfigOverrides) {
        if let Some(default_ui_behavior_name) = overrides.default_ui_behavior_name {
            self.default_ui_behavior_name = Some(default_ui_behavior_name);
        }
        if let Some(default_work_behavior_name) = overrides.default_work_behavior_name {
            self.default_work_behavior_name = Some(default_work_behavior_name);
        }
        if let Some(self_check_timer_secs) = overrides.self_check_timer_secs {
            self.self_check_timer_secs = self_check_timer_secs;
        }
    }

    pub fn normalize(&mut self) -> Result<(), String> {
        if self.max_steps_per_wakeup == 0 {
            return Err("max_steps_per_wakeup must be > 0".to_string());
        }
        if self.max_walltime_ms == 0 {
            return Err("max_walltime_ms must be > 0".to_string());
        }
        if self.default_sleep_ms == 0
            || self.max_sleep_ms == 0
            || self.default_sleep_ms > self.max_sleep_ms
        {
            return Err(
                "sleep config invalid: require 0 < default_sleep_ms <= max_sleep_ms".to_string(),
            );
        }
        if self.session_worker_threads == 0 {
            return Err("session_worker_threads must be > 0".to_string());
        }
        if self.hp_max == 0 {
            return Err("hp_max must be > 0".to_string());
        }
        if self.memory_token_limit == 0 {
            self.memory_token_limit = DEFAULT_MEMORY_TOKEN_LIMIT;
        }
        self.default_ui_behavior_name =
            normalize_optional_string(self.default_ui_behavior_name.take());
        self.default_work_behavior_name =
            normalize_optional_string(self.default_work_behavior_name.take());
        Ok(())
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_optional_string(value: Option<&Json>, field_name: &str) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };

    let Some(raw) = value.as_str() else {
        return Err(format!("{field_name} must be a string"));
    };

    Ok(normalize_optional_string(Some(raw.to_string())))
}

fn parse_optional_u64(value: Option<&Json>, field_name: &str) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };

    if let Some(number) = value.as_u64() {
        return Ok(Some(number));
    }

    if let Some(raw) = value.as_str() {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        return raw
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("{field_name} must be an unsigned integer"));
    }

    Err(format!("{field_name} must be an unsigned integer"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_local_overrides_reads_supported_fields() {
        let overrides = AgentLocalConfigOverrides::from_json(&json!({
            "default_ui_behavior_name": " resolve_router ",
            "default_work_behavior": "plan",
            "self_check_timer": "0"
        }))
        .expect("parse overrides");

        assert_eq!(
            overrides.default_ui_behavior_name.as_deref(),
            Some("resolve_router")
        );
        assert_eq!(
            overrides.default_work_behavior_name.as_deref(),
            Some("plan")
        );
        assert_eq!(overrides.self_check_timer_secs, Some(0));
    }

    #[test]
    fn normalize_clears_blank_behavior_overrides() {
        let mut cfg = AIAgentConfig::new("/tmp/test-agent");
        cfg.default_ui_behavior_name = Some("  resolve_router ".to_string());
        cfg.default_work_behavior_name = Some("   ".to_string());

        cfg.normalize().expect("normalize config");

        assert_eq!(
            cfg.default_ui_behavior_name.as_deref(),
            Some("resolve_router")
        );
        assert_eq!(cfg.default_work_behavior_name, None);
        assert_eq!(cfg.self_check_timer_secs, DEFAULT_SELF_CHECK_TIMER_SECS);
    }
}
