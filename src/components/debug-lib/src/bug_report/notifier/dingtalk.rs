use super::super::request::PanicReportRequest;
use crate::panic::BugReportHandler;
use crate::panic::PanicInfo;
use buckyos_kit::{get_channel, BuckyOSChannel};

/*
const NOTIFY_MSG: &str = r#"{
    "msgtype": "text",
    "text": {
        "content": ${content},
    },
    "at": {
        "atMobiles": [],
        "isAtAll": true
    }
};"#;
*/

#[derive(Clone)]
pub struct DingtalkNotifier {
    // Notify base on dingtalk robot
    dingtalk_url: String,
}

impl DingtalkNotifier {
    pub fn new(dingtalk_url: &str) -> Self {
        info!("new dingtalk bug reporter: {}", dingtalk_url);
        Self {
            dingtalk_url: dingtalk_url.to_owned(),
        }
    }

    pub async fn notify(&self, req: PanicReportRequest) -> Result<(), Box<dyn std::error::Error>> {
        let content = format!(
            "BuckyOS service panic report: \nproduct:{}\nservice:{}\nbin:{}\nchannel:{}\ntarget:{}\nversion:{}\nmsg:{}",
            req.product_name,
            req.service_name,
            req.exe_name,
            req.channel,
            req.target,
            req.version,
            req.info_to_string(),
        );

        let at_all = match get_channel() {
            BuckyOSChannel::Nightly => false,
            _ => true,
        };

        let msg = serde_json::json!({
            "msgtype": "text",
            "text": {
                "content": content,
            },
            "at": {
                "atMobiles": [],
                "isAtAll": at_all,
            }
        });

        let client = surf::client();
        let req = surf::post(&self.dingtalk_url).body(msg);

        let mut _res = client.send(req).await.map_err(|e| {
            let msg = format!("report to dingtalk error! {}", e);
            error!("{}", msg);
            msg
        })?;

        info!("Report to dingtalk success!");
        Ok(())
    }
}

impl BugReportHandler for DingtalkNotifier {
    fn notify(
        &self,
        product_name: &str,
        service_name: &str,
        panic_info: &PanicInfo,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let req = PanicReportRequest::new(product_name, service_name, panic_info.to_owned());

        let this = self.clone();
        tokio::runtime::Handle::current().block_on(async move {
            let _ = this.notify(req).await;
        });

        Ok(())
    }
}
