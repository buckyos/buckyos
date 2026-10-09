use buckyos_api::{
    init_buckyos_api_runtime, set_buckyos_api_runtime, zone_document_hostname, BuckyOSRuntimeType, HomeStationSettings, HOMESTATION_SERVICE_NAME,
    HOMESTATION_SERVICE_PORT,
};
use clap::{Arg, ArgAction, Command};
use homestation::auth::RuntimeAuth;
use homestation::contacts::{CachedContacts, MsgCenterContacts};
use homestation::db::Db;
use homestation::directory::NameDirectory;
use homestation::evaluation::{AiccModel, ModelClient};
use homestation::http::{serve, AppState};
use homestation::objects::NdmChunkStore;
use homestation::sign::Signer;
use homestation::{Station, StationConfig};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let matches = Command::new("homestation")
        .arg(Arg::new("data-dir").long("data-dir").help("Data directory (standalone mode)"))
        .arg(Arg::new("config").long("config").help("Standalone config JSON"))
        .arg(Arg::new("listen").long("listen").help("Listen address (standalone mode)"))
        .arg(Arg::new("keygen").long("keygen").action(ArgAction::SetTrue).help("Print a new ed25519 key (pem, x) and exit"))
        .arg(Arg::new("log-level").long("log-level").default_value("info"))
        .get_matches();
    if matches.get_flag("keygen") {
        let (pem, x) = homestation::sign::generate_key();
        println!("{}", serde_json::json!({ "private_key_pem": pem, "public_key_x": x, "did_dev": format!("did:dev:{x}") }));
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Some(data_dir) = matches.get_one::<String>("data-dir").cloned() {
        let level = matches.get_one::<String>("log-level").cloned().unwrap_or_else(|| "info".into());
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
        let Some(config) = matches.get_one::<String>("config") else {
            eprintln!("standalone mode needs --config");
            std::process::exit(2);
        };
        let config_path = PathBuf::from(config);
        let cfg: homestation::standalone::StandaloneConfig = match std::fs::read(&config_path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(c) => c,
            None => {
                eprintln!("cannot read config {}", config_path.display());
                std::process::exit(2);
            }
        };
        let workers = cfg.workers.unwrap_or(true);
        let listen_arg = matches.get_one::<String>("listen").cloned();
        rt.block_on(async move {
            let standalone = match homestation::standalone::build(cfg, &PathBuf::from(data_dir), config_path.parent(), None) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("startup failed: {e}");
                    std::process::exit(1);
                }
            };
            let listen = listen_arg.unwrap_or(standalone.listen.clone());
            if workers {
                standalone.station.start_workers();
            }
            if let Err(e) = serve(standalone.state, &listen).await {
                eprintln!("serve failed: {e}");
                std::process::exit(1);
            }
        });
    } else {
        buckyos_kit::init_logging("homestation", true);
        rt.block_on(service_main());
    }
}

/// BuckyOS service mode: kernel-service login, device key as the publisher's authorized
/// signer, Message Center contacts, the zone named store, gateway route `/kapi/homestation`
/// plus the zone-level `/home/` protocol paths.
async fn service_main() {
    let mut runtime = match init_buckyos_api_runtime(HOMESTATION_SERVICE_NAME, None, BuckyOSRuntimeType::KernelService).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("init runtime failed: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = runtime.login().await {
        log::error!("login failed: {e}");
        std::process::exit(1);
    }
    if let Err(e) = runtime.renew_token_from_verify_hub().await {
        log::error!("token exchange failed: {e}");
        std::process::exit(1);
    }
    runtime.set_main_service_port(HOMESTATION_SERVICE_PORT).await;
    if let Err(e) = runtime.load_device_private_key() {
        log::error!("device key unavailable: {e}");
        std::process::exit(1);
    }
    let settings: HomeStationSettings = match runtime.get_my_settings().await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(e) => {
            log::warn!("settings unavailable, using defaults: {e}");
            HomeStationSettings::default()
        }
    };
    let data_dir = match runtime.get_data_folder() {
        Ok(d) => d,
        Err(e) => {
            log::error!("data folder unavailable: {e}");
            std::process::exit(1);
        }
    };
    let zone_doc = match runtime.get_zone_config().map(|z| z.zone_document()) {
        Some(Ok(doc)) => doc,
        _ => {
            log::error!("zone document unavailable");
            std::process::exit(1);
        }
    };
    let zone = zone_document_hostname(&zone_doc).trim_end_matches('.').to_ascii_lowercase();
    let owner = zone_doc.owner.to_string();
    let owner_name = zone_doc.owner.id.split('.').next().unwrap_or("owner").to_string();
    let device_did = runtime.device_config.as_ref().map(|d| d.id.to_string()).unwrap_or_default();
    let signer = Signer::new(runtime.device_private_key.clone().expect("device key loaded"), format!("{device_did}#main_key"));
    let ndm = match runtime.get_named_store().await {
        Ok(n) => n,
        Err(e) => {
            log::error!("named store unavailable: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = set_buckyos_api_runtime(runtime) {
        log::error!("set runtime failed: {e}");
        std::process::exit(1);
    }
    let db = match Db::open(&data_dir.join("homestation.db")) {
        Ok(db) => db,
        Err(e) => {
            log::error!("database: {e}");
            std::process::exit(1);
        }
    };
    let mut cfg = StationConfig::new(&owner, &owner_name, &zone);
    cfg.collector = settings.collector;
    cfg.disclose_admission = settings.disclose_admission;
    cfg.spider_enabled = settings.spider;
    cfg.reading_window = settings.reading_window;
    let directory = NameDirectory {
        local_owner: owner.clone(),
        local_zone: zone.clone(),
        local_devices: vec![device_did],
        origins: settings.peers.clone(),
        zones: Default::default(),
    };
    let contacts = CachedContacts::new(Box::new(MsgCenterContacts { owner: owner.clone() }), Duration::from_secs(30));
    let model: Option<Arc<dyn ModelClient>> = settings.evaluation_model.clone().map(|m| Arc::new(AiccModel { logical_model: m }) as Arc<dyn ModelClient>);
    let station = Station::new(cfg, db, signer, Arc::new(directory), contacts, Arc::new(NdmChunkStore { ndm }), model);
    station.start_workers();
    let state = AppState { station, auth: Arc::new(RuntimeAuth::default()) };
    if let Err(e) = serve(state, &format!("127.0.0.1:{HOMESTATION_SERVICE_PORT}")).await {
        log::error!("serve failed: {e}");
        std::process::exit(1);
    }
}
