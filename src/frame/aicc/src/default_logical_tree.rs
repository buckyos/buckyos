// Default level-2 logical directory built per `doc/aicc/aicc 逻辑模型目录.md` §4.
//
// Each template defines a task-oriented logical path (e.g. `llm.plan`) whose
// items reference level-1 provider mounts (e.g. `llm.opus`, `llm.gpt-pro`).
// The applied SessionConfig is a verbatim transcription of the doc — items
// are NOT filtered against the current inventory, so the directory tree
// always reflects the designed intent; the router falls back through items
// at request time when an underlying provider is unavailable.
//
// Currently only LLM templates are populated; the doc's embedding/image/
// audio/video sections still need usage-based subdivision before they can be
// codified safely.

use crate::model_session::{LogicalNode, SessionConfig};
use crate::model_types::{
    FallbackMode, FallbackRule, LockedValue, ModelItem, PolicyConfig, SchedulerProfile,
};
use std::collections::BTreeMap;

pub const DEFAULT_LOGICAL_TREE_REVISION: &str = "builtin-aicc-router-v2";

struct Level2Item {
    name: &'static str,
    target: &'static str,
    weight: f64,
}

enum FallbackPreset {
    Parent,
    Strict,
    Disabled,
}

struct Level2Template {
    path: &'static str,
    items: &'static [Level2Item],
    fallback: FallbackPreset,
    profile: Option<SchedulerProfile>,
}

const LLM_TEMPLATES: &[Level2Template] = &[
    Level2Template {
        path: "llm.plan",
        items: &[
            Level2Item { name: "opus", target: "llm.opus", weight: 2.5 },
            Level2Item { name: "gpt_pro", target: "llm.gpt-pro", weight: 2.5 },
            Level2Item { name: "gemini", target: "llm.gemini-pro", weight: 2.4 },
            Level2Item { name: "qwen_max", target: "llm.qwen-max", weight: 1.8 },
            Level2Item { name: "deepseek", target: "llm.deepseek-pro", weight: 1.5 },
        ],
        fallback: FallbackPreset::Parent,
        profile: Some(SchedulerProfile::QualityFirst),
    },
    Level2Template {
        path: "llm.code",
        items: &[
            Level2Item { name: "opus", target: "llm.opus", weight: 2.5 },
            Level2Item { name: "gpt_pro", target: "llm.gpt-pro", weight: 2.5 },
            Level2Item { name: "gemini", target: "llm.gemini-pro", weight: 2.4 },
            Level2Item { name: "qwen_coder", target: "llm.qwen-coder", weight: 2.0 },
            Level2Item { name: "kimi", target: "llm.kimi", weight: 2.0 },
            Level2Item { name: "glm", target: "llm.glm", weight: 1.5 },
            Level2Item { name: "deepseek", target: "llm.deepseek-pro", weight: 1.5 },
        ],
        fallback: FallbackPreset::Parent,
        profile: None,
    },
    Level2Template {
        path: "llm.swift",
        items: &[
            Level2Item { name: "haiku", target: "llm.haiku", weight: 2.5 },
            Level2Item { name: "flash_lite", target: "llm.gemini-flash-lite", weight: 2.5 },
            Level2Item { name: "gpt_nano", target: "llm.gpt-nano", weight: 2.5 },
            Level2Item { name: "grok_fast", target: "llm.grok-fast", weight: 2.0 },
            Level2Item { name: "qwen_small", target: "llm.qwen-small", weight: 2.0 },
            Level2Item { name: "glm_flash", target: "llm.glm-flash", weight: 1.5 },
        ],
        fallback: FallbackPreset::Parent,
        profile: Some(SchedulerProfile::LatencyFirst),
    },
    Level2Template {
        path: "llm.reason",
        items: &[
            Level2Item {
                name: "gemini_deepthink",
                target: "llm.gemini-deepthink",
                weight: 2.5,
            },
            Level2Item { name: "opus", target: "llm.opus", weight: 2.5 },
            Level2Item { name: "gpt_pro", target: "llm.gpt-pro", weight: 2.5 },
            Level2Item { name: "grok_heavy", target: "llm.grok-heavy", weight: 2.0 },
            Level2Item {
                name: "kimi_thinking",
                target: "llm.kimi-thinking",
                weight: 2.0,
            },
            Level2Item {
                name: "deepseek_reasoner",
                target: "llm.deepseek-reasoner",
                weight: 2.0,
            },
        ],
        fallback: FallbackPreset::Disabled,
        profile: Some(SchedulerProfile::QualityFirst),
    },
    Level2Template {
        path: "llm.vision",
        items: &[
            Level2Item { name: "opus", target: "llm.opus", weight: 2.5 },
            Level2Item { name: "gpt", target: "llm.gpt", weight: 2.5 },
            Level2Item { name: "gemini", target: "llm.gemini-pro", weight: 2.5 },
            Level2Item { name: "qwen", target: "llm.qwen-max", weight: 1.0 },
        ],
        fallback: FallbackPreset::Parent,
        profile: None,
    },
    Level2Template {
        path: "llm.long",
        items: &[
            Level2Item { name: "gemini", target: "llm.gemini-pro", weight: 2.0 },
            Level2Item { name: "qwen", target: "llm.qwen-max", weight: 2.0 },
            Level2Item { name: "sonnet", target: "llm.sonnet", weight: 1.5 },
        ],
        fallback: FallbackPreset::Parent,
        profile: None,
    },
    Level2Template {
        path: "llm.fallback",
        items: &[
            Level2Item { name: "haiku", target: "llm.haiku", weight: 1.0 },
            Level2Item { name: "flash_lite", target: "llm.gemini-flash-lite", weight: 1.0 },
            Level2Item { name: "gpt_nano", target: "llm.gpt-nano", weight: 1.0 },
            Level2Item { name: "qwen_small", target: "llm.qwen-small", weight: 1.0 },
        ],
        fallback: FallbackPreset::Disabled,
        profile: None,
    },
];

