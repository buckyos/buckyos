extern crate core;
mod package_cmd;
use buckyos_kit::get_buckyos_system_etc_dir;
use clap::{value_parser, Arg, ArgMatches, Command};
use jsonwebtoken::EncodingKey;
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};
use package_cmd::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command as SystemCommand;
use std::time::{SystemTime, UNIX_EPOCH};

const CONFIG_FILE: &str = "~/.buckycli/config";

#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    zone_name: String,                        // $name.buckyos.org or did:ens:$name
    owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner
    owner_name: String,                       //owner's name
    device_doc_jwt: String,                   //device document,jwt string,siged by owner
    zone_nonce: String,                       // random string, is default password of some service
                                              //device_private_key: ,storage in partical file
}

fn load_identity_config(node_id: &str) -> Result<NodeIdentityConfig, String> {
    //load ./node_identity.toml for debug
    //load from /opt/buckyos/etc/node_identity.toml
    let mut file_path = PathBuf::from(format!("{}_identity.toml", node_id));
    let path = Path::new(&file_path);
    if !path.exists() {
        let etc_dir = get_buckyos_system_etc_dir();
        file_path = etc_dir.join(format!("{}_identity.toml", node_id));
    }

    let contents = std::fs::read_to_string(file_path.clone())
        .map_err(|err| format!("read node identity config failed! {}", err))?;

    let config: NodeIdentityConfig = toml::from_str(&contents)
        .map_err(|err| format!("Failed to parse NodeIdentityConfig TOML: {}", err))?;

    Ok(config)
}

fn load_device_private_key(node_id: &str) -> Result<EncodingKey, String> {
    let mut file_path = format!("{}_private_key.pem", node_id);
    let path = Path::new(file_path.as_str());
    if !path.exists() {
        let etc_dir = get_buckyos_system_etc_dir();
        file_path = format!("{}/{}_private_key.pem", etc_dir.to_string_lossy(), node_id);
    }
    let private_key = std::fs::read_to_string(file_path.clone())
        .map_err(|err| format!("read device private key failed! {}", err))?;

    let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key.as_bytes())
        .map_err(|err| format!("parse device private key failed! {}", err))?;

    Ok(private_key)
}

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
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
        .subcommand(Command::new("create_token").about("Create device session token"))
        .subcommand(Command::new("version").about("buckyos version"))
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

    let default_node_id = "node".to_string();
    let node_id = matches.get_one::<String>("id").unwrap_or(&default_node_id);
    let node_config = match load_identity_config(node_id.as_ref()) {
        Ok(node_config) => node_config,
        Err(e) => {
            println!("{}", e);
            return Err(e);
        }
    };

    let device_private_key = match load_device_private_key(node_id.as_str()) {
        Ok(device_private_key) => device_private_key,
        Err(e) => {
            println!("{}", e);
            return Err(e);
        }
    };

    let device_doc_json = match decode_json_from_jwt_with_default_pk(
        &node_config.device_doc_jwt,
        &node_config.owner_public_key,
    ) {
        Ok(device_doc_json) => device_doc_json,
        Err(e) => {
            println!("decode device doc from jwt failed!");
            return Err("decode device doc from jwt failed!".to_string());
        }
    };

    let device_doc: DeviceConfig = match serde_json::from_value(device_doc_json) {
        Ok(device_doc) => device_doc,
        Err(e) => {
            println!("parse device doc failed! {}", e);
            return Err("parse device doc failed!".to_string());
        }
    };

    match matches.subcommand() {
        Some(("create_token", matches)) => {
            let now = SystemTime::now();
            let since_the_epoch = now.duration_since(UNIX_EPOCH).expect("Time went backwards");
            let timestamp = since_the_epoch.as_secs();
            let device_session_token = kRPC::RPCSessionToken {
                token_type: kRPC::RPCSessionTokenType::JWT,
                nonce: None,
                userid: Some(device_doc.name.clone()),
                appid: Some("kernel".to_string()),
                exp: Some(timestamp + 3600 * 24 * 7),
                iss: Some(device_doc.name.clone()),
                token: None,
            };

            let device_session_token_jwt = device_session_token
                .generate_jwt(Some(device_doc.did.clone()), &device_private_key)
                .map_err(|err| {
                    println!("generate device session token failed! {}", err);
                    return String::from("generate device session token failed!");
                })?;
            println!("{}", device_session_token_jwt)
        }
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
            let now = SystemTime::now();
            let since_the_epoch = now.duration_since(UNIX_EPOCH).expect("Time went backwards");
            let timestamp = since_the_epoch.as_secs();
            let device_session_token = kRPC::RPCSessionToken {
                token_type: kRPC::RPCSessionTokenType::JWT,
                nonce: None,
                userid: Some(device_doc.name.clone()),
                appid: Some("kernel".to_string()),
                exp: Some(timestamp + 3600 * 24 * 7),
                iss: Some(device_doc.name.clone()),
                token: None,
            };

            let device_session_token_jwt = device_session_token
                .generate_jwt(Some(device_doc.did.clone()), &device_private_key)
                .map_err(|err| {
                    println!("generate device session token failed! {}", err);
                    return String::from("generate device session token failed!");
                })?;
            //从args中取出参数
            let pkg_path = matches.get_one::<String>("pkg_path").unwrap();
            let pem_file = matches.get_one::<String>("pem").unwrap();
            let url = matches.get_one::<String>("url").unwrap();
            match publish_package(pkg_path, pem_file, url, &device_session_token_jwt).await {
                Ok(_) => {
                    println!("publish package success!");
                }
                Err(e) => {
                    println!("publish package failed! {}", e);
                    return Err("publish package failed!".to_string());
                }
            }
        }
        Some(("pub_index", matches)) => {
            let now = SystemTime::now();
            let since_the_epoch = now.duration_since(UNIX_EPOCH).expect("Time went backwards");
            let timestamp = since_the_epoch.as_secs();
            let device_session_token = kRPC::RPCSessionToken {
                token_type: kRPC::RPCSessionTokenType::JWT,
                nonce: None,
                userid: Some(device_doc.name.clone()),
                appid: Some("kernel".to_string()),
                exp: Some(timestamp + 3600 * 24 * 7),
                iss: Some(device_doc.name.clone()),
                token: None,
            };

            let device_session_token_jwt = device_session_token
                .generate_jwt(Some(device_doc.did.clone()), &device_private_key)
                .map_err(|err| {
                    println!("generate device session token failed! {}", err);
                    return String::from("generate device session token failed!");
                })?;
            //从args中取出参数
            let pem_file = matches.get_one::<String>("pem").unwrap();
            let version = matches.get_one::<String>("version").unwrap();
            let url = matches.get_one::<String>("url").unwrap();
            match publish_index(pem_file, version, url, &device_session_token_jwt).await {
                Ok(_) => {
                    println!("publish index success!");
                }
                Err(e) => {
                    println!("publish index failed! {}", e);
                    return Err("publish index failed!".to_string());
                }
            }
        }
        Some(("pack_pkg", matches)) => {
            let pkg_path = matches.get_one::<String>("pkg_path").unwrap();
            match pack(pkg_path).await {
                Ok(_) => {
                    println!("pack package success!");
                }
                Err(e) => {
                    println!("pack package failed! {}", e);
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
                    println!("install package success!");
                }
                Err(e) => {
                    println!("install package failed! {}", e);
                    return Err("install package failed!".to_string());
                }
            }
        }
        _ => {
            println!("unknown command!");
            return Err("unknown command!".to_string());
        }
    }

    // let _ = handle_matches(matches).await?;

    Ok(())
}
