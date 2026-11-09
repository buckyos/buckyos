//! Shared conformance vectors of xllm (`doc/llm_context/xllm_ts_runtime.md`
//! §7): every case is computed here from the functions xllm runs, written to
//! `src/frame/llm_context/tests/vectors/` with `XLLM_VECTORS_REGEN=1` and
//! compared with the files otherwise. The TypeScript port consumes copies of
//! the files (`buckyos-websdk/tests/fixtures/xllm/`).
//!
//! ```text
//! cargo test -p agent_tool --lib xllm::vectors                       # check
//! XLLM_VECTORS_REGEN=1 cargo test -p agent_tool --lib xllm::vectors  # rewrite, then review the diff
//! ```

use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex as StdMutex;

use ::kRPC::{RPCContext, RPCErrorDetail};
use buckyos_api::{AiccHandler, CancelResponse, LlmChatInvokeResponse};

use llm_context::behavior_loop::{StepMeta, StepRecord};
use llm_context::state::LLMContextState;

use super::*;
use crate::hints::HINT_NS_ONCE;
use crate::llm_compress::{
    compress_history, merge_keep_exact, parse_summary_message, segment, summary_message,
    CompressOptions, CompressedSummary, UnitKind, STEP_RESULTS_PREFIX, SUMMARIZE_MODEL_ALIAS,
};
use crate::result_view::ToolResultStatusView;
use crate::runtime::ssh::parse_probe;
use crate::runtime::{derive_runtime_id, native_host_id_from, native_target, runtime_descriptor};
use crate::task_model::{TaskFollowup, TaskResult};
use crate::{ResultDedup, ResultHint};

const VECTORS_VERSION: u32 = 1;
const REGEN_ENV: &str = "XLLM_VECTORS_REGEN";
/// `origin` of the `.llm_context` cases (diagnostics and section sources).
const ORIGIN: &str = "/vectors/project/.llm_context";
/// `origin` of the hosted cases.
const HOST_ORIGIN: &str = "demo.notes";

fn vectors_dir() -> PathBuf {
    normalize_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../llm_context/tests/vectors"))
}

/// Write `cases` as `file` (regen mode) or check that the file holds exactly
/// them.
fn check_or_regen(file: &str, cases: Vec<Value>) {
    let mut names = BTreeSet::new();
    for case in &cases {
        let name = case["name"].as_str().expect("every case has a name");
        assert!(
            names.insert(name.to_string()),
            "{file}: duplicate case `{name}`"
        );
    }
    let doc = json!({ "vectors_version": VECTORS_VERSION, "cases": cases });
    let path = vectors_dir().join(file);
    if std::env::var(REGEN_ENV).as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = serde_json::to_string_pretty(&doc).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let hint = format!(
        "if the change is intended, re-run with `{REGEN_ENV}=1 cargo test -p agent_tool --lib xllm::vectors` and review `git diff {}`",
        path.display()
    );
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}; {hint}", path.display()));
    let saved: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not JSON: {e}; {hint}", path.display()));
    if saved == doc {
        return;
    }
    let saved_cases = saved["cases"].as_array().cloned().unwrap_or_default();
    let computed = doc["cases"].as_array().unwrap();
    let detail = if saved["vectors_version"] != doc["vectors_version"] {
        format!(
            "vectors_version {} in the file, {} here",
            saved["vectors_version"], doc["vectors_version"]
        )
    } else if let Some((i, case)) = computed
        .iter()
        .enumerate()
        .find(|(i, case)| saved_cases.get(*i) != Some(case))
    {
        format!(
            "case #{i} `{}` differs\n  file:     {}\n  computed: {}",
            case["name"].as_str().unwrap_or("?"),
            saved_cases
                .get(i)
                .map(|c| truncate_chars(&c.to_string(), 4000))
                .unwrap_or_else(|| "(missing)".into()),
            truncate_chars(&case.to_string(), 4000)
        )
    } else {
        format!(
            "the file has {} cases, the Rust xllm computes {}",
            saved_cases.len(),
            computed.len()
        )
    };
    panic!("{} is out of date: {detail}\n{hint}", path.display());
}

/// `XllmError` as `{ kind, field? }` (the variant in snake_case, as the TS
/// `XllmError.kind`).
fn xllm_error(e: &XllmError) -> Value {
    let kind = match e {
        XllmError::Config { .. } => "config",
        XllmError::Input(_) => "input",
        XllmError::Capability(_) => "capability",
        XllmError::RuntimeMismatch(_) => "runtime_mismatch",
        XllmError::RecoveryBlocked(_) => "recovery_blocked",
        XllmError::Tools(_) => "tools",
        XllmError::Template { .. } => "template",
        XllmError::RunNotFound { .. } => "run_not_found",
        XllmError::RunBusy { .. } => "run_busy",
        XllmError::WorkdirBusy { .. } => "workdir_busy",
        XllmError::RunTerminal { .. } => "run_terminal",
        XllmError::NotResumable { .. } => "not_resumable",
        XllmError::CorruptedRun { .. } => "corrupted_run",
        XllmError::Storage(_) => "storage",
        XllmError::Io(_) => "io",
        XllmError::Extract(_) => "extract",
        XllmError::Compressor(_) => "compressor",
        XllmError::Other(_) => "other",
    };
    match e {
        XllmError::Config { field, .. } => json!({ "kind": kind, "field": field }),
        _ => json!({ "kind": kind }),
    }
}

// =========================================================================
// Fixtures: tools and runtime variables
// =========================================================================

fn tool(name: &str, description: &str, args_schema: Value, source: &str) -> ResolvedTool {
    ResolvedTool {
        name: name.into(),
        description: description.into(),
        args_schema,
        source: source.into(),
    }
}

const BASH: &str = "groupname:bash";
const BASH_CORE: &str = "groupname:bash_core";

fn read_file(source: &str) -> ResolvedTool {
    tool(
        "read_file",
        "Read a file",
        json!({"type": "object", "properties": {"path": {"type": "string", "description": "File path"}}, "required": ["path"]}),
        source,
    )
}

fn shell(source: &str) -> ResolvedTool {
    tool(
        TOOL_SHELL,
        "Run a command",
        json!({"type": "object", "properties": {"command": {"type": "string", "description": "Command line"}, "timeout_ms": {"type": "integer"}}, "required": ["command"]}),
        source,
    )
}