fn fallback_to_rule(preset: &FallbackPreset) -> FallbackRule {
    match preset {
        FallbackPreset::Parent => FallbackRule::parent(),
        FallbackPreset::Strict => FallbackRule::strict(),
        FallbackPreset::Disabled => FallbackRule {
            mode: FallbackMode::Disabled,
            target: None,
        },
    }
}

fn descend_or_create<'a>(
    root: &'a mut BTreeMap<String, LogicalNode>,
    path: &str,
) -> &'a mut LogicalNode {
    let mut segments = path.split('.').filter(|seg| !seg.is_empty());
    let first = segments.next().expect("path must have at least one segment");
    let mut node = root
        .entry(first.to_string())
        .or_insert_with(LogicalNode::default);
    for segment in segments {
        node = node
            .children
            .entry(segment.to_string())
            .or_insert_with(LogicalNode::default);
    }
    node
}

/// Build a SessionConfig containing the default level-2 logical tree from the
/// static templates verbatim. Items are NOT filtered against the current
/// inventory — the directory tree always reflects the designed intent and the
/// router skips unresolvable items at request time.
pub fn build_default_session_config() -> SessionConfig {
    let mut tree: BTreeMap<String, LogicalNode> = BTreeMap::new();
    let mut applied_nodes = 0usize;

    for template in LLM_TEMPLATES {
        let mut items: BTreeMap<String, ModelItem> = BTreeMap::new();
        for item in template.items {
            items.insert(
                item.name.to_string(),
                ModelItem::new(item.target.to_string(), item.weight),
            );
        }
        let node = descend_or_create(&mut tree, template.path);
        node.items = Some(items);
        node.fallback = Some(fallback_to_rule(&template.fallback));
        if let Some(profile) = template.profile.clone() {
            let mut policy = node.policy.clone().unwrap_or_default();
            policy.profile = Some(LockedValue::new(profile));
            node.policy = Some(policy);
        }
        applied_nodes += 1;
    }

    let mut config = SessionConfig::default();
    config.logical_tree = tree;
    config.revision = Some(format!(
        "{}-{}-nodes",
        DEFAULT_LOGICAL_TREE_REVISION, applied_nodes
    ));
    config
}

