//! Launcher with two modes (same pattern as nfs_server):
//!
//! Standalone (independent testing):
//!
//!   aiworkspace --data-dir ./aiws-data --listen 127.0.0.1:4120 \
//!               --auth-file tokens.json [--fixture-sources] [--log-level info]
//!
//!   tokens.json: { "tokens": { "<token>": { "principal": "alice" } } }
//!
//! BuckyOS service mode (no --data-dir; how node_daemon launches it): logs in
//! via buckyos-api (KernelService runtime), keeps Workspace folders under the
//! service data folder (user data, kept across soft reset), listens on
//! AIWORKSPACE_SERVICE_PORT and is reached through the generic
//! `/kapi/aiworkspace` gateway route. Every request is authenticated here.
//!
//! `AIWS_FAILPOINT=before_txn|in_txn|after_commit[:abort][:N]` injects a fault
//! into the N-th commit (crash tests).

use aiworkspace_server::auth::{Authenticator, RuntimeAuth, StaticTokens};
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{AppState, Limits, Wake};
use aiworkspace_store::urlsource::GeneratedSource;
use aiworkspace_store::workspace::FailPoint;
use aiworkspace_store::Service;
use buckyos_api::{
    init_buckyos_api_runtime, set_buckyos_api_runtime, AiWorkspaceSettings, BuckyOSRuntimeType, AIWORKSPACE_SERVICE_NAME,
    AIWORKSPACE_SERVICE_PORT,
};
use buckyos_kit::init_logging;
use clap::{Arg, ArgAction, Command};
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let matches = Command::new("aiworkspace")
        .about("BuckyOS AI Workspace artifact management service")
        .arg(Arg::new("data-dir").long("data-dir").help("Data directory (standalone mode)"))
        .arg(Arg::new("listen").long("listen").help("Address to listen on (standalone; default 127.0.0.1:4120)"))
        .arg(Arg::new("auth-file").long("auth-file").help("Static test identities JSON (standalone mode)"))
        .arg(Arg::new("fixture-sources").long("fixture-sources").action(ArgAction::SetTrue).help("Register the generated fixture:// data source (dev/test)"))
        .arg(Arg::new("wish-config").long("wish-config").help("Wish runs (standalone): JSON { provider: { type, base_url, api_key_env }, analyze_model, execute_model, map_model, deno, … }"))
        .arg(Arg::new("log-level").long("log-level").default_value("info"))
        .get_matches();
    // wish runs drive the xllm loop on worker threads: deep futures need more than the default stack
    let rt = tokio::runtime::Builder::new_multi_thread().thread_stack_size(8 * 1024 * 1024).enable_all().build().expect("tokio runtime");
    if let Some(data_dir) = matches.get_one::<String>("data-dir") {
        let level = matches.get_one::<String>("log-level").unwrap().clone();
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
        let auth_file = matches.get_one::<String>("auth-file").unwrap_or_else(|| {
            eprintln!("error: --auth-file is required in standalone mode");
            std::process::exit(2);
        });
        let tokens: StaticTokens = std::fs::read(auth_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| {
                eprintln!("error: cannot read --auth-file {auth_file}");
                std::process::exit(2);
            });
        let listen = matches.get_one::<String>("listen").cloned().unwrap_or_else(|| "127.0.0.1:4120".to_string());
        let fixtures = matches.get_flag("fixture-sources");
        let mut wish = WishConfig::default();
        if let Some(f) = matches.get_one::<String>("wish-config") {
            match std::fs::read(f).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
                Some(v) => wish.apply_json(&v),
                None => {
                    eprintln!("error: cannot read --wish-config {f}");
                    std::process::exit(2);
                }
            }
        }
        rt.block_on(run(PathBuf::from(data_dir), listen, Arc::new(tokens), Limits::default(), fixtures, None, wish));
    } else {
        init_logging("aiworkspace", true);
        rt.block_on(buckyos_service_main(matches.get_flag("fixture-sources")));
    }
}

