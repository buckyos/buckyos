//! Workflow DSL 类型已经搬到 buckyos-api 共享给所有需要提交 / 解析 workflow
//! 定义的组件（agent、CLI、UI 等）。这里保留一层 re-export 让 workflow crate
//! 内部仍然可以通过 `crate::dsl::*` 访问，并承载一个端到端编译测试。

pub use buckyos_api::workflow_dsl::*;

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
                    "executor": "/skill/fs.scan",
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
                    "executor": "/agent/planner",
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
                    "executor": "/skill/publish",
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
                    "executor": "/agent/revise",
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

        // 语义链接 (`/skill/...` / `/agent/...`) 编译后落到
        // `ExecutorRef::SemanticPath`，序列化形态是 `{ "SemanticPath": "..." }`。
        // 这一步还不会展开到实际 executor 定义，所以 fun_id 必须为 null。
        assert_eq!(
            expr_tree["collect"]["expr"]["Apply"]["executor"]["SemanticPath"],
            "/skill/fs.scan"
        );
        assert!(expr_tree["collect"]["expr"]["Apply"]["fun_id"].is_null());

        assert_eq!(
            expr_tree["draft"]["expr"]["Apply"]["executor"]["SemanticPath"],
            "/agent/planner"
        );
        assert!(expr_tree["draft"]["expr"]["Apply"]["fun_id"].is_null());
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
