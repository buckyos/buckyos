use crate::error::RuleResult;
use std::{net::SocketAddr, sync::Arc};
use fast_socks5::util::target_addr::TargetAddr;
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

impl RuleInput {
    pub fn new_socks_request(src: &SocketAddr, dest: &TargetAddr) -> Self {
        match dest {
            TargetAddr::Domain(domain, port) => {
                // TODO now in the domain, we just use http protocol
                let url = Url::parse(&format!("http://{}:{}", domain, port)).unwrap();
                RuleInput {
                    source: RequestSourceInfo {
                        ip: src.ip().to_string(),
                        http_headers: vec![],
                        protocol: "http".to_string(),
                    },
                    dest: url,
                }
            }
            TargetAddr::Ip(addr) => {
                let url = Url::parse(&format!("tcp://{}:{}", addr.ip(), addr.port())).unwrap();
                RuleInput {
                    source: RequestSourceInfo {
                        ip: src.ip().to_string(),
                        http_headers: vec![],
                        protocol: "tcp".to_string(),
                    },
                    dest: url,
                }
            }
        }
    }
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
