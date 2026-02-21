use std::path::PathBuf;

const DEFAULT_ROLE_MD: &str = "role.md";
const DEFAULT_SELF_MD: &str = "self.md";
const DEFAULT_BEHAVIORS_DIR: &str = "behaviors";
const DEFAULT_ENVIRONMENT_DIR: &str = "environment";
const DEFAULT_WORKLOG_FILE: &str = "worklog/agent-loop.jsonl";
const DEFAULT_MEMORY_TOKEN_LIMIT: u32 = 1_500;

#[derive(Clone, Debug)]
pub struct AIAgentConfig {
    pub agent_root: PathBuf,
    pub behaviors_dir_name: String,
    pub environment_dir_name: String,
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
    pub memory_token_limit: u32,
}

impl AIAgentConfig {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_root: agent_root.into(),
            behaviors_dir_name: DEFAULT_BEHAVIORS_DIR.to_string(),
            environment_dir_name: DEFAULT_ENVIRONMENT_DIR.to_string(),
            role_file_name: DEFAULT_ROLE_MD.to_string(),
            self_file_name: DEFAULT_SELF_MD.to_string(),
            worklog_file_rel_path: PathBuf::from(DEFAULT_WORKLOG_FILE),
            max_steps_per_wakeup: 8,
            max_behavior_hops: 3,
            max_walltime_ms: 120_000,
            hp_max: 10_000,
            hp_floor: 1,
            hp_per_token: 1,
            hp_per_action: 10,
            default_sleep_ms: 2_000,
            max_sleep_ms: 120_000,
            memory_token_limit: DEFAULT_MEMORY_TOKEN_LIMIT,
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
        if self.hp_max == 0 {
            return Err("hp_max must be > 0".to_string());
        }
        if self.memory_token_limit == 0 {
            self.memory_token_limit = DEFAULT_MEMORY_TOKEN_LIMIT;
        }
        Ok(())
    }
}
