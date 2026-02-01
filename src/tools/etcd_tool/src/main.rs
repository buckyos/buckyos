use std::collections::HashMap;
use clap::{Arg, ArgMatches, Command, value_parser};
use buckyos_kit::get_version;
use etcd_client::EtcdClient;
use sfo_result::into_err as into_etcd_err;

#[derive(Eq, PartialEq, Copy, Clone, Default, Debug)]
enum EtcdErrorCode {
    #[default]
    Failed
}
type EtcdResult<T> = sfo_result::Result<T>;
type EtcdError = sfo_result::Error<EtcdErrorCode>;

#[tokio::main]
async fn main() {
    let matches = Command::new("etcd_tool")
        .about("A tool for etcd")
        .version(get_version())
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("import_node_config")
            .about("Import the node configuration")
            .arg(Arg::new("file")
                .help("The file to import")
                .required(true)
                .short('f')
                .long("file"))
            .arg(Arg::new("etcd")
                .help("The etcd server")
                .required(false)
                .short('e')
                .long("etcd")
                .default_value("http://127.0.0.1:2379"))
        )
        .subcommand(Command::new("get")
            .about("Import the node configuration")
            .arg(Arg::new("key")
                .help("The key to get")
                .required(true))
            .arg(Arg::new("etcd")
                .help("The etcd server")
                .required(false)
                .short('e')
                .long("etcd")
                .default_value("http://127.0.0.1:2379"))
        )
        .get_matches();

    match matches.subcommand() {
        Some(("import_node_config", encode_matches)) => {
            let file: &String = encode_matches.get_one("file").unwrap();
            let etcd: &String = encode_matches.get_one("etcd").unwrap();
            if let Err(e) = import_node_config(file, etcd).await {
                println!("Error: {}", e);
            }
        }
        Some(("get", encode_matches)) => {
            let key: &String = encode_matches.get_one("key").unwrap();
            let etcd: &String = encode_matches.get_one("etcd").unwrap();
            if let Err(e) = get_key(key, etcd).await {
                println!("Error: {}", e);
            }
        }
        _ => unreachable!(),
    }
}

async fn import_node_config(file: &str, etcd: &str) -> EtcdResult<()> {
    let file = tokio::fs::read(file).await?;
    let config: HashMap<String, serde_json::Value> = serde_json::from_slice(&file)?;

    let etcd_client = EtcdClient::connect(etcd).await.map_err(into_etcd_err!(EtcdErrorCode::Failed, "connect etcd error"))?;
    for (key, value) in config {
        let vision = etcd_client.set(&format!("{}_node_config", key), &value.to_string()).await.map_err(into_etcd_err!(EtcdErrorCode::Failed, "put etcd error"))?;
        println!("{}", vision);
    }

    Ok(())
}

async fn get_key(key: &str, etcd: &str) -> EtcdResult<()> {
    let etcd_client = EtcdClient::connect(etcd).await.map_err(into_etcd_err!(EtcdErrorCode::Failed, "connect etcd error"))?;
    let (value, revision) = etcd_client.get(key).await.map_err(into_etcd_err!(EtcdErrorCode::Failed, "get etcd error"))?;
    println!("{}: {}", revision, value);
    Ok(())
}
