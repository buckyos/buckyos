#[allow(unused_mut, dead_code, unused_variables)]
extern crate core;
mod package_cmd;
mod util;
mod sys_config;
use buckyos_api::*;
use clap::{Arg, Command};
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig, CURRENT_DEVICE_CONFIG};
use package_cmd::*;
use util::*;

const CONFIG_FILE: &str = "~/.buckycli/config";


async fn load_buckyos_identity_config(node_id: &str) -> Result<DeviceConfig, String> {
    unimplemented!()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();

    let matches = Command::new("buckyos control tool")
        .author("buckyos")
        .about("control tools")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("id")
                .long("node_id")
                .help("This node's id")
                .required(false),
        )
        .subcommand(Command::new("version").about("buckycli version"))
        .subcommand(
            Command::new("pub_pkg")
                .about("publish package")
                .arg(
                    Arg::new("pkg_path")
                        .long("pkg_path")
                        .help("package path")
                        .required(true),
                )
                .arg(
                    Arg::new("pem")
                        .long("pem_file")
                        .help("pem file path")
                        .required(true),
                )
                .arg(Arg::new("url").long("url").help("repo url").required(true)),
        )
        .subcommand(
            Command::new("pub_app")
                .about("publish app")
                .arg(
                    Arg::new("app_path")
                        .long("app_path")
                        .help("app dir path")
                        .required(true),
                )
                .arg(
                    Arg::new("pem")
                        .long("pem_file")
                        .help("pem file path")
                        .required(true),
                )
                .arg(Arg::new("url").long("url").help("repo url").required(true)),
        )
        .subcommand(
            Command::new("pub_index")
                .about("publish index")
                .arg(
                    Arg::new("pem")
                        .long("pem_file")
                        .help("pem file path")
                        .required(true),
                )
                .arg(
                    Arg::new("version")
                        .long("version")
                        .help("index version")
                        .required(true),
                )
                .arg(
                    Arg::new("hostname")
                        .long("hostname")
                        .help("author hostname")
                        .required(true),
                )
                .arg(Arg::new("url").long("url").help("repo url").required(true)),
        )
        .subcommand(
            Command::new("pack_pkg").about("pack package").arg(
                Arg::new("pkg_path")
                    .long("pkg_path")
                    .help("package path")
                    .required(true),
            ),
        )
        .subcommand(
            Command::new("install_pkg")
                .about("install pkg")
                .arg(
                    Arg::new("pkg_name")
                        .long("pkg_name")
                        .help("pkg name")
                        .required(true),
                )
                .arg(
                    Arg::new("version")
                        .long("version")
                        .help("index version")
                        .required(true),
                )
                .arg(
                    Arg::new("dest_dir")
                        .long("dest_dir")
                        .help("dest dir")
                        .required(true),
                )
                .arg(
                    Arg::new("url")
                        .long("url")
                        .help("local repo url")
                        .required(true),
                ),
        )
        .subcommand(
            Command::new("connect")
                .about("connect system config as a client")
                .arg(
                    Arg::new("target_url")
                        .long("target_url")
                        .help("system config service url, default 'http://127.0.0.1:3200/kapi/system_config' ")
                )
                .arg(
                    Arg::new("node_id")
                        .long("node_id")
                        .help("node_id in current machine, default 'node'")
                )
        )
        .get_matches();

    init_global_buckyos_value_by_load_identity_config(BuckyOSRuntimeType::AppClient).await.map_err(|e| {
        let err_msg = format!("init_global_buckyos_value_by_load_identity_configfailed! {}", e);
        println!("{}", err_msg.as_str());
        err_msg
    })?;

    init_buckyos_api_runtime("buckyos-cli",None,BuckyOSRuntimeType::AppClient).await.map_err(|e| {
        let err_msg = format!("init buckyos api runtime failed! {}", e);
        println!("{}", err_msg.as_str());
        err_msg
    })?;

    let buckyos_runtime = get_buckyos_api_runtime().unwrap();
    println!("Connect to {:?} @ {:?}",buckyos_runtime.owner_user_id,buckyos_runtime.zone_config);


    match matches.subcommand() {
        Some(("version", _)) => {
            let version = option_env!("CARGO_PKG_VERSION").unwrap_or("unknown");
            // let git_hash = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
            println!("Build Timestamp: {}", env!("VERGEN_BUILD_TIMESTAMP"));
            println!(
                "buckyos control tool version {} {}",
                version,
                env!("VERGEN_GIT_DESCRIBE")
            );
        }
        Some(("pub_pkg", matches)) => {
            unimplemented!()
        }
        Some(("pub_app", matches)) => {
            unimplemented!()
        }
        Some(("repo_publish", matches)) => {
            
        }
        Some(("pack_pkg", matches)) => {
            let pkg_path = matches.get_one::<String>("pkg_path").unwrap();
            match pack_dapp_pkg(pkg_path).await {
                Ok(_) => {
                    println!("############\nPack package success!");
                }
                Err(e) => {
                    println!("############\nPack package failed! {}", e);
                    return Err("pack package failed!".to_string());
                }
            }
        }
        Some(("install_pkg", matches)) => {
            let pkg_name = matches.get_one::<String>("pkg_name").unwrap();
            let version = matches.get_one::<String>("version").unwrap();
            let dest_dir = matches.get_one::<String>("dest_dir").unwrap();
            let url = matches.get_one::<String>("url").unwrap();
            match install_pkg(pkg_name, version, dest_dir, url).await {
                Ok(_) => {
                    println!("############\nInstall package success!");
                }
                Err(e) => {
                    println!("############\nInstall package failed! {}", e);
                    return Err("install package failed!".to_string());
                }
            }
        }
        Some(("repo_publish", matches)) => {
            unimplemented!()
        }
        Some(("connect", matches)) => {

            sys_config::connect_into().await;
        }
        _ => {
            println!("unknown command!");
            return Err("unknown command!".to_string());
        }
    }

    // let _ = handle_matches(matches).await?;

    Ok(())
}

