#[allow(unused_mut, dead_code, unused_variables)]
mod package_cmd;
mod sys_config;
mod did;
mod app;
mod ndn;

use std::path::Path;
use buckyos_api::*;
use clap::{Arg, Command};
use package_cmd::*;


fn is_local_cmd(cmd_name: &str) -> bool {
    const LOCAL_COMMANDS: &[&str] = &[
        "version",
        "install_pkg", 
        "pack_pkg",
        "load_pkg",
        "set_pkg_meta",
        "did",
        "create_chunk"
    ];
    LOCAL_COMMANDS.contains(&cmd_name)
}

#[tokio::main]
async fn main() -> Result<(), String> {
    buckyos_kit::init_logging("buckycli", false);

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
                        .long("target_dir")
                        .help("app output dir,contain app doc and app sub pkgs")
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
                    .help("source package path,which dir contain pkg_meta.json")
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
                .about("install pkg in current pkg env or target pkg env")
                .arg(
                    Arg::new("pkg_id")
                        .index(1)
                        .long("pkg_id")
                        .help("pkg id is pkg name with version")
                        .required(true),
                        
                )
                .arg(
                    Arg::new("env")
                        .long("env")
                        .help("target env path, default is current dir")
                        .required(false),
                )
        )
        .subcommand(
            Command::new("load_pkg")
                .about("try load pkg in current pkg env,will return pkg media info")
                .arg(
                    Arg::new("pkg_id")
                        .index(1)
                        .long("pkg_id")
                        .help("pkg id is pkg name with version")
                        .required(true),
                )
                .arg(
                    Arg::new("env")
                        .long("env")
                        .help("target env path, default is current dir")
                        .required(false),
                )
        )
        .subcommand(
            Command::new("set_pkg_meta")
                .about("set(add or update) pkg meta to meta-index-db")
                .arg(
                    Arg::new("meta_path")
                        .index(1)
                        .long("meta_path")
                        .help("meta path")
                        .required(true),
                )
                .arg(
                    Arg::new("db_path")
                        .index(2)
                        .long("db_path")
                        .help("db path")
                        .required(true),
                )
        )
        .subcommand(
            Command::new("update_index")
                .about("update zone repo service's meta-index-db from remote source")
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
        .subcommand(
            Command::new("sys_config")
               .about("Quick interaction mode for system config")
               .arg(
                    Arg::new("get")
                       .long("get")
                       .value_name("key")
                       .help("get system config, buckycli sys_config --get $key")
                )
                .arg(
                    Arg::new("set")
                        .long("set")
                        .value_names(&["key", "value"])  // 定义两个占位符名称
                        .num_args(2)
                        .help("set system config,
    buckycli sys_config --set $key $value")
                )
                .arg(
                    Arg::new("list")
                      .long("list")
                      .value_name("key")
                        .help("get system config, buckycli sys_config --list [$key]")
                )
                .arg(
                    Arg::new("set_file")
                        .long("set_file")
                        .value_names(&["key", "$filename"])  // 定义两个占位符名称
                        .num_args(2)
                        .help("set system config with file content. filename = file path.
    buckycli sys_config --set_file $key $filename")
                )
                .arg(
                    Arg::new("append")
                        .long("append")
                        .value_names(&["key", "value"])  // 定义两个占位符名称
                        .num_args(2)
                        .help("append system config,
    buckycli sys_config --append $key $value")
                )
        )
        .subcommand(
            Command::new("did")
                .about("did manager")
                .subcommand(Command::new("genkey").about("generate a  pair of did key"))
                .arg(
                    Arg::new("open")
                      .long("open")
                      .value_name("filepath")
                      .help("Open config file and display")
                )
                .arg(
                    Arg::new("create_user")
                      .long("create_user")
                      .value_names(&["name", "owner_jwk"])  // 定义两个占位符名称
                      .num_args(2)
                      .help("Create the user_config.json file in current dir
owner_jwk look like this '{\"crv\":\"Ed25519\",\"kty\":\"OKP\",\"x\":\"14pk3c3XO9_xro5S6vSr_Tvq5eTXbFY8Mop-Vj1D0z8\"}'")
                )
                .arg(
                    Arg::new("create_device")
                      .long("create_device")
                      .value_names(&["user_name", "zone_name", "owner_jwk", "user_private_key"]) 
                      .num_args(4)
                      .help("create a device (deviceconfig).
The arg `user_private_key` is a file path")
                )
                .arg(
                    Arg::new("create_zoneboot")
                      .long("create_zoneboot")
                      .value_names(&["oods", "sn_host"])
                      .num_args(2)
                      .help("create a zone_boot_config.
oods look like this 'ood1,ood2'.")
                )
                .arg(
                    Arg::new("create_zone")
                      .long("create_zone")
                      .help("create zone config")
                )
        )
        .subcommand(
            Command::new("sign")
                .about("sign any data")
                .arg(
                    Arg::new("json")
                    .long("json")
                    .value_name("data")
                    .help("sign any given json data and return the JWT content")
                    .required(true)
                )
        )
        .subcommand(
            Command::new("app")
                .about("App controller")
                .arg(
                        Arg::new("create")
                        .long("create")
                        .value_name("mata_file")
                        .help("Quickly create an app, buckycli app --create $meta_file")
                )
        )
        .subcommand(
            Command::new("create_chunk")
                .about("ndn operator")
                .arg(
                    Arg::new("create")
                    .value_name("filepath")
                    .help("crate ndn chunk by filepath")
                )
                .arg(
                    Arg::new("target")
                    .value_name("target ndn data dir")
                    .help("chunk will store at target ndn data dir")
                )
        )
        .get_matches();

    let mut private_key = None;
    let subcommand = matches.subcommand();

    let cmd_name = subcommand.clone().unwrap().0;
    let mut runtime = init_buckyos_api_runtime("buckycli",None,BuckyOSRuntimeType::AppClient).await.map_err(|e| {
        println!("Failed to init buckyos runtime: {}", e);
        return e.to_string();
    })?;
    if !is_local_cmd(cmd_name) {
        runtime.login().await.map_err(|e| {
            println!("Failed to login: {}", e);
            return e.to_string();
        })?;
        set_buckyos_api_runtime(runtime);
        let buckyos_runtime = get_buckyos_api_runtime().unwrap();
        let zone_host_name = buckyos_runtime.zone_id.to_host_name();
        println!("Connect to {:?} @ {:?}",buckyos_runtime.user_id,zone_host_name);

    } else {
        if runtime.user_private_key.is_some() {
            println!("Warning: You are using a developer private key, please make sure you are on a secure development machine!!!");
            private_key = Some((runtime.user_id.clone().unwrap(),runtime.user_private_key.clone().unwrap()));
        }
        set_buckyos_api_runtime(runtime);
    }

    // 处理子命令
    match subcommand {
        Some(("version", _)) => {
            let version = option_env!("CARGO_PKG_VERSION").unwrap_or("unknown");
            let git_hash = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
            println!("Build Timestamp: {}", option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown"));
            println!(
                "buckyos control tool version {} {}",
                version,
                git_hash,
            );
        }
        Some(("pub_pkg", matches)) => {
            let target_dir = matches.get_one::<String>("target_dir").unwrap();
            //需要便利target_dir目录下的所有pkg，并发布
            // 遍历target_dir目录下的所有pkg目录
            let mut pkg_path_list = Vec::new();
            let target_path = Path::new(target_dir);
            
            if !target_path.exists() || !target_path.is_dir() {
                return Err(format!("目标目录 {} 不存在或不是一个目录", target_dir));
            }
            
            // 读取目录下的所有条目
            let entries = std::fs::read_dir(target_path).map_err(|e| {
                format!("读取目录 {} 失败: {}", target_dir, e.to_string())
            })?;
            
            // 遍历所有条目，找出所有目录
            for entry in entries {
                let entry = entry.map_err(|e| {
                    format!("读取目录条目失败: {}", e.to_string())
                })?;
                
                let path = entry.path();
                if path.is_dir() {
                    // 检查是否包含pkg_meta.jwt文件，这表明它是一个有效的包目录
                    let pkg_meta_jwt_path = path.join("pkg_meta.jwt");
                    if pkg_meta_jwt_path.exists() {
                        println!("找到有效的packed pkg目录: {}", path.display());
                        pkg_path_list.push(path);
                    }
                }
            }
            
            if pkg_path_list.is_empty() {
                return Err(format!("在目录 {} 中没有找到有效的包", target_dir));
            }
            
            println!("找到 {} 个包准备发布", pkg_path_list.len());
            let pub_result = publish_raw_pkg(&pkg_path_list).await;
            if pub_result.is_err() {
                println!("Publish pkg failed! {}", pub_result.err().unwrap());
                return Err("publish pkg failed!".to_string());
            }
            println!("############\nPublish pkg success!");
        }
        Some(("pub_app", matches)) => {
            let app_name = matches.get_one::<String>("app_name").unwrap();
            let app_dir_path = matches.get_one::<String>("target_dir").unwrap();
            let pub_result = publish_app_pkg(app_name, app_dir_path,true).await;
            if pub_result.is_err() {
                println!("Publish app failed! {}", pub_result.err().unwrap());
                return Err("publish app failed!".to_string());
            }
        }
        Some(("pack_pkg", matches)) => {
            let src_pkg_path = matches.get_one::<String>("src_pkg_path").unwrap();
            let target_path = matches.get_one::<String>("target_path").unwrap();

            match pack_raw_pkg(src_pkg_path, target_path,private_key.clone()).await {
                Ok(_) => {
                    println!("############\nPack package success!");
                }
                Err(e) => {
                    println!("############\nPack package failed! {}", e);
                    return Err("pack package failed!".to_string());
                }
            }
        }
        Some(("set_pkg_meta", matches)) => {
            let meta_path = matches.get_one::<String>("meta_path").unwrap();
            let db_path = matches.get_one::<String>("db_path").unwrap();
            let set_result = set_pkg_meta(meta_path, db_path).await;
            if set_result.is_err() {
                println!("Set pkg meta failed! {}", set_result.err().unwrap());
                return Err("set pkg meta failed!".to_string());
            }
            println!("############\nSet pkg meta {} to db {} success!", meta_path, db_path);
        }
        
        Some(("install_pkg", matches)) => {
            let pkg_id = matches.get_one::<String>("pkg_id").unwrap();
            let target_env = matches.get_one::<String>("env");
            let real_target_env:String = if target_env.is_some() {
                target_env.unwrap().to_string()
            } else {
                // 获取当前目录作为默认环境
                std::env::current_dir()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|_| ".".to_string())
            };

            println!("start install pkg: {} to target env: {}", pkg_id, real_target_env.as_str());
            
            match install_pkg(pkg_id, real_target_env.as_str()).await {
                Ok(_) => {
                    println!("############\nInstall package success!");
                }
                Err(e) => {
                    println!("############\nInstall package failed! {}", e);
                    return Err("install package failed!".to_string());
                }
            }
        }
        Some(("load_pkg", matches)) => {
            let pkg_id = matches.get_one::<String>("pkg_id").unwrap();
            let target_env = matches.get_one::<String>("env");
            let real_target_env:String = if target_env.is_some() {
                target_env.unwrap().to_string()
            } else {
                // 获取当前目录作为默认环境
                std::env::current_dir()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|_| ".".to_string())
            };

            println!("start load pkg: {} to target env: {}", pkg_id, real_target_env.as_str());
            let load_result = load_pkg(pkg_id, real_target_env.as_str()).await;
            if load_result.is_err() {
                println!("Load package failed! {}", load_result.err().unwrap());
            }
        }
        Some(("pub_index", _matches)) => {
            let pub_result = publish_repo_index().await;
            if pub_result.is_err() {
                println!("Publish repo index failed! {}", pub_result.err().unwrap());
                return Err("publish repo index failed!".to_string());
            }
            println!("############\nPublish repo index success!");
        }
        Some(("update_index", _matches)) => {
            let sync_result = sync_from_remote_source().await;
            if sync_result.is_err() {
                println!("Sync from remote source failed! {}", sync_result.err().unwrap());
                return Err("sync from remote source failed!".to_string());
            }
        }
        Some(("sys_config", matches)) => {
            if let Some(key) = matches.get_one::<String>("get") {
                // println!("Get system config, key[{}]", key);
                sys_config::get_config(key).await;
                return Ok(());
            }

            if let Some(_key) = matches.get_one::<String>("set") {
                let config_values: Vec<&String> = matches
                    .get_many::<String>("set")
                    .expect("必须提供 key 和 value 参数")
                    .collect();
                let key = config_values[0];
                let value = config_values[1];
                println!("Set system config, key[{}]: {}", key, value);
                sys_config::set_config(key, value).await;
                return Ok(());
            }
            if let Some(key) = matches.get_one::<String>("list") {
                // println!("List system config, key[{}]", key);
                sys_config::list_config(key).await;
                return Ok(());
            }
            if let Some(_key) = matches.get_one::<String>("append") {
                let config_values: Vec<&String> = matches
                    .get_many::<String>("append")
                    .expect("必须提供 key 和 value 参数")
                    .collect();
                let key = config_values[0];
                let value = config_values[1];
                println!("Append system config, key[{}]: {}", key, value);
                sys_config::append_config(key, value).await;
                return Ok(());
            }
            if let Some(_key) = matches.get_one::<String>("set_file") {
                let config_values: Vec<&String> = matches
                    .get_many::<String>("set_file")
                    .expect("必须提供 key 和 file 参数")
                    .collect();
                let key = config_values[0];
                let filepath = config_values[1];
                let content = std::fs::read_to_string(filepath)
                    .unwrap_or_else(|_| panic!("无法读取文件: {}", filepath));
                sys_config::set_config(key, &content).await;
                return Ok(());
            }
        }
        Some(("connect", _matches)) => {
            sys_config::connect_into().await;
        }
        Some(("sign", matches)) => {
            did::sign_json_data(matches, private_key.clone()).await;
        }
        Some(("did", sub_matches)) => {
            did::did_matches(sub_matches);
        }
        Some(("app", matches)) => {
            if let Some(meta_file) = matches.get_one::<String>("create") {
                app::create_app(meta_file).await;
                return Ok(());
            }
        }
        Some(("create_chunk", matches)) => {
            if let Some(filepath) = matches.get_one::<String>("create") {
                if let Some(target) = matches.get_one::<String>("target") {
                    ndn::create_ndn_chunk(filepath,target).await;
                    return Ok(());
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

