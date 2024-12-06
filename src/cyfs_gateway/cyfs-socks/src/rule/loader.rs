use crate::error::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

use super::pac::PacScriptManager;
use super::selector::{RuleSelector, RuleSelectorRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleType {
    PAC,
}

pub struct RuleItem {
    pub _type: RuleType,
    pub selector: RuleSelectorRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum RuleConfigItem {
    PAC { file: String },
    Include { file: String },
}

impl RuleConfigItem {
    pub fn get_type(&self) -> RuleType {
        match self {
            RuleConfigItem::PAC { .. } => RuleType::PAC,
            RuleConfigItem::Include { .. } => unreachable!("Include item should be used here"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuleFileTarget {
    // Load the rule file from a local path.
    Local(PathBuf),

    // Load the rule file from a remote URL.
    Remote(String),
}

impl RuleFileTarget {
    pub fn new(s: &str) -> Self {
        // Parse the target string to create a new RuleFileTarget.
        // If is a valid URL, then create a remote target, otherwise as local target.
        match Url::parse(s) {
            Ok(_) => Self::Remote(s.to_string()),
            Err(_) => Self::Local(PathBuf::from(s)),
        }
    }

    pub fn is_local(&self) -> bool {
        match self {
            RuleFileTarget::Local(_) => true,
            _ => false,
        }
    }

    pub fn is_remote(&self) -> bool {
        match self {
            RuleFileTarget::Remote(_) => true,
            _ => false,
        }
    }
}
pub struct RuleFileTargetLoader {
    // RuleFileLoader is a struct that loads and parses a rule file
    // from a given target, and provides a method to evaluate the rule
    // for a given URL.
    target: RuleFileTarget,
}

impl RuleFileTargetLoader {
    pub fn new(target: RuleFileTarget) -> Self {
        // Create a new RuleFileLoader to load a rule file from a local path.
        Self {
            target,
        }
    }

    pub fn target(&self) -> &RuleFileTarget {
        &self.target
    }

    // Load the rule file from the target.
    pub async fn load(&self) -> RuleResult<String> {
        let s = match &self.target {
            RuleFileTarget::Local(path) => {
                // Load the rule file from a local path.
                self.load_local(path).await?
            }
            RuleFileTarget::Remote(url) => {
                // Load the rule file from a remote URL.
                self.load_remote(url).await?
            }
        };

        // Parse the rule file content.
        Ok(s)
    }

    // Load the rule file from a local path.
    async fn load_local(&self, path: &PathBuf) -> RuleResult<String> {
        // Load the rule file from a local path.
        // First check if file exists
        if !path.exists() {
            let msg = format!("rule file not found: {:?}", path);
            error!("{}", msg);

            return Err(RuleError::NotFound(msg));
        }

        // Load file to string
        let s = tokio::fs::read_to_string(path).await.map_err(|e| {
            let msg = format!("failed to read rule file: {:?}", e);
            error!("{}", msg);
            RuleError::IoError(msg)
        })?;

        Ok(s)
    }

    // Load the rule file from a remote URL.
    async fn load_remote(&self, url: &str) -> RuleResult<String> {
        // First parse the URL
        let url = Url::parse(url).map_err(|e| {
            let msg = format!("failed to parse URL: {:?}", e);
            error!("{}", msg);
            RuleError::InvalidFormat(msg)
        })?;

        // Load the rule file from a remote URL async.
        let s = reqwest::get(url)
            .await
            .map_err(|e| {
                let msg = format!("failed to get rule file: {:?}", e);
                error!("{}", msg);
                RuleError::HttpError(msg)
            })?
            .text()
            .await
            .map_err(|e| {
                let msg = format!("failed to read rule file: {:?}", e);
                error!("{}", msg);
                RuleError::HttpError(msg)
            })?;

        Ok(s)
    }
}

/**
 * Rule file content in json format like this
 * [{
 *   "type": "PAC",
 *   "file": "http://example.com/pac.js"
 * }],
 * [{
 *   "type": "include",
 *   "file": "./default.json"
 * }]
 */
pub struct RuleFileLoader {
    // The root dir of the rule file.
    root_dir: PathBuf,

    // The root file name of the rule file, default is root(root.json)
    root_file_name: String,
}

impl RuleFileLoader {
    pub fn new(root_dir: &Path, root_file_name: Option<String>) -> Self {
        // Create a new RuleFileParser to parse a rule file content.
        Self {
            root_dir: root_dir.to_owned(),
            root_file_name: root_file_name.unwrap_or("root".to_owned()),
        }
    }

    // Parse the rule file content.
    pub async fn load(&self) -> RuleResult<Vec<RuleItem>> {
        // First load the root rule file and then parse it in json format.
        if !self.root_dir.is_dir() {
            let msg = format!("Root rule file not found: {:?}", self.root_dir);
            error!("{}", msg);

            return Err(RuleError::NotFound(msg));
        }

        // Load root config file to string
        let s = self.try_load_rule_file(&self.root_file_name).await?;

        let items: Vec<RuleConfigItem> = serde_json::from_str(&s).map_err(|e| {
            let msg = format!("Failed to parse rule file: {:?}", e);
            error!("{}", msg);
            RuleError::InvalidFormat(msg)
        })?;

        let mut all = Vec::new();

        // Expand the include items at the top level
        for item in items.into_iter() {
            match item {
                RuleConfigItem::Include { file } => {
                    let s = self.try_load_rule_file(&file).await?;
                    let include_items: Vec<RuleConfigItem> = serde_json::from_str(&s).map_err(|e| {
                        let msg = format!("Failed to parse rule file: {:?}", e);
                        error!("{}", msg);
                        RuleError::InvalidFormat(msg)
                    })?;
                    
                    all.extend(include_items);
                }
                _ => {
                    all.push(item);
                }
            }
        }
        
        let mut result = Vec::new();
        for item in all.into_iter() {
            match self.load_rule_item(&item).await {
                Ok(selector) => {

                    let _type = item.get_type();
                    let rule_item = RuleItem {
                        _type,
                        selector,
                    };

                    result.push(rule_item);
                }
                Err(e) => {
                    // TODO if load failed or check failed, what should we do? return error or ignore this item?
                    error!("Failed to load rule item: {:?}, {:?}", item, e);
                }
            }
        }

        Ok(result)
    }

    async fn try_load_rule_file(&self, name: &str) -> RuleResult<String> {
        // Load file to string
        // First try to load ${root_dir}/{name} if exists, then try to load ${root_dir}/{name}.json
        let rule_files = [name.to_owned(), format!("{}.json", name)];

        let mut rule_file = None;
        for file in rule_files.iter() {
            let path = self.root_dir.join(file);
            if path.exists() {
                rule_file = Some(path);
                break;
            }
        }

        if rule_file.is_none() {
            let msg = format!("Root rule file not found: {:?}, {}", rule_file, name);
            error!("{}", msg);

            return Err(RuleError::NotFound(msg));
        }
        let rule_file = rule_file.unwrap();

        let s = tokio::fs::read_to_string(&rule_file).await.map_err(|e| {
            let msg = format!("Failed to read rule file: {:?}, {:?}", rule_file, e);
            error!("{}", msg);
            RuleError::IoError(msg)
        })?;

        // Parse the rule file content.
        let content = serde_json::from_str(&s).map_err(|e| {
            let msg = format!("Failed to parse rule file: {:?}", e);
            error!("{}", msg);
            RuleError::InvalidFormat(msg)
        })?;

        Ok(content)
    }

    async fn load_rule_item(&self, item: &RuleConfigItem) -> RuleResult<RuleSelectorRef> {
        match item {
            RuleConfigItem::PAC { file } => {
                // Load the PAC file
                let target = RuleFileTarget::new(&file);

                Self::load_pac_selector(target).await
            }
            RuleConfigItem::Include { file: _} => {
                // Load the included file
                let msg = format!("Invalid rule item: {:?}", item);
                error!("{}", msg);
                Err(RuleError::NotSupport(msg))
            }
        }
    }

    pub async fn load_pac_selector(target: RuleFileTarget) -> RuleResult<RuleSelectorRef> {
        let loader = RuleFileTargetLoader::new(target);
        let content = loader.load().await?;

        let pac_script = PacScriptManager::new(content);
        if let Ok(_) = pac_script.check_valid() {
            let selector = Arc::new(Box::new(pac_script) as Box<dyn RuleSelector>);
            Ok(selector)
        } else {
            let msg = format!("Invalid PAC script: {:?}", loader.target());
            error!("{}", msg);
            Err(RuleError::InvalidScript(msg))
        }
    }
}
