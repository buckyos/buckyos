//! Offline probe for context compression on a real session worklog.
//!
//! Rebuilds the function-call history of one run from a libopendan
//! `worklog.jsonl` (entries with `--from < seq <= --upto`, default up to the
//! first `context_rewritten` outcome) and tries two things:
//!
//! 1. the real `llm_compress::compress` with a mock summariser (what the
//!    current code would do);
//! 2. a prototype of the proposed shape `system + head + [summary] + hint +
//!    raw tail`: mechanical Min lines for the middle first, and if that is
//!    not enough (or `--mode llm`) one rolling LLM summary (merged with
//!    `--prev-summary`) through an OpenAI-compatible endpoint.
//!
//! ```text
//! OPENAI_API_KEY=… cargo run -p agent_tool --example compress_probe -- \
//!     --worklog <session>/.opendan_agent_session/worklog.jsonl \
//!     --system <run>/snapshots/0123.json --target 76800 \
//!     --mode llm --base-url http://127.0.0.1:8317/v1 --model gpt-6.1-sol \
//!     --out /tmp/summary.md
//! ```

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole};
use llm_context::context_window::estimate_messages;
use llm_context::deps::{
    LLMContextDeps, LlmClient, LlmInferenceRequest, ToolCallCtx, ToolDispatchError, ToolManager,
};
use llm_context::error::LLMComputeError;
use llm_context::observation::Observation;
use serde_json::Value;

const HOT_TAIL_ROUNDS: usize = 2;
const HINT: &str = "[runtime hint] Everything before this line was compressed into the summary above. Files you wrote in the session directory are intact and the evidence you collected is still on disk; re-read only what you actually need, do not redo finished work.";

struct MockSummarizer {
    reply: String,
}

#[async_trait]
impl LlmClient for MockSummarizer {
    async fn infer(&self, _req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        Ok(AiResponse::text(self.reply.clone()))
    }
}

struct StubTools;
#[async_trait]
impl ToolManager for StubTools {
    async fn call_tool(
        &self,
        call: buckyos_api::AiToolCall,
        _ctx: ToolCallCtx,
    ) -> Result<Observation, ToolDispatchError> {
        Ok(Observation::Error {
            call_id: call.call_id,
            message: "stub".into(),
            host: None,
        })
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

struct Round {
    assistant: AiMessage,
    results: Vec<AiMessage>,
}

fn round_msgs(r: &Round) -> Vec<AiMessage> {
    let mut v = vec![r.assistant.clone()];
    v.extend(r.results.iter().cloned());
    v
}

fn min_line(calls: &HashMap<String, String>, m: &AiMessage) -> AiMessage {
    if let AiContent::ToolResult {
        call_id, is_error, ..
    } = &m.content[0]
    {
        let line = format!(
            "{} => {}",
            calls.get(call_id).cloned().unwrap_or_default(),
            if *is_error { "error" } else { "ok" }
        );
        return AiMessage::new(
            AiRole::Tool,
            vec![AiContent::tool_result_text(call_id, line, *is_error)],
        );
    }
    m.clone()
}

fn render_round(i: usize, r: &Round) -> String {
    let mut s = format!("### round {i}\n");
    for c in &r.assistant.content {
        match c {
            AiContent::Text { text } if !text.trim().is_empty() => {
                s.push_str("assistant: ");
                s.push_str(text);
                s.push('\n');
            }
            AiContent::ToolUse { call_id, name, args } => {
                s.push_str(&format!(
                    "call {call_id}: {name}({})\n",
                    serde_json::to_string(args).unwrap_or_default()
                ));
            }
            _ => {}
        }
    }
    for m in &r.results {
        if let AiContent::ToolResult {
            call_id,
            content,
            is_error,
        } = &m.content[0]
        {
            let text: String = content
                .iter()
                .filter_map(|x| x.text_str().map(str::to_string))
                .collect::<Vec<_>>()
                .join("\n");
            s.push_str(&format!(
                "result {call_id} [{}]:\n{text}\n",
                if *is_error { "error" } else { "ok" }
            ));
        }
    }
    s
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let worklog = arg(&args, "--worklog").expect("--worklog");
    let system_snapshot = arg(&args, "--system");
    let target: u32 = arg(&args, "--target")
        .map(|s| s.parse().unwrap())
        .unwrap_or(76_800);
    let from: u64 = arg(&args, "--from").map(|s| s.parse().unwrap()).unwrap_or(0);
    let upto: Option<u64> = arg(&args, "--upto").map(|s| s.parse().unwrap());
    let mode = arg(&args, "--mode").unwrap_or_else(|| "auto".into());
    let model = arg(&args, "--model").unwrap_or_else(|| "gpt-6.1-sol".into());
    let base_url = arg(&args, "--base-url");
    let out_path = arg(&args, "--out");
    let prev_summary = arg(&args, "--prev-summary")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_default();

    // --- rebuild history ---------------------------------------------------
    let mut system: Vec<AiMessage> = Vec::new();
    if let Some(p) = &system_snapshot {
        let snap: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        for m in snap["request"]["input"].as_array().unwrap() {
            if m["role"] == "system" {
                system.push(AiMessage::text(
                    AiRole::System,
                    m["content"][0]["text"].as_str().unwrap(),
                ));
            }
        }
    }
    let mut opening: Vec<AiMessage> = Vec::new();
    let mut rounds: Vec<Round> = Vec::new();
    let mut calls: HashMap<String, String> = HashMap::new();
    let mut stop_seq = None;
    for line in std::fs::read_to_string(&worklog).unwrap().lines() {
        if line.trim().is_empty() {
            continue;
        }
        let e: Value = serde_json::from_str(line).unwrap();
        let seq = e["seq"].as_u64().unwrap_or(0);
        if seq <= from {
            continue;
        }
        if let Some(u) = upto {
            if seq > u {
                break;
            }
        }
        match e["t"].as_str().unwrap_or("") {
            "user_message" => {
                let m = AiMessage::text(AiRole::User, e["content"].as_str().unwrap_or(""));
                if rounds.is_empty() {
                    opening.push(m);
                } else {
                    // a mid-run input: attach to the last round as a plain user message
                    rounds.last_mut().unwrap().results.push(m);
                }
            }
            "assistant_message" => {
                let mut content = Vec::new();
                let text = e["assistant"].as_str().unwrap_or("");
                if !text.trim().is_empty() {
                    content.push(AiContent::text(text));
                }
                for c in e["tool_calls"].as_array().cloned().unwrap_or_default() {
                    let a: HashMap<String, Value> = c["args"]
                        .as_object()
                        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default();
                    let call_id = c["call_id"].as_str().unwrap().to_string();
                    let name = c["tool"].as_str().unwrap().to_string();
                    let short: String = serde_json::to_string(&a)
                        .unwrap_or_default()
                        .chars()
                        .take(120)
                        .collect();
                    calls.insert(call_id.clone(), format!("{name}({short})"));
                    content.push(AiContent::ToolUse {
                        call_id,
                        name,
                        args: a,
                    });
                }
                rounds.push(Round {
                    assistant: AiMessage::new(AiRole::Assistant, content),
                    results: Vec::new(),
                });
            }
            "action_result" => {
                let is_error = e["status"] == "error";
                let m = AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::tool_result_text(
                        e["call_id"].as_str().unwrap(),
                        e["result"].as_str().unwrap_or(""),
                        is_error,
                    )],
                );
                if let Some(r) = rounds.last_mut() {
                    r.results.push(m);
                }
            }
            "outcome" if e["kind"] == "context_rewritten" => {
                stop_seq = Some(seq);
                break;
            }
            _ => {}
        }
    }
    let mut history: Vec<AiMessage> = system.clone();
    history.extend(opening.iter().cloned());
    for r in &rounds {
        history.extend(round_msgs(r));
    }

