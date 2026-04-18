use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub trigger: Value,
    pub steps: Vec<StepDefinition>,
    #[serde(default)]
    pub nodes: Vec<ControlNodeDefinition>,
    pub edges: Vec<EdgeDefinition>,
    #[serde(default)]
    pub guards: Option<GuardConfig>,
    #[serde(default)]
    pub defs: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub executor: Option<String>,
    #[serde(rename = "type")]
    pub step_type: StepType,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub input_schema: Option<Value>,
    pub output_schema: Value,
    #[serde(default)]
    pub subject_ref: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default = "default_true")]
    pub idempotent: bool,
    #[serde(default = "default_true")]
    pub skippable: bool,
    #[serde(default)]
    pub output_mode: OutputMode,
    #[serde(default)]
    pub guards: Option<GuardConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    Autonomous,
    HumanConfirm,
    HumanRequired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Single,
    FiniteSeekable,
    FiniteSequential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlNodeDefinition {
    #[serde(rename = "branch")]
    Branch(BranchNodeDefinition),
    #[serde(rename = "parallel")]
    Parallel(ParallelNodeDefinition),
    #[serde(rename = "for_each")]
    ForEach(ForEachNodeDefinition),
}

impl ControlNodeDefinition {
    pub fn id(&self) -> &str {
        match self {
            Self::Branch(node) => &node.id,
            Self::Parallel(node) => &node.id,
            Self::ForEach(node) => &node.id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchNodeDefinition {
    pub id: String,
    pub on: String,
    pub paths: BTreeMap<String, String>,
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelNodeDefinition {
    pub id: String,
    pub branches: Vec<String>,
    pub join: JoinMode,
    #[serde(default)]
    pub n: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForEachNodeDefinition {
    pub id: String,
    pub items: String,
    pub steps: Vec<String>,
    pub max_items: u32,
    #[serde(default = "default_one")]
    pub concurrency: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JoinMode {
    All,
    Any,
    NOfM,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDefinition {
    pub from: String,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardConfig {
    #[serde(default)]
    pub budget: Option<BudgetGuard>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub retry: Option<RetryGuard>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub amendment_auto_approve: Option<bool>,
    #[serde(default)]
    pub max_cost_usdb: Option<f64>,
    #[serde(default)]
    pub max_duration: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetGuard {
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_cost_usdb: Option<f64>,
    #[serde(default)]
    pub max_duration: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetryGuard {
    #[serde(default = "default_retry_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff: Option<String>,
    #[serde(default)]
    pub fallback: Option<RetryFallback>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetryFallback {
    Human,
    Abort,
}

impl Default for RetryFallback {
    fn default() -> Self {
        Self::Human
    }
}

pub fn default_true() -> bool {
    true
}

pub fn default_one() -> u32 {
    1
}

pub fn default_retry_attempts() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::WorkflowDefinition;
    use crate::compiler::compile_workflow;
    use serde_json::json;

    #[test]
    fn workflow_json_can_compile_into_expr_tree() {
        // This mirrors the end-to-end flow:
        // JSON DSL -> WorkflowDefinition -> compiled Expr tree.
        let workflow_json = json!({
            "schema_version": "0.2.0",
            "id": "wf-json-demo",
            "name": "JSON Demo",
            "trigger": {
                "type": "manual"
            },
            "steps": [
                {
                    "id": "collect",
                    "name": "Collect Inputs",
                    "executor": "skill/fs.scan",
                    "type": "autonomous",
                    "skippable": false,
                    "output_schema": {
                        "type": "object",
                        "properties": {
                            "files": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                }
                            },
                            "ticket": {
                                "type": "string"
                            }
                        },
                        "required": ["files", "ticket"]
                    }
                },
                {
                    "id": "draft",
                    "name": "Draft Plan",
                    "executor": "agent/planner",
                    "type": "autonomous",
                    "skippable": false,
                    "input": {
                        "files": "${collect.output.files}",
                        "ticket": "${collect.output.ticket}",
                        "options": {
                            "mode": "brief",
                            "include_risks": true
                        }
                    },
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "files": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                }
                            },
                            "ticket": {
                                "type": "string"
                            },
                            "options": {
                                "type": "object",
                                "properties": {
                                    "mode": {
                                        "type": "string"
                                    },
                                    "include_risks": {
                                        "type": "boolean"
                                    }
                                },
                                "required": ["mode", "include_risks"]
                            }
                        },
                        "required": ["files", "ticket", "options"]
                    },
                    "output_schema": {
                        "type": "object",
                        "properties": {
                            "summary": {
                                "type": "string"
                            }
                        },
                        "required": ["summary"]
                    }
                },
                {
                    "id": "review",
                    "name": "Review",
                    "type": "human_required",
                    "skippable": false,
                    "prompt": "Approve the draft?",
                    "output_schema": {
                        "type": "object",
                        "properties": {
                            "decision": {
                                "type": "string",
                                "enum": ["approved", "rejected"]
                            },
                            "comment": {
                                "type": "string"
                            }
                        },
                        "required": ["decision"]
                    }
                },
                {
                    "id": "publish",
                    "name": "Publish",
                    "executor": "skill/publish",
                    "type": "autonomous",
                    "skippable": false,
                    "input": {
                        "summary": "${draft.output.summary}"
                    },
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "summary": {
                                "type": "string"
                            }
                        },
                        "required": ["summary"]
                    },
                    "output_schema": {
                        "type": "object"
                    }
                },
                {
                    "id": "revise",
                    "name": "Revise",
                    "executor": "agent/revise",
                    "type": "autonomous",
                    "skippable": false,
                    "input": {
                        "summary": "${draft.output.summary}",
                        "comment": "${review.output.comment}"
                    },
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "summary": {
                                "type": "string"
                            },
                            "comment": {
                                "type": ["string", "null"]
                            }
                        },
                        "required": ["summary"]
                    },
                    "output_schema": {
                        "type": "object"
                    }
                }
            ],
            "nodes": [
                {
                    "type": "branch",
                    "id": "decision",
                    "on": "${review.output.decision}",
                    "paths": {
                        "approved": "publish",
                        "rejected": "revise"
                    },
                    "max_iterations": 1
                }
            ],
            "edges": [
                {
                    "from": "collect",
                    "to": "draft"
                },
                {
                    "from": "draft",
                    "to": "review"
                },
                {
                    "from": "review",
                    "to": "decision"
                },
                {
                    "from": "publish"
                },
                {
                    "from": "revise"
                }
            ]
        });

        let definition: WorkflowDefinition = serde_json::from_value(workflow_json).unwrap();
        let compiled = compile_workflow(definition).unwrap().workflow;
        let expr_tree = serde_json::to_value(&compiled.nodes).unwrap();
        let expr_tree_str = serde_json::to_string_pretty(&expr_tree).unwrap();
        println!("{}", expr_tree_str);
        assert_eq!(compiled.graph.start_nodes, vec!["collect".to_string()]);

        assert_eq!(
            expr_tree["collect"]["expr"]["Apply"]["executor"],
            "skill/fs.scan"
        );

        assert_eq!(
            expr_tree["draft"]["expr"]["Apply"]["executor"],
            "agent/planner"
        );
        assert_eq!(
            expr_tree["draft"]["expr"]["Apply"]["params"]["files"]["Reference"]["node_id"],
            "collect"
        );
        assert_eq!(
            expr_tree["draft"]["expr"]["Apply"]["params"]["files"]["Reference"]["field_path"],
            json!(["files"])
        );
        assert_eq!(
            expr_tree["draft"]["expr"]["Apply"]["params"]["options"]["Object"]["mode"]["Literal"],
            "brief"
        );
        assert_eq!(
            expr_tree["draft"]["expr"]["Apply"]["params"]["options"]["Object"]["include_risks"]
                ["Literal"],
            true
        );

        assert_eq!(expr_tree["review"]["expr"]["Await"]["kind"], "Required");
        assert_eq!(
            expr_tree["review"]["expr"]["Await"]["prompt"],
            "Approve the draft?"
        );

        assert_eq!(
            expr_tree["decision"]["expr"]["Match"]["on"]["node_id"],
            "review"
        );
        assert_eq!(
            expr_tree["decision"]["expr"]["Match"]["on"]["field_path"],
            json!(["decision"])
        );
        assert_eq!(
            expr_tree["decision"]["expr"]["Match"]["cases"],
            json!({
                "approved": "publish",
                "rejected": "revise"
            })
        );
    }
}
