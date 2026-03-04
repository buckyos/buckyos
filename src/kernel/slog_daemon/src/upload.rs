use crate::constants::DEFAULT_UPLOAD_TIMEOUT_SECS;
use slog::SystemLogRecord;
use std::time::Duration;

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
    pub fn new(node: String, service_endpoint: String, timeout_secs: u64) -> Self {
        let timeout_secs = if timeout_secs == 0 {
            DEFAULT_UPLOAD_TIMEOUT_SECS
        } else {
            timeout_secs
        };
        let timeout = Duration::from_secs(timeout_secs);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .build()
            .unwrap_or_else(|e| {
                error!("failed to build reqwest client with timeout: {}", e);
                reqwest::Client::new()
            });

        Self {
            node,
            service_endpoint,
            client,
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

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            let msg = format!("failed to read upload response body from server: {}", e);
            error!("{}", msg);
            msg
        })?;

        Self::validate_upload_response(status, &body)
    }

    fn validate_upload_response(status: reqwest::StatusCode, body: &str) -> Result<(), String> {
        if !status.is_success() {
            let msg = format!("server returned error status: {}, body: {}", status, body);
            error!("{}", msg);
            return Err(msg);
        }

        let response = serde_json::from_str::<UploadResponse>(body).map_err(|e| {
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

#[cfg(test)]
mod tests {
    use super::LogUploader;

    #[test]
    fn test_validate_upload_response_success_when_ret_is_zero() {
        let result = LogUploader::validate_upload_response(
            reqwest::StatusCode::OK,
            r#"{"ret":0,"message":"ok"}"#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_upload_response_fails_when_status_is_not_success() {
        let result = LogUploader::validate_upload_response(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "internal error",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("500 Internal Server Error"));
    }

    #[test]
    fn test_validate_upload_response_fails_when_ret_is_non_zero() {
        let result = LogUploader::validate_upload_response(
            reqwest::StatusCode::OK,
            r#"{"ret":1,"message":"db write failed"}"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ret=1"));
    }

    #[test]
    fn test_validate_upload_response_fails_when_response_body_is_invalid_json() {
        let result = LogUploader::validate_upload_response(reqwest::StatusCode::OK, "not-json");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("failed to parse upload response from server")
        );
    }
}
