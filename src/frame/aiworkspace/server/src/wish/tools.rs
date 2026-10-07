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

    fn spec(self) -> (&'static str, Value) {
        let target = json!({ "type": "string", "description": "句柄（如 @T1、@B2、@T1.f3）或实体 id" });
        match self {
            Kind::Outline => (
                "展开数据树或画布中的一个节点（包括地图中折叠的部分）。不给 target 时列出数据树与画布的顶层。",
                json!({ "type": "object", "properties": { "target": target, "depth": { "type": "integer", "minimum": 1, "maximum": 3 } } }),
            ),
            Kind::Find => (
                "按名称、字段名、正文查找数据与 Block，返回句柄和匹配片段。",
                json!({ "type": "object", "properties": { "text": { "type": "string" }, "kinds": { "type": "array", "items": { "type": "string" }, "description": "table / richtext / record / asset / annotation / cell" } }, "required": ["text"] }),
            ),
            Kind::Profile => (
                "内容画像：表格的行数、字段类型与取值范围、空值、示例行、疑似问题；富文本的标题与开头；记录的属性。Block 句柄返回它显示的数据的画像。",
                json!({ "type": "object", "properties": { "target": target }, "required": ["target"] }),
            ),
            Kind::Neighbors => (
                "某个 Block 周围的 Block：方向（左/右/上/下）、距离、绑定的数据。",
                json!({ "type": "object", "properties": { "cell": target, "radius": { "type": "number" } }, "required": ["cell"] }),
            ),
            Kind::Read => (
                "读取富文本（Markdown）、记录属性、资产信息、文件夹成员、某条记录，或表格的前若干行（大量数据请用程序处理）。",
                json!({ "type": "object", "properties": { "target": target, "limit": { "type": "integer", "minimum": 1, "maximum": 50 }, "cursor": { "type": "string" } }, "required": ["target"] }),
            ),
            Kind::Query => (
                "查询表格；给表格视图 Block 的句柄时按视图的筛选与排序读取。filter 写 {\"字段名\": 值} 或 {\"op\":\"cmp\",\"field\":\"字段名\",\"operator\":\"gt\",\"value\":100}（and/or/not 组合）；sorts 写 [\"-销售额\"] 或 [{\"field\":\"月份\",\"direction\":\"asc\"}]。每页最多 50 行。",
                json!({ "type": "object", "properties": {
                    "table_or_view": target, "filter": { "type": "object" }, "sorts": { "type": "array" },
                    "fields": { "type": "array", "items": { "type": "string" } }, "limit": { "type": "integer", "minimum": 1, "maximum": 50 }, "cursor": { "type": "string" } },
                    "required": ["table_or_view"] }),
            ),
            Kind::SubmitAnalysis => (
                "提交分析结果（wish.analysis.v2）。宿主当场校验：句柄是否存在、绑定是否可读、提示词中的句柄是否都已绑定、输出约定与 Renderer 是否合法、生产方式是否符合编程优先规则。返回问题清单时修正后再提交；被接受后结束。",
                json!({ "type": "object", "properties": { "analysis": { "type": "object", "description": "wish.analysis.v2 对象" } }, "required": ["analysis"] }),
            ),
            Kind::RunProgram => (
                "以规范方式运行 program/main.js（与以后的“只重跑程序”相同）：收集程序写出的结果、facts 与检查，按输出约定校验，返回每个结果的预览、检查结果和错误。可以多次调用，以最后一次成功运行为准。",
                json!({ "type": "object", "properties": {} }),
            ),
            Kind::PutResult => (
                "提交一个约定为 direct 的结果（解读、建议等文字，或 output/ 下非程序产出的文件），立即校验并返回预览；同名再次提交即替换。文字中的数字只能引用 facts 或结果表。",
                json!({ "type": "object", "properties": {
                    "name": { "type": "string", "description": "输出约定中的结果名" },
                    "type": { "type": "string", "enum": ["text", "record", "file", "html"] },
                    "markdown": { "type": "string", "description": "type=text 时的 Markdown；可用 [标题](result:结果名) / [标题](input:输入名) 链接到其他结果或输入" },
                    "props": { "type": "object", "description": "type=record 时的属性 { 名称: 值 }" },
                    "path": { "type": "string", "description": "type=file 时 output/ 下的文件路径" },
                    "html": { "type": "string" }, "css": { "type": "string" }, "js": { "type": "string" },
                    "bindings": { "type": "object", "description": "type=html 时的绑定 { 绑定名: \"input:输入名\" | \"result:结果名\" }" },
                    "title": { "type": "string" } }, "required": ["name", "type"] }),
            ),
            Kind::CheckResults => (
                "汇总全部结果：是否满足输出约定、检查是否通过、文字中引用的数字能否在 facts 和结果中找到。",
                json!({ "type": "object", "properties": {} }),
            ),
            Kind::Finish => (
                "结束执行。存在宿主级错误或程序型检查未执行时会被拒绝并说明原因。review_notes 写对 review 型检查的自评：[{\"id\":\"检查id\",\"note\":\"…\"}]。有反馈时，refinements 写从反馈中整理出的、以后重跑也要遵守的简洁要求。",
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

fn ok(text: String) -> AgentToolResult {
    AgentToolResult::from_details(json!({})).with_output(text)
}

fn fail(text: String) -> AgentToolResult {
    AgentToolResult::from_details(json!({})).with_status(AgentToolStatus::Error).with_result(text)
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
        Ok(res)
    }
}

fn blocking_call(state: &Arc<StageState>, kind: Kind, args: &Value) -> AgentToolResult {
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
                    ok(format!("分析已接受（status = {status}，{n} 个输入）。用一句话结束，不要再调用工具。"))
                }
                Ok(Err(problems)) => fail(format!("分析未被接受，请修正以下问题后重新提交：\n- {}", problems.join("\n- "))),
                Err(e) => fail(format!("宿主错误：{}", e.detail)),
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

fn put_result(state: &Arc<StageState>, args: &Value) -> AgentToolResult {
    let mut c = state.collector.lock().unwrap();
    let name = args["name"].as_str().unwrap_or("").to_string();
    if c.program.as_ref().is_some_and(|p| p.results.iter().any(|r| r["name"] == json!(name))) {
        return fail(format!("结果「{name}」已由程序产生：名称冲突。direct 结果用约定中其他的名称"));
    }
    let mut raw = args.clone();
    if raw["type"] == json!("record") && raw.get("props").is_none() {
        raw["props"] = json!({});
    }
    match c.normalize(&raw, "direct") {
        Ok(r) => {
            let preview = c.preview(&r);
            c.direct.insert(name.clone(), r);
            ok(format!("已收到结果「{name}」（同名再次提交会替换）。预览：\n{}", pretty(&preview)))
        }
        Err(errs) => fail(format!("结果「{name}」未被接受：\n- {}", errs.join("\n- "))),
    }
}

fn check_results(state: &Arc<StageState>) -> AgentToolResult {
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
        text.push_str("\n\n注意：uncited_numbers 中的数字在 facts 和结果表中找不到。把它们加入程序的 aiws.facts()，或改为引用已有的数字。");
    }
    if report["blocking"].as_array().is_some_and(|a| !a.is_empty()) {
        text.push_str("\n\nblocking 中的问题不解决，finish 会被拒绝。");
    }
    ok(text)
}

fn finish(state: &Arc<StageState>, args: &Value) -> AgentToolResult {
    let mut c = state.collector.lock().unwrap();
    let blocking = c.blocking();
    if !blocking.is_empty() {
        return fail(format!("还不能结束：\n- {}", blocking.join("\n- ")));
    }
    if state.feedback.is_some() && !args["refinements"].as_array().is_some_and(|a| !a.is_empty()) {
        return fail("这一轮是按用户反馈修改：请在 refinements 中写出从反馈整理出的、以后重跑也要遵守的简洁要求（每条一句）".into());
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
    let mut msg = "已结束。结果将作为候选交给用户预览。用一句话结束，不要再调用工具。".to_string();
    if !failed.is_empty() {
        msg.push_str(&format!("（注意：检查 {} 未通过，会在预览中醒目显示）", failed.join("、")));
    }
    if !uncited.is_empty() {
        msg.push_str("（注意：有数字无法追溯到 facts，会在预览中提示用户核对）");
    }
    ok(msg)
}

async fn run_program(state: &Arc<StageState>) -> AgentToolResult {
    match runner::run(state).await {
        Err(e) => fail(e),
        Ok(out) => {
            if out.timed_out {
                return fail(format!("程序运行超时（{} 秒）。请优化程序或缩小范围。\n{}", state.runtime.config.program_timeout_secs, out.tail()));
            }
            let raw = out.results.clone().unwrap_or(json!({}));
            if let Some(err) = raw.get("error").and_then(Value::as_str) {
                return fail(format!("程序抛出错误：\n{err}\n{}", out.tail()));
            }
            if out.results.is_none() {
                return fail(format!("程序没有写出 output/.aiws/results.json（退出码 {:?}）。\n{}", out.code, out.tail()));
            }
            let source = std::fs::read_to_string(state.workdir.join("program/main.js")).unwrap_or_default();
            let mut c = state.collector.lock().unwrap();
            match program_output(&c, &raw, &source, &out.tail()) {
                Ok(run) => {
                    let previews: Vec<Value> = run.results.iter().map(|r| c.preview(r)).collect();
                    let facts = run.facts.clone();
                    c.program = Some(run);
                    let checks = c.checks();
                    state.note_program_run();
                    let mut text = format!("程序运行成功。\n结果预览：\n{}\nfacts：{}\n检查：{}", pretty(&json!(previews)), facts, pretty(&json!(checks)));
                    let remaining = c.blocking();
                    if !remaining.is_empty() {
                        text.push_str(&format!("\n尚未完成：\n- {}", remaining.join("\n- ")));
                    }
                    if !out.stdout.trim().is_empty() {
                        text.push_str(&format!("\n\n程序输出：\n{}", out.stdout.chars().take(2000).collect::<String>()));
                    }
                    ok(text)
                }
                Err(errs) => fail(format!("程序运行了，但结果没有通过校验：\n- {}\n修改程序后再调用 run_program。", errs.join("\n- "))),
            }
        }
    }
}