    // --- deps (mock llm for the stock compressor) ----------------------------
    let reply = serde_json::json!({ "summary": "x ".repeat(3000) }).to_string();
    let mock: Arc<dyn LlmClient> = Arc::new(MockSummarizer { reply });
    let tools: Arc<dyn ToolManager> = Arc::new(StubTools);
    let deps = LLMContextDeps::new(mock, tools);
    let tok = deps.tokenizer.as_ref();
    let est = |msgs: &[AiMessage]| estimate_messages(tok, msgs);

    let total = est(&history);
    let mut by_role: BTreeMap<&str, u64> = BTreeMap::new();
    for m in &history {
        let r = match m.role {
            AiRole::System => "system",
            AiRole::User => "user",
            AiRole::Assistant => "assistant",
            AiRole::Tool => "tool",
            _ => "other",
        };
        *by_role.entry(r).or_default() += est(std::slice::from_ref(m));
    }
    println!("== {worklog} (seq {from}..{:?})", stop_seq.or(upto));
    println!(
        "messages={} rounds={} total≈{total} tokens by role {:?}; target={target}",
        history.len(),
        rounds.len(),
        by_role
    );

    // --- 1. stock llm_compress -------------------------------------------------
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt
        .block_on(agent_tool::compress(&history, &deps, target, "mock", None))
        .unwrap();
    println!(
        "stock llm_compress: {} → {} messages, ≈{} tokens, unchanged={}",
        history.len(),
        out.len(),
        est(&out),
        out == history
    );

