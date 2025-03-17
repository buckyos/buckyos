#[allow(unused_mut, dead_code, unused_variables)]
extern crate core;
mod package_cmd;
mod util;
mod sys_config;
use buckyos_api::*;
use clap::{Arg, Command};
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig, CURRENT_DEVICE_CONFIG};
use package_cmd::*;
use crate::package_cmd::*;



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
                .about("publish packed raw package to local repo")
                .arg(
                    Arg::new("target_dir")
                        .long("target_dir")
                        .help("target dir,which contain packed raw package")
                        .required(true)
                        .index(1)
                )
        )
        .subcommand(
            Command::new("pub_app")
                .about("update app doc and publish app to local repo")
                .arg(
                    Arg::new("app_name")
                        .long("app_name")
                        .help("app name")
                        .required(true),
                )
                .arg(
                    Arg::new("target_dir")
                        .long("app_path")
                        .help("app dir path")
                        .required(true),
                )
        )
        .subcommand(
            Command::new("pub_index")
                .about("let local repo publish wait-pub-pkg_meta_index database")
        )
        .subcommand(
            Command::new("pack_pkg").about("pack package").arg(
                Arg::new("src_pkg_path")
                    .index(1)
                    .long("src_pkg_path")
                    .help("source package path,which dir contain .pkg_meta.json")
                    .required(true)
            ).arg(
                Arg::new("target_path")
                    .index(2)
                    .long("target path")
                    .help("packed package will store at /target_path/pkg_name/")
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

    init_buckyos_api_by_load_config("buckyos-cli",BuckyOSRuntimeType::AppClient).await.map_err(|e| {
        let err_msg = format!("init_global_buckyos_value_by_load_identity_configfailed! {}", e);
        println!("{}", err_msg.as_str());
        err_msg
    })?;

    //TODO: Support login to verify-hub via command line to obtain a valid session_token, to avoid requiring a private key locally


    let buckyos_runtime = get_buckyos_api_runtime().unwrap();
    let _session_token = buckyos_runtime.generate_session_token().await.map_err(|e| {
        println!("Failed to get session token: {}", e);
        return e.to_string();
    })?;
    let mut private_key = None;
    println!("Connect to {:?} @ {:?}",buckyos_runtime.user_did,buckyos_runtime.zone_config.name);
    if buckyos_runtime.user_private_key.is_some() {
        println!("Warning: You are using a developer private key, please make sure you are on a secure development machine!!!");
        private_key = Some((buckyos_runtime.user_did.as_deref().unwrap(),buckyos_runtime.user_private_key.as_ref().unwrap()));
    }

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
            let app_name = matches.get_one::<String>("app_name").unwrap();
            let app_dir_path = matches.get_one::<String>("target_dir").unwrap();
            let pub_result = publish_app_pkg(app_name, app_dir_path,false).await;
            if pub_result.is_err() {
                println!("Publish app failed! {}", pub_result.err().unwrap());
                return Err("publish app failed!".to_string());
            }
        }
        Some(("pack_pkg", matches)) => {
            let src_pkg_path = matches.get_one::<String>("src_pkg_path").unwrap();
            let target_path = matches.get_one::<String>("target_path").unwrap();

            match pack_raw_pkg(src_pkg_path, target_path,private_key).await {
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
        Some(("pub_index", _matches)) => {
            let pub_result =publish_repo_index().await;
            if pub_result.is_err() {
                println!("Publish repo index failed! {}", pub_result.err().unwrap());
                return Err("publish repo index failed!".to_string());
            }
            println!("############\nPublish repo index success!");
        }
        Some(("connect", _matches)) => {
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

