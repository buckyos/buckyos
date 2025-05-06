use super::super::request::PanicReportRequest;
use crate::panic::BugReportHandler;
use crate::panic::PanicInfo;
use http_types::StatusCode;
use std::str::FromStr;
use tide::http::Url;

// Default notify addr
// const NOTIFY_ADDR: &str = "http://127.0.0.1:40001/bugs/";

#[derive(Clone)]
pub struct HttpBugReporter {
    notify_addr: Url,
}

impl HttpBugReporter {
    pub fn new(addr: &str) -> Self {
        info!("new http bug reporter: {}", addr);

        let url = Url::from_str(addr).unwrap();
        Self { notify_addr: url }
    }

    pub async fn notify(&self, req: PanicReportRequest) -> Result<(), Box<dyn std::error::Error>> {
        self.post(req).await
    }

    async fn post(&self, req: PanicReportRequest) -> Result<(), Box<dyn std::error::Error>> {
        let report_url = self.notify_addr.join(&req.info.hash).unwrap();

        let mut resp = surf::post(report_url).body_json(&req)?.await?;
        match resp.status() {
            StatusCode::Ok => {
                info!("post to http notify addr success");

                Ok(())
            }
            code @ _ => {
                let body = resp.body_string().await;
                let msg = format!(
                    "post to http notify addr failed! addr={}, status={}, msg={:?}",
                    self.notify_addr, code, body
                );
                error!("{}", msg);
                Err(msg.into())
            }
        }
    }
}

impl BugReportHandler for HttpBugReporter {
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
