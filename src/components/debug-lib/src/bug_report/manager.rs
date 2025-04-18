use super::notifier::{DingtalkNotifier, HttpBugReporter};
use crate::debug_config::DebugConfig;
use crate::panic::{BugReportHandler, PanicInfo};
use buckyos_kit::{get_channel, BuckyOSChannel};

pub(crate) struct BugReportManager {
    list: Vec<Box<dyn BugReportHandler>>,
}

impl BugReportManager {
    pub fn new(list: Vec<Box<dyn BugReportHandler>>) -> Self {
        Self { list }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn load_from_config(&mut self) {
        if let Some(config_node) = DebugConfig::get_config("report") {
            if let Err(e) = self.load_config_value(config_node) {
                error!("load report config error! {}", e);
            }
        }

        if self.list.is_empty() {
            match *get_channel() {
                BuckyOSChannel::Nightly => {
                    // TODO Add default nightly bug report
                }
                BuckyOSChannel::Beta => {
                    // TODO Add default beta bug report
                }
                BuckyOSChannel::Stable => {}
            }
        }
    }

    fn load_config_value(
        &mut self,
        config_node: &toml::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let node = config_node.as_table().ok_or_else(|| {
            let msg = format!("invalid dump config format! content={}", config_node,);
            error!("{}", msg);

            msg
        })?;

        for (k, v) in node {
            match k.as_str() {
                "http" => {
                    if let Some(v) = v.as_str() {
                        info!("load report.http from config: {}", v);

                        let reporter = HttpBugReporter::new(v);
                        self.list.push(Box::new(reporter));
                    } else {
                        error!("unknown report.http config node: {:?}", v);
                    }
                }
                "dingtalk" => {
                    if let Some(v) = v.as_str() {
                        info!("load report.dingtalk from config: {}", v);
                        let reporter = DingtalkNotifier::new(v);
                        self.list.push(Box::new(reporter));
                    } else {
                        error!("unknown report.dingtalk config node: {:?}", v);
                    }
                }

                key @ _ => {
                    error!("unknown report config node: {}={:?}", key, v);
                }
            }
        }

        Ok(())
    }
}

impl BugReportHandler for BugReportManager {
    fn notify(
        &self,
        product_name: &str,
        service_name: &str,
        panic_info: &PanicInfo,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for reporter in &self.list {
            let _ = reporter.notify(product_name, service_name, panic_info);
        }

        Ok(())
    }
}
