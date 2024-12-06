use crate::error::RuleResult;
use std::sync::Arc;
use url::Url;

use super::action::RuleAction;

#[derive(Debug, Clone)]
pub struct RequestSourceInfo {
    pub ip: String,
    pub http_headers: Vec<(String, String)>,
    pub protocol: String,
}

#[derive(Debug, Clone)]
pub struct RuleInput {
    pub source: RequestSourceInfo,
    pub dest: Url,
}

#[derive(Debug, Clone)]
pub struct RuleOutput {
    pub actions: Vec<RuleAction>,
}

#[async_trait::async_trait]
pub trait RuleSelector: Sync + Send {
    async fn select(&self, input: RuleInput) -> RuleResult<RuleOutput>;
}

pub type RuleSelectorRef = Arc<Box<dyn RuleSelector>>;
