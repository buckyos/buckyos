use std::path::PathBuf;

use buckyos_api::node_control::{
    detect_host_control_state, node_check, node_start, node_stop, print_check_report,
    NodeBlackboxStopPolicy, NodeCheckOptions, NodeCheckReport, NodeFaultLevel,
    NodeHostControlPolicy, NodeStartMode, NodeStartRequest, NodeStopMode, NodeStopReport,
    NodeStopRequest,
};
use clap::{value_parser, Arg, ArgAction, Command};

pub fn build_node_command() -> Command {
    Command::new("node")
        .about("local node runtime control (replaces start.py / stop.py / check.py)")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("check")
                .about("read-only diagnostic of the local BuckyOS runtime")
                .arg(
                    Arg::new("level")
                        .long("level")
                        .value_parser(["minimal", "basic", "standard"])
                        .default_value("standard")
                        .help("check level"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(ArgAction::SetTrue)
                        .help("emit JSON report instead of human text"),
                )
                .arg(
                    Arg::new("buckyos_root")
                        .long("buckyos_root")
                        .help("override BUCKYOS_ROOT for this invocation"),
                ),
        )
        .subcommand(
            Command::new("start")
                .about("ensure the local BuckyOS runtime is running")
                .arg(
                    Arg::new("mode")
                        .long("mode")
                        .value_parser([
                            "normal",
                            "activation",
                            "recovery",
                            "desktop-daemon",
                            "safe-mode",
                        ])
                        .default_value("normal")
                        .help("start mode"),
                )
                .arg(
                    Arg::new("host_control")
                        .long("host_control")
                        .value_parser([
                            "auto",
                            "service-manager",
                            "scheduled-launcher",
                            "direct-process",
                            "refuse-if-unknown",
                        ])
                        .default_value("auto"),
                )
                .arg(
                    Arg::new("no_stop_conflicting")
                        .long("no_stop_conflicting")
                        .action(ArgAction::SetTrue)
                        .help("skip killing existing node_daemon before start"),
                )
                .arg(
                    Arg::new("buckyos_root")
                        .long("buckyos_root")
                        .help("override BUCKYOS_ROOT for this invocation"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("ensure-running")
                .about("alias for `node start --mode activation`")
                .arg(
                    Arg::new("mode")
                        .long("mode")
                        .value_parser([
                            "normal",
                            "activation",
                            "recovery",
                            "desktop-daemon",
                            "safe-mode",
                        ])
                        .default_value("activation"),
                )
                .arg(
                    Arg::new("host_control")
                        .long("host_control")
                        .value_parser([
                            "auto",
                            "service-manager",
                            "scheduled-launcher",
                            "direct-process",
                            "refuse-if-unknown",
                        ])
                        .default_value("auto"),
                )
                .arg(
                    Arg::new("buckyos_root")
                        .long("buckyos_root")
                        .help("override BUCKYOS_ROOT"),
                )
                .arg(Arg::new("json").long("json").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("stop")
                .about("stop the local BuckyOS runtime")
                .arg(
                    Arg::new("mode")
                        .long("mode")
                        .value_parser([
                            "graceful",
                            "graceful-then-force",
                            "stop-host-service",
                            "blackbox-force",
                            "kill-all",
                        ])
                        .default_value("graceful-then-force")
                        .help("stop mode"),
                )
                .arg(
                    Arg::new("blackbox")
                        .long("blackbox")
                        .value_parser([
                            "disabled",
                            "buckyos-processes-only",
                            "include-managed-containers",
                            "include-host-service",
                            "full-runtime",
                        ])
                        .default_value("include-managed-containers"),
                )
                .arg(
                    Arg::new("host_control")
                        .long("host_control")
                        .value_parser([
                            "auto",
                            "service-manager",
                            "scheduled-launcher",
                            "direct-process",
                            "refuse-if-unknown",
                        ])
                        .default_value("auto"),
                )
                .arg(
                    Arg::new("timeout")
                        .long("timeout")
                        .value_parser(value_parser!(u64))
                        .default_value("30")
                        .help("graceful timeout in seconds"),
                )
                .arg(
                    Arg::new("dev")
                        .long("dev")
                        .action(ArgAction::SetTrue)
                        .help("dev kill-all preset (overrides --mode/--blackbox)"),
                )
                .arg(
                    Arg::new("buckyos_root")
                        .long("buckyos_root")
                        .help("override BUCKYOS_ROOT"),
                )
                .arg(Arg::new("json").long("json").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("restart")
                .about("stop then start the local BuckyOS runtime")
                .arg(
                    Arg::new("stop_mode")
                        .long("stop_mode")
                        .value_parser([
                            "graceful",
                            "graceful-then-force",
                            "blackbox-force",
                            "kill-all",
                        ])
                        .default_value("graceful-then-force"),
                )
                .arg(
                    Arg::new("start_mode")
                        .long("start_mode")
                        .value_parser([
                            "normal",
                            "activation",
                            "recovery",
                            "desktop-daemon",
                            "safe-mode",
                        ])
                        .default_value("normal"),
                )
                .arg(
                    Arg::new("timeout")
                        .long("timeout")
                        .value_parser(value_parser!(u64))
                        .default_value("30"),
                )
                .arg(
                    Arg::new("buckyos_root")
                        .long("buckyos_root")
                        .help("override BUCKYOS_ROOT"),
                )
                .arg(Arg::new("json").long("json").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("detect-host-control")
                .about("print detected host service registration state"),
        )
}

pub async fn handle_node_command(matches: &clap::ArgMatches) -> Result<(), String> {
    match matches.subcommand() {
        Some(("check", sub)) => handle_check(sub),
        Some(("start", sub)) | Some(("ensure-running", sub)) => handle_start(sub),
        Some(("stop", sub)) => handle_stop(sub),
        Some(("restart", sub)) => handle_restart(sub),
        Some(("detect-host-control", _)) => handle_detect_host_control(),
        _ => Err("unknown node subcommand".to_string()),
    }
}

fn root_override(matches: &clap::ArgMatches) -> Option<PathBuf> {
    matches
        .get_one::<String>("buckyos_root")
        .map(|s| PathBuf::from(s))
}

fn handle_check(matches: &clap::ArgMatches) -> Result<(), String> {
    let level = matches
        .get_one::<String>("level")
        .map(String::as_str)
        .unwrap_or("standard");
    let opts = match level {
        "minimal" => NodeCheckOptions {
            include_logs: false,
            include_host_control: false,
        },
        "basic" => NodeCheckOptions::basic(),
        _ => NodeCheckOptions::standard(),
    };
    let report = node_check(root_override(matches), opts);
    let want_json = matches.get_flag("json");
    if want_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| format!("serialize report failed: {}", e))?
        );
    } else {
        print_check_report(&report);
    }
    if exit_code_for_report(&report) != 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn exit_code_for_report(report: &NodeCheckReport) -> i32 {
    match report.overall {
        NodeFaultLevel::Green | NodeFaultLevel::Blue => 0,
        _ => 1,
    }
}

fn parse_start_mode(value: &str) -> NodeStartMode {
    match value {
        "activation" => NodeStartMode::Activation,
        "recovery" => NodeStartMode::Recovery,
        "desktop-daemon" => NodeStartMode::DesktopDaemon,
        "safe-mode" => NodeStartMode::SafeMode,
        _ => NodeStartMode::Normal,
    }
}

fn parse_host_control(value: &str) -> NodeHostControlPolicy {
    match value {
        "service-manager" => NodeHostControlPolicy::PreferServiceManager,
        "scheduled-launcher" => NodeHostControlPolicy::PreferScheduledLauncher,
        "direct-process" => NodeHostControlPolicy::DirectProcessOnly,
        "refuse-if-unknown" => NodeHostControlPolicy::RefuseIfUnknown,
        _ => NodeHostControlPolicy::Auto,
    }
}

fn parse_stop_mode(value: &str) -> NodeStopMode {
    match value {
        "graceful" => NodeStopMode::Graceful,
        "stop-host-service" => NodeStopMode::StopHostService,
        "blackbox-force" => NodeStopMode::BlackboxForce,
        "kill-all" => NodeStopMode::KillAll,
        _ => NodeStopMode::GracefulThenForce,
    }
}

fn parse_blackbox_policy(value: &str) -> NodeBlackboxStopPolicy {
    match value {
        "disabled" => NodeBlackboxStopPolicy::Disabled,
        "buckyos-processes-only" => NodeBlackboxStopPolicy::BuckyOSProcessesOnly,
        "include-host-service" => NodeBlackboxStopPolicy::IncludeHostService,
        "full-runtime" => NodeBlackboxStopPolicy::FullRuntime,
        _ => NodeBlackboxStopPolicy::IncludeManagedContainers,
    }
}

fn handle_start(matches: &clap::ArgMatches) -> Result<(), String> {
    let mode = parse_start_mode(
        matches
            .get_one::<String>("mode")
            .map(String::as_str)
            .unwrap_or("normal"),
    );
    let host_control_policy = parse_host_control(
        matches
            .get_one::<String>("host_control")
            .map(String::as_str)
            .unwrap_or("auto"),
    );
    let stop_conflicting = !matches
        .try_get_one::<bool>("no_stop_conflicting")
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false);

    let req = NodeStartRequest {
        mode,
        host_control_policy,
        buckyos_root: root_override(matches),
        stop_conflicting,
        reason: Some("buckycli node start".to_string()),
    };

    let report = node_start(req).map_err(|e| format!("node start failed: {}", e))?;
    let want_json = matches
        .try_get_one::<bool>("json")
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false);
    if want_json {
        let value = serde_json::json!({
            "used_model": format!("{:?}", report.used_model),
            "actions": report.actions,
            "started_pid": report.started_pid,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("=== node start ===");
        println!("- used model: {:?}", report.used_model);
        if let Some(pid) = report.started_pid {
            println!("- node_daemon pid: {}", pid);
        }
        for action in &report.actions {
            println!("  - {}", action);
        }
    }
    Ok(())
}

fn handle_stop(matches: &clap::ArgMatches) -> Result<(), String> {
    let dev = matches.get_flag("dev");
    let req = if dev {
        let mut r = NodeStopRequest::dev_kill_all();
        r.buckyos_root = root_override(matches);
        r
    } else {
        let mode = parse_stop_mode(
            matches
                .get_one::<String>("mode")
                .map(String::as_str)
                .unwrap_or("graceful-then-force"),
        );
        let blackbox = parse_blackbox_policy(
            matches
                .get_one::<String>("blackbox")
                .map(String::as_str)
                .unwrap_or("include-managed-containers"),
        );
        let host_control_policy = parse_host_control(
            matches
                .get_one::<String>("host_control")
                .map(String::as_str)
                .unwrap_or("auto"),
        );
        let timeout = matches.get_one::<u64>("timeout").copied().unwrap_or(30);
        NodeStopRequest {
            mode,
            blackbox_policy: blackbox,
            host_control_policy,
            timeout_secs: timeout,
            buckyos_root: root_override(matches),
            reason: Some("buckycli node stop".to_string()),
        }
    };

    let report = node_stop(req).map_err(|e| format!("node stop failed: {}", e))?;
    let want_json = matches.get_flag("json");
    if want_json {
        let value = serde_json::json!({
            "stopped_processes": report.stopped_processes,
            "stopped_containers": report.stopped_containers,
            "host_service_actions": report.host_service_actions,
            "remaining": report.remaining,
            "actions": report.actions,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        print_stop_report(&report);
    }
    Ok(())
}

fn print_stop_report(report: &NodeStopReport) {
    println!("=== node stop ===");
    if !report.host_service_actions.is_empty() {
        println!("- host service actions:");
        for line in &report.host_service_actions {
            println!("  - {}", line);
        }
    }
    if !report.stopped_processes.is_empty() {
        println!("- stopped processes:");
        for line in &report.stopped_processes {
            println!("  - {}", line);
        }
    } else {
        println!("- stopped processes: none");
    }
    if !report.stopped_containers.is_empty() {
        println!("- stopped containers:");
        for line in &report.stopped_containers {
            println!("  - {}", line);
        }
    }
    if !report.actions.is_empty() {
        println!("- actions:");
        for line in &report.actions {
            println!("  - {}", line);
        }
    }
    if report.remaining.is_empty() {
        println!("- remaining: clean");
    } else {
        println!("- remaining (still alive):");
        for line in &report.remaining {
            println!("  - {}", line);
        }
    }
}

fn handle_restart(matches: &clap::ArgMatches) -> Result<(), String> {
    let stop_mode = parse_stop_mode(
        matches
            .get_one::<String>("stop_mode")
            .map(String::as_str)
            .unwrap_or("graceful-then-force"),
    );
    let start_mode = parse_start_mode(
        matches
            .get_one::<String>("start_mode")
            .map(String::as_str)
            .unwrap_or("normal"),
    );
    let timeout = matches.get_one::<u64>("timeout").copied().unwrap_or(30);
    let buckyos_root = root_override(matches);
    let want_json = matches.get_flag("json");

    let stop_report = node_stop(NodeStopRequest {
        mode: stop_mode,
        blackbox_policy: NodeBlackboxStopPolicy::IncludeManagedContainers,
        host_control_policy: NodeHostControlPolicy::Auto,
        timeout_secs: timeout,
        buckyos_root: buckyos_root.clone(),
        reason: Some("buckycli node restart (stop phase)".to_string()),
    })
    .map_err(|e| format!("restart stop phase failed: {}", e))?;

    let start_report = node_start(NodeStartRequest {
        mode: start_mode,
        host_control_policy: NodeHostControlPolicy::Auto,
        buckyos_root,
        stop_conflicting: false,
        reason: Some("buckycli node restart (start phase)".to_string()),
    })
    .map_err(|e| format!("restart start phase failed: {}", e))?;

    if want_json {
        let value = serde_json::json!({
            "stop": {
                "stopped_processes": stop_report.stopped_processes,
                "stopped_containers": stop_report.stopped_containers,
                "host_service_actions": stop_report.host_service_actions,
                "remaining": stop_report.remaining,
                "actions": stop_report.actions,
            },
            "start": {
                "used_model": format!("{:?}", start_report.used_model),
                "actions": start_report.actions,
                "started_pid": start_report.started_pid,
            }
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        print_stop_report(&stop_report);
        println!();
        println!("=== node start ===");
        println!("- used model: {:?}", start_report.used_model);
        if let Some(pid) = start_report.started_pid {
            println!("- node_daemon pid: {}", pid);
        }
        for action in &start_report.actions {
            println!("  - {}", action);
        }
    }
    Ok(())
}

fn handle_detect_host_control() -> Result<(), String> {
    let state = detect_host_control_state();
    println!("{}", serde_json::to_string_pretty(&state).unwrap());
    Ok(())
}
