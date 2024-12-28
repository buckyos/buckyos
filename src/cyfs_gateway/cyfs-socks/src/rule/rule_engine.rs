//定义一个异步的trait,用于使用规则
//规则的核心是输入 (来源，目标)，输出 TunnelBuilder
//TunnelBuilder用于构建一个Tunnel,Tunnel负责转发数

use super::action::RuleAction;
use super::loader::{RuleFileLoader, RuleFileTarget};
use super::loader::{RuleType, RuleItem};
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

impl std::fmt::Debug for RuleEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleEngine")
            .field("root_dir", &self.root_dir)
            .finish()
    }
}

impl RuleEngine {
    pub fn new(root_dir: &Path) -> Self {
        Self {
            root_dir: root_dir.to_owned(),
            rules: Arc::new(Mutex::new(Vec::new())),
        }
    }

    // Load rules from the root dir, this is the default way to load rule, without "rule_config" in the config file
    pub async fn load_rules(&self) -> RuleResult<()> {
        let loader = RuleFileLoader::new(&self.root_dir, None);
        let items = loader.load().await?;

        *self.rules.lock().await = items;
        
        info!("load rules from {:?} success", self.root_dir);

        Ok(())
    }

    // Just load from target string, maybe a file name or a url
    // If target is a file name, it should be a relative path to the root dir
    // If target is a url, it should be a http or https url
    pub async fn load_target(&self, target: &str) -> RuleResult<()> {
        let target: RuleFileTarget = RuleFileTarget::new(target);
        match &target {
            RuleFileTarget::Local(filename) => {
                let loader = RuleFileLoader::new(
                    &self.root_dir,
                    Some(filename.to_string_lossy().to_string()),
                );
                let items = loader.load().await?;

                *self.rules.lock().await = items;
            }
            RuleFileTarget::Remote(_url) => {
                let selector = RuleFileLoader::load_pac_selector(target).await?;
                let item = RuleItem {
                    _type: RuleType::PAC,
                    selector: selector,
                };
                self.rules.lock().await.push(item);
            }
        }

        Ok(())
    }

    pub async fn select(&self, input: RuleInput) -> RuleResult<RuleAction> {
        let start = chrono::Utc::now();
        let url = input.dest.url.clone();

        let ret = self.select_inner(input).await;

        let end = chrono::Utc::now();
        let duration = end - start;

        if ret.is_ok() {
            info!("rule select for {} -> {:?} in {:?} ms", url, ret, duration.num_milliseconds());
        } else {
            error!("rule select failed for {} -> {:?} in {:?} ms", url, ret, duration.num_milliseconds());
        }

        ret
    }

    async fn select_inner(&self, input: RuleInput) -> RuleResult<RuleAction> {
        let rules: tokio::sync::MutexGuard<'_, Vec<RuleItem>> = self.rules.lock().await;
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
