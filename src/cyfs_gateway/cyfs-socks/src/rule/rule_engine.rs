//定义一个异步的trait,用于使用规则
//规则的核心是输入 (来源，目标)，输出 TunnelBuilder
//TunnelBuilder用于构建一个Tunnel,Tunnel负责转发数

use super::action::RuleAction;
use super::loader::{RuleFileLoader, RuleItem};
use super::selector::RuleInput;
use crate::error::{RuleError, RuleResult};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

// Manager the rules load from config files
pub struct RuleEngine {
    root_dir: PathBuf,
    rules: Arc<Mutex<Vec<RuleItem>>>,
}

impl RuleEngine {
    pub fn new(root_dir: &Path) -> Self {
        Self {
            root_dir: root_dir.to_owned(),
            rules: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn load_rules(&self) -> RuleResult<()> {
        let loader = RuleFileLoader::new(&self.root_dir);
        let items = loader.load().await?;

        *self.rules.lock().await = items;

        Ok(())
    }

    pub async fn select(&self, input: RuleInput) -> RuleResult<RuleAction> {
        let rules = self.rules.lock().await;
        for rule in rules.iter() {
            match rule.selector.select(input.clone()).await {
                Ok(output) => {
                    if output.actions.len() == 0 {
                        continue;
                    }

                    let action = output.actions[0].clone();

                    match action {
                        RuleAction::Direct => {
                            return Ok(action);
                        }
                        RuleAction::Proxy(_) => {
                            return Ok(action);
                        }
                        RuleAction::Reject => {
                            return Ok(action);
                        }
                        RuleAction::Pass => {
                            continue;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to select rule: {:?}", e);
                }
            }
        }

        Err(RuleError::NotFound("No rule matched".to_string()))
    }
}
