#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{capabilities, driver};

#[cfg(not(target_os = "macos"))]
pub fn capabilities() -> serde_json::Value {
    let unsupported =
        serde_json::json!({"supported": false, "ready": false, "state": "unsupported"});
    serde_json::json!({
        "ui": unsupported,
        "capture": unsupported,
        "input": unsupported,
        "accessibility": unsupported
    })
}
