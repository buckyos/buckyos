//! Standalone launcher. v1 deliberately starts without any buckyos runtime
//! dependency (nfs_server.md is a frame service; node_daemon integration and
//! buckyos-api login arrive with the auth milestone).
//!
//!   nfs_server --listen 127.0.0.1:3260 --data-dir ./nfs-data \
//!              --export home=/srv/home --export media=/srv/media \
//!              [--scan-interval-secs 30] [--debug-api] [--log-level info]

use clap::{Arg, ArgAction, Command};
use nfs_server::config::{parse_export_spec, ServerConfig};
use nfs_server::state::AppState;

fn main() {
    let matches = Command::new("nfs_server")
        .about("BuckyOS NFSP v0 file service (standalone v1)")
        .arg(
            Arg::new("listen")
                .long("listen")
                .default_value("127.0.0.1:3260")
                .help("Address to listen on"),
        )
        .arg(
            Arg::new("data-dir")
                .long("data-dir")
                .required(true)
                .help("Directory for filedb.sqlite and upload staging"),
        )
        .arg(
            Arg::new("export")
                .long("export")
                .action(ArgAction::Append)
                .required(true)
                .help("Export root mapping name=/abs/path (repeatable)"),
        )
        .arg(
            Arg::new("scan-interval-secs")
                .long("scan-interval-secs")
                .default_value("0")
                .help("Reconciler scan interval in seconds (0 = disabled)"),
        )
        .arg(
            Arg::new("debug-api")
                .long("debug-api")
                .action(ArgAction::SetTrue)
                .help("Enable POST /nfs/v1/debug/* (dev/test only)"),
        )
        .arg(
            Arg::new("log-level")
                .long("log-level")
                .default_value("info")
                .help("Log level: error|warn|info|debug|trace"),
        )
        .get_matches();

    let level = matches.get_one::<String>("log-level").unwrap().clone();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();

    let mut exports = Vec::new();
    for spec in matches.get_many::<String>("export").unwrap() {
        match parse_export_spec(spec) {
            Ok(root) => exports.push(root),
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(2);
            }
        }
    }
    let mut config = ServerConfig::new(
        matches.get_one::<String>("data-dir").unwrap().into(),
        exports,
    );
    config.listen = matches.get_one::<String>("listen").unwrap().clone();
    config.scan_interval_secs = matches
        .get_one::<String>("scan-interval-secs")
        .unwrap()
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("error: bad --scan-interval-secs");
            std::process::exit(2);
        });
    config.debug_api = matches.get_flag("debug-api");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(async_main(config));
}

async fn async_main(config: ServerConfig) {
    let listen = config.listen.clone();
    let scan_interval = config.scan_interval_secs;
    let state = match AppState::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("startup failed: {}", e);
            std::process::exit(1);
        }
    };
    for export in &state.config.exports {
        log::info!("export dfs://{} -> {}", export.id, export.path.display());
    }
    if scan_interval > 0 {
        log::info!("reconciler scan every {}s", scan_interval);
        tokio::spawn(nfs_server::reconciler::reconcile_loop(state.clone(), scan_interval));
    } else {
        log::info!("reconciler scan loop disabled (--scan-interval-secs 0)");
    }
    let app = nfs_server::server::build_router(state);
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {} failed: {}", listen, e);
            std::process::exit(1);
        }
    };
    log::info!("nfs_server listening on http://{}", listen);
    if let Err(e) = axum::serve(listener, app).await {
        log::error!("server error: {}", e);
        std::process::exit(1);
    }
}
