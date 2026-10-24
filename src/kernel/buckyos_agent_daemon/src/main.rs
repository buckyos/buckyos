use std::path::PathBuf;

use buckyos_agent_daemon::{
    access::AccessUrl,
    daemon::{run, DaemonOptions},
    local,
};
use clap::{Parser, Subcommand};
use tokio::sync::watch;

#[derive(Parser)]
#[command(
    version,
    about = "BuckyOS Agent Daemon: foreground executor for Linux, macOS and Windows",
    arg_required_else_help = true
)]
struct Args {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Run {
        #[arg(long, group = "url_source")]
        url: Option<String>,
        #[arg(long, group = "url_source")]
        url_stdin: bool,
        #[arg(long, group = "url_source")]
        url_file: Option<PathBuf>,
        #[arg(long)]
        shell: Option<PathBuf>,
        #[arg(long, hide = true)]
        allow_loopback_ws: bool,
    },
    Doctor {
        #[arg(long)]
        executor: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Status {
        #[arg(long)]
        executor: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Stop {
        #[arg(long)]
        executor: Option<String>,
    },
    Pause {
        #[arg(long)]
        executor: Option<String>,
    },
    Resume {
        #[arg(long)]
        executor: Option<String>,
    },
}

fn read_url(url: Option<String>, stdin: bool, file: Option<PathBuf>) -> Result<String, String> {
    if let Some(url) = url {
        return Ok(url);
    }
    if let Some(path) = file {
        #[cfg(windows)]
        {
            return String::from_utf8(local::windows::read_private_file(&path, 4096)?)
                .map_err(|_| "URL file must contain UTF-8".into());
        }
        #[cfg(not(windows))]
        {
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| "cannot read URL file")?;
            if !metadata.is_file() || metadata.len() > 4096 {
                return Err("URL file must be a regular file of at most 4096 bytes".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if metadata.uid() != local::current_uid()
                    || metadata.permissions().mode() & 0o077 != 0
                {
                    return Err("URL file must belong to the current user with mode 0600".into());
                }
            }
            return std::fs::read_to_string(path).map_err(|_| "cannot read URL file".into());
        }
    }
    if stdin {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin()
            .take(4097)
            .read_to_string(&mut input)
            .map_err(|_| "cannot read URL from stdin")?;
        if input.len() > 4096 {
            return Err("access URL is too long".into());
        }
        return Ok(input);
    }
    Err("provide --url, --url-stdin, or --url-file; no anonymous executor is started".into())
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some(node_executor::worker::WORKER_ARG) {
        node_executor::worker::worker_main();
    }
    let args = Args::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime");
    if let Err(error) = runtime.block_on(main_async(args)) {
        eprintln!("buckyos_agent_daemon: {error}");
        std::process::exit(1);
    }
}

async fn main_async(args: Args) -> Result<(), String> {
    let data_dir = match args.data_dir {
        Some(path) => path,
        None => local::default_data_dir()?,
    };
    match args.command {
        Command::Run {
            url,
            url_stdin,
            url_file,
            shell,
            allow_loopback_ws,
        } => {
            let access = AccessUrl::parse(&read_url(url, url_stdin, url_file)?, allow_loopback_ws)?;
            let (tx, rx) = watch::channel(false);
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    if let Ok(mut term) =
                        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    {
                        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
                    } else {
                        let _ = tokio::signal::ctrl_c().await;
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
                let _ = tx.send(true);
            });
            run(
                DaemonOptions {
                    access,
                    data_dir,
                    shell: shell.unwrap_or_else(node_executor::default_shell),
                },
                rx,
            )
            .await
        }
        Command::Doctor { executor, json } | Command::Status { executor, json } => {
            let instances = local::statuses(&data_dir, executor.as_deref())?;
            let capabilities = local::capabilities(&node_executor::default_shell());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"), "identity": local::identity(),
                        "capabilities": capabilities, "instances": instances
                    }))
                    .map_err(|_| "cannot encode status")?
                );
            } else {
                println!(
                    "BuckyOS Agent Daemon {} — {} backend",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS
                );
                println!(
                    "Identity: current_user ({}); UI: {}",
                    local::current_identity(),
                    capabilities["ui"]["state"]
                        .as_str()
                        .unwrap_or("unavailable")
                );
                for name in ["command", "file", "capture", "input", "accessibility"] {
                    let capability = &capabilities[name];
                    println!(
                        "{name}: supported={}, ready={}{}",
                        capability["supported"].as_bool().unwrap_or(false),
                        capability["ready"].as_bool().unwrap_or(false),
                        capability["reason"]
                            .as_str()
                            .map(|reason| format!(" ({reason})"))
                            .unwrap_or_default()
                    );
                }
                if instances.is_empty() {
                    println!("No local executor instance. Start with run --url-stdin.");
                }
                for state in instances {
                    println!(
                        "{}: {}; generation {:?}; expires {:?}",
                        state.executor_id, state.state, state.generation, state.expires_at_ms
                    );
                }
            }
            Ok(())
        }
        Command::Stop { executor } => local::stop(&data_dir, executor.as_deref()).await,
        Command::Pause { executor } => {
            local::set_computer_paused(&data_dir, executor.as_deref(), true)
        }
        Command::Resume { executor } => {
            local::set_computer_paused(&data_dir, executor.as_deref(), false)
        }
    }
}
