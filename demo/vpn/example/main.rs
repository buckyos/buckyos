use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use flexi_logger::{Cleanup, Criterion, DeferredNow, Duplicate, FileSpec, Naming};
use log::Record;
use serde::{Deserialize, Serialize};
use vpn::{VpnClient, VpnServer};

#[derive(Serialize, Deserialize)]
struct ClientConfig {
    pub server: String,
    pub client_key: String,
}

#[derive(Serialize, Deserialize)]
struct Client {
    pub client_key: String,
    pub ip: String,
}

#[derive(Serialize, Deserialize)]
struct ServerConfig {
    pub ip: String,
    pub port: u16,
    pub clients: Vec<Client>,
}

#[derive(Serialize, Deserialize)]
struct Config {
    pub server: Option<ServerConfig>,
    pub client: Option<ClientConfig>,
}

fn custom_format(writer: &mut dyn std::io::Write, now: &mut DeferredNow, record: &Record) -> std::io::Result<()> {
    let file = match record.file() {
        None => {
            "<unknown>".to_string()
        }
        Some(path) => {
            Path::new(path).file_name().map(|v| v.to_string_lossy().to_string()).unwrap_or("<unknown>".to_string())
        }
    };
    write!(
        writer,
        "{} [{}] [{}:{}] [{}] - {}",
        now.format("%Y-%m-%d %H:%M:%S"),
        record.level(),
        file,
        record.line().unwrap_or(0),
        thread::current().name().unwrap_or("<unnamed>"),
        &record.args()
    )
}


#[tokio::main]
async fn main() {
    flexi_logger::Logger::try_with_str("debug")
        .unwrap()
        .log_to_file(FileSpec::default().directory(std::env::current_dir().unwrap().join("logs")))
        .duplicate_to_stderr(Duplicate::All)
        .rotate(Criterion::Size(10 * 1024 * 1024), // 文件大小达到 10MB 时轮转
                Naming::Numbers, // 使用数字命名轮转文件
                Cleanup::KeepLogFiles(7), // 保留最近 7 个日志文件
        ).format(custom_format)
        .start().unwrap();

    let matches = clap::Command::new("vpn")
        .arg(clap::Arg::new("config").short('c').long("config").required(false)
            .default_value("./")
            .help("config path")).get_matches();

    let data_folder = matches.get_one::<String>("config").unwrap();

    let config = std::fs::read_to_string(Path::new(data_folder.as_str()).join("config.json")).unwrap();
    let config: Config = serde_json::from_str(config.as_str()).unwrap();

    if config.server.is_some() {
        let server_config = config.server.as_ref().unwrap();
        let mut client_map = HashMap::new();
        for client in server_config.clients.iter() {
            client_map.insert(client.client_key.clone(), client.ip.clone());
        }
        let server = Arc::new(VpnServer::bind((server_config.ip.as_str(), server_config.port), client_map).await.unwrap());
        server.start().await;
    }

    if config.client.is_some() {
        let client_config = config.client.as_ref().unwrap();
        let client = VpnClient::new(client_config.server.as_str(), client_config.client_key.as_str());
        client.start().await;
    }
    std::future::pending::<()>().await;
}
