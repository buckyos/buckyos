use clap::{Arg, Command};
use std::process::Command as SystemCommand;

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

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    let matches = Command::new("buckyos node control tool")
        .version("0.1.0")
        .author("buckyos")
        .about("node control tool")
        .arg(
            Arg::new("snapshot")
                .short('s')
                .long("snapshot")
                .help("Takes a snapshot of the etcd server"),
        )
        .arg(
            Arg::new("save")
                .short('f')
                .long("file")
                .help("Specifies the file path to save the snapshot"),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("snapshot", matches)) => {
            let file_path = matches.get_one::<String>("save").unwrap();
            take_snapshot(file_path);
        }
        _ => unreachable!("clap should ensure we don't get here"),
    };

    // if matches.is_present("snapshot") {
    //     let file_path = matches.value_of("save").unwrap_or("default_snapshot.db");
    //     take_snapshot(file_path);
    // } else {
    //     println!("No action requested, add -s to take a snapshot.");
    // }
    // Your code here
    Ok(())
}
