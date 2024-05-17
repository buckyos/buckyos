use clap::{Arg, Command};
use std::fs;
use std::process::Command as SystemCommand;
use tokio::main;

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

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    let matches = Command::new("buckyos control tool")
        .version("0.1.0")
        .author("buckyos")
        .about("control tools")
        .arg(
            Arg::new("init")
                .long("init")
                .num_args(2)
                .value_names(&["ZONE_ID", "PRIVATE_KEY_PATH"])
                .help("Initialize with zone_id and private_key_path"),
        )
        .arg(
            Arg::new("command")
                .help("The command to execute")
                .required(false)
                .index(1),
        )
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

    let _ = handle_matches(matches).await?;

    Ok(())
}