pub fn level2_node_count(config: &SessionConfig) -> usize {
    fn walk(node: &LogicalNode) -> usize {
        let mut count = if node.items.is_some() { 1 } else { 0 };
        for child in node.children.values() {
            count += walk(child);
        }
        count
    }
    config
        .logical_tree
        .values()
        .map(walk)
        .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_targets(items: &BTreeMap<String, ModelItem>) -> Vec<&str> {
        let mut out: Vec<&str> = items.values().map(|item| item.target.as_str()).collect();
        out.sort();
        out
    }

    #[test]
    fn all_seven_llm_level2_nodes_present() {
        let config = build_default_session_config();
        let llm = config.logical_tree.get("llm").expect("llm root");
        for child in [
            "plan", "code", "swift", "reason", "vision", "long", "fallback",
        ] {
            assert!(
                llm.children.contains_key(child),
                "llm.{} should be present",
                child
            );
        }
        assert_eq!(level2_node_count(&config), 7);
    }

    #[test]
    fn llm_plan_matches_doc_section_4() {
        let config = build_default_session_config();
        let plan = config
            .logical_tree
            .get("llm")
            .and_then(|node| node.children.get("plan"))
            .and_then(|node| node.items.as_ref())
            .expect("llm.plan items");
        // Doc §4: llm.plan items = opus / gpt_pro / gemini / qwen_max / deepseek
        let names: Vec<&str> = plan.keys().map(String::as_str).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["deepseek", "gemini", "gpt_pro", "opus", "qwen_max"]
        );
        assert_eq!(plan.get("opus").unwrap().target, "llm.opus");
        assert_eq!(plan.get("opus").unwrap().weight, 2.5);
        assert_eq!(plan.get("gpt_pro").unwrap().target, "llm.gpt-pro");
        assert_eq!(plan.get("gemini").unwrap().target, "llm.gemini-pro");
        assert_eq!(plan.get("gemini").unwrap().weight, 2.4);
        assert_eq!(plan.get("qwen_max").unwrap().weight, 1.8);
        assert_eq!(plan.get("deepseek").unwrap().target, "llm.deepseek-pro");
    }

    #[test]
    fn llm_swift_matches_doc_section_4() {
        let config = build_default_session_config();
        let swift = config
            .logical_tree
            .get("llm")
            .and_then(|node| node.children.get("swift"))
            .and_then(|node| node.items.as_ref())
            .expect("llm.swift items");
        let targets = item_targets(swift);
        assert_eq!(
            targets,
            vec![
                "llm.gemini-flash-lite",
                "llm.glm-flash",
                "llm.gpt-nano",
                "llm.grok-fast",
                "llm.haiku",
                "llm.qwen-small",
            ]
        );
    }

    #[test]
    fn fallback_disabled_for_reason_and_fallback_paths() {
        let config = build_default_session_config();
        let llm = config.logical_tree.get("llm").unwrap();
        for path in ["reason", "fallback"] {
            let node = llm.children.get(path).unwrap();
            assert_eq!(
                node.fallback.as_ref().map(|rule| rule.mode.clone()),
                Some(FallbackMode::Disabled),
                "{} should have fallback mode disabled",
                path
            );
        }
        for path in ["plan", "code", "swift", "vision", "long"] {
            let node = llm.children.get(path).unwrap();
            assert_eq!(
                node.fallback.as_ref().map(|rule| rule.mode.clone()),
                Some(FallbackMode::Parent),
                "{} should have fallback mode parent",
                path
            );
        }
    }
}
