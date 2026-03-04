use slog::SystemLogRecord;

#[derive(serde::Deserialize)]
struct UploadResponse {
    ret: i32,
    message: String,
}

pub struct LogUploader {
    node: String,
    service_endpoint: String,
    client: reqwest::Client,
}

impl LogUploader {
    pub fn new(node: String, service_endpoint: String) -> Self {
        Self {
            node,
            service_endpoint,
            client: reqwest::Client::new(),
        }
    }

    pub async fn upload_logs(
        &self,
        service: &str,
        records: Vec<SystemLogRecord>,
    ) -> Result<(), String> {
        // Prepare the payload
        let payload = serde_json::json!({
            "node": self.node,
            "service": service,
            "logs": records,
        });

        // Send the HTTP POST request
        let response = self
            .client
            .post(&self.service_endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("failed to send logs to server: {}", e);
                error!("{}", msg);
                msg
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let msg = format!("server returned error status: {}, body: {}", status, body);
            error!("{}", msg);
            return Err(msg);
        }

        let response = response.json::<UploadResponse>().await.map_err(|e| {
            let msg = format!("failed to parse upload response from server: {}", e);
            error!("{}", msg);
            msg
        })?;

        if response.ret != 0 {
            let msg = format!(
                "server returned upload failure: ret={}, message={}",
                response.ret, response.message
            );
            error!("{}", msg);
            return Err(msg);
        }

        Ok(())
    }
}
