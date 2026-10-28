//! Host tools of a wish stage (许愿格 §5.5, §7.2, §8.4): the read-only `ws_*` tools answered from
//! the stage snapshot, `submit_analysis` (analysis), and the delivery tools `run_program`,
//! `put_result`, `check_results`, `finish` (execution). Every answer is checked on the spot and
//! errors go back to the model in the same run, so it can correct itself.

use super::runner;
use super::StageState;
use agent_tool::{AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, CallingConventions, SessionRuntimeContext, ToolSpec};
use aiworkspace_store::wish::results::program_output;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Outline,
    Find,
    Profile,
    Neighbors,
    Read,
    Query,
    SubmitAnalysis,
    RunProgram,
    PutResult,
    CheckResults,
    Finish,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Outline => "ws_outline",
            Kind::Find => "ws_find",
            Kind::Profile => "ws_profile",
            Kind::Neighbors => "ws_neighbors",
            Kind::Read => "ws_read",
            Kind::Query => "ws_query",
            Kind::SubmitAnalysis => "submit_analysis",
            Kind::RunProgram => "run_program",
            Kind::PutResult => "put_result",
            Kind::CheckResults => "check_results",
            Kind::Finish => "finish",
        }
    }

    pub const READ: [Kind; 6] = [Kind::Outline, Kind::Find, Kind::Profile, Kind::Neighbors, Kind::Read, Kind::Query];
    pub const DELIVERY: [Kind; 4] = [Kind::RunProgram, Kind::PutResult, Kind::CheckResults, Kind::Finish];

    pub fn spec(self) -> (&'static str, Value) {
        let target = json!({ "type": "string", "description": "A handle (such as @T1, @B2, @T1.f3) or an entity id" });
        match self {
            Kind::Outline => (
                "Expand a node of the data tree or of a canvas (including the parts folded in the map). Without target, lists the top level of the data tree and of the canvases.",
                json!({ "type": "object", "properties": { "target": target, "depth": { "type": "integer", "minimum": 1, "maximum": 3 } } }),
            ),
            Kind::Find => (
                "Find data and Blocks by name, field name or body text; returns handles and the matching snippets.",
                json!({ "type": "object", "properties": { "text": { "type": "string" }, "kinds": { "type": "array", "items": { "type": "string" }, "description": "table / richtext / record / asset / annotation / cell" } }, "required": ["text"] }),
            ),
            Kind::Profile => (
                "Content profile: for a table, the row count, field types and value ranges, empty values, sample rows and suspected problems; for rich text, the headings and the opening; for a record, its properties. A Block handle returns the profile of the data the Block shows.",
                json!({ "type": "object", "properties": { "target": target }, "required": ["target"] }),
            ),
            Kind::Neighbors => (
                "The Blocks around a Block: direction (left/right/above/below), distance and bound data.",
                json!({ "type": "object", "properties": { "cell": target, "radius": { "type": "number" } }, "required": ["cell"] }),
            ),
            Kind::Read => (
                "Read rich text (Markdown), record properties, asset information, folder members, a single record, or the first rows of a table (process large amounts of data with a program).",
                json!({ "type": "object", "properties": { "target": target, "limit": { "type": "integer", "minimum": 1, "maximum": 50 }, "cursor": { "type": "string" } }, "required": ["target"] }),
            ),
            Kind::Query => (
                "Query a table; given the handle of a table view Block, reads through the view's filter and sort. filter is {\"field name\": value} or {\"op\":\"cmp\",\"field\":\"field name\",\"operator\":\"gt\",\"value\":100} (combined with and/or/not); sorts is [\"-Sales\"] or [{\"field\":\"Month\",\"direction\":\"asc\"}]. At most 50 rows per page.",
                json!({ "type": "object", "properties": {
                    "table_or_view": target, "filter": { "type": "object" }, "sorts": { "type": "array" },
                    "fields": { "type": "array", "items": { "type": "string" } }, "limit": { "type": "integer", "minimum": 1, "maximum": 50 }, "cursor": { "type": "string" } },
                    "required": ["table_or_view"] }),
            ),
            Kind::SubmitAnalysis => (
                "Submit the analysis (wish.analysis.v2). The host validates it on the spot: whether the handles exist, whether the bindings are readable, whether every handle in the prompt is bound, whether the output contract and the Renderers are valid, and whether the production approaches follow the programs-first rule. When a list of problems comes back, correct them and submit again; once it is accepted, finish.",
                json!({ "type": "object", "properties": { "analysis": { "type": "object", "description": "the wish.analysis.v2 object" } }, "required": ["analysis"] }),
            ),
            Kind::RunProgram => (
                "Run program/main.js the canonical way (the same as a later \"re-run only the program\"): collect the results, facts and checks the program writes, validate them against the output contract, and return each result's preview, the check results and the errors. Can be called several times; the last successful run counts.",
                json!({ "type": "object", "properties": {} }),
            ),
            Kind::PutResult => (
                "Submit a result agreed as direct (commentary, recommendations and other text, a file under output/ not produced by the program, or a control such as a mind map given as an outline); it is validated immediately and a preview is returned; submitting the same name again replaces it. Numbers in text may only cite facts or result tables.",
                json!({ "type": "object", "properties": {
                    "name": { "type": "string", "description": "The result name in the output contract" },
                    "type": { "type": "string", "enum": ["text", "record", "file", "html", "control"] },
                    "markdown": { "type": "string", "description": "The Markdown when type=text; [title](result:result name) / [title](input:input name) link to other results or inputs" },
                    "props": { "type": "object", "description": "The properties when type=record: { name: value }" },
                    "path": { "type": "string", "description": "The path of a file under output/ when type=file" },
                    "html": { "type": "string" }, "css": { "type": "string" }, "js": { "type": "string" },
                    "bindings": { "type": "object", "description": "The bindings when type=html: { binding name: \"input:input name\" | \"result:result name\" }" },
                    "kind": { "type": "string", "description": "The control when type=control (catalog/controls.json), such as mindmap" },
                    "config": { "type": "object", "description": "The control's configuration when type=control (optional)" },
                    "children": { "description": "The tree when type=control: a Markdown outline (nested list) or [{ \"text\": …, \"children\": [...] }]; no coordinates", "anyOf": [{ "type": "string" }, { "type": "array", "items": { "type": "object" } }] },
                    "under": { "type": "string", "description": "When type=control: the input name of a node of an existing control; the tree goes below that node instead of into a new control" },
                    "title": { "type": "string" } }, "required": ["name", "type"] }),
            ),
            Kind::CheckResults => (
                "Summarize all results: whether the output contract is met, whether the checks pass, and whether the numbers cited in text can be found in the facts and results.",
                json!({ "type": "object", "properties": {} }),
            ),
            Kind::Finish => (
                "End the execution. Refused, with the reason, when there are host-level errors or program checks have not run. review_notes holds the self-assessment of the review checks: [{\"id\":\"check id\",\"note\":\"…\"}]. When there is feedback, refinements holds the concise requirements distilled from the feedback that later re-runs must also follow.",
                json!({ "type": "object", "properties": {
                    "summary": { "type": "string" },
                    "assumptions": { "type": "array", "items": { "type": "string" } },
                    "warnings": { "type": "array", "items": { "type": "string" } },
                    "review_notes": { "type": "array", "items": { "type": "object" } },
                    "refinements": { "type": "array", "items": { "type": "string" } } }, "required": ["summary"] }),
            ),
        }
    }
}