fn write_file(source: &str) -> ResolvedTool {
    tool(
        "write_file",
        "Write a file",
        json!({"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]}),
        source,
    )
}

fn edit_file(source: &str) -> ResolvedTool {
    tool(
        "edit_file",
        "",
        json!({"type": "object", "properties": {"path": {"type": "string"}, "old_string": {"type": "string"}, "new_string": {"type": "string"}}, "required": ["path", "old_string", "new_string"]}),
        source,
    )
}

/// No body argument; scalar attributes and an object child tag.
fn search(source: &str) -> ResolvedTool {
    tool(
        "search",
        "Search the notes",
        json!({"type": "object", "properties": {"pattern": {"type": "string"}, "limit": {"type": "integer", "description": "Max hits"}, "filters": {"type": "object", "description": "Field filters"}}}),
        source,
    )
}

/// A body argument next to an array child tag.
fn annotate(source: &str) -> ResolvedTool {
    tool(
        "annotate",
        "Annotate a note",
        json!({"type": "object", "properties": {"text": {"type": "string"}, "labels": {"type": "array"}, "pinned": {"type": "boolean"}}, "required": ["text"]}),
        source,
    )
}

fn report() -> ResolvedTool {
    tool(
        "report",
        "Submit a report",
        json!({"type": "object", "properties": {"report": {"type": "string"}, "artifacts": {"type": "array", "items": {"type": "string"}}, "result": {}, "is_end": {"type": "boolean"}}, "required": ["report"]}),
        "name:report",
    )
}

fn tools_of(native: Vec<ResolvedTool>, actions: Vec<ResolvedTool>) -> EffectiveTools {
    let exec_enabled = native.iter().chain(&actions).any(|t| t.name == TOOL_SHELL);
    EffectiveTools {
        enabled: true,
        native,
        actions,
        exec_enabled,
        ..Default::default()
    }
}

fn rg_manual() -> Vec<BashToolManual> {
    vec![
        BashToolManual {
            name: "rg".into(),
            description: "search ".into(),
            command: "rg".into(),
            usage: Some("rg -n <pattern>".into()),
        },
        BashToolManual {
            name: "jq".into(),
            description: "Filter JSON.".into(),
            command: "jq".into(),
            usage: None,
        },
    ]
}

fn session() -> HostProtocolFlavor {
    HostProtocolFlavor::Session {
        intro: "You are the agent of this Session. The user talks to you across Turns.\n".into(),
    }
}

fn flavor_json(f: &HostProtocolFlavor) -> Value {
    match f {
        HostProtocolFlavor::OneShot => json!({ "kind": "one_shot" }),
        HostProtocolFlavor::Session { intro } => json!({ "kind": "session", "intro": intro }),
    }
}

fn sections_json(s: HostPromptSections) -> Value {
    json!({
        "capabilities": s.capabilities,
        "cmd_manual": s.cmd_manual,
        "runtime_protocol": s.runtime_protocol,
    })
}

fn runtime_vars() -> BTreeMap<String, String> {
    RuntimeInfo {
        id: "local".into(),
        kind: "local".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        hostname: "devbox".into(),
        shell: "/bin/bash".into(),
        cwd: "/work/project".into(),
        tools: vec!["git".into(), "rg".into()],
        current_time: "2026-11-07 10:00:00 +08:00".into(),
        timezone: "Asia/Shanghai".into(),
        target: json!({"kind": "local"}),
        platform: "linux".into(),
        shell_kind: "bash".into(),
        baseline: "standard".into(),
    }
    .template_values()
}

// =========================================================================
// (a) prompt_assembly.json
// =========================================================================

fn protocol_case(
    name: &str,
    flavor: HostProtocolFlavor,
    loop_model: LoopModel,
    tools: EffectiveTools,
    json_out: bool,
) -> Value {
    json!({
        "name": name,
        "fn": "runtime_protocol",
        "flavor": flavor_json(&flavor),
        "loop_model": loop_model,
        "tools": tools,
        "json": json_out,
        "expect": build_runtime_protocol_for(&flavor, loop_model, &tools, json_out),
    })
}

fn rules_case(name: &str, loop_model: LoopModel, tools: EffectiveTools) -> Value {
    json!({
        "name": name,
        "fn": "rules",
        "loop_model": loop_model,
        "tools": tools,
        "expect": build_rules_system_text(loop_model, &tools),
    })
}

fn cmd_manual_case(name: &str, tools: EffectiveTools) -> Value {
    json!({
        "name": name,
        "fn": "cmd_manual",
        "tools": tools,
        "expect": build_cmd_manual_system_text(&tools),
    })
}

/// One `.llm_context` layer in its JSON form, parsed and merged as
/// `prepare_hosted` does.
fn merged_from_json(llm_context: &Value, origin: &str) -> Result<MergedConfig, XllmError> {
    let raw = if llm_context.is_null() {
        "{}".to_string()
    } else {
        serde_json::to_string(llm_context).unwrap()
    };
    let path = PathBuf::from(origin);
    let file = parse_llm_context_file(&path, &raw)?;
    merge_config_layers(&[ConfigLayer { path, file }])
}

struct AssembleInput<'a> {
    name: &'a str,
    llm_context: Value,
    env: &'a [(&'a str, &'a str)],
    loop_model: LoopModel,
    tools: EffectiveTools,
    json: bool,
    override_system: Option<&'a str>,
    override_select: Option<&'a str>,
}

fn assemble_case(a: AssembleInput<'_>) -> Value {
    let mut env_map = BTreeMap::new();
    for (k, v) in a.env {
        std::env::set_var(k, v);
        env_map.insert(k.to_string(), v.to_string());
    }
    let vars = runtime_vars();
    let overrides = TaskOverrides {
        system: a.override_system.map(str::to_string),
        select: a.override_select.map(str::to_string),
        ..Default::default()
    };
    let result = merged_from_json(&a.llm_context, ORIGIN).and_then(|merged| {
        let group_name = select_prompt_group(&merged, &overrides)?;
        let group = group_name
            .as_ref()
            .and_then(|n| merged.prompt.groups.get(n));
        let env = TemplateEnv {
            runtime: vars.clone(),
            ..Default::default()
        };
        assemble_prompt(&PromptAssembly {
            merged: &merged,
            overrides: &overrides,
            group_name: group_name.as_deref(),
            group,
            loop_model: a.loop_model,
            tools: &a.tools,
            env: &env,
            json: a.json,
        })
    });
    let expect = match result {
        Ok(plan) => {
            let mut e = json!({
                "mode": plan.mode,
                "system_prompt": plan.system_prompt,
                "runtime_protocol": plan.runtime_protocol,
                "sections": plan.sections,
                "template_vars": plan.template_vars,
            });
            if let Some(u) = plan.user_request {
                e["user_request"] = json!(u);
            }
            e
        }
        Err(e) => json!({ "error": xllm_error(&e) }),
    };
    let mut case = json!({
        "name": a.name,
        "fn": "assemble_prompt",
        "llm_context": a.llm_context,
        "origin": ORIGIN,
        "runtime_vars": vars,
        "env": env_map,
        "loop_model": a.loop_model,
        "tools": a.tools,
        "json": a.json,
        "expect": expect,
    });
    if let Some(s) = a.override_system {
        case["override_system"] = json!(s);
    }
    if let Some(s) = a.override_select {
        case["override_select"] = json!(s);
    }
    case
}

fn hosted_case(
    name: &str,
    host_system: &str,
    sections: HostPromptSections,
    flavor: HostProtocolFlavor,
    loop_model: LoopModel,
    tools: EffectiveTools,
    json_out: bool,
) -> Value {
    let mut vars = runtime_vars();
    vars.remove("current_time");
    vars.remove("timezone");
    let env = TemplateEnv {
        runtime: vars.clone(),
        ..Default::default()
    };
    let result = check_hosted_loop(HOST_ORIGIN, sections, loop_model, json_out).and_then(|()| {
        assemble_hosted_system(
            host_system,
            HOST_ORIGIN,
            &env,
            sections,
            &flavor,
            loop_model,
            &tools,
            json_out,
        )
    });
    let expect = match result {
        Ok(h) => json!({
            "system_prompt": h.system_prompt,
            "runtime_protocol": h.runtime_protocol,
            "template_vars": h.template_vars,
        }),
        Err(e) => json!({ "error": xllm_error(&e) }),
    };
    json!({
        "name": name,
        "fn": "hosted_system",
        "host_system": host_system,
        "origin": HOST_ORIGIN,
        "runtime_vars": vars,
        "env": {},
        "sections": sections_json(sections),
        "flavor": flavor_json(&flavor),
        "loop_model": loop_model,
        "tools": tools,
        "json": json_out,
        "expect": expect,
    })
}

fn parse_case(name: &str, llm_context: Value) -> Value {
    let expect = match merged_from_json(&llm_context, ORIGIN) {
        Ok(_) => json!({ "ok": true }),
        Err(e) => json!({ "error": xllm_error(&e) }),
    };
    json!({
        "name": name,
        "fn": "parse_config",
        "llm_context": llm_context,
        "origin": ORIGIN,
        "expect": expect,
    })
}

fn review_config() -> Value {
    json!({
        "prompt": {
            "select": "review",
            "groups": {
                "review": {
                    "default_user": "Review the code in {{runtime.cwd}}.\n",
                    "sections": {
                        "role": {"text": "You are a code reviewer."},
                        "contexts": {"text": "Project: {{env.XLLM_VECTOR_PROJECT}}. Cwd: {{runtime.cwd}}."},
                        "rules": {"text": "Read before judging."},
                        "cmd_manual": {"text": "Prefer rg."},
                        "output_format": {"text": "Answer in the report field."}
                    }
                },
                "docs": {"sections": {"role": {"text": "You are a docs editor."}}}
            },
            "sections": {
                "rules": {"text": "Only report grounded findings."},
                "25": {"name": "project_conventions", "text": "Public API changes need caller checks."},
                "50": {"name": "delivery_checklist", "text": ""},
                "100": {"text": "Report file, evidence, suggestion."}
            }
        }
    })
}

fn prompt_assembly_cases() -> Vec<Value> {
    use LoopModel::{Behavior, FunctionCall};
    let fc_tools = || tools_of(vec![read_file(BASH), shell(BASH)], vec![]);
    let fc_report = || tools_of(vec![shell(BASH), report()], vec![]);
    let actions_all = || {
        tools_of(
            vec![],
            vec![
                shell(BASH),
                write_file(BASH),
                edit_file(BASH),
                read_file(BASH),
                search("name:search"),
                annotate("name:annotate"),
            ],
        )
    };
    let mut cases = vec![
        // ---- runtime_protocol ----
        protocol_case(
            "protocol_fc_tools",
            HostProtocolFlavor::OneShot,
            FunctionCall,
            fc_tools(),
            false,
        ),
        protocol_case(
            "protocol_fc_report_tool",
            HostProtocolFlavor::OneShot,
            FunctionCall,
            fc_report(),
            false,
        ),
        protocol_case(
            "protocol_fc_session_report_tool",
            session(),
            FunctionCall,
            fc_report(),
            false,
        ),
        protocol_case(
            "protocol_fc_without_tools",
            HostProtocolFlavor::OneShot,
            FunctionCall,
            tools_of(vec![], vec![]),
            false,
        ),
        protocol_case(
            "protocol_fc_tools_disabled",
            HostProtocolFlavor::OneShot,
            FunctionCall,
            EffectiveTools {
                enabled: false,
                ..fc_tools()
            },
            false,
        ),
        protocol_case(
            "protocol_fc_json_with_tools",
            HostProtocolFlavor::OneShot,
            FunctionCall,
            fc_tools(),
            true,
        ),
        protocol_case(
            "protocol_fc_json_without_tools",
            session(),
            FunctionCall,
            tools_of(vec![], vec![]),
            true,
        ),
        protocol_case(
            "protocol_behavior_action_shapes",
            HostProtocolFlavor::OneShot,
            Behavior,
            actions_all(),
            false,
        ),
        protocol_case(
            "protocol_behavior_session_actions_and_native",
            session(),
            Behavior,
            tools_of(vec![read_file(BASH)], vec![shell(BASH)]),
            false,
        ),
        protocol_case(
            "protocol_behavior_native_only",
            HostProtocolFlavor::OneShot,
            Behavior,
            tools_of(vec![read_file(BASH), shell(BASH)], vec![]),
            false,
        ),
        protocol_case(
            "protocol_behavior_session_nothing",
            session(),
            Behavior,
            tools_of(vec![], vec![]),
            false,
        ),
        protocol_case(
            "protocol_behavior_json",
            HostProtocolFlavor::OneShot,
            Behavior,
            tools_of(vec![], vec![shell(BASH)]),
            true,
        ),
        // ---- rules ----
        rules_case("rules_native_bash_workspace", FunctionCall, fc_tools()),
        rules_case(
            "rules_tools2actions_bash_core_unrestricted",
            Behavior,
            EffectiveTools {
                tools2actions: true,
                filesystem_policy: FilesystemPolicy::Unrestricted,
                ..tools_of(vec![report()], vec![read_file(BASH_CORE), shell(BASH_CORE)])
            },
        ),
        rules_case(
            "rules_without_builtin_sources",
            FunctionCall,
            tools_of(
                vec![report(), search("mcp:http://127.0.0.1:9000/mcp")],
                vec![],
            ),
        ),
        rules_case(
            "rules_enabled_but_empty",
            FunctionCall,
            tools_of(vec![], vec![]),
        ),
        rules_case(
            "rules_tools_disabled",
            Behavior,
            EffectiveTools {
                enabled: false,
                ..fc_tools()
            },
        ),
        // ---- cmd_manual ----
        cmd_manual_case(
            "cmd_manual_with_bash_tools",
            EffectiveTools {
                bash_tools: rg_manual(),
                ..fc_tools()
            },
        ),
        cmd_manual_case("cmd_manual_without_bash_tools", fc_tools()),
        cmd_manual_case(
            "cmd_manual_exec_disabled",
            EffectiveTools {
                bash_tools: rg_manual(),
                ..tools_of(vec![read_file(BASH)], vec![])
            },
        ),
        cmd_manual_case(
            "cmd_manual_tools_disabled",
            EffectiveTools {
                enabled: false,
                bash_tools: rg_manual(),
                ..fc_tools()
            },
        ),
    ];

    // ---- assemble_prompt ----
    std::env::remove_var("XLLM_VECTOR_UNSET");
    let with_manual = || EffectiveTools {
        bash_tools: rg_manual(),
        ..fc_tools()
    };
    cases.extend([
        assemble_case(AssembleInput {
            name: "assemble_group_with_top_level_sections",
            llm_context: review_config(),
            env: &[("XLLM_VECTOR_PROJECT", "demo")],
            loop_model: FunctionCall,
            tools: with_manual(),
            json: false,
            override_system: None,
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_select_override_and_alias_names",
            llm_context: json!({
                "prompt": {
                    "select": "a",
                    "groups": {
                        "a": {"default_user": "Task A.", "sections": {"role": "You are A."}},
                        "b": {
                            "default_user": "Task B on {{runtime.os}}.",
                            "sections": {"role": "You are B.", "env": {"text": "Work in {{runtime.cwd}}."}}
                        }
                    },
                    "sections": {
                        "10": {"name": "persona"},
                        "contexts": {"name": "environment"},
                        "60": {"text": "Keep notes short.", "name": "notes"}
                    }
                }
            }),
            env: &[],
            loop_model: FunctionCall,
            tools: fc_tools(),
            json: false,
            override_system: None,
            override_select: Some("b"),
        }),
        assemble_case(AssembleInput {
            name: "assemble_exec_disabled_drops_cmd_manual",
            llm_context: json!({
                "prompt": {"sections": {"role": "You read code.", "cmd_manual": "Use make.", "output_format": {"text": ""}}}
            }),
            env: &[],
            loop_model: FunctionCall,
            tools: EffectiveTools {
                bash_tools: rg_manual(),
                ..tools_of(vec![read_file(BASH)], vec![])
            },
            json: true,
            override_system: None,
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_custom_mode",
            llm_context: json!({"prompt": {"mode": "custom", "system": "You translate on {{runtime.os}}.\n"}}),
            env: &[],
            loop_model: FunctionCall,
            tools: with_manual(),
            json: false,
            override_system: None,
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_override_system_wins",
            llm_context: review_config(),
            env: &[("XLLM_VECTOR_PROJECT", "demo")],
            loop_model: FunctionCall,
            tools: tools_of(vec![], vec![]),
            json: false,
            override_system: Some("Answer briefly about {{runtime.hostname}}."),
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_template_escape_and_referenced_contexts",
            llm_context: json!({
                "prompt": {"sections": {
                    "role": "Write \\{{name}} literally on {{ runtime.hostname }}.",
                    "contexts": "OS: {{runtime.os}}, shell {{runtime.shell_kind}}."
                }}
            }),
            env: &[],
            loop_model: FunctionCall,
            tools: fc_tools(),
            json: false,
            override_system: None,
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_behavior_group_default_user",
            llm_context: json!({
                "prompt": {"select": "edit", "groups": {"edit": {
                    "default_user": "Fix the typo in README.md.",
                    "sections": {"role": "You edit files.", "rules": "Edit only what is asked."}
                }}}
            }),
            env: &[],
            loop_model: Behavior,
            tools: tools_of(vec![], vec![shell(BASH), write_file(BASH)]),
            json: false,
            override_system: None,
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_missing_runtime_variable",
            llm_context: json!({"prompt": {"sections": {"role": "You run on {{runtime.gpu}}."}}}),
            env: &[],
            loop_model: FunctionCall,
            tools: fc_tools(),
            json: false,
            override_system: None,
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_unset_env_variable",
            llm_context: json!({"prompt": {"sections": {"role": "Project {{env.XLLM_VECTOR_UNSET}}."}}}),
            env: &[],
            loop_model: FunctionCall,
            tools: fc_tools(),
            json: false,
            override_system: None,
            override_select: None,
        }),
        assemble_case(AssembleInput {
            name: "assemble_unknown_group",
            llm_context: json!({"prompt": {"select": "missing", "groups": {"a": {"sections": {"role": "A"}}}}}),
            env: &[],
            loop_model: FunctionCall,
            tools: fc_tools(),
            json: false,
            override_system: None,
            override_select: None,
        }),
    ]);

    // ---- hosted_system ----
    cases.extend([
        hosted_case(
            "hosted_all_sections_function_call",
            "You help with the notes in {{runtime.cwd}} on {{runtime.hostname}}.\n\n",
            HostPromptSections::default(),
            HostProtocolFlavor::OneShot,
            FunctionCall,
            with_manual(),
            false,
        ),
        hosted_case(
            "hosted_no_sections_json",
            "You help with notes.",
            HostPromptSections::NONE,
            HostProtocolFlavor::OneShot,
            FunctionCall,
            fc_tools(),
            true,
        ),
        hosted_case(
            "hosted_session_behavior_without_cmd_manual",
            "You are Jarvis.",
            HostPromptSections {
                cmd_manual: false,
                ..Default::default()
            },
            session(),
            Behavior,
            EffectiveTools {
                bash_tools: rg_manual(),
                ..tools_of(vec![report()], vec![shell(BASH)])
            },
            false,
        ),
        hosted_case(
            "hosted_empty_host_text",
            "",
            HostPromptSections {
                runtime_protocol: false,
                ..Default::default()
            },
            HostProtocolFlavor::OneShot,
            FunctionCall,
            with_manual(),
            false,
        ),
        hosted_case(
            "hosted_behavior_needs_runtime_protocol",
            "You are Jarvis.",
            HostPromptSections::NONE,
            HostProtocolFlavor::OneShot,
            Behavior,
            tools_of(vec![], vec![shell(BASH)]),
            false,
        ),
        hosted_case(
            "hosted_json_needs_function_call",
            "You are Jarvis.",
            HostPromptSections::default(),
            HostProtocolFlavor::OneShot,
            Behavior,
            tools_of(vec![], vec![shell(BASH)]),
            true,
        ),
        hosted_case(
            "hosted_has_no_current_time",
            "Today is {{runtime.current_time}}.",
            HostPromptSections::default(),
            HostProtocolFlavor::OneShot,
            FunctionCall,
            fc_tools(),
            false,
        ),
    ]);

    // ---- parse_config ----
    cases.extend([
        parse_case(
            "parse_full_config",
            json!({
                "model": "llm.chat",
                "max_tool_iterations": 6,
                "llm_timeout": 120,
                "timeout": 900,
                "context_window": 128000,
                "context_yield_ratio": 0.75,
                "loop_model": "function_call",
                "run_logs": "warn",
                "result_format": "result.answer",
                "json": true,
                "json_schema": {"type": "object", "properties": {"answer": {"type": "string"}}},
                "provider": {"type": "buckyos"},
                "runs_dir": "none",
                "tools": {
                    "enabled": true,
                    "filesystem_policy": "workspace",
                    "tools": [{"groupname": "bash"}, {"name": "report"}],
                    "bash_tools": [{"name": "rg", "description": "search", "command": "rg", "usage": "rg -n <pattern>"}],
                    "shell": {"timeout_ms": 60000}
                },
                "prompt": {
                    "select": "review",
                    "groups": {"review": {
                        "default_user": "Review it.",
                        "sections": {"role": "You review.", "50": {"name": "checklist", "text": "Check refs."}},
                        "tools": {"enabled": true}
                    }},
                    "sections": {"rules": {"text": "Be precise."}, "contexts": null}
                }
            }),
        ),
        parse_case("parse_unknown_root_key", json!({"model": "llm.chat", "temperature": 0.2})),
        parse_case("parse_unknown_nested_key", json!({"tools": {"shell": {"wait_ms": 1000}}})),
        parse_case("parse_wrong_type_integer", json!({"max_tool_iterations": "8"})),
        parse_case("parse_wrong_type_bool", json!({"tools": {"enabled": "yes"}})),
        parse_case(
            "parse_alias_and_line_conflict",
            json!({"prompt": {"sections": {"rules": "a", "30": "b"}}}),
        ),
        parse_case(
            "parse_alias_name_on_other_line",
            json!({"prompt": {"sections": {"50": {"name": "rules", "text": "x"}}}}),
        ),
        parse_case(
            "parse_custom_with_select",
            json!({"prompt": {"mode": "custom", "system": "x", "select": "a"}}),
        ),
        parse_case("parse_runs_dir_empty", json!({"runs_dir": "  "})),
        parse_case("parse_context_yield_ratio_out_of_range", json!({"context_yield_ratio": 1.5})),
        parse_case("parse_planned_runtime_kind", json!({"runtime": {"kind": "container"}})),
        parse_case(
            "parse_api_key_sources_exclusive",
            json!({"provider": {"type": "openai", "api_key": "k", "api_key_env": "OPENAI_KEY"}}),
        ),
        parse_case(
            "parse_tool_source_with_two_kinds",
            json!({"tools": {"tools": [{"groupname": "bash", "name": "report"}]}}),
        ),
    ]);
    cases
}

#[test]
fn prompt_assembly() {
    check_or_regen("prompt_assembly.json", prompt_assembly_cases());
}

// =========================================================================
// (b) xml_behavior.json
// =========================================================================

fn behavior_case(
    name: &str,
    actions: &[ResolvedTool],
    text: &str,
    tool_calls: Vec<AiToolCall>,
) -> Value {
    let response = if tool_calls.is_empty() {
        AiResponse::text(text)
    } else {
        AiResponse::from_parts(
            (!text.is_empty()).then(|| text.to_string()),
            tool_calls.clone(),
            vec![],
        )
    };
    let expect = match XllmActionParser::new(actions).parse(&response) {
        Ok(result) => json!({ "result": result }),
        Err(error) => json!({ "error": error }),
    };
    let mut case = json!({
        "name": name,
        "actions": actions,
        "text": text,
        "expect": expect,
    });
    if !tool_calls.is_empty() {
        case["tool_calls"] = json!(tool_calls);
    }
    case
}

fn xml_behavior_cases() -> Vec<Value> {
    let parser_tools = vec![
        tool(
            TOOL_SHELL,
            "",
            json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
            "t",
        ),
        tool(
            "write_file",
            "",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
            "t",
        ),
        tool(
            "edit_file",
            "",
            json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"}},"required":["path","old_string","new_string"]}),
            "t",
        ),
        tool(
            "mytool",
            "",
            json!({"type":"object","properties":{"foo":{"type":"integer"},"bar":{"type":"string"},"opts":{"type":"object"}}}),
            "t",
        ),
    ];
    let read = vec![read_file("test")];
    let write = vec![write_file("test")];
    let none: Vec<ResolvedTool> = Vec::new();
    let native = AiToolCall {
        name: TOOL_SHELL.into(),
        args: HashMap::from([("command".into(), json!("echo side-effect"))]),
        call_id: "native-1".into(),
    };
    vec![
        behavior_case(
            "fenced_actions_attrs_cdata_children",
            &parser_tools,
            r#"```xml
<response>
  <intent>plan</intent>
  <actions>
    <shell><![CDATA[ls -la && echo "<done>"]]></shell>
    <write_file path="a.txt"><![CDATA[hi
there]]></write_file>
    <edit_file path="x.rs"><old_string><![CDATA[a]]></old_string><new_string>b &amp; c</new_string></edit_file>
    <mytool foo="7" bar="x"><![CDATA[{"opts":{"k":1}}]]></mytool>
    <mytool foo="8"><opts><![CDATA[{"k":2}]]></opts></mytool>
  </actions>
</response>
```"#,
            vec![],
        ),
        behavior_case(
            "legacy_thinking_is_the_intent",
            &parser_tools,
            "<response><thinking>old</thinking><actions><shell>ls</shell></actions></response>",
            vec![],
        ),
        behavior_case(
            "final_report_in_cdata",
            &parser_tools,
            r#"<response><report end="true"><![CDATA[final <answer>]]></report></response>"#,
            vec![],
        ),
        behavior_case(
            "unknown_action_tag_still_dispatched",
            &parser_tools,
            "<response><actions><nope x=\"1\"/></actions></response>",
            vec![],
        ),
        behavior_case("blank_reply_is_an_error", &parser_tools, "   ", vec![]),
        behavior_case(
            "progress_report_without_end",
            &none,
            "<response><report>progress</report></response>",
            vec![],
        ),
        behavior_case(
            "progress_report_with_next_behavior",
            &none,
            r#"<response><observation>saw it</observation><report end="false">progress</report><next_behavior>WAIT_USER_MSG</next_behavior></response>"#,
            vec![],
        ),
        behavior_case(
            "final_report_with_artifacts_and_result",
            &none,
            r#"<response><actions/><report end="true">final</report><artifacts>["out.txt"]</artifacts><result>{"ok":true}</result></response>"#,
            vec![],
        ),
        behavior_case(
            "final_report_with_actions",
            &none,
            r#"<response><report end="true">final</report><actions><shell>echo side-effect</shell></actions></response>"#,
            vec![],
        ),
        behavior_case(
            "final_report_with_next_behavior",
            &none,
            r#"<response><report end="true">final</report><next_behavior>CHECK</next_behavior></response>"#,
            vec![],
        ),
        behavior_case(
            "final_report_empty",
            &none,
            r#"<response><report end="true"> </report></response>"#,
            vec![],
        ),
        behavior_case(
            "report_end_is_case_sensitive",
            &none,
            r#"<response><report end="TRUE">final</report></response>"#,
            vec![],
        ),
        behavior_case(
            "report_end_twice",
            &none,
            r#"<response><report end="false" end="true">final</report></response>"#,
            vec![],
        ),
        behavior_case(
            "two_reports",
            &none,
            r#"<response><report>first</report><report>second</report></response>"#,
            vec![],
        ),
        behavior_case(
            "next_behavior_end_is_invalid",
            &none,
            r#"<response><report>progress</report><next_behavior>END</next_behavior></response>"#,
            vec![],
        ),
        behavior_case(
            "artifacts_need_a_report",
            &none,
            r#"<response><artifacts>[]</artifacts></response>"#,
            vec![],
        ),
        behavior_case(
            "artifacts_must_be_strings",
            &none,
            r#"<response><report>progress</report><artifacts>{}</artifacts></response>"#,
            vec![],
        ),
        behavior_case(
            "result_must_be_json",
            &none,
            r#"<response><report>progress</report><result>invalid</result></response>"#,
            vec![],
        ),
        behavior_case(
            "result_with_artifacts_must_be_an_object",
            &none,
            r#"<response><report>progress</report><artifacts>["a.txt"]</artifacts><result>1</result></response>"#,
            vec![],
        ),
        behavior_case(
            "native_calls_conflict_with_final_report",
            &none,
            r#"<response><report end="true">final</report></response>"#,
            vec![native.clone()],
        ),
        behavior_case("native_calls_without_text", &none, "", vec![native]),
        behavior_case(
            "body_carries_the_only_required_string",
            &read,
            "<response><actions><read_file><![CDATA[fixture.txt]]></read_file></actions></response>",
            vec![],
        ),
        behavior_case(
            "attribute_wins_over_blank_body",
            &read,
            r#"<response><actions><read_file path="fixture.txt"><![CDATA[   ]]></read_file></actions></response>"#,
            vec![],
        ),
        behavior_case(
            "child_tag_argument",
            &read,
            "<response><actions><read_file><path>fixture.txt</path></read_file></actions></response>",
            vec![],
        ),
        behavior_case(
            "raw_body_value_keeps_a_field_prefix",
            &write,
            r#"<response><actions><write_file path="out.txt"><![CDATA[path: fixture.txt]]></write_file></actions></response>"#,
            vec![],
        ),
    ]
}

#[test]
fn xml_behavior() {
    check_or_regen("xml_behavior.json", xml_behavior_cases());
}

// =========================================================================
// (c) loop_scripts.json
// =========================================================================

/// The messages of a request and the tools it offered (`None`: tool calls
/// not allowed).
type SeenRequest = (Vec<AiMessage>, Option<Vec<String>>);

/// Replies in script order; records how many inferences were made and the
/// last request.
struct ScriptLlm {
    script: StdMutex<VecDeque<Result<AiResponse, LLMComputeError>>>,
    calls: AtomicU32,
    last: StdMutex<Option<SeenRequest>>,
}

#[async_trait]
impl LlmClient for ScriptLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some((
            req.messages.clone(),
            req.allow_tool_calls
                .then(|| req.tool_specs.iter().map(|s| s.name.clone()).collect()),
        ));
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .expect("the script of a loop vector must not run out")
    }
}

/// Lists `specs`; answers a call from the scripted result of its call id.
struct ReplayTools {
    specs: Vec<ToolSpecLite>,
    results: BTreeMap<String, Value>,
    dispatched: StdMutex<Vec<String>>,
}

#[async_trait]
impl ToolManager for ReplayTools {
    async fn call_tool(
        &self,
        call: AiToolCall,
        _ctx: ToolCallCtx,
    ) -> Result<Observation, ToolDispatchError> {
        self.dispatched.lock().unwrap().push(call.call_id.clone());
        let result = self
            .results
            .get(&call.call_id)
            .unwrap_or_else(|| panic!("no scripted result for call `{}`", call.call_id));
        if let Some(obs) = result.get("observation") {
            return Ok(serde_json::from_value(obs.clone()).unwrap());
        }
        let e = &result["dispatch_error"];
        Err(ToolDispatchError {
            message: e["message"].as_str().unwrap().to_string(),
            effect_unknown: e["effect_unknown"].as_bool().unwrap(),
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        self.specs.clone()
    }
}

fn base_request() -> LLMContextRequest {
    LLMContextRequest {
        owner: json!({"id": "t"}),
        trace: Some("trace-1".into()),
        objective: "test".into(),
        behavior_name: String::new(),
        input: vec![AiMessage::text(AiRole::User, "hello")],
        model_policy: ModelPolicy {
            preferred: "test-model".into(),
            ..ModelPolicy::default()
        },
        tool_policy: ToolPolicy {
            mode: ToolMode::All,
            max_tool_iterations: 4,
            max_calls_per_round: 4,
            ..ToolPolicy::default()
        },
        output: OutputSpec::Text,
        budget: BudgetSpec::default(),
        error_policy: ErrorPolicy::default(),
        forbid_next_behavior: false,
    }
}

fn reply(text: &str) -> Result<AiResponse, LLMComputeError> {
    Ok(AiResponse::text(text))
}

fn reply_with_usage(text: &str, total: u64) -> Result<AiResponse, LLMComputeError> {
    Ok(AiResponse {
        usage: Some(AiUsage {
            input_tokens: Some(total / 2),
            output_tokens: Some(total - total / 2),
            total_tokens: Some(total),
            ..Default::default()
        }),
        ..AiResponse::text(text)
    })
}

fn call(name: &str, id: &str, args: Value) -> AiToolCall {
    AiToolCall {
        name: name.into(),
        args: serde_json::from_value(args).unwrap(),
        call_id: id.into(),
    }
}

fn calls(text: Option<&str>, calls: Vec<AiToolCall>) -> Result<AiResponse, LLMComputeError> {
    Ok(AiResponse::from_parts(
        text.map(str::to_string),
        calls,
        vec![],
    ))
}

fn fail(failure: ProviderFailure, message: &str) -> Result<AiResponse, LLMComputeError> {
    Err(LLMComputeError::provider(failure, message))
}

fn ok_result(id: &str, content: Value) -> (String, Value) {
    let bytes = content.to_string().len();
    let observation = Observation::Success {
        call_id: id.into(),
        content,
        bytes,
        truncated: false,
        host: None,
        images: Vec::new(),
    };
    (id.to_string(), json!({ "observation": observation }))
}

fn err_result(id: &str, message: &str) -> (String, Value) {
    let observation = Observation::Error {
        call_id: id.into(),
        message: message.into(),
        host: None,
    };
    (id.to_string(), json!({ "observation": observation }))
}

fn dispatch_error(id: &str, message: &str, effect_unknown: bool) -> (String, Value) {
    (
        id.to_string(),
        json!({ "dispatch_error": { "message": message, "effect_unknown": effect_unknown } }),
    )
}

/// The outcome as `{ kind, which?, error?: { kind, failure? }, output_kind? }`.
fn outcome_summary(outcome: &LLMContextOutcome) -> Value {
    let full = serde_json::to_value(outcome).unwrap();
    let mut out = json!({ "kind": full["kind"] });
    match outcome {
        LLMContextOutcome::Done { output, .. } => {
            out["output_kind"] = serde_json::to_value(output).unwrap()["kind"].clone();
        }
        LLMContextOutcome::BudgetExhausted { which, .. } => out["which"] = json!(which),
        LLMContextOutcome::ContextLimitReached { which, .. } => out["which"] = json!(which),
        LLMContextOutcome::Error { error, .. } => {
            let e = serde_json::to_value(error).unwrap();
            let mut error = json!({ "kind": e["kind"] });
            if let Some(failure) = e.get("failure") {
                error["failure"] = failure.clone();
            }
            out["error"] = error;
        }
        LLMContextOutcome::Interrupted { .. } | LLMContextOutcome::Settled { .. } => {}
    }
    out
}

/// The run status xllm records for `step` as `{ status, kind?, limit_reason? }`.
/// A context limit compacts and continues (`running`); the vectors record
/// what happens when no compaction is left: the run fails with
/// `context_capacity` (`XllmRun::compact_context`).
fn run_summary(step: &RunStep) -> Value {
    let mut out = json!({ "status": step.status() });
    match step {
        RunStep::Error { error, .. } => out["kind"] = json!(error.kind),
        RunStep::Wait { kind, .. } => out["kind"] = json!(kind),
        RunStep::LimitReached { reason, .. } => out["limit_reason"] = json!(reason),
        RunStep::Compact { .. } => {
            out = json!({ "status": RunStatus::Failed, "kind": CONTEXT_CAPACITY_EXHAUSTED.0 });
        }
        RunStep::Complete { .. }
        | RunStep::HostReport { .. }
        | RunStep::Handover { .. }
        | RunStep::Interrupted { .. } => {}
    }
    out
}

struct LoopInput<'a> {
    name: &'a str,
    source: &'a str,
    request: LLMContextRequest,
    tools: &'a [&'a str],
    tool_results: Vec<(String, Value)>,
    script: Vec<Result<AiResponse, LLMComputeError>>,
}

/// A behavior loop case: xllm's action parser and step renderer.
struct BehaviorSetup {
    actions: Vec<ResolvedTool>,
    /// `XmlStepRenderer::new()` (true) or `.without_timestamps()`.
    timestamps: bool,
    seed: Option<Seed>,
}

/// State a behavior case starts from: `LLMContextState::from_request(request,
/// 1)` with these steps and next step index, `inner_transcript` appended to
/// `accumulated`, resumed with `ResumeFromMidRun`.
struct Seed {
    steps: Vec<StepRecord>,
    next_step_index: u32,
    inner_transcript: Vec<AiMessage>,
}

async fn loop_case(c: LoopInput<'_>) -> Value {
    run_loop_case(c, None).await
}

async fn run_loop_case(c: LoopInput<'_>, behavior: Option<&BehaviorSetup>) -> Value {
    let specs: Vec<ToolSpecLite> = c
        .tools
        .iter()
        .map(|name| ToolSpecLite {
            name: name.to_string(),
            description: String::new(),
            args_schema: json!({}),
        })
        .collect();
    let results: BTreeMap<String, Value> = c.tool_results.into_iter().collect();
    let llm = Arc::new(ScriptLlm {
        script: StdMutex::new(c.script.iter().cloned().collect()),
        calls: AtomicU32::new(0),
        last: StdMutex::new(None),
    });
    let tools = Arc::new(ReplayTools {
        specs: specs.clone(),
        results: results.clone(),
        dispatched: StdMutex::new(Vec::new()),
    });
    let mut deps = LLMContextDeps::new(llm.clone(), tools.clone());
    if let Some(b) = behavior {
        let renderer = if b.timestamps {
            XmlStepRenderer::new()
        } else {
            XmlStepRenderer::new().without_timestamps()
        };
        deps = deps
            .with_result_parser(Arc::new(XllmActionParser::new(&b.actions)))
            .with_step_renderer(Arc::new(renderer));
    }
    let mut ctx = match behavior.and_then(|b| b.seed.as_ref()) {
        None => LLMContext::new(c.request.clone(), deps),
        Some(seed) => {
            let mut state = LLMContextState::from_request(&c.request, 1);
            state.steps = seed.steps.clone();
            state.next_step_index = seed.next_step_index;
            state
                .accumulated
                .extend(seed.inner_transcript.iter().cloned());
            let snapshot = LLMContextSnapshot {
                request: c.request.clone(),
                state,
            };
            LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps)
                .expect("a seeded behavior context resumes")
        }
    };
    let outcome = ctx.run().await;
    let behavior_result = match &outcome {
        LLMContextOutcome::Done {
            behavior_result: Some(r),
            ..
        } => {
            let mut v = json!({ "report_end": r.report_end });
            if let Some(s) = &r.self_report {
                v["self_report"] = json!(s);
            }
            if let Some(s) = &r.next_behavior {
                v["next_behavior"] = json!(s);
            }
            if let Some(s) = &r.report_result {
                v["report_result"] = s.clone();
            }
            Some(v)
        }
        _ => None,
    };
    let state = ctx.snapshot().state;
    // The limits of the run the request was built from (`hosted_request`).
    let limits = RunLimits {
        max_tokens: c.request.model_policy.max_completion_tokens,
        max_tool_iterations: c.request.tool_policy.max_tool_iterations,
        timeout_secs: DEFAULT_TIMEOUT_SECS,
        llm_timeout_secs: DEFAULT_LLM_TIMEOUT_SECS,
        context_window_tokens: c.request.budget.context_window_tokens,
        context_yield_ratio: None,
    };
    let outcome_json = outcome_summary(&outcome);
    let step = run_step(outcome, false, &[], &limits, &ProviderConfig::default());
    let mut case = json!({
        "name": c.name,
        "source": c.source,
        "request": c.request,
        "tools": specs.iter().map(|s| json!({
            "name": s.name,
            "description": s.description,
            "args_schema": s.args_schema,
        })).collect::<Vec<_>>(),
        "tool_results": results,
        "script": c.script.iter().map(|r| match r {
            Ok(response) => json!({ "response": response }),
            Err(error) => json!({ "error": error }),
        }).collect::<Vec<_>>(),
        "expect": {
            "outcome": outcome_json,
            "inferences": llm.calls.load(Ordering::SeqCst),
            "dispatched": *tools.dispatched.lock().unwrap(),
            "state": {
                "tool_iterations_left": state.tool_iterations_left,
                "consecutive_errors": state.consecutive_errors,
                "output_repairs": state.output_repairs,
            },
            "run": run_summary(&step),
        },
    });
    if matches!(step, RunStep::Compact { .. }) {
        case["note"] = json!(
            "xllm compacts the context and continues the same run; `run` is what it records once no compaction is left (failed, context_capacity)"
        );
    }
    if let Some(b) = behavior {
        let mut setup = json!({
            "actions": b.actions,
            "renderer": if b.timestamps { "timestamps" } else { "no_timestamps" },
        });
        if let Some(seed) = &b.seed {
            setup["seed"] = json!({
                "steps": seed.steps,
                "next_step_index": seed.next_step_index,
                "inner_transcript": seed.inner_transcript,
            });
        }
        case["behavior"] = setup;
        let expect = &mut case["expect"];
        if let Some(r) = behavior_result {
            expect["behavior_result"] = r;
        }
        expect["state"]["steps"] = json!(state.steps.len());
        expect["state"]["last_step"] = json!(state.last_step.is_some());
        expect["state"]["next_step_index"] = json!(state.next_step_index);
        expect["state"]["next_action_id"] = json!(state.next_action_id);
        let (messages, offered) = llm
            .last
            .lock()
            .unwrap()
            .clone()
            .expect("one inference at least");
        expect["last_request_messages"] = json!(messages);
        expect["last_request_tools"] = json!(offered);
    }
    case
}

/// An AgentToolResult as `XllmToolManager` turns it into an observation
/// (the renderer reads the envelope in `host`).
fn tool_obs(
    id: &str,
    tool: &str,
    status: AgentToolStatus,
    summary: &str,
    output: &str,
) -> (String, Value) {
    let result: AgentToolResult = serde_json::from_value(json!({
        "agent_tool_protocol": crate::AGENT_TOOL_PROTOCOL_VERSION,
        "tool": tool,
        "status": status,
        "summary": summary,
        "output": output,
    }))
    .unwrap();
    let observation = map_result_to_observation_with_hints(id.into(), result, &[]);
    (id.to_string(), json!({ "observation": observation }))
}

fn behavior_request(behavior_name: &str) -> LLMContextRequest {
    let mut r = base_request();
    r.behavior_name = behavior_name.into();
    r.input = vec![
        AiMessage::text(AiRole::System, "You act with the XML actions of this run."),
        AiMessage::text(AiRole::User, "Do the task."),
    ];
    r
}

fn plan_steps() -> Vec<StepRecord> {
    let step = |index: u32, started: u64, ended: u64, text: &str, thought: &str| StepRecord {
        meta: StepMeta {
            behavior_name: "plan".into(),
            step_index: index,
            started_at_ms: started,
            ended_at_ms: Some(ended),
            compression_level: Default::default(),
        },
        assistant_text: text.into(),
        thought: Some(thought.into()),
        ..Default::default()
    };
    vec![
        step(
            0,
            10,
            20,
            "<response><intent>create todos</intent></response>",
            "created todos",
        ),
        step(
            1,
            21,
            30,
            "<response><intent>switch to do</intent><next_behavior>DO</next_behavior></response>",
            "switch to do",
        ),
    ]
}

async fn behavior_loop_cases() -> Vec<Value> {
    use AgentToolStatus::{Error as Failed, Success};
    let acts = || vec![shell("t"), read_file("t"), write_file("t")];
    let setup = |timestamps: bool| BehaviorSetup {
        actions: acts(),
        timestamps,
        seed: None,
    };
    let final_report = || {
        reply(r#"<response><intent>saw it</intent><report end="true">finished</report></response>"#)
    };
    let with = |name: &str, f: &dyn Fn(&mut LLMContextRequest)| {
        let mut r = behavior_request(name);
        f(&mut r);
        r
    };
    let seeded = |timestamps: bool| BehaviorSetup {
        actions: acts(),
        timestamps,
        seed: Some(Seed {
            steps: plan_steps(),
            next_step_index: 2,
            inner_transcript: vec![AiMessage::text(AiRole::User, "Continue TASK_ANCHOR.")],
        }),
    };
    let inputs: Vec<(LoopInput<'_>, BehaviorSetup)> = vec![
        (
            LoopInput {
                name: "behavior_final_report",
                source: "llm_context behavior_loop::behavior_loop_assigns_step_metadata",
                request: behavior_request("xllm"),
                tools: &[],
                tool_results: vec![],
                script: vec![reply(
                    "<response><intent>done</intent><report end=\"true\">finished</report></response>",
                )],
            },
            setup(true),
        ),
        (
            LoopInput {
                name: "behavior_actions_then_final_report",
                source: "xllm tests::behavior_loop_end_to_end_with_tools2actions",
                request: behavior_request("xllm"),
                tools: &[],
                tool_results: vec![tool_obs("1", TOOL_SHELL, Success, "exit 0", "behavior-77\n")],
                script: vec![
                    reply("<response><intent>run</intent><actions><shell><![CDATA[echo behavior-77]]></shell></actions></response>"),
                    reply("<response><observation>saw it</observation><report end=\"true\"><![CDATA[{\"report\":\"结论 77\"}]]></report></response>"),
                ],
            },
            setup(false),
        ),
        (
            LoopInput {
                name: "behavior_progress_report_yields",
                source: "xllm tests::behavior_progress_report_continues_until_explicit_final_report",
                request: behavior_request("xllm"),
                tools: &[],
                tool_results: vec![],
                script: vec![reply("<response><report>intermediate finding</report></response>")],
            },
            setup(false),
        ),
        (
            LoopInput {
                name: "behavior_parse_error_correction_step",
                source: "llm_context behavior_loop::behavior_loop_parse_error_feeds_back_as_user_message_and_recovers",
                request: behavior_request("xllm"),
                tools: &[],
                tool_results: vec![],
                script: vec![reply(""), final_report()],
            },
            setup(true),
        ),
        (
            LoopInput {
                name: "behavior_final_report_with_actions_corrected",
                source: "llm_context behavior_loop::behavior_loop_rejects_ending_report_with_actions_before_side_effects",
                request: behavior_request("xllm"),
                tools: &[],
                tool_results: vec![],
                script: vec![
                    reply(r#"<actions><shell>must not run</shell></actions><report end="true">invalid</report>"#),
                    reply(r#"<report end="true">corrected final</report>"#),
                ],
            },
            setup(false),
        ),
        (
            LoopInput {
                name: "behavior_failing_action_stops_the_step",
                source: "llm_context error_policy::behavior_actions_stop_after_first_business_error_and_record_skipped",
                request: behavior_request("xllm"),
                tools: &[],
                tool_results: vec![
                    tool_obs("1", TOOL_SHELL, Success, "exit 0", "a\n"),
                    tool_obs("2", TOOL_SHELL, Failed, "exit 1", "b: command failed\n"),
                ],
                script: vec![
                    reply("<response>\n<intent>three actions</intent>\n<actions><shell>echo a</shell><shell>echo b</shell><shell>echo c</shell></actions>\n</response>"),
                    final_report(),
                ],
            },
            setup(false),
        ),
        (
            LoopInput {
                name: "behavior_native_calls_inside_a_step",
                source: "llm_context budgets::behavior_step_spans_rounds_and_holds_several_actions",
                request: with("xllm", &|r| r.tool_policy.max_tool_iterations = 5),
                tools: &["read_file"],
                tool_results: vec![
                    tool_obs("native-1", "read_file", Success, "read 1 line", "line 1\n"),
                    tool_obs("1", TOOL_SHELL, Success, "exit 0", "a\n"),
                    tool_obs("2", TOOL_SHELL, Success, "exit 0", "b\n"),
                ],
                script: vec![
                    calls(None, vec![call("read_file", "native-1", json!({"path": "notes.txt"}))]),
                    reply("<response><actions><shell>echo a</shell><shell>echo b</shell></actions></response>"),
                    final_report(),
                ],
            },
            setup(false),
        ),
        (
            LoopInput {
                name: "behavior_step_exhausts_tool_iterations",
                source: "xllm tests::behavior_file_actions_respect_arguments_and_persist_tool_iteration_limit",
                request: with("xllm", &|r| r.tool_policy.max_tool_iterations = 2),
                tools: &[],
                tool_results: vec![
                    tool_obs("1", "read_file", Success, "read 1 line", "behavior-fixture-3142"),
                    tool_obs("2", "read_file", Failed, "missing.txt: not found", ""),
                ],
                script: vec![
                    reply(r#"<response><actions><read_file path="fixture.txt"><![CDATA[]]></read_file></actions></response>"#),
                    reply("<response><actions><read_file><![CDATA[missing.txt]]></read_file></actions></response>"),
                    reply(r#"<response><actions><write_file path="excess.txt"><![CDATA[excess]]></write_file></actions></response>"#),
                ],
            },
            setup(true),
        ),
        (
            LoopInput {
                name: "behavior_native_calls_conflict_with_final_report",
                source: "llm_context behavior_loop::native_ending_report_conflict_pairs_calls_without_dispatch",
                request: behavior_request("xllm"),
                tools: &["read_file"],
                tool_results: vec![],
                script: vec![
                    calls(
                        Some(r#"<report end="true">invalid</report>"#),
                        vec![call("read_file", "blocked", json!({"path": "a.txt"}))],
                    ),
                    reply(r#"<report end="true">corrected</report>"#),
                ],
            },
            setup(false),
        ),
        (
            LoopInput {
                name: "behavior_action_dispatch_error",
                source: "llm_context error_policy::behavior_action_dispatch_failure_ends_run_with_sedimented_partial_step",
                request: behavior_request("xllm"),
                tools: &[],
                tool_results: vec![dispatch_error("1", "sandbox connection lost", true)],
                script: vec![
                    reply("<response>\n<intent>two actions</intent>\n<actions><shell>echo a</shell><shell>echo b</shell></actions>\n</response>"),
                    reply("never reached"),
                ],
            },
            setup(false),
        ),
        (
            LoopInput {
                name: "behavior_inherited_steps_with_timestamps",
                source: "llm_context behavior_loop::inner_transcript_renders_after_inherited_steps_and_clears_after_inference",
                request: behavior_request("do"),
                tools: &[],
                tool_results: vec![],
                script: vec![reply("<response><intent>start do</intent><report end=\"true\">finished</report></response>")],
            },
            seeded(true),
        ),
        (
            LoopInput {
                name: "behavior_inherited_steps_without_timestamps",
                source: "llm_context behavior_loop::inner_transcript_renders_after_inherited_steps_and_clears_after_inference",
                request: behavior_request("do"),
                tools: &[],
                tool_results: vec![],
                script: vec![reply("<response><intent>start do</intent><report end=\"true\">finished</report></response>")],
            },
            seeded(false),
        ),
    ];
    let mut cases = Vec::new();
    for (input, setup) in inputs {
        cases.push(run_loop_case(input, Some(&setup)).await);
    }
    cases
}

fn reply_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["reply"],
        "properties": {"reply": {"type": "string"}, "artifacts": {"type": "array", "items": {"type": "string"}}}
    })
}

async fn loop_script_cases() -> Vec<Value> {
    let echo = |id: &str| call("echo", id, json!({"msg": "ping"}));
    let with = |f: &dyn Fn(&mut LLMContextRequest)| {
        let mut r = base_request();
        f(&mut r);
        r
    };
    let strict_json = |r: &mut LLMContextRequest| {
        r.output = OutputSpec::Json {
            schema: None,
            strict: true,
            max_repairs: None,
        }
    };
    let contract = |r: &mut LLMContextRequest| {
        r.output = OutputSpec::Json {
            schema: Some(reply_schema()),
            strict: true,
            max_repairs: Some(1),
        }
    };
    let scripted_tools: &[&str] = &["a", "fail:b", "dispatch:b", "c", "shell"];
    let inputs = vec![
        LoopInput {
            name: "plain_done",
            source: "llm_context function_call_loop::done_without_tool_calls",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![reply_with_usage("hi there", 8)],
        },
        LoopInput {
            name: "tool_batch_then_done",
            source: "llm_context function_call_loop::one_tool_batch_then_done",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![ok_result("c-1", json!({"echo": {"msg": "ping"}}))],
            script: vec![
                calls(Some("calling echo"), vec![echo("c-1")]),
                reply("done after echo"),
            ],
        },
        LoopInput {
            name: "batches_count_tool_iterations_not_rounds",
            source: "llm_context budgets::function_call_loop_counts_rounds_and_tool_iterations_separately",
            request: with(&|r| r.tool_policy.max_tool_iterations = 5),
            tools: &["echo"],
            tool_results: vec![
                ok_result("c1", json!({"echo": {}})),
                ok_result("c2", json!({"echo": {}})),
                ok_result("c3", json!({"echo": {}})),
            ],
            script: vec![
                calls(None, vec![call("echo", "c1", json!({}))]),
                calls(None, vec![call("echo", "c2", json!({})), call("echo", "c3", json!({}))]),
                reply("final answer"),
            ],
        },
        LoopInput {
            name: "tool_iteration_budget",
            source: "llm_context function_call_loop::tool_iterations_budget_exhausted",
            request: with(&|r| r.tool_policy.max_tool_iterations = 2),
            tools: &["echo"],
            tool_results: vec![
                ok_result("c-1", json!({"echo": {"msg": "ping"}})),
                ok_result("c-2", json!({"echo": {"msg": "ping"}})),
            ],
            script: vec![
                calls(None, vec![echo("c-1")]),
                calls(None, vec![echo("c-2")]),
                calls(None, vec![echo("c-3")]),
            ],
        },
        LoopInput {
            name: "tool_failure_fed_back_then_clean_batch",
            source: "llm_context error_policy::clean_tool_batch_resets_consecutive_errors",
            request: base_request(),
            tools: scripted_tools,
            tool_results: vec![err_result("c-1", "bad args"), ok_result("c-2", json!("ok"))],
            script: vec![
                calls(None, vec![call("fail:b", "c-1", json!({}))]),
                calls(None, vec![call("a", "c-2", json!({}))]),
                reply("done"),
            ],
        },
        LoopInput {
            name: "batch_errors_count_once",
            source: "llm_context error_policy::multiple_tool_errors_in_one_batch_count_as_one_failure",
            request: with(&|r| r.error_policy.max_consecutive_errors = 2),
            tools: scripted_tools,
            tool_results: vec![
                err_result("c-1", "bad args"),
                err_result("c-2", "bad args"),
                err_result("c-3", "bad args"),
            ],
            script: vec![
                calls(
                    None,
                    vec![
                        call("fail:b", "c-1", json!({})),
                        call("fail:b", "c-2", json!({})),
                        call("fail:b", "c-3", json!({})),
                    ],
                ),
                reply("gave up on tools"),
            ],
        },
        LoopInput {
            name: "consecutive_errors_over_the_cap",
            source: "llm_context error_policy::successful_inference_does_not_reset_consecutive_errors",
            request: base_request(),
            tools: scripted_tools,
            tool_results: (1..=4).map(|i| err_result(&format!("c-{i}"), "bad args")).collect(),
            script: (1..=4)
                .map(|i| calls(None, vec![call("fail:b", &format!("c-{i}"), json!({}))]))
                .chain([reply("never reached")])
                .collect(),
        },
        LoopInput {
            name: "max_calls_per_round_rejected",
            source: "llm_context context_loop::run_inner max_calls_per_round (no dedicated test; shaped like error_policy::policy_rejection_keeps_transcript_paired_and_is_correctable)",
            request: base_request(),
            tools: scripted_tools,
            tool_results: vec![],
            script: vec![
                calls(None, (1..=5).map(|i| call("a", &format!("c-{i}"), json!({}))).collect()),
                reply("done without tools"),
            ],
        },
        LoopInput {
            name: "token_budget",
            source: "llm_context budgets::behavior_inner_context_inherits_outer_budget (function call form)",
            request: with(&|r| r.budget.max_total_tokens = Some(10)),
            tools: &["echo"],
            tool_results: vec![ok_result("c-1", json!({"echo": {"msg": "ping"}}))],
            script: vec![
                Ok(AiResponse {
                    usage: Some(AiUsage {
                        total_tokens: Some(8),
                        ..Default::default()
                    }),
                    ..AiResponse::from_parts(None, vec![echo("c-1")], vec![])
                }),
                reply_with_usage("finished", 8),
            ],
        },
        LoopInput {
            name: "provider_transient",
            source: "llm_context error_policy::provider_failure_ends_run_without_feedback_or_second_inference",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![
                fail(ProviderFailure::Transient, "temporary aicc failure"),
                reply("never reached"),
            ],
        },
        LoopInput {
            name: "provider_permanent",
            source: "llm_context error_policy::permanent_provider_error_and_adapter_internal_are_terminal",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![
                fail(ProviderFailure::Permanent, "invalid api key"),
                reply("never reached"),
            ],
        },
        LoopInput {
            name: "provider_unknown",
            source: "xllm tests::permanent_and_credential_errors_are_classified (Unknown)",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![fail(ProviderFailure::Unknown, "?")],
        },
        LoopInput {
            name: "provider_timeout",
            source: "llm_context error_policy::permanent_provider_error_and_adapter_internal_are_terminal (Timeout)",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![Err(LLMComputeError::Timeout), reply("never reached")],
        },
        LoopInput {
            name: "refused_credentials",
            source: "xllm tests::permanent_and_credential_errors_are_classified",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![Err(provider_error_from_rpc(
                "aicc helper.llm_chat failed",
                RPCErrors::TokenExpired("expired".into()),
            ))],
        },
        LoopInput {
            name: "refused_quota",
            source: "xllm tests::aicc_refusals_stay_refused_even_when_marked_retriable",
            request: base_request(),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![fail(
                ProviderFailure::Refused {
                    code: REFUSED_QUOTA_EXHAUSTED.into(),
                },
                "token quota is exhausted",
            )],
        },
        LoopInput {
            name: "provider_context_limit",
            source: "xllm tests::provider_context_refusal_compacts_and_continues_the_same_run",
            request: base_request(),
            tools: &["shell"],
            tool_results: vec![ok_result("c1", json!("before-limit"))],
            script: vec![
                calls(None, vec![call("shell", "c1", json!({"command": "echo before-limit"}))]),
                fail(ProviderFailure::ContextLimit, "context_length_exceeded"),
            ],
        },
        LoopInput {
            name: "dispatch_error_effect_unknown",
            source: "llm_context error_policy::tool_dispatch_failure_stops_batch_and_keeps_partial_results",
            request: base_request(),
            tools: scripted_tools,
            tool_results: vec![
                ok_result("c-1", json!("ok")),
                dispatch_error("c-2", "sandbox connection lost", true),
            ],
            script: vec![
                calls(
                    None,
                    vec![
                        call("a", "c-1", json!({})),
                        call("dispatch:b", "c-2", json!({})),
                        call("c", "c-3", json!({})),
                    ],
                ),
                reply("never reached in this run"),
            ],
        },
        LoopInput {
            name: "strict_json_corrected",
            source: "llm_context error_policy::strict_json_parse_error_is_fed_back_then_corrected",
            request: with(&strict_json),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![reply_with_usage("not json", 2), reply_with_usage(r#"{"answer": 42}"#, 3)],
        },
        LoopInput {
            name: "strict_json_errors_exhausted",
            source: "llm_context error_policy::persistent_strict_json_errors_terminate_on_fourth_failure",
            request: with(&strict_json),
            tools: &["echo"],
            tool_results: vec![],
            script: (0..6).map(|_| reply("still not json")).collect(),
        },
        LoopInput {
            name: "reply_contract_repaired",
            source: "llm_context reply_contract::a_mismatch_is_repaired_once",
            request: with(&contract),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![reply(r#"{"answer": "hi"}"#), reply(r#"{"reply": "hi"}"#)],
        },
        LoopInput {
            name: "reply_contract_exhausted",
            source: "llm_context reply_contract::a_second_failure_returns_the_raw_reply",
            request: with(&contract),
            tools: &["echo"],
            tool_results: vec![],
            script: vec![
                reply("plain words"),
                reply("still plain words"),
                reply(r#"{"reply": "never reached"}"#),
            ],
        },
    ];
    let mut cases = Vec::new();
    for input in inputs {
        cases.push(loop_case(input).await);
    }
    cases
}

#[tokio::test]
async fn loop_scripts() {
    let mut cases = loop_script_cases().await;
    cases.extend(behavior_loop_cases().await);
    check_or_regen("loop_scripts.json", cases);
}

// =========================================================================
// (d) provider_errors.json
// =========================================================================

/// An in-process AICC answering every chat call with `response`.
struct FixedAicc {
    response: LlmChatInvokeResponse,
}

#[async_trait]
impl AiccHandler for FixedAicc {
    async fn handle_cancel(
        &self,
        _task_id: &str,
        _ctx: RPCContext,
    ) -> Result<CancelResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod("cancel".into()))
    }

    async fn handle_chat_completions_create(
        &self,
        _request: LlmChatInvokeRequest,
        _ctx: RPCContext,
    ) -> Result<LlmChatInvokeResponse, RPCErrors> {
        Ok(self.response.clone())
    }

    async fn handle_helper_llm_chat(
        &self,
        _request: LlmChatHelperRequest,
        _ctx: RPCContext,
    ) -> Result<LlmChatInvokeResponse, RPCErrors> {
        Ok(self.response.clone())
    }
}

fn inference_request() -> LlmInferenceRequest {
    LlmInferenceRequest {
        trace_id: Some("xllm-trace".into()),
        messages: vec![AiMessage::text(AiRole::User, "hello")],
        model_alias: "llm.chat".into(),
        fallbacks: vec![],
        temperature: None,
        max_completion_tokens: None,
        force_json: false,
        json_schema: None,
        provider_options: None,
        disable_capabilities: vec![],
        tool_specs: vec![],
        allow_tool_calls: false,
        abort: llm_context::interrupt::InferenceAbortToken::noop(),
    }
}

/// The `RPCErrors` only the Rust kRPC client produces (`rpc_local`).
async fn local_rpc_error(input: &Value) -> RPCErrors {
    let message = input["message"].as_str().unwrap_or("x").to_string();
    match input["variant"].as_str().unwrap() {
        "invalid_token" => RPCErrors::InvalidToken(message),
        "token_expired" => RPCErrors::TokenExpired(message),
        "no_permission" => RPCErrors::NoPermission(message),
        "invalid_password" => RPCErrors::InvalidPassword,
        "user_not_found" => RPCErrors::UserNotFound(message),
        "unknown_method" => RPCErrors::UnknownMethod(message),
        "service_not_valid" => RPCErrors::ServiceNotValid(message),
        "s2s_permanent" => RPCErrors::S2sPermanentError(message),
        "s2s_transient" => RPCErrors::S2sTransientError(message),
        "parse_request_error" => RPCErrors::ParseRequestError(message),
        "key_not_exist" => RPCErrors::KeyNotExist(message),
        // A refused connection to a port nobody listens on.
        "http_transport_connect" => {
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let err = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(format!("http://127.0.0.1:{port}/kapi/aicc"))
                .send()
                .await
                .unwrap_err();
            RPCErrors::HttpTransportError(err)
        }
        // What an in-process AICC returns (`AiccError::to_krpc_error`).
        "aicc_in_process" => serde_json::from_value::<AiccError>(input["aicc_error"].clone())
            .unwrap()
            .to_krpc_error(),
        other => panic!("unknown rpc_local variant `{other}`"),
    }
}

/// Evaluate one `input` the way xllm meets it.
async fn provider_result(input: &Value) -> Result<(), LLMComputeError> {
    const CONTEXT: &str = "aicc helper.llm_chat failed";
    let str_of = |key: &str| input[key].as_str().unwrap().to_string();
    match input["kind"].as_str().unwrap() {
        "rpc_detailed" => Err(provider_error_from_rpc(
            CONTEXT,
            RPCErrors::Detailed(RPCErrorDetail::new(
                str_of("code"),
                str_of("message"),
                input.get("data").cloned(),
            )),
        )),
        // A failed kRPC response without `code`: the Rust client's form.
        "rpc_reason" => Err(provider_error_from_rpc(
            CONTEXT,
            RPCErrors::ReasonError(format!("rpc call error: {}", str_of("message"))),
        )),
        // A non-2xx kRPC response: the Rust client's form.
        "rpc_http_status" => {
            let status = input["status"].as_u64().unwrap();
            Err(provider_error_from_rpc(
                CONTEXT,
                RPCErrors::Detailed(RPCErrorDetail::new(
                    "http_error",
                    format!("rpc call error: {status}"),
                    Some(json!({ "status": status })),
                )),
            ))
        }
        "rpc_local" => Err(provider_error_from_rpc(
            CONTEXT,
            local_rpc_error(input).await,
        )),
        "aicc_response" => {
            let client = AiccClient::new_in_process(Box::new(FixedAicc {
                response: serde_json::from_value(input["response"].clone()).unwrap(),
            }));
            AiccLlmClient::infer_with_client(&client, inference_request())
                .await
                .map(|_| ())
        }
        "openai_http" => Err(openai_http_failure(
            input["status"].as_u64().unwrap() as u16,
            input["body"].as_str().unwrap(),
        )),
        other => panic!("unknown provider input `{other}`"),
    }
}

async fn provider_case(name: &str, input: Value) -> Value {
    let expect = match provider_result(&input).await {
        Ok(()) => json!({ "ok": true }),
        Err(LLMComputeError::Provider { failure, .. }) => json!({ "failure": failure }),
        Err(other) => panic!("{name}: not a provider failure: {other:?}"),
    };
    let ts = !matches!(input["kind"].as_str(), Some("rpc_local"));
    json!({ "name": name, "input": input, "expect": expect, "ts": ts })
}

/// `rpc_detailed` input of an AICC error as the AICC service sends a
/// policy refusal (`AiccError::to_krpc_error`: code + the error as data).
fn aicc_detailed(code: AiccErrorCode, retriable: bool, details: Option<Value>) -> Value {
    let mut error = AiccError::new(code, format!("aicc {}", code.as_str()));
    error.retriable = retriable;
    error.details = details;
    json!({
        "kind": "rpc_detailed",
        "code": code.as_str(),
        "message": format!("{} ({})", error.message, code.as_str()),
        "data": error,
    })
}

fn aicc_failed(error: Option<AiccError>) -> Value {
    let response = LlmChatInvokeResponse {
        task_id: "aicc-task".into(),
        status: AiMethodStatus::Failed,
        message: None,
        tool_calls: vec![],
        usage: None,
        cost: None,
        finish_reason: None,
        provider_task_ref: None,
        route_trace: None,
        event_ref: Some("event-1".into()),
        error,
    };
    json!({ "kind": "aicc_response", "response": response })
}

fn aicc_error(code: AiccErrorCode, retriable: bool) -> AiccError {
    let mut e = AiccError::new(code, format!("aicc {}", code.as_str()));
    e.retriable = retriable;
    e
}

async fn provider_error_cases() -> Vec<Value> {
    use AiccErrorCode as C;
    let succeeded = |message: Option<AiMessage>| {
        json!({
            "kind": "aicc_response",
            "response": {
                "task_id": "aicc-task",
                "status": "succeeded",
                "message": message,
                "usage": {"input_tokens": 4, "output_tokens": 2, "total_tokens": 6}
            }
        })
    };
    let running = json!({
        "kind": "aicc_response",
        "response": {"task_id": "aicc-task", "status": "running"}
    });
    let inputs: Vec<(&str, Value)> = vec![
        // ---- rpc_detailed: AICC errors carried as data ----
        (
            "rpc_quota_exhausted",
            aicc_detailed(C::QuotaExhausted, false, None),
        ),
        (
            "rpc_quota_exhausted_marked_retriable",
            aicc_detailed(C::QuotaExhausted, true, None),
        ),
        (
            "rpc_quota_budget_exceeded",
            aicc_detailed(C::QuotaBudgetExceeded, false, None),
        ),
        (
            "rpc_resource_not_permitted",
            aicc_detailed(C::ResourceNotPermitted, false, None),
        ),
        (
            "rpc_attribution_unavailable",
            aicc_detailed(C::AttributionUnavailable, false, None),
        ),
        (
            "rpc_quota_unavailable",
            aicc_detailed(C::QuotaUnavailable, false, None),
        ),
        (
            "rpc_context_length_exceeded",
            aicc_detailed(C::ContextLengthExceeded, false, None),
        ),
        (
            "rpc_provider_413",
            aicc_detailed(C::ProviderError, false, Some(json!({"http_status": 413}))),
        ),
        ("rpc_aicc_timeout", aicc_detailed(C::Timeout, false, None)),
        (
            "rpc_aicc_retriable",
            aicc_detailed(C::ProviderError, true, None),
        ),
        (
            "rpc_aicc_not_retriable",
            aicc_detailed(C::ProviderError, false, None),
        ),
        (
            "rpc_detailed_without_aicc_data",
            json!({"kind": "rpc_detailed", "code": "access_denied", "message": "denied (access_denied)", "data": {"reason": "x"}}),
        ),
        // ---- rpc_http_status ----
        (
            "rpc_http_413",
            json!({"kind": "rpc_http_status", "status": 413}),
        ),
        (
            "rpc_http_408",
            json!({"kind": "rpc_http_status", "status": 408}),
        ),
        (
            "rpc_http_504",
            json!({"kind": "rpc_http_status", "status": 504}),
        ),
        (
            "rpc_http_401",
            json!({"kind": "rpc_http_status", "status": 401}),
        ),
        (
            "rpc_http_403",
            json!({"kind": "rpc_http_status", "status": 403}),
        ),
        (
            "rpc_http_500",
            json!({"kind": "rpc_http_status", "status": 500}),
        ),
        (
            "rpc_http_502",
            json!({"kind": "rpc_http_status", "status": 502}),
        ),
        // ---- rpc_reason: no code on the wire ----
        (
            "rpc_reason_text_is_not_read",
            json!({"kind": "rpc_reason", "message": "Failed due to reason: {\"code\":\"timeout\",\"message\":\"slow\",\"retriable\":true}"}),
        ),
        // ---- rpc_local (Rust only) ----
        (
            "local_invalid_token",
            json!({"kind": "rpc_local", "variant": "invalid_token", "message": "bad signature"}),
        ),
        (
            "local_token_expired",
            json!({"kind": "rpc_local", "variant": "token_expired", "message": "expired"}),
        ),
        (
            "local_no_permission",
            json!({"kind": "rpc_local", "variant": "no_permission", "message": "denied"}),
        ),
        (
            "local_unknown_method",
            json!({"kind": "rpc_local", "variant": "unknown_method", "message": "helper.llm_chat"}),
        ),
        (
            "local_service_not_valid",
            json!({"kind": "rpc_local", "variant": "service_not_valid", "message": "aicc"}),
        ),
        (
            "local_s2s_transient",
            json!({"kind": "rpc_local", "variant": "s2s_transient", "message": "timeout"}),
        ),
        (
            "local_s2s_permanent",
            json!({"kind": "rpc_local", "variant": "s2s_permanent", "message": "bad key"}),
        ),
        (
            "local_key_not_exist",
            json!({"kind": "rpc_local", "variant": "key_not_exist", "message": "x"}),
        ),
        (
            "local_http_transport_connect",
            json!({"kind": "rpc_local", "variant": "http_transport_connect"}),
        ),
        (
            "local_aicc_in_process_context_length",
            json!({"kind": "rpc_local", "variant": "aicc_in_process", "aicc_error": aicc_error(C::ContextLengthExceeded, false)}),
        ),
        (
            "local_aicc_in_process_retriable",
            json!({"kind": "rpc_local", "variant": "aicc_in_process", "aicc_error": aicc_error(C::ProviderError, true)}),
        ),
        // ---- aicc_response ----
        (
            "aicc_failed_quota_exhausted",
            aicc_failed(Some(aicc_error(C::QuotaExhausted, true))),
        ),
        (
            "aicc_failed_context_length",
            aicc_failed(Some(aicc_error(C::ContextLengthExceeded, false))),
        ),
        (
            "aicc_failed_timeout",
            aicc_failed(Some(aicc_error(C::Timeout, false))),
        ),
        (
            "aicc_failed_retriable",
            aicc_failed(Some(aicc_error(C::ProviderError, true))),
        ),
        (
            "aicc_failed_not_retriable",
            aicc_failed(Some(aicc_error(C::ProviderError, false))),
        ),
        ("aicc_failed_without_error", aicc_failed(None)),
        ("aicc_running_is_not_polled", running),
        (
            "aicc_succeeded",
            succeeded(Some(AiMessage::text(AiRole::Assistant, "done"))),
        ),
        ("aicc_succeeded_without_message", succeeded(None)),
        // ---- openai_http ----
        (
            "openai_401",
            json!({"kind": "openai_http", "status": 401, "body": "{\"error\":{\"message\":\"Incorrect API key\",\"code\":\"invalid_api_key\"}}"}),
        ),
        (
            "openai_403",
            json!({"kind": "openai_http", "status": 403, "body": "forbidden"}),
        ),
        (
            "openai_context_length_code",
            json!({"kind": "openai_http", "status": 400, "body": "{\"error\":{\"message\":\"maximum context length is 8192 tokens\",\"type\":\"invalid_request_error\",\"code\":\"context_length_exceeded\"}}"}),
        ),
        (
            "openai_400",
            json!({"kind": "openai_http", "status": 400, "body": "{\"error\":{\"message\":\"maximum context length is 8192 tokens\",\"code\":null}}"}),
        ),
        (
            "openai_404",
            json!({"kind": "openai_http", "status": 404, "body": "model not found"}),
        ),
        (
            "openai_422",
            json!({"kind": "openai_http", "status": 422, "body": ""}),
        ),
        (
            "openai_429",
            json!({"kind": "openai_http", "status": 429, "body": "rate limited"}),
        ),
        (
            "openai_503",
            json!({"kind": "openai_http", "status": 503, "body": ""}),
        ),
        (
            "openai_413",
            json!({"kind": "openai_http", "status": 413, "body": "payload too large"}),
        ),
        (
            "openai_418",
            json!({"kind": "openai_http", "status": 418, "body": ""}),
        ),
    ];
    let mut cases = Vec::new();
    for (name, input) in inputs {
        cases.push(provider_case(name, input).await);
    }
    cases
}

#[tokio::test]
async fn provider_errors() {
    check_or_regen("provider_errors.json", provider_error_cases().await);
}

// =========================================================================
// (f) hints.json
// =========================================================================

fn tool_result(status: AgentToolStatus, summary: &str, output: &str) -> AgentToolResult {
    serde_json::from_value(json!({
        "agent_tool_protocol": crate::AGENT_TOOL_PROTOCOL_VERSION,
        "tool": "read",
        "status": status,
        "summary": summary,
        "output": output,
    }))
    .unwrap()
}

fn hinted(mut r: AgentToolResult, hints: &[(&str, Option<&str>)]) -> AgentToolResult {
    r.hints = hints
        .iter()
        .map(|(text, once)| ResultHint {
            text: text.to_string(),
            once: once.map(str::to_string),
        })
        .collect();
    r
}

fn deduped(mut r: AgentToolResult, key: &str, hint: &str) -> AgentToolResult {
    r.dedup = Some(ResultDedup {
        key: key.into(),
        hint: hint.into(),
    });
    r
}

enum HintStep {
    Result(AgentToolResult),
    ClearDedup,
}

/// Run `steps` over one ledger as `XllmToolManager::call_tool` does.
fn hints_case(name: &str, steps: Vec<HintStep>) -> Value {
    let ledger = MemoryHintLedger::new();
    let mut out = Vec::new();
    for (i, step) in steps.into_iter().enumerate() {
        match step {
            HintStep::ClearDedup => {
                ledger.clear(HINT_NS_DEDUP);
                out.push(json!({ "clear_dedup": true }));
            }
            HintStep::Result(result) => {
                let input = serde_json::to_value(&result).unwrap();
                let mut res = result;
                let shown = apply_hints(&mut res, ledger.as_ref());
                let expect =
                    match map_result_to_observation_with_hints(format!("c{i}"), res, &shown) {
                        Observation::Success { content, .. } => {
                            json!({ "kind": "success", "text": content.as_str().unwrap() })
                        }
                        Observation::Error { message, .. } => {
                            json!({ "kind": "error", "text": message })
                        }
                        other => panic!("{other:?}"),
                    };
                out.push(json!({ "result": input, "expect": expect }));
            }
        }
    }
    json!({ "name": name, "steps": out, "expect_once": ledger.keys(HINT_NS_ONCE) })
}

fn hints_cases() -> Vec<Value> {
    use AgentToolStatus::{Error, Success};
    let body = || tool_result(Success, "read 1 line", "body\n");
    let followup = "Follow a long command with `wait_task`.";
    vec![
        hints_case(
            "once_hint_shows_once_plain_hint_always",
            vec![
                HintStep::Result(hinted(
                    body(),
                    &[(followup, Some("shell.task_followup")), ("always", None)],
                )),
                HintStep::Result(hinted(
                    body(),
                    &[(followup, Some("shell.task_followup")), ("always", None)],
                )),
                HintStep::Result(hinted(body(), &[(followup, Some("shell.task_followup:2"))])),
            ],
        ),
        hints_case(
            "dedup_repeat_and_clear",
            vec![
                HintStep::Result(deduped(body(), "read:/a:1", "unchanged since you read it")),
                HintStep::Result(deduped(body(), "read:/a:1", "unchanged since you read it")),
                HintStep::Result(deduped(body(), "read:/b:1", "unchanged since you read it")),
                HintStep::ClearDedup,
                HintStep::Result(deduped(body(), "read:/a:1", "unchanged since you read it")),
            ],
        ),
        hints_case(
            "dedup_with_hints",
            vec![
                HintStep::Result(hinted(
                    deduped(body(), "read:/a:1", "unchanged"),
                    &[("first read", Some("read.first")), ("plain", None)],
                )),
                HintStep::Result(hinted(
                    deduped(body(), "read:/a:1", "unchanged"),
                    &[("first read", Some("read.first")), ("plain", None)],
                )),
            ],
        ),
        hints_case(
            "empty_hints_dropped",
            vec![
                HintStep::Result(hinted(
                    body(),
                    &[
                        ("   ", Some("blank.once")),
                        ("", None),
                        (" kept ", Some("")),
                    ],
                )),
                HintStep::Result(hinted(body(), &[(" kept ", Some(""))])),
            ],
        ),
        hints_case(
            "error_status_rendering",
            vec![
                HintStep::Result(hinted(
                    tool_result(Error, "exit 2", "stderr: no such file\n"),
                    &[("Check the path first.", Some("read.path"))],
                )),
                HintStep::Result(tool_result(Error, "exit 2", "")),
                HintStep::Result(tool_result(Error, "", "")),
            ],
        ),
        hints_case(
            "success_without_output_uses_summary",
            vec![HintStep::Result(hinted(
                tool_result(Success, "wrote 3 bytes", " "),
                &[("Read it back to verify.", None)],
            ))],
        ),
    ]
}

#[test]
fn hints() {
    check_or_regen("hints.json", hints_cases());
}

// =========================================================================
// (g) task_text.json
// =========================================================================

fn followup_json(f: &TaskFollowup) -> Value {
    match f {
        TaskFollowup::Tools => json!({ "kind": "tools" }),
        TaskFollowup::Command(cli) => json!({ "kind": "command", "cli": cli }),
        TaskFollowup::Input(None) => json!({ "kind": "input" }),
        TaskFollowup::Input(Some(cli)) => json!({ "kind": "input", "cli": cli }),
    }
}

fn observation_case(name: &str, state: TaskState, followup: TaskFollowup) -> Value {
    let (call_id, task_id) = ("call-1", "local:7");
    json!({
        "name": name,
        "fn": "observation",
        "call_id": call_id,
        "task_id": task_id,
        "state": state,
        "followup": followup_json(&followup),
        "expect": task_state_observation_via(call_id, task_id, &state, &followup),
    })
}

fn general_hint_case(name: &str, followup: TaskFollowup) -> Value {
    json!({
        "name": name,
        "fn": "general_hint",
        "followup": followup_json(&followup),
        "expect": followup.general_hint(),
    })
}

fn task_text_cases() -> Vec<Value> {
    let running = |elapsed_ms: u64, tail: &str, cancellable: bool| TaskState::Running {
        brief: "cargo build --release".into(),
        elapsed_ms,
        output_tail: tail.into(),
        cancellable,
    };
    let finished = |success: bool, output: &str| {
        TaskState::Finished(TaskResult {
            success,
            output: output.into(),
            tool_result: None,
        })
    };
    let cli = "buckyos-task";
    vec![
        observation_case(
            "running_seconds_no_tail",
            running(5_400, "", false),
            TaskFollowup::Tools,
        ),
        observation_case(
            "running_minutes_with_tail",
            running(65_000, "Compiling x v0.1.0\nCompiling y v0.2.0\n", false),
            TaskFollowup::Tools,
        ),
        observation_case(
            "running_hours_cancellable",
            running(3_725_000, "   ", true),
            TaskFollowup::Tools,
        ),
        observation_case(
            "running_command_cancellable",
            running(12_000, "step 3/9", true),
            TaskFollowup::Command(cli.into()),
        ),
        observation_case(
            "running_command_not_cancellable",
            running(600_000, "", false),
            TaskFollowup::Command(cli.into()),
        ),
        observation_case(
            "running_input_with_cli_cancellable",
            running(1_000, "", true),
            TaskFollowup::Input(Some(cli.into())),
        ),
        observation_case(
            "running_input_with_cli_not_cancellable",
            running(1_000, "", false),
            TaskFollowup::Input(Some(cli.into())),
        ),
        observation_case(
            "running_input_without_cli_cancellable",
            running(59_999, "tail", true),
            TaskFollowup::Input(None),
        ),
        observation_case(
            "finished_success",
            finished(true, "done\n"),
            TaskFollowup::Tools,
        ),
        observation_case(
            "finished_success_with_tool_result",
            TaskState::Finished(TaskResult {
                success: true,
                output: "built".into(),
                tool_result: Some(ToolResultView {
                    status: ToolResultStatusView::Success,
                    summary: "cargo build => success".into(),
                    return_code: Some(0),
                    ..Default::default()
                }),
            }),
            TaskFollowup::Tools,
        ),
        observation_case(
            "finished_failure",
            finished(false, "exit=2"),
            TaskFollowup::Tools,
        ),
        observation_case(
            "finished_failure_empty_output",
            finished(false, " \n"),
            TaskFollowup::Tools,
        ),
        observation_case(
            "unknown",
            TaskState::Unknown {
                reason: "task manager restarted".into(),
            },
            TaskFollowup::Command(cli.into()),
        ),
        general_hint_case("general_hint_tools", TaskFollowup::Tools),
        general_hint_case("general_hint_command", TaskFollowup::Command(cli.into())),
        general_hint_case(
            "general_hint_input_with_cli",
            TaskFollowup::Input(Some(cli.into())),
        ),
        general_hint_case("general_hint_input_without_cli", TaskFollowup::Input(None)),
    ]
}

#[test]
fn task_text() {
    check_or_regen("task_text.json", task_text_cases());
}

// =========================================================================
// (i) compress.json
// =========================================================================

/// Answers summary call `i` with `replies[i]`; records every request.
struct SummaryLlm {
    replies: Vec<String>,
    requests: StdMutex<Vec<Value>>,
}

#[async_trait]
impl LlmClient for SummaryLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        let mut requests = self.requests.lock().unwrap();
        let reply = self
            .replies
            .get(requests.len())
            .expect("a compress vector lists a reply for every summary call")
            .clone();
        requests.push(json!({
            "model_alias": req.model_alias,
            "max_completion_tokens": req.max_completion_tokens,
            "force_json": req.force_json,
            "json_schema": req.json_schema,
            "allow_tool_calls": req.allow_tool_calls,
            "system": req.messages[0].text_content(),
            "user": req.messages[1].text_content(),
        }));
        Ok(AiResponse::text(reply))
    }
}

fn summary_deps(replies: &[String]) -> (LLMContextDeps, Arc<SummaryLlm>) {
    let llm = Arc::new(SummaryLlm {
        replies: replies.to_vec(),
        requests: StdMutex::new(Vec::new()),
    });
    let tools = Arc::new(ReplayTools {
        specs: Vec::new(),
        results: BTreeMap::new(),
        dispatched: StdMutex::new(Vec::new()),
    });
    (LLMContextDeps::new(llm.clone(), tools), llm)
}

fn summary_reply(summary: &str, verbatim: &[&str]) -> String {
    json!({"summary": summary, "verbatim": verbatim}).to_string()
}

fn tool_use(id: &str, name: &str, args: Value) -> AiMessage {
    AiMessage::new(
        AiRole::Assistant,
        vec![AiContent::ToolUse {
            call_id: id.into(),
            name: name.into(),
            args: serde_json::from_value(args).unwrap(),
        }],
    )
}

fn tool_result_message(id: &str, text: String) -> AiMessage {
    AiMessage::new(
        AiRole::Tool,
        vec![AiContent::tool_result_text(id, text, false)],
    )
}

/// One function call round: a `shell` call (one argument, so its JSON text
/// is deterministic) and its result.
fn fc_round(i: usize, result_chars: usize) -> Vec<AiMessage> {
    let id = format!("call-{i}");
    vec![
        tool_use(&id, "shell", json!({"command": format!("cmd {i}")})),
        tool_result_message(&id, format!("out {i} {}", "x".repeat(result_chars))),
    ]
}

fn fc_history(rounds: usize, result_chars: usize) -> Vec<AiMessage> {
    let mut h = vec![
        AiMessage::text(AiRole::System, "sys"),
        AiMessage::text(AiRole::User, "task: research the thing"),
    ];
    for i in 0..rounds {
        h.extend(fc_round(i, result_chars));
    }
    h
}

fn options_json(o: &CompressOptions) -> Value {
    let mut v = json!({
        "target_token_budget": o.target_token_budget,
        "model_alias": o.model_alias,
        "summary_chunk_tokens": o.summary_chunk_tokens,
    });
    if let Some(s) = &o.extra_focus_prompt {
        v["extra_focus_prompt"] = json!(s);
    }
    if let Some(s) = &o.note {
        v["note"] = json!(s);
    }
    v
}

/// `compress_history` over `history`; `replies` are the summarizer's
/// replies by call number (the case keeps the ones used).
async fn compress_case(
    name: &str,
    source: &str,
    history: Vec<AiMessage>,
    opts: CompressOptions,
    replies: &[String],
) -> Value {
    let (deps, llm) = summary_deps(replies);
    let result = compress_history(&history, &deps, &opts).await;
    let calls = llm.requests.lock().unwrap().clone();
    let mut expect = match result {
        Ok(out) => {
            let mut report = serde_json::to_value(&out.report).unwrap();
            report.as_object_mut().unwrap().remove("elapsed_ms");
            json!({
                "messages": out.messages,
                "head_end": out.head_end,
                "tail_start": out.tail_start,
                "summary": out.summary,
                "memory_candidates": out.memory_candidates,
                "report": report,
            })
        }
        Err(e) => json!({ "error": { "kind": serde_json::to_value(&e).unwrap()["kind"] } }),
    };
    expect["calls"] = json!(calls);
    json!({
        "name": name,
        "fn": "compress_history",
        "source": source,
        "history": history,
        "options": options_json(&opts),
        "replies": replies[..calls.len()].to_vec(),
        "expect": expect,
    })
}

fn segment_case(name: &str, history: Vec<AiMessage>, start: usize) -> Value {
    let units: Vec<Value> = segment(&history, start)
        .iter()
        .map(|u| {
            json!({
                "start": u.start,
                "end": u.end,
                "kind": if u.kind == UnitKind::Summary { "summary" } else { "round" },
            })
        })
        .collect();
    json!({ "name": name, "fn": "segment", "history": history, "start": start, "expect": units })
}

fn summary_round_trip_case(name: &str, summary: CompressedSummary, note: Option<&str>) -> Value {
    let message = summary_message(&summary, note);
    let mut case = json!({
        "name": name,
        "fn": "summary_message",
        "summary": summary,
        "expect": { "message": message, "parsed": parse_summary_message(&message) },
    });
    if let Some(n) = note {
        case["note"] = json!(n);
    }
    case
}

async fn compress_cases() -> Vec<Value> {
    let r = |s: &str| vec![s.to_string()];
    let phone = "+1 (408) 555-0100";
    let sha = "3f2a9c1e";
    let url = "https://example.com/a?b=1";
    // The history of a run compressed once before.
    let (deps, _) = summary_deps(&[summary_reply("FIRST", &[phone, sha])]);
    let mut rolled = compress_history(
        &fc_history(10, 1_000),
        &deps,
        &CompressOptions::new(2_000, "m"),
    )
    .await
    .unwrap()
    .messages;
    for i in 100..108 {
        rolled.extend(fc_round(i, 1_000));
    }
    let mut oversized_head = fc_history(8, 1_000);
    oversized_head[3] = tool_result_message("call-0", "h".repeat(4_000));
    let mut steps = vec![
        AiMessage::text(AiRole::System, "sys"),
        AiMessage::text(AiRole::User, "task: tidy the notes"),
    ];
    for i in 0..10 {
        steps.push(AiMessage::text(
            AiRole::Assistant,
            format!("<response><intent>step {i}</intent><actions><shell>echo step {i}</shell></actions></response>"),
        ));
        steps.push(AiMessage::text(
            AiRole::User,
            format!(
                "{STEP_RESULTS_PREFIX}>>\n[{i}] shell => success\n{}",
                "s".repeat(800)
            ),
        ));
    }
    let mut big_system = fc_history(3, 100);
    big_system[0] = AiMessage::text(AiRole::System, "S".repeat(9_000));
    let mut reports = fc_history(1, 10);
    reports.push(tool_use(
        "rep-1",
        "report",
        json!({"content": format!("{}END", "R".repeat(3_000))}),
    ));
    reports.push(tool_result_message("rep-1", "submitted".into()));
    reports.push(tool_use(
        "rep-2",
        "report",
        json!({"content": format!("{}TAIL", "L".repeat(5_000))}),
    ));
    reports.push(tool_result_message("rep-2", "submitted".into()));
    for i in 10..18 {
        reports.extend(fc_round(i, 1_000));
    }
    let chunk_replies: Vec<String> = (1..=10)
        .map(|i| summary_reply(&format!("C{i}"), &[]))
        .collect();

    let mut cases = vec![
        compress_case(
            "under_budget_unchanged",
            "llm_compress tests::under_budget_returns_as_is",
            fc_history(2, 10),
            CompressOptions::new(10_000, "m"),
            &[],
        )
        .await,
        compress_case(
            "head_summary_note_and_raw_tail",
            "llm_compress tests::function_call_history_keeps_head_summary_and_raw_tail",
            fc_history(20, 2_000),
            CompressOptions::new(10_000, SUMMARIZE_MODEL_ALIAS).with_note("NOTE"),
            &r(&summary_reply("SUMMARY_1", &[])),
        )
        .await,
        compress_case(
            "rolling_merge_keeps_one_summary_and_keep_exact",
            "llm_compress tests::rolling_merge_keeps_a_single_summary + keep_exact_entries_survive_rolling_merges_verbatim",
            rolled,
            CompressOptions::new(2_000, "m"),
            &r(&summary_reply("SECOND", &[sha, url, "  "])),
        )
        .await,
        compress_case(
            "large_range_folded_in_chunks",
            "llm_compress tests::a_large_range_is_folded_in_chunks",
            fc_history(10, 1_000),
            CompressOptions::new(2_000, "m").with_chunk_tokens(600),
            &chunk_replies,
        )
        .await,
        compress_case(
            "loops_until_the_target_is_met",
            "llm_compress tests::loops_until_the_target_is_met",
            fc_history(70, 500),
            CompressOptions::new(9_000, "m"),
            &[summary_reply(&"B".repeat(36_000), &[]), summary_reply("SMALL", &[])],
        )
        .await,
        compress_case(
            "oversized_head_keeps_the_opening_input",
            "llm_compress tests::an_oversized_head_is_summarized",
            oversized_head,
            CompressOptions::new(2_000, "m"),
            &r(&summary_reply("S", &[])),
        )
        .await,
        compress_case(
            "empty_summary_is_an_error",
            "llm_compress tests::empty_summary_text_errors",
            fc_history(10, 800),
            CompressOptions::new(2_000, "m"),
            &r("   "),
        )
        .await,
        compress_case(
            "plain_text_reply_is_the_summary",
            "llm_compress tests::plain_text_reply_is_the_summary",
            fc_history(10, 800),
            CompressOptions::new(2_000, "m"),
            &r("just markdown"),
        )
        .await,
        compress_case(
            "behavior_steps_with_focus_prompt",
            "llm_compress tests::function_call_rounds_and_dialogue_turns (behavior steps)",
            steps,
            CompressOptions {
                extra_focus_prompt: Some("Keep the step numbers.".into()),
                ..CompressOptions::new(2_000, "m")
            },
            &r(&summary_reply("steps 1-9 done", &["echo step 9"])),
        )
        .await,
        compress_case(
            "system_prefix_over_target_is_no_range",
            "llm_compress compress_history (no_range)",
            big_system,
            CompressOptions::new(2_000, "m"),
            &[],
        )
        .await,
        compress_case(
            "reports_reach_the_summarizer_clipped",
            "llm_compress tests::reports_and_intents_reach_the_summarizer_in_full",
            reports,
            CompressOptions::new(2_000, "m"),
            &r(&summary_reply("S", &[])),
        )
        .await,
    ];

    // ---- segment ----
    let mut dialogue = vec![AiMessage::text(AiRole::System, "sys")];
    for i in 0..3 {
        dialogue.push(AiMessage::text(AiRole::User, format!("q{i}")));
        dialogue.push(AiMessage::text(AiRole::Assistant, format!("a{i}")));
    }
    let mut mixed = fc_history(2, 10);
    mixed.insert(6, AiMessage::text(AiRole::User, "new input"));
    mixed.push(AiMessage::text(AiRole::Assistant, "<step>plan</step>"));
    mixed.push(AiMessage::text(
        AiRole::User,
        format!("{STEP_RESULTS_PREFIX} behavior=\"plan\" step=\"3\">>\nok"),
    ));
    mixed.insert(
        4,
        summary_message(
            &CompressedSummary {
                summary: "earlier work".into(),
                keep_exact: vec![],
            },
            None,
        ),
    );
    cases.extend([
        segment_case("segment_function_call_rounds", fc_history(3, 10), 1),
        segment_case("segment_dialogue_turns", dialogue, 1),
        segment_case("segment_mid_run_input_step_results_and_summary", mixed, 1),
    ]);

    // ---- summary_message / parse_summary_message ----
    cases.push(summary_round_trip_case(
        "summary_message_round_trip",
        CompressedSummary {
            summary: "\n  Verified v2.\n\nNext: ship.  \n".into(),
            keep_exact: vec![phone.into(), url.into()],
        },
        Some("  Files in /work are intact.  "),
    ));
    let not_user = AiMessage::text(
        AiRole::Assistant,
        summary_message(
            &CompressedSummary {
                summary: "x".into(),
                keep_exact: vec![],
            },
            None,
        )
        .text_content(),
    );
    cases.push(json!({
        "name": "parse_summary_message_needs_the_user_role",
        "fn": "parse_summary_message",
        "message": not_user,
        "expect": parse_summary_message(&not_user),
    }));

    // ---- merge_keep_exact ----
    let (deps, _) = summary_deps(&[]);
    let keep_before = vec!["kept-1".to_string()];
    let mut new: Vec<String> = vec![
        "kept-1".into(),
        "  spaced    out\tentry ".into(),
        "".into(),
        "L".repeat(1_100),
    ];
    new.extend((0..400).map(|i| format!("entry-{i:03} {}", "k".repeat(30))));
    let mut keep = keep_before.clone();
    let mut candidates = Vec::new();
    merge_keep_exact(&mut keep, new.clone().into_iter(), &deps, &mut candidates);
    cases.push(json!({
        "name": "merge_keep_exact_normalizes_and_moves_the_oldest_out",
        "fn": "merge_keep_exact",
        "source": "llm_compress tests::keep_exact_overflow_moves_the_oldest_to_memory_candidates",
        "keep": keep_before,
        "new": new,
        "expect": { "keep": keep, "candidates": candidates },
    }));
    cases
}

#[tokio::test]
async fn compress() {
    check_or_regen("compress.json", compress_cases().await);
}

// =========================================================================
// (h) runtime_identity.json
// =========================================================================

fn host_id_case(
    name: &str,
    agent_tool_host_id: Option<&str>,
    etc_hostname: Option<&str>,
    hostname_env: Option<&str>,
    machine_id: Option<&str>,
) -> Value {
    json!({
        "name": name,
        "fn": "native_host_id",
        "agent_tool_host_id": agent_tool_host_id,
        "etc_hostname": etc_hostname,
        "hostname_env": hostname_env,
        "machine_id": machine_id,
        "expect": native_host_id_from(agent_tool_host_id, etc_hostname, hostname_env, machine_id),
    })
}

fn identity_case(name: &str, configured_id: Option<&str>, host_id: &str, uid: &str) -> Value {
    let info = RuntimeInfo {
        cwd: "/work/project".into(),
        os: "linux".into(),
        tools: vec!["bash".into(), "realpath".into(), "base64".into()],
        ..Default::default()
    };
    let target = native_target(host_id, uid);
    let runtime_id = derive_runtime_id(configured_id, "native", &target);
    json!({
        "name": name,
        "fn": "native_identity",
        "configured_id": configured_id,
        "host_id": host_id,
        "probe": { "uid": uid, "cwd": info.cwd, "os": info.os, "tools": info.tools },
        "expect": {
            "target": target,
            "target_text": target.to_string(),
            "runtime_id": runtime_id,
            "descriptor": runtime_descriptor(runtime_id.clone(), "native".into(), target.clone(), &info),
        },
    })
}

fn probe_case(name: &str, fields: &[&str]) -> Value {
    let stdout: String = fields.iter().map(|f| format!("{f}\0")).collect();
    let expect = match parse_probe(stdout.as_bytes()) {
        Ok(p) => json!({
            "identity": { "machine": p.identity.machine, "hostname": p.identity.hostname, "uid": p.identity.uid },
            "info": {
                "os": p.info.os,
                "arch": p.info.arch,
                "hostname": p.info.hostname,
                "shell": p.info.shell,
                "cwd": p.info.cwd,
                "current_time": p.info.current_time,
                "timezone": p.info.timezone,
                "tools": p.info.tools,
            },
            "path": p.path,
        }),
        Err(e) => json!({ "error": xllm_error(&e) }),
    };
    json!({ "name": name, "fn": "parse_probe", "stdout": stdout, "expect": expect })
}

fn lock_path_case(name: &str, lock_dir: &str, workdir: &str) -> Value {
    let path = workdir_lock_path(Path::new(lock_dir), Path::new(workdir));
    json!({
        "name": name,
        "fn": "workdir_lock_path",
        "lock_dir": lock_dir,
        "workdir": workdir,
        "expect": {
            "file_name": path.file_name().unwrap().to_string_lossy(),
            "path": path.display().to_string(),
        },
    })
}

fn runtime_identity_cases() -> Vec<Value> {
    let fixture = native_host_id_from(Some("xllm-fixture"), None, None, None);
    let probe = [
        "0123456789abcdef0123456789abcdef",
        "devbox",
        "1000",
        "linux",
        "x86_64",
        "/usr/bin/bash",
        "/work/project",
        "/usr/local/bin:/usr/bin:/bin",
        "2026-11-07T10:00:00+0800",
        "CST+0800",
        "bash",
        "realpath",
        "base64",
    ];
    let mut incomplete = probe;
    incomplete[2] = "";
    vec![
        host_id_case(
            "host_id_from_agent_tool_host_id",
            Some("  xllm-fixture "),
            Some("devbox\n"),
            None,
            Some("m\n"),
        ),
        host_id_case(
            "host_id_blank_env_falls_back",
            Some("  "),
            Some("devbox\n"),
            None,
            Some("0123abcd\n"),
        ),
        host_id_case(
            "host_id_from_hostname_env",
            None,
            None,
            Some("ci-runner"),
            None,
        ),
        host_id_case("host_id_unknown", None, None, None, None),
        identity_case("native_identity_fixture_host", None, &fixture, "1000"),
        identity_case("native_identity_root", None, &fixture, "0"),
        identity_case(
            "native_identity_machine_host",
            None,
            "host:devbox:0123abcd",
            "1000",
        ),
        identity_case(
            "native_identity_configured_id",
            Some("my-runtime"),
            &fixture,
            "1000",
        ),
        probe_case("parse_probe_local", &probe),
        probe_case("parse_probe_incomplete", &incomplete),
        lock_path_case("lock_path_plain", "/home/u/.xllm/locks", "/work/project"),
        lock_path_case(
            "lock_path_trailing_slash_differs",
            "/home/u/.xllm/locks",
            "/work/project/",
        ),
        lock_path_case(
            "lock_path_unicode_and_space",
            "/home/u/.xllm/locks",
            "/home/u/项目 a",
        ),
        lock_path_case(
            "lock_path_windows_style",
            "/home/u/.xllm/locks",
            "C:\\Users\\u\\proj",
        ),
    ]
}

#[test]
fn runtime_identity() {
    check_or_regen("runtime_identity.json", runtime_identity_cases());
}
