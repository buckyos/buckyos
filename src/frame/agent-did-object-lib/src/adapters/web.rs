use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use super::{
    unsupported_event_subscription, unsupported_event_unsubscription, AdapterEventSubscription,
    AdapterReadRequest, AdapterReadResponse, AdapterSubscribeEventRequest,
    AdapterUnsubscribeEventRequest, AdapterXCallRequest, AdapterXCallResponse, AgentObjectAdapter,
};
use crate::error::AgentDIDObjectError;
use crate::types::{
    render_json_for_llm, render_xml_for_llm, LlmRenderOptions, PromptGuidance, ReadMeta,
    TrustGuidance,
};

pub struct WebAdapter {
    id: String,
    client: Client,
}

impl WebAdapter {
    pub fn new(id: String) -> Self {
        Self {
            id,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl AgentObjectAdapter for WebAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let response = self.client.get(&req.object_ref.normalized).send().await?;
        let final_url = response.url().to_string();
        let status = response.status();
        let headers = response.headers().clone();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let content_length = headers
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let body = response.text().await?;
        if !status.is_success() {
            return Err(AgentDIDObjectError::HttpError(format!(
                "GET {} returned {}",
                req.object_ref.normalized, status
            )));
        }

        let (content, title) = render_web_content(&body, &content_type);
        Ok(AdapterReadResponse {
            object: req.object_ref.normalized.clone(),
            object_did: None,
            content: Some(content),
            meta: ReadMeta {
                title,
                content_type: Some(content_type.clone()),
                size: content_length.or(Some(body.len() as u64)),
                updated_at: None,
                source: Some(final_url.clone()),
                extra: json!({}),
            },
            prompt_guidance: vec![PromptGuidance {
                message: "This is a traditional web resource, not a DID Object profile."
                    .to_string(),
            }],
            trust_guidance: vec![TrustGuidance {
                message: if final_url.starts_with("https://") {
                    "Content was read over HTTPS; transport integrity is protected, but author identity is the website's claim.".to_string()
                } else {
                    "Content was read over HTTP; transport and author identity are not independently verified.".to_string()
                },
            }],
            errors: vec![],
            cache_key: Some(req.object_ref.normalized.clone()),
            version: headers
                .get(reqwest::header::ETAG)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            route: req.route_trace,
            adapt_meta: json!({
                "status": status.as_u16(),
                "final_url": final_url,
                "content_type": content_type,
                "content_length": content_length,
            }),
        })
    }

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
        Err(AgentDIDObjectError::UnsupportedMethod(format!(
            "web adapter does not support undeclared x_call action {}",
            req.input.action
        )))
    }

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
        unsupported_event_subscription(&self.id, &req)
    }

    async fn unsubscribe_event(
        &self,
        _req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError> {
        unsupported_event_unsubscription(&self.id)
    }
}

fn render_web_content(body: &str, content_type: &str) -> (String, Option<String>) {
    if content_type.contains("json") {
        let rendered = serde_json::from_str(body)
            .map(|value| render_json_for_llm(&value, LlmRenderOptions { max_chars: None }))
            .unwrap_or_else(|_| body.to_string());
        return (rendered, None);
    }
    if content_type.contains("html")
        || body.trim_start().starts_with("<!DOCTYPE")
        || body.contains("<html")
    {
        let title = extract_title(body);
        let text = render_xml_for_llm(body, LlmRenderOptions { max_chars: None });
        return (text, title);
    }
    (body.to_string(), None)
}

fn extract_title(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    Some(input[start..end].trim().to_string()).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_json_for_llm() {
        let (content, title) =
            render_web_content(r#"{"name":"cam","status":"ok"}"#, "application/json");
        assert!(content.contains("- name: cam"));
        assert!(title.is_none());
    }

    #[test]
    fn extracts_html_text_and_title() {
        let (content, title) = render_web_content(
            "<html><head><title>Hello</title></head><body><h1>Main</h1><p>Text</p></body></html>",
            "text/html",
        );
        assert_eq!(title.as_deref(), Some("Hello"));
        assert!(content.contains("Main"));
        assert!(!content.contains("<h1>"));
    }
}
