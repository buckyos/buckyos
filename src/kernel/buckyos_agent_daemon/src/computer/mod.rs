#[cfg(any(target_os = "macos", windows))]
mod desktop;

#[cfg(any(target_os = "macos", windows))]
pub use desktop::{capabilities, driver};

#[cfg(not(any(target_os = "macos", windows)))]
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
