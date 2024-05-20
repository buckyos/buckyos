use std::collections::HashMap;
use clap::{Arg, Command, value_parser};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::process::Command as SystemCommand;
use bucky_name_service::{DnsTxtCodec, NameInfo, NSProvider};
use etcd_client::EtcdClient;

fn take_snapshot(file_path: &str) {
    println!("Taking snapshot and saving to {}", file_path);
    let status = SystemCommand::new("etcdctl")
        .args(["snapshot", "save", file_path])
        .status()
        .expect("Failed to execute etcdctl");

    if status.success() {
        println!("Snapshot successfully saved to {}", file_path);
    } else {
        eprintln!("Failed to take snapshot");
    }
}

async fn init(zone_id: &str, private_key_path: &str) -> Result<(), String> {
    // Perform initialization logic, such as saving these values to a config file
    // Here we just print them for demonstration purposes
    println!(
        "Initializing with zone_id: {} and private_key_path: {}",
        zone_id, private_key_path
    );

    // Save zone_id and private_key_path to a configuration file
    save_config(zone_id, private_key_path)?;

    Ok(())
}

async fn exec(command: &str) -> Result<(), String> {
    println!("Executing command: {}", command);
    // Here you would implement the logic to execute the command
    // For demonstration purposes, we just print the command
    // In real scenario, you might want to use SystemCommand to run it
    let output = SystemCommand::new(command)
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    println!("Command output: {:?}", output);

    Ok(())
}

async fn handle_matches(matches: clap::ArgMatches) -> std::result::Result<(), String> {
    if let Some(values) = matches.get_many::<String>("init") {
        let values: Vec<&String> = values.collect();
        let zone_id = values[0];
        let private_key_path = values[1];

        // Perform initialization logic, such as saving these values to a config file
        // Here we just print them for demonstration purposes
        println!(
            "Initializing with zone_id: {} and private_key_path: {}",
            zone_id, private_key_path
        );

        // Save zone_id and private_key_path to a configuration file
        save_config(zone_id, private_key_path)?;

        // Initialization done, we can exit here
        return Ok(());
    }

    // Check if the tool is initialized by attempting to load the configuration
    let (zone_id, private_key_path) = load_config().map_err(|e| {
        "Tool is not initialized. Please run `buckycli --init <zone_id> <private_key_path>`"
            .to_string()
    })?;

    // If a command is provided, execute it
    if let Some(command) = matches.get_one::<String>("command") {
        println!("Executing command: {}", command);
        // Here you would implement the logic to execute the command
        // For demonstration purposes, we just print the command
        // In real scenario, you might want to use SystemCommand to run it
        let output = SystemCommand::new(command)
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))?;

        println!("Command output: {:?}", output);
    }

    Ok(())
}

fn save_config(zone_id: &str, private_key_path: &str) -> std::result::Result<(), String> {
    let config = format!("{}\n{}", zone_id, private_key_path);
    fs::write("config.txt", config).map_err(|e| format!("Failed to save config: {}", e))
}

fn load_config() -> std::result::Result<(String, String), String> {
    let config =
        fs::read_to_string("config.txt").map_err(|e| format!("Failed to load config: {}", e))?;
    let lines: Vec<&str> = config.lines().collect();
    if lines.len() != 2 {
        return Err("Invalid config format".to_string());
    }
    Ok((lines[0].to_string(), lines[1].to_string()))
}

fn encode_file(file_path: &String, txt_limit: usize) -> Result<Vec<String>, String> {
    let mut file = File::open(file_path).map_err(|_e| {
        format!("Failed to open file: {}", file_path)
    })?;
    let mut contents = String::new();
    let read_len = file.read_to_string(&mut contents).map_err(|_e| {
        format!("Failed to read file: {}", file_path)
    })?;

    let content = match serde_json::from_str::<serde_json::Value>(&contents[..read_len]) {
        Ok(json) => {
            json.to_string()
        },
        Err(_) => {
            contents
        }
    };
    let list = DnsTxtCodec::encode(content.as_str(), txt_limit).map_err(|_e| {
        format!("Failed to encode text {}", content)
    })?;

    Ok(list)
}

async fn query(name: &str) -> Result<NameInfo, String> {
    let dns_provider = bucky_name_service::DNSProvider::new(None);

    let name_info = dns_provider.query(name).await.map_err(|_e| "Failed to query name".to_string())?;
    Ok(name_info)
}

