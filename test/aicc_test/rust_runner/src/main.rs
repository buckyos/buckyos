use anyhow::{anyhow, Result};
use buckyos_api::{
    AiMessage, AiPayload, AiccClient, Capability, CompleteRequest, ModelSpec, Requirements,
};
use clap::{Parser, Subcommand};
use ::kRPC::{kRPC, RPCContext};
use serde::Serialize;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TOKENS_BASIC_COMPLETE: u32 = 2048;

#[derive(Parser, Debug)]
#[command(name = "aicc-remote-rust-runner")]
#[command(about = "Standalone AiccClient remote case runner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Complete(CommonArgs),
    Cancel(CommonArgs),
}

#[derive(Parser, Debug, Clone)]
struct CommonArgs {
    #[arg(long)]
    endpoint: String,
    #[arg(long)]
    model_alias: String,
    #[arg(long)]
    prompt: String,
    #[arg(long)]
    token: Option<String>,
    #[arg(long, default_value = "aicc-rust-runner")]
    trace_id: String,
}

#[derive(Serialize)]
struct CompleteOutput {
    task_id: String,
    status: String,
}

#[derive(Serialize)]
struct CancelOutput {
    task_id: String,
    accepted: bool,
}

fn request(args: &CommonArgs) -> CompleteRequest {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    CompleteRequest::new(
        Capability::LlmRouter,
        ModelSpec::new(args.model_alias.clone(), None),
        Requirements::new(vec![], Some(10_000), Some(0.2), None),
        AiPayload::new(
            None,
            vec![AiMessage::new("user".to_string(), args.prompt.clone())],
            vec![],
            vec![],
            None,
            Some(json!({
                "temperature": 0.1,
                "max_tokens": MAX_TOKENS_BASIC_COMPLETE
            })),
        ),
        Some(format!("rust-runner-{}", now)),
    )
}

fn status_to_string(status: &buckyos_api::CompleteStatus) -> &'static str {
    match status {
        buckyos_api::CompleteStatus::Succeeded => "succeeded",
        buckyos_api::CompleteStatus::Running => "running",
        buckyos_api::CompleteStatus::Failed => "failed",
    }
}

async fn build_client(args: &CommonArgs) -> AiccClient {
    let client = AiccClient::new(kRPC::new(args.endpoint.as_str(), None));
    client
        .set_context(RPCContext {
            token: args.token.clone(),
            trace_id: Some(args.trace_id.clone()),
            ..Default::default()
        })
        .await;
    client
}

async fn run_complete(args: CommonArgs) -> Result<()> {
    let client = build_client(&args).await;
    let response = client.complete(request(&args)).await?;
    if response.task_id.trim().is_empty() {
        return Err(anyhow!("missing task_id in complete response"));
    }

    let output = CompleteOutput {
        task_id: response.task_id,
        status: status_to_string(&response.status).to_string(),
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

async fn run_cancel(args: CommonArgs) -> Result<()> {
    let client = build_client(&args).await;
    let started = client.complete(request(&args)).await?;
    if started.task_id.trim().is_empty() {
        return Err(anyhow!("missing task_id before cancel"));
    }
    let cancel = client.cancel(started.task_id.as_str()).await?;

    let output = CancelOutput {
        task_id: cancel.task_id,
        accepted: cancel.accepted,
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Complete(args) => run_complete(args).await,
        Command::Cancel(args) => run_cancel(args).await,
    };

    if let Err(err) = result {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}
