use crate::AppDoc;
use serde_json::Value;
use std::collections::HashSet;

pub fn validate_content_handlers(doc: &AppDoc) -> Result<(), String> {
    if doc.content_handlers.is_empty() { return Ok(()); }
    if doc.pkg_list.web.is_none() && !doc.service_config_tips.service_endpoints.contains_key("www") {
        return Err("content handlers require a Web host".into());
    }
    let mut ids = HashSet::new();
    for h in &doc.content_handlers {
        let id = h["handler_id"].as_str().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .ok_or("invalid handler_id")?;
        if !ids.insert(id) { return Err("duplicate handler_id".into()); }
        if h["version"].as_u64().unwrap_or(0) == 0 { return Err("invalid handler version".into()); }
        let selectors = h["selectors"].as_array().filter(|s| !s.is_empty()).ok_or("missing selectors")?;
        for s in selectors {
            if !s.is_object() || !["mime", "objType", "schema", "ext"].iter().any(|k| s.get(k).is_some()) {
                return Err("invalid selector".into());
            }
            for key in ["mime", "objType", "schema"] {
                if let Some(v) = s.get(key) {
                    if !v.as_str().is_some_and(|s| !s.is_empty()) && !v.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|i| i.as_str().is_some_and(|s| !s.is_empty()))) {
                        return Err(format!("invalid selector {key}"));
                    }
                }
            }
            if s.get("maxSize").is_some_and(|v| v.as_u64().is_none()) { return Err("invalid maxSize".into()); }
        }
        let intents = h["intents"].as_object().ok_or("missing intents")?;
        if intents.len() != 1 || !intents.contains_key("open") { return Err("only open is supported".into()); }
        let open = &intents["open"];
        if open["entry"]["type"] != "web" { return Err("open entry must be web".into()); }
        let path = open["entry"]["path"].as_str().ok_or("missing entry path")?;
        let lower = path.to_ascii_lowercase();
        if !path.starts_with('/') || path.starts_with("//") || path.contains("..") || path.contains('\\') || path.contains("://") || path.contains('#') || path.chars().any(char::is_control)
            || lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
            return Err("unsafe entry path".into());
        }
        if let Some(priority) = open.get("priority") {
            if !priority.as_u64().is_some_and(|n| n <= 80) { return Err("third-party priority must be 0..80".into()); }
        }
        if open.get("window").is_some_and(|v| v != "new" && v != "reuse") { return Err("invalid window policy".into()); }
        if open.get("multiSource").is_some_and(|v| !v.is_boolean()) { return Err("invalid multiSource".into()); }
        if open.get("modes").is_some_and(|v| !v.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|m| m == "view" || m == "edit"))) { return Err("invalid modes".into()); }
        if let Some(permissions) = h.get("permissions") {
            for p in permissions.as_array().ok_or("invalid handler permissions")? {
                let scope = p.as_str().ok_or("handler permissions must be scope strings")?;
                if !doc.permissions.iter().any(|p| p.scope_path == scope) { return Err("handler permission exceeds App permissions".into()); }
            }
        }
    }
    Ok(())
}

pub fn preview_content_handler() -> Value {
    serde_json::json!({"provider":"system", "handler_id":"preview", "handler_version":1,
        "selectors":[{"mime":"*/*"}], "enabled":true, "registered_at":0, "permissions":[],
        "intents":{"open":{"entry":{"type":"builtin","target":"preview"},"modes":["view"],"fidelity":"partial","window":"reuse","priority":90}}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn handlers_are_part_of_identity_and_reject_unsafe_entries() {
        use ndn_lib::NamedObject;
        let mut doc: AppDoc = serde_json::from_str(include_str!("../../../../doc/fixtures/appdoc-v1.json")).unwrap();
        let before = doc.gen_obj_id().0;
        doc.content_handlers = vec![json!({"handler_id":"editor","version":1,"selectors":[{"mime":"text/*"}],"intents":{"open":{"entry":{"type":"web","path":"/open?src={source}"},"priority":60}}})];
        assert_ne!(before, doc.gen_obj_id().0);
        let roundtrip: AppDoc = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
        assert_eq!(doc.content_handlers, roundtrip.content_handlers);
        if doc.pkg_list.web.is_none() {
            doc.pkg_list.web = Some(serde_json::from_value(json!({"pkg_id":"example.web#0.1.0"})).unwrap());
        }
        assert!(validate_content_handlers(&doc).is_ok());
        doc.content_handlers[0]["intents"]["open"]["entry"]["path"] = json!("//evil.example/open");
        assert!(validate_content_handlers(&doc).is_err());
    }
}
