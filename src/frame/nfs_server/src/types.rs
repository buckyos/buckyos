//! NFSP wire types (requests, refs, locators, capabilities).
//!
//! Only the shapes needed by the v1 method set are modeled; unknown JSON fields
//! are ignored on input (serde default behavior) per NFSP G7.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// `{"type":"live","node_id":"...","gen":N}` or `{"type":"object","obj_id":"...","inner_path":...}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WireRef {
    #[serde(rename = "live")]
    Live {
        node_id: String,
        #[serde(default)]
        gen: u64,
    },
    #[serde(rename = "object")]
    Object {
        obj_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inner_path: Option<String>,
    },
}

impl WireRef {
    pub fn live(node_id: impl Into<String>, gen: u64) -> Self {
        WireRef::Live { node_id: node_id.into(), gen }
    }
}

/// The `at` field: path, uri, or ref. All three may appear; priority ref > uri > path.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Locator {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<WireRef>,
}

/// Request envelope for `POST /nfs/v1/{method}` (NFSP §4.2).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Envelope {
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub seq: Option<u64>,
    #[serde(default)]
    pub at: Option<Locator>,
    #[serde(default)]
    pub want: Option<Vec<String>>,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub ext: Vec<ExtRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtRecord {
    pub id: String,
    #[serde(default)]
    pub critical: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub payload: Value,
}

/// Attribute groups requested via `want` (NFSP §3.3).
#[derive(Debug, Clone, Default)]
pub struct WantMask {
    groups: BTreeSet<String>,
}

impl WantMask {
    pub fn from_opt(want: &Option<Vec<String>>) -> Self {
        let mut groups = BTreeSet::new();
        match want {
            None => {
                // Default: base only.
                groups.insert("base".to_string());
            }
            Some(list) => {
                for g in list {
                    groups.insert(g.clone());
                }
                if groups.is_empty() {
                    groups.insert("base".to_string());
                }
            }
        }
        WantMask { groups }
    }

    pub fn has(&self, g: &str) -> bool {
        self.groups.contains(g)
    }
}

/// Effective capabilities of a node (NFSP §3.1.1).
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub list: bool,
    pub read: bool,
    pub accepts_content: bool,
    pub accepts_references: bool,
    pub remove_semantics: &'static str, // "destroy" | "unlink" | "none"
    pub ordered: bool,
}

impl Capabilities {
    pub fn for_kind(kind: &str, writable: bool) -> Capabilities {
        match kind {
            "dir" => Capabilities {
                list: true,
                read: false,
                accepts_content: writable,
                accepts_references: writable,
                remove_semantics: "destroy",
                ordered: false,
            },
            "file" | "symlink" => Capabilities {
                list: false,
                read: true,
                accepts_content: writable,
                accepts_references: false,
                remove_semantics: "destroy",
                ordered: false,
            },
            "view" => Capabilities {
                list: true,
                read: false,
                accepts_content: false,
                accepts_references: false,
                remove_semantics: "none",
                ordered: true,
            },
            "collection" => Capabilities {
                list: true,
                read: false,
                accepts_content: false,
                accepts_references: writable,
                remove_semantics: "unlink",
                ordered: true,
            },
            "group" => Capabilities {
                list: true,
                read: false,
                accepts_content: false,
                accepts_references: writable,
                remove_semantics: "unlink",
                ordered: true,
            },
            _ => Capabilities {
                list: false,
                read: false,
                accepts_content: false,
                accepts_references: false,
                remove_semantics: "none",
                ordered: false,
            },
        }
    }
}

/// Session-level features this server implements and advertises in `hello`.
/// Not advertised (not implemented): frozen-subtree, search.semantic, repr,
/// search.stream, get_tree, publish_dir.
pub const SERVER_FEATURES: &[&str] = &[
    "view",
    "collection",
    "reference-binding",
    "watch.sse",
    "search.name",
];

pub const PROTOCOL_VERSION: &str = "nfsp/0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_roundtrip() {
        let r = WireRef::live("n_5", 1);
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"type\":\"live\""));
        let back: WireRef = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
        let obj: WireRef =
            serde_json::from_str(r#"{"type":"object","obj_id":"sha256:ab"}"#).unwrap();
        match obj {
            WireRef::Object { obj_id, inner_path } => {
                assert_eq!(obj_id, "sha256:ab");
                assert!(inner_path.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn want_mask_default_is_base() {
        let m = WantMask::from_opt(&None);
        assert!(m.has("base"));
        assert!(!m.has("ident"));
        let m = WantMask::from_opt(&Some(vec!["ident".into(), "access".into()]));
        assert!(m.has("ident") && m.has("access") && !m.has("base"));
    }

    #[test]
    fn envelope_ignores_unknown_fields() {
        let e: Envelope = serde_json::from_str(
            r#"{"session":"s","seq":1,"args":{"x":1},"future_field":true}"#,
        )
        .unwrap();
        assert_eq!(e.session.as_deref(), Some("s"));
        assert_eq!(e.seq, Some(1));
    }
}
