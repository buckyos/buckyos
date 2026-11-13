//! Host tools of a wish stage (许愿格 §5.5, §7.2, §8.4; AI Native 第一期 §2.1): the read-only `ws_*` tools answered
//! from the stage snapshot, `submit_analysis` (analysis), and the coding tools `run_program` (the executor runs the
//! macro and hands it in with `proc.program`), `check_results`, `finish`. Nothing writes a result directly: every
//! result comes from the macro. Every answer is checked on the spot and errors go back to the model in the same run,
//! so it can correct itself.

use super::StageState;
use aiworkspace_store::wish::results::program_output;
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
            Kind::CheckResults => "check_results",
            Kind::Finish => "finish",
        }
    }

    pub const READ: [Kind; 6] = [Kind::Outline, Kind::Find, Kind::Profile, Kind::Neighbors, Kind::Read, Kind::Query];
    pub const DELIVERY: [Kind; 3] = [Kind::RunProgram, Kind::CheckResults, Kind::Finish];

    pub fn spec(self) -> (&'static str, Value) {
        let target = json!({ "type": "string", "description": "A handle (such as @T1, @B2, @T1.f3) or an entity id" });
        match self {
            Kind::Outline => (
                "Expand a node of the data tree or of a canvas (including the parts folded in the map). Without target, lists the top level of the data tree and of the canvases.",
                json!({ "type": "object", "properties": { "target": target, "depth": { "type": "integer", "minimum": 1, "maximum": 3 } } }),
            ),
            Kind::Find => (
                "Find a literal text in the names, titles and bodies of the data and Blocks (a select by its label, a reference by its target's title); returns handles, paths and the matching snippets (a table hit names the record and field).",
                json!({ "type": "object", "properties": { "text": { "type": "string" }, "kinds": { "type": "array", "items": { "type": "string" }, "description": "table / richtext / record / asset / annotation / block / container / wish / def" }, "at": { "type": "string", "description": "Search under this node only (a handle, an id or a path)" } }, "required": ["text"] }),
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
                "Read an object: rich text, records, notes, wishes and definitions as their plain text (body), a table or a table view Block as rows with handles (and the view's SQL), a single record, a folder's members, a control as its outline. Process large amounts of data with a program.",
                json!({ "type": "object", "properties": { "target": target, "limit": { "type": "integer", "minimum": 1, "maximum": 50 }, "cursor": { "type": "string" } }, "required": ["target"] }),
            ),
            Kind::Query => (
                "One SQL SELECT over the tables of the snapshot (SQLite). Name a table by its path (\"/data/sales\"), its handle (\"@T1\") or its id; columns are the field names with the displayed values (a select's label, a reference's path, a currency's amount), \"<field>#raw\" the stored values (option ids, exact decimals), and _id (record id). Joins, GROUP BY and window functions work; no writes. At most 200 rows per call. A table view Block's SQL is in ws_read of the Block.",
                json!({ "type": "object", "properties": {
                    "sql": { "type": "string" }, "tables": { "type": "object", "description": "Optional aliases { alias: handle | path | id }" },
                    "max_rows": { "type": "integer", "minimum": 1, "maximum": 200 } },
                    "required": ["sql"] }),
            ),
            Kind::SubmitAnalysis => (
                "Submit the analysis (wish.analysis.v3). The host validates it on the spot: whether the handles exist, whether the bindings are readable, whether every handle in the prompt is bound, and whether the output contract and the Renderers are valid. When a list of problems comes back, correct them and submit again; once it is accepted, finish.",
                json!({ "type": "object", "properties": { "analysis": { "type": "object", "description": "the wish.analysis.v3 object" } }, "required": ["analysis"] }),
            ),
            Kind::RunProgram => (
                "Trial-run the macro exactly as every later run will run it: collect the results, facts, checks and manifest it writes, validate them against the output contract, and return each result's preview, the check results and the errors. Can be called several times; the last successful run is the candidate.",
                json!({ "type": "object", "properties": { "source": { "type": "string", "description": "The complete program/main.js (an ES module exporting `default async function main(aiws)` that starts with aiws.declare)" } }, "required": ["source"] }),
            ),
            Kind::CheckResults => (
                "Summarize the macro's results: whether the output contract is met, whether the checks pass, and whether the numbers in its text can be found in its facts and tables.",
                json!({ "type": "object", "properties": {} }),
            ),
            Kind::Finish => (
                "End the coding. Refused, with the reason, until a trial run succeeded with a manifest, the contract is met and the program checks ran. review_notes holds the self-assessment of the review checks: [{\"id\":\"check id\",\"note\":\"…\"}]. When there is feedback, refinements holds the concise requirements distilled from the feedback that later re-runs must also follow.",
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

/// Every tool answers text: `Ok` is what the model reads on, `Err` what it must correct.
pub type ToolAnswer = Result<String, String>;

fn ok(text: String) -> ToolAnswer {
    Ok(text)
}

fn fail(text: String) -> ToolAnswer {
    Err(text)
}

/// `Kind` by tool name (an executor outside the service names them over `proc.tool`).
pub fn kind_of(name: &str) -> Option<Kind> {
    [Kind::Outline, Kind::Find, Kind::Profile, Kind::Neighbors, Kind::Read, Kind::Query, Kind::SubmitAnalysis, Kind::RunProgram, Kind::CheckResults, Kind::Finish].into_iter().find(|k| k.name() == name)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
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
        Kind::CheckResults => {
            drop(stage);
            return check_results(state);
        }
        Kind::Finish => {
            drop(stage);
            return finish(state, args);
        }
        Kind::RunProgram => return fail("run the macro on the executor and hand it in with proc.program".into()),
    };
    match r {
        Ok(text) => ok(text),
        Err(e) => fail(e.detail),
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
        text.push_str("\n\nNote: the numbers in uncited_numbers cannot be found in the macro's facts or result tables. Put them in aiws.facts() and give the model only those facts when it writes text.");
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

/// What a trial run of the macro produced (`proc.program` while coding), judged against the contract.
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
            let ai = run.ai.clone();
            let manifest = run.manifest.clone();
            c.program = Some(run);
            let checks = c.checks();
            state.note_program_run(&ai);
            let mut text = format!("The program ran successfully.\nResult previews:\n{}\nfacts: {}\nChecks: {}", pretty(&json!(previews)), facts, pretty(&json!(checks)));
            match &manifest {
                Some(m) => text.push_str(&format!("\nManifest (what every run reads, may call and writes):\n{}", aiworkspace_core::body::manifest_text(m))),
                None => text.push_str("\nNo manifest: the macro must start with aiws.declare({ inputs: { … } })."),
            }
            if !ai.is_null() {
                text.push_str(&format!("\nAI calls: {ai}"));
            }
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
