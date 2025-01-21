use crate::error::*;
use std::str::FromStr;

// Action constants
pub const ACTION_DIRECT: &str = "DIRECT";
pub const ACTION_PROXY: &str = "PROXY";
pub const ACTION_PROXY_SOCKS: &str = "SOCKS";
pub const ACTION_PROXY_SOCKS5: &str = "SOCKS5";
pub const ACTION_REJECT: &str = "REJECT";
pub const ACTION_PASS: &str = "PASS";

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RuleAction {
    Direct,        // Directly connect to the destination
    Proxy(String), // Connect to the destination through a proxy,
    Reject,        // Reject the connection
    Pass,          // Pass the connection to the next rule
}

impl FromStr for RuleAction {
    type Err = RuleError;

    fn from_str(s: &str) -> RuleResult<RuleAction> {
        let parts = s
            .split_whitespace()
            .map(|s| s.trim())
            .collect::<Vec<&str>>();
        if parts.len() < 1 {
            return Err(RuleError::InvalidFormat(s.to_string()));
        }

        match parts[0].to_uppercase().as_str() {
            ACTION_DIRECT => Ok(RuleAction::Direct),
            ACTION_PROXY | ACTION_PROXY_SOCKS | ACTION_PROXY_SOCKS5 => {
                if parts.len() > 1 {
                    Ok(RuleAction::Proxy(parts[1].to_string()))
                } else {
                    Ok(RuleAction::Proxy("".to_string()))
                }
            }
            ACTION_REJECT => Ok(RuleAction::Reject),
            ACTION_PASS => Ok(RuleAction::Pass),
            _ => {
                let msg = format!("Invalid rule action: {}", s);
                Err(RuleError::InvalidFormat(msg))
            }
        }
    }
}

impl ToString for RuleAction {
    fn to_string(&self) -> String {
        match self {
            RuleAction::Direct => ACTION_DIRECT.to_string(),
            RuleAction::Proxy(p) => {
                if p.is_empty() {
                    ACTION_PROXY.to_string()
                } else {
                    format!("{} {}", ACTION_PROXY, p)
                }
            }
            RuleAction::Reject => ACTION_REJECT.to_string(),
            RuleAction::Pass => ACTION_PASS.to_string(),
        }
    }
}

impl RuleAction {
    // Parse a list of actions from a string like "DIRECT;PROXY xxx;REJECT"
    pub fn from_str_list(s: &str) -> RuleResult<Vec<RuleAction>> {
        // Multiple actions are separated by a semicolon.
        let items = s.split(';').map(|s| s.trim()).collect::<Vec<&str>>();

        let mut actions = Vec::new();
        for item in items {
            if item.is_empty() {
                continue;
            }
            
            let action = RuleAction::from_str(item)?;
            actions.push(action);
        }

        Ok(actions)
    }
}