async fn buckyos_service_main(fixtures: bool) {
    let mut runtime = match init_buckyos_api_runtime(AIWORKSPACE_SERVICE_NAME, None, BuckyOSRuntimeType::KernelService).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("aiworkspace init buckyos runtime failed: {:?}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = runtime.login().await {
        log::error!("aiworkspace login to system failed: {:?}", e);
        std::process::exit(1);
    }
    // the startup assertion is not accepted by other services (AICC): exchange it before serving
    if let Err(e) = runtime.renew_token_from_verify_hub().await {
        log::error!("aiworkspace exchange login assertion failed: {:?}", e);
        std::process::exit(1);
    }
    runtime.set_main_service_port(AIWORKSPACE_SERVICE_PORT).await;
    // a zone booted before this service existed has no settings key yet
    let settings: AiWorkspaceSettings = match runtime.get_my_settings().await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(e) => {
            log::warn!("load aiworkspace settings failed, using defaults: {:?}", e);
            AiWorkspaceSettings::default()
        }
    };
    // Workspace documents are user data: the service data folder survives a soft reset
    let data_dir = match runtime.get_data_folder() {
        Ok(d) => d,
        Err(e) => {
            log::error!("aiworkspace data folder unavailable: {:?}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = set_buckyos_api_runtime(runtime) {
        log::error!("register aiworkspace runtime failed: {:?}", e);
        std::process::exit(1);
    }
    let limits = Limits {
        max_commit_bytes: settings.max_commit_bytes as usize,
        max_wait_ms: settings.max_wait_ms,
        max_asset_bytes: settings.max_asset_bytes,
        ..Limits::default()
    };
    // wake-up hints over kevent: only "there is a new head", never content or entity ids
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Wake>();
    tokio::spawn(async move {
        while let Some(wake) = rx.recv().await {
            let Ok(runtime) = buckyos_api::get_buckyos_api_runtime() else { continue };
            let Ok(client) = runtime.get_kevent_client().await else { continue };
            let (path, data) = match wake {
                Wake::Head { workspace_id, epoch, head_seq } => {
                    (format!("/aiworkspace/{workspace_id}/head"), serde_json::json!({ "epoch": epoch, "head_seq": head_seq }))
                }
                Wake::Locks { workspace_id, counter } => (format!("/aiworkspace/{workspace_id}/locks"), serde_json::json!({ "counter": counter })),
            };
            if let Err(e) = client.pub_event(&path, data).await {
                log::debug!("kevent publish {path} failed: {:?}", e); // clients fall back to wait_changes
            }
        }
    });
    let listen = format!("127.0.0.1:{}", AIWORKSPACE_SERVICE_PORT);
    log::info!("aiworkspace buckyos mode: data in {}", data_dir.display());
    // models are reached through AICC with this service's own session
    let wish = WishConfig::from_settings(&settings.wish, serde_json::json!({ "type": "buckyos" }));
    run(data_dir, listen, Arc::new(RuntimeAuth), limits, fixtures, Some(tx), wish).await;
}

async fn run(data_dir: PathBuf, listen: String, auth: Arc<dyn Authenticator>, limits: Limits, fixtures: bool,
             wake: Option<tokio::sync::mpsc::UnboundedSender<Wake>>, wish: WishConfig) {
    let mut svc = match Service::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            log::error!("startup failed: {e}");
            eprintln!("startup failed: {e}");
            std::process::exit(1);
        }
    };
    if fixtures {
        svc.sources.register("fixture", Arc::new(GeneratedSource::default()));
    }
    if let Ok(spec) = std::env::var("AIWS_FAILPOINT") {
        svc.failpoint = FailPoint::parse(&spec);
        log::warn!("failpoint armed: {spec}");
    }
    let state = AppState::new(svc, auth, limits, wake, wish);
    if let Err(e) = aiworkspace_server::serve(state, &listen).await {
        log::error!("server error on {listen}: {e}");
        eprintln!("server error on {listen}: {e}");
        std::process::exit(1);
    }
}
