use slog::SystemLogRecord;

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

    pub async fn upload_logs(&self, service: &str, records: Vec<SystemLogRecord>) -> Result<(), String> {
        // Prepare the payload
        let payload = serde_json::json!({
            "node": self.node,
            "service": service,
            "logs": records,
        });

        // Send the HTTP POST request
        let response = self.client.post(&self.service_endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("failed to send logs to server: {}", e);
                error!("{}", msg);
                msg
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            let msg = format!("server returned error status: {}", response.status());
            error!("{}", msg);
            Err(msg)
        }
    }
}