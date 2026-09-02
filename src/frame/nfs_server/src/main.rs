//! Launcher with two modes:
//!
//! Standalone (kept for independent testing — see test/test_nfs_server/):
//!
//!   nfs_server --listen 127.0.0.1:3260 --data-dir ./nfs-data \
//!              --export home=/srv/home --export media=/srv/media \
//!              [--scan-interval-secs 30] [--debug-api] [--log-level info]
//!
//! BuckyOS service mode (no --data-dir/--export; how node_daemon launches it):
//! logs in via buckyos-api (KernelService runtime, heartbeat included), serves
//! `cyfs:///` from $BUCKYOS_ROOT/data (every visible first-level directory is
//! an export root), keeps filedb + staging in $BUCKYOS_ROOT/data/.fsdb, and
//! listens on NFS_SERVER_SERVICE_PORT. The zone gateway routes the NFSP
//! zone-level protocol path `/nfs/v1/*` here verbatim (boot_gateway.yaml) —
//! NFSP is a cross-zone protocol at the URL root, not a /kapi/<service> API.
//! No per-request auth yet (the whole tree is visible); that arrives with the
//! auth milestone.

use buckyos_api::{
    init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType, NfsServerSettings,
    NFS_SERVER_SERVICE_NAME, NFS_SERVER_SERVICE_PORT,
};
use buckyos_kit::{get_buckyos_root_dir, init_logging};
use clap::{Arg, ArgAction, Command};
use nfs_server::config::{config_from_data_root, parse_export_spec, ServerConfig};
use nfs_server::state::AppState;

fn main() {
    let matches = Command::new("nfs_server")
        .about("BuckyOS NFSP v0 file service")
        .arg(
            Arg::new("listen")
                .long("listen")
                .help("Address to listen on (standalone; default 127.0.0.1:3260)"),
        )
        .arg(
            Arg::new("data-dir")
                .long("data-dir")
                .help("Directory for filedb.sqlite and upload staging (standalone mode)"),
        )
        .arg(
            Arg::new("export")
                .long("export")
                .action(ArgAction::Append)
                .help("Export root mapping name=/abs/path (repeatable; standalone mode)"),
        )
        .arg(
            Arg::new("scan-interval-secs")
                .long("scan-interval-secs")
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
                .help("Log level: error|warn|info|debug|trace (standalone mode)"),
        )
        .get_matches();

    let standalone =
        matches.get_one::<String>("data-dir").is_some() || matches.contains_id("export");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if standalone {
        let level = matches.get_one::<String>("log-level").unwrap().clone();
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
        let config = standalone_config(&matches);
        rt.block_on(serve(config));
    } else {
        init_logging("nfs_server", true);
        rt.block_on(buckyos_service_main(&matches));
    }
}

fn standalone_config(matches: &clap::ArgMatches) -> ServerConfig {
    let data_dir = matches.get_one::<String>("data-dir").unwrap_or_else(|| {
        eprintln!("error: --data-dir is required in standalone mode (given --export)");
        std::process::exit(2);
    });
    let mut exports = Vec::new();
    let specs = matches.get_many::<String>("export").unwrap_or_else(|| {
        eprintln!("error: at least one --export is required in standalone mode");
        std::process::exit(2);
    });
    for spec in specs {
        match parse_export_spec(spec) {
            Ok(root) => exports.push(root),
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(2);
            }
        }
    }
    let mut config = ServerConfig::new(data_dir.into(), exports);
    if let Some(listen) = matches.get_one::<String>("listen") {
        config.listen = listen.clone();
    }
    config.scan_interval_secs = parse_scan_interval(matches).unwrap_or(0);
    config.debug_api = matches.get_flag("debug-api");
    config
}

fn parse_scan_interval(matches: &clap::ArgMatches) -> Option<u64> {
    matches.get_one::<String>("scan-interval-secs").map(|s| {
        s.parse().unwrap_or_else(|_| {
            eprintln!("error: bad --scan-interval-secs");
            std::process::exit(2);
        })
    })
}

/// BuckyOS mode: buckyos-api login + heartbeat, then the same server core.
async fn buckyos_service_main(matches: &clap::ArgMatches) {
    let mut runtime = match init_buckyos_api_runtime(
        NFS_SERVER_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("nfs-server init buckyos runtime failed: {:?}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = runtime.login().await {
        log::error!("nfs-server login to system failed: {:?}", e);
        std::process::exit(1);
    }
    runtime.set_main_service_port(NFS_SERVER_SERVICE_PORT).await;

    // Settings are optional: a zone booted before this service existed has no
    // services/nfs-server/settings key yet — fall back to defaults.
    let settings: NfsServerSettings = match runtime.get_my_settings().await {
        Ok(v) => serde_json::from_value(v).unwrap_or_else(|e| {
            log::warn!("bad nfs-server settings, using defaults: {}", e);
            NfsServerSettings::default()
        }),
        Err(e) => {
            log::warn!("load nfs-server settings failed, using defaults: {:?}", e);
            NfsServerSettings::default()
        }
    };
    if let Err(e) = set_buckyos_api_runtime(runtime) {
        log::error!("register nfs-server runtime failed: {:?}", e);
        std::process::exit(1);
    }

    let data_root = get_buckyos_root_dir().join("data");
    let mut config = match config_from_data_root(&data_root) {
        Ok(c) => c,
        Err(e) => {
            log::error!("prepare data root {} failed: {}", data_root.display(), e);
            std::process::exit(1);
        }
    };
    config.listen = format!("127.0.0.1:{}", NFS_SERVER_SERVICE_PORT);
    config.scan_interval_secs = settings.scan_interval_secs;
    // CLI --debug-api / --scan-interval-secs still win over settings (dev use).
    config.debug_api = settings.debug_api || matches.get_flag("debug-api");
    if let Some(secs) = parse_scan_interval(matches) {
        config.scan_interval_secs = secs;
    }
    log::info!(
        "nfs-server buckyos mode: cyfs:/// -> {}, filedb in {}",
        data_root.display(),
        config.data_dir.display()
    );
    serve(config).await;
}

async fn serve(config: ServerConfig) {
    let listen = config.listen.clone();
    let scan_interval = config.scan_interval_secs;
    let state = match AppState::new(config) {
        Ok(s) => s,
        Err(e) => {
            log::error!("startup failed: {}", e);
            eprintln!("startup failed: {}", e);
            std::process::exit(1);
        }
    };
    for export in &state.config.exports {
        log::info!("export cyfs:///{} -> {}", export.id, export.path.display());
    }
    if scan_interval > 0 {
        log::info!("reconciler scan every {}s", scan_interval);
        tokio::spawn(nfs_server::reconciler::reconcile_loop(state.clone(), scan_interval));
    } else {
        log::info!("reconciler scan loop disabled (scan interval 0)");
    }
    let app = nfs_server::server::build_router(state);
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("bind {} failed: {}", listen, e);
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