    // --- 2. proposed shape ----------------------------------------------------
    if rounds.len() <= HOT_TAIL_ROUNDS + 1 {
        println!("too few rounds for head + middle + tail");
        return;
    }
    let head: Vec<AiMessage> = opening
        .iter()
        .cloned()
        .chain(round_msgs(&rounds[0]))
        .collect();
    let middle = &rounds[1..rounds.len() - HOT_TAIL_ROUNDS];
    let tail: Vec<AiMessage> = rounds[rounds.len() - HOT_TAIL_ROUNDS..]
        .iter()
        .flat_map(round_msgs)
        .collect();
    let middle_raw: Vec<AiMessage> = middle.iter().flat_map(round_msgs).collect();
    let middle_mech: Vec<AiMessage> = middle
        .iter()
        .flat_map(|r| {
            let mut v = vec![r.assistant.clone()];
            v.extend(r.results.iter().map(|m| min_line(&calls, m)));
            v
        })
        .collect();
    let (sys_t, head_t, mid_t, mech_t, tail_t) = (
        est(&system),
        est(&head),
        est(&middle_raw),
        est(&middle_mech),
        est(&tail),
    );
    println!(
        "shape: system≈{sys_t} head≈{head_t} middle≈{mid_t} (rounds {}..{}) tail≈{tail_t} (last {HOT_TAIL_ROUNDS} rounds)",
        1,
        rounds.len() - HOT_TAIL_ROUNDS
    );
    let mech_total = sys_t + head_t + mech_t + tail_t;
    println!(
        "mechanical (middle results → Min lines): middle≈{mech_t}, whole≈{mech_total} → {}",
        if mech_total <= target as u64 {
            "reaches target (mechanical would stop here)"
        } else {
            "still over target → LLM summary"
        }
    );
    if mode == "mechanical" || (mode == "auto" && mech_total <= target as u64) {
        return;
    }

    // --- LLM summary -------------------------------------------------------------
    let Some(base) = base_url else {
        println!("no --base-url: skipping the LLM summary");
        return;
    };
    let key = std::env::var("OPENAI_API_KEY").ok();
    let llm: Arc<dyn LlmClient> = Arc::new(
        agent_tool::xllm::OpenAiLlmClient::new(Some(base), key, BTreeMap::new()).unwrap(),
    );
    let mut segment = String::new();
    for (i, r) in middle.iter().enumerate() {
        segment.push_str(&render_round(i + 1, r));
        segment.push('\n');
    }
    let seg_tokens = tok.count_tokens(&segment);
    let system_prompt = "You are the OpenDAN Agent Runtime history compressor.\n\n\
Compress the supplied work history of an agent session into a summary that lets the same agent continue the task without the original messages. The agent has a separate long-term memory system, so be bold: drop raw tool output, page dumps, command transcripts and anything the agent can re-derive from files it already wrote. Keep, precisely and with names: the task and its constraints; what has been verified (with the key numbers / versions / quotes that matter); decisions made and why; files, scripts and evidence written in the session directory (paths) and what each contains; interim reports already submitted; open problems and what remains to be done. Merge with the previous summary when one is given: the result must stand alone. Do not invent facts; mark uncertainty. Write in the language of the task. Reply with the summary text only (markdown), no preamble.";
    let user_prompt = format!(
        "<previous_summary>\n{}\n</previous_summary>\n\n<history rounds=\"{}\">\n{}\n</history>",
        if prev_summary.trim().is_empty() {
            "(none)"
        } else {
            prev_summary.trim()
        },
        middle.len(),
        segment
    );
    println!(
        "LLM summary: model={model}, segment≈{seg_tokens} tokens (+ previous summary≈{}) …",
        tok.count_tokens(&prev_summary)
    );
    let req = LlmInferenceRequest {
        trace_id: None,
        messages: vec![
            AiMessage::text(AiRole::System, system_prompt),
            AiMessage::text(AiRole::User, user_prompt),
        ],
        model_alias: model.clone(),
        fallbacks: Vec::new(),
        temperature: None,
        max_completion_tokens: Some(4096),
        force_json: false,
        json_schema: None,
        provider_options: None,
        disable_capabilities: Vec::new(),
        tool_specs: Vec::new(),
        allow_tool_calls: false,
        abort: llm_context::InferenceAbortToken::noop(),
    };
    let started = Instant::now();
    let resp = rt.block_on(llm.infer(req));
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            println!("LLM call failed after {:?}: {e}", started.elapsed());
            return;
        }
    };
    let summary = resp.message.text_content().trim().to_string();
    let sum_t = tok.count_tokens(&summary);
    println!(
        "LLM summary done in {:?}: ≈{sum_t} tokens; usage={:?}",
        started.elapsed(),
        resp.usage.map(|u| (u.input_tokens, u.output_tokens))
    );
    let mut rewritten: Vec<AiMessage> = system.clone();
    rewritten.extend(head.iter().cloned());
    rewritten.push(AiMessage::text(
        AiRole::User,
        format!("<history_summary>\n{summary}\n</history_summary>\n\n{HINT}"),
    ));
    rewritten.extend(tail.iter().cloned());
    let rew_t = est(&rewritten);
    println!(
        "rewritten: {} messages ≈{rew_t} tokens ({:.0}% of {total}); target {target}",
        rewritten.len(),
        100.0 * rew_t as f64 / total as f64
    );
    if let Some(p) = out_path {
        std::fs::write(&p, &summary).unwrap();
        println!("summary written to {p}");
    } else {
        println!("----- summary -----\n{summary}");
    }
}