async fn import_node_config(file_path: &str, etcd: &str) -> Result<(), String> {
    let file = tokio::fs::read(file_path).await.map_err(|_e| format!("Failed to read file: {}", file_path))?;
    let config: HashMap<String, serde_json::Value> = serde_json::from_slice(&file).map_err(|_e| "Failed to parse json".to_string())?;

    let etcd_client = EtcdClient::connect(etcd).await.map_err(|_e| "connect etcd error".to_string())?;
    for (key, value) in config {
        etcd_client.set(&format!("{}_node_config", key), &value.to_string()).await.map_err(|_e| "put etcd error".to_string())?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    let matches = Command::new("buckyos control tool")
        .version("0.1.0")
        .author("buckyos")
        .about("control tools")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("init")
            .about("Initialize with zone_id and private_key_path")
            .arg(Arg::new("param")
                .required(true)
                .num_args(2)
                .value_names(&["ZONE_ID", "PRIVATE_KEY_PATH"])
            )
        )
        .subcommand(Command::new("exec")
            .about("Execute a command")
            .disable_help_flag(true)
            .arg(Arg::new("command")
                .trailing_var_arg(true)
                .allow_hyphen_values(true)
                .num_args(..)
                .help("The command to execute")
                .required(true))
        )
        .subcommand(Command::new("import_zone_config")
            .about("Import the zone configuration")
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
        .subcommand(Command::new("encode_dns")
            .about("Encode the contents of a file into a DNS configurable record")
            .arg(Arg::new("file")
                .help("The file to encode")
                .required(true)
                .short('f')
                .long("file"))
            .arg(Arg::new("txt-limit")
                .help("The maximum length of a TXT record")
                .short('l')
                .long("limit")
                .value_parser(value_parser!(usize))
                .default_value("1024"))
        )
        .subcommand(Command::new("query_dns")
            .about("Query the dns configuration of the specified name")
            .arg(Arg::new("name")
                .help("The name of the service to be queried")
                .required(true)))
        // .arg(
        //     Arg::new("snapshot")
        //         .short('s')
        //         .long("snapshot")
        //         .help("Takes a snapshot of the etcd server"),
        // )
        // .arg(
        //     Arg::new("save")
        //         .short('f')
        //         .long("file")
        //         .help("Specifies the file path to save the snapshot"),
        // )
        .get_matches();

    match matches.subcommand() {
        Some(("init", matches)) => {
            let values: Vec<&String> = matches.get_many("param").unwrap().collect();
            let zone_id = values[0];
            let private_key_path = values[1];

            match init(zone_id, private_key_path).await {
                Ok(_) => {
                    println!("Initialization successful");
                },
                Err(e) => {
                    println!("{}", e);
                }
            }
        },
        Some(("exec", matches)) => {
            let command: Vec<_> = matches.get_many::<String>("command").unwrap().map(|v| v.to_string()).collect();
            match exec(command.join(" ").as_str()).await {
                Ok(_) => {
                    println!("Command executed successfully");
                },
                Err(e) => {
                    println!("{}", e);
                }
            }
        },
        Some(("encode_dns", encode_matches)) => {
            let file: &String = encode_matches.get_one("file").unwrap();
            let txt_limit: usize = *encode_matches.get_one("txt-limit").unwrap();
            match encode_file(file, txt_limit) {
                Ok(list) => {
                    for item in list {
                        println!("{}", item);
                    }
                },
                Err(e) => {
                    println!("{}", e);
                }
            }
        },
        Some(("query_dns", name_matches)) => {
            let name: &String = name_matches.get_one("name").unwrap();
            match query(name).await {
                Ok(name_info) => {
                    println!("{}", serde_json::to_string_pretty(&name_info).unwrap());
                },
                Err(e) => {
                    println!("{}", e);
                }
            }
        },
        Some(("import_zone_config", encode_matches)) => {
            let file: &String = encode_matches.get_one("file").unwrap();
            let etcd: &String = encode_matches.get_one("etcd").unwrap();
            if let Err(e) = import_node_config(file, etcd).await {
                println!("{}", e);
            }
        },
        _ => unreachable!(),
    }

    // let _ = handle_matches(matches).await?;

    Ok(())
}