pub struct WishTool {
    pub kind: Kind,
    pub state: Arc<StageState>,
}

pub fn tools(state: &Arc<StageState>, kinds: &[Kind]) -> Vec<Arc<dyn AgentTool>> {
    kinds.iter().map(|k| Arc::new(WishTool { kind: *k, state: state.clone() }) as Arc<dyn AgentTool>).collect()
}

/// Every tool answers text: `Ok` is what the model reads on, `Err` what it must correct.
pub type ToolAnswer = Result<String, String>;

fn ok(text: String) -> ToolAnswer {
    Ok(text)
}

fn fail(text: String) -> ToolAnswer {
    Err(text)
}

fn to_result(answer: ToolAnswer) -> AgentToolResult {
    match answer {
        Ok(text) => AgentToolResult::from_details(json!({})).with_output(text),
        Err(text) => AgentToolResult::from_details(json!({})).with_status(AgentToolStatus::Error).with_result(text),
    }
}

/// `Kind` by tool name (an executor outside the service names them over `proc.tool`).
pub fn kind_of(name: &str) -> Option<Kind> {
    [Kind::Outline, Kind::Find, Kind::Profile, Kind::Neighbors, Kind::Read, Kind::Query, Kind::SubmitAnalysis, Kind::RunProgram, Kind::PutResult, Kind::CheckResults, Kind::Finish].into_iter().find(|k| k.name() == name)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

#[async_trait]
impl AgentTool for WishTool {
    fn spec(&self) -> ToolSpec {
        let (description, args_schema) = self.kind.spec();
        ToolSpec { name: self.kind.name().into(), description: description.into(), args_schema, output_schema: json!({ "type": "string" }), usage: None }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn call(&self, _ctx: &SessionRuntimeContext, args: Value) -> Result<AgentToolResult, AgentToolError> {
        if self.state.cancelled() {
            return Err(AgentToolError::Cancelled { message: "the run was cancelled".into(), effect_unknown: false });
        }
        self.state.note_tool(self.kind.name());
        let state = self.state.clone();
        let kind = self.kind;
        let res = match kind {
            Kind::RunProgram => run_program(&state).await,
            _ => tokio::task::spawn_blocking(move || blocking_call(&state, kind, &args)).await.map_err(|e| AgentToolError::ExecFailed(e.to_string()))?,
        };
        Ok(to_result(res))
    }
}

pub fn blocking_call(state: &Arc<StageState>, kind: Kind, args: &Value) -> ToolAnswer {
    let mut stage = state.stage.lock().unwrap();
    let r = match kind {
        Kind::Outline => stage.ws_outline(args),
        Kind::Find => stage.ws_find(args),
        Kind::Profile => stage.ws_profile(args),
        Kind::Neighbors => stage.ws_neighbors(args),
        Kind::Read => stage.ws_read(args),
        Kind::Query => stage.ws_query(args),
        Kind::SubmitAnalysis => {
            let a = args.get("analysis").cloned().unwrap_or_else(|| args.clone());
            return match stage.validate_analysis(&a) {
                Ok(Ok(v)) => {
                    let status = v.analysis["status"].as_str().unwrap_or("").to_string();
                    let n = v.inputs.len();
                    *state.analysis.lock().unwrap() = Some(v);
                    ok(format!("Analysis accepted (status = {status}, {}). Finish with one sentence; do not call any more tools.", aiworkspace_store::wish::profile::count(n as u64, "input", "inputs")))
                }
                Ok(Err(problems)) => fail(format!("Analysis not accepted; fix the following problems and submit again:\n- {}", problems.join("\n- "))),
                Err(e) => fail(format!("Host error: {}", e.detail)),
            };
        }
        Kind::PutResult => {
            drop(stage);
            return put_result(state, args);
        }
        Kind::CheckResults => {
            drop(stage);
            return check_results(state);
        }
        Kind::Finish => {
            drop(stage);
            return finish(state, args);
        }
        Kind::RunProgram => unreachable!(),
    };
    match r {
        Ok(text) => ok(text),
        Err(e) => fail(e.detail),
    }
}

fn put_result(state: &Arc<StageState>, args: &Value) -> ToolAnswer {
    let mut c = state.collector.lock().unwrap();
    let name = args["name"].as_str().unwrap_or("").to_string();
    if c.program.as_ref().is_some_and(|p| p.results.iter().any(|r| r["name"] == json!(name))) {
        return fail(format!("Result \"{name}\" is already produced by the program: name conflict. Use another name from the contract for a direct result"));
    }
    let mut raw = args.clone();
    if raw["type"] == json!("record") && raw.get("props").is_none() {
        raw["props"] = json!({});
    }
    match c.normalize(&raw, "direct") {
        Ok(r) => {
            let preview = c.preview(&r);
            c.direct.insert(name.clone(), r);
            ok(format!("Received result \"{name}\" (submitting the same name again replaces it). Preview:\n{}", pretty(&preview)))
        }
        Err(errs) => fail(format!("Result \"{name}\" not accepted:\n- {}", errs.join("\n- "))),
    }
}

fn check_results(state: &Arc<StageState>) -> ToolAnswer {
    let c = state.collector.lock().unwrap();
    let texts = state.prompt_texts();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let report = json!({
        "results": c.all().iter().map(|r| c.preview(r)).collect::<Vec<_>>(),
        "checks": c.checks(),
        "uncited_numbers": c.uncited_numbers(&refs),
        "blocking": c.blocking(),
    });
    let mut text = pretty(&report);
    if !report["uncited_numbers"].as_array().is_some_and(|a| a.is_empty()) {
        text.push_str("\n\nNote: the numbers in uncited_numbers cannot be found in the facts or the result tables. Add them to the program's aiws.facts(), or cite existing numbers instead.");
    }
    if report["blocking"].as_array().is_some_and(|a| !a.is_empty()) {
        text.push_str("\n\nUntil the problems in blocking are solved, finish will be refused.");
    }
    ok(text)
}

fn finish(state: &Arc<StageState>, args: &Value) -> ToolAnswer {
    let mut c = state.collector.lock().unwrap();
    let blocking = c.blocking();
    if !blocking.is_empty() {
        return fail(format!("Cannot finish yet:\n- {}", blocking.join("\n- ")));
    }
    if state.feedback.is_some() && !args["refinements"].as_array().is_some_and(|a| !a.is_empty()) {
        return fail("This round revises the results according to user feedback: write in refinements the concise requirements distilled from the feedback that later re-runs must also follow (one sentence each)".into());
    }
    let mut fin = json!({
        "summary": args["summary"].as_str().unwrap_or(""),
        "assumptions": args.get("assumptions").cloned().unwrap_or(json!([])),
        "warnings": args.get("warnings").cloned().unwrap_or(json!([])),
        "review_notes": args.get("review_notes").cloned().unwrap_or(json!([])),
    });
    if let Some(r) = args.get("refinements") {
        fin["refinements"] = r.clone();
    }
    c.finished = Some(fin);
    let texts = state.prompt_texts();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let uncited = c.uncited_numbers(&refs);
    let failed: Vec<String> = c.checks().iter().filter(|x| x["status"] == json!("failed")).map(|x| x["id"].as_str().unwrap_or("").to_string()).collect();
    let mut msg = "Finished. The results go to the user as candidates to preview. Finish with one sentence; do not call any more tools.".to_string();
    if !failed.is_empty() {
        msg.push_str(&format!(" (Note: failed checks {} will be highlighted in the preview.)", failed.join(", ")));
    }
    if !uncited.is_empty() {
        msg.push_str(" (Note: some numbers cannot be traced to the facts; the preview will ask the user to verify them.)");
    }
    ok(msg)
}

async fn run_program(state: &Arc<StageState>) -> ToolAnswer {
    match runner::run(state).await {
        Err(e) => fail(e),
        Ok(out) => {
            if out.timed_out {
                return fail(format!("The program timed out ({} s). Optimize the program or narrow the range.\n{}", state.runtime.config.program_timeout_secs, out.tail()));
            }
            judge_program(state, out.results.as_ref(), &format!("exit code {:?}", out.code), &out.tail(), &out.stdout)
        }
    }
}

/// What a program run produced, judged against the contract: the same words whether the service's
/// Deno or an executor outside the service ran it (`proc.program`).
pub fn judge_program(state: &Arc<StageState>, results: Option<&Value>, ended: &str, tail: &str, stdout: &str) -> ToolAnswer {
    let raw = results.cloned().unwrap_or(json!({}));
    if let Some(err) = raw.get("error").and_then(Value::as_str) {
        return fail(format!("The program threw an error:\n{err}\n{tail}"));
    }
    if results.is_none() {
        return fail(format!("The program did not write output/.aiws/results.json ({ended}).\n{tail}"));
    }
    let source = std::fs::read_to_string(state.workdir.join("program/main.js")).unwrap_or_default();
    let mut c = state.collector.lock().unwrap();
    match program_output(&c, &raw, &source, tail) {
        Ok(run) => {
            let previews: Vec<Value> = run.results.iter().map(|r| c.preview(r)).collect();
            let facts = run.facts.clone();
            c.program = Some(run);
            let checks = c.checks();
            state.note_program_run();
            let mut text = format!("The program ran successfully.\nResult previews:\n{}\nfacts: {}\nChecks: {}", pretty(&json!(previews)), facts, pretty(&json!(checks)));
            let remaining = c.blocking();
            if !remaining.is_empty() {
                text.push_str(&format!("\nNot done yet:\n- {}", remaining.join("\n- ")));
            }
            if !stdout.trim().is_empty() {
                text.push_str(&format!("\n\nProgram output:\n{}", stdout.chars().take(2000).collect::<String>()));
            }
            ok(text)
        }
        Err(errs) => fail(format!("The program ran, but its results failed validation:\n- {}\nFix the program, then call run_program again.", errs.join("\n- "))),
    }
}
