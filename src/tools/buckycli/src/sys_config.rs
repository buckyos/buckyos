use std::time::{SystemTime, UNIX_EPOCH};
use jsonwebtoken::EncodingKey;
use serde::Deserialize;
use buckyos_kit::get_buckyos_system_etc_dir;
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};
use sys_config::{SystemConfigClient};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    zone_name: String,// $name.buckyos.org or did:ens:$name
    owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner
    owner_name:String,//owner's name
    device_doc_jwt:String,//device document,jwt string,siged by owner
    zone_nonce:String,// random string, is default password of some service
    //device_private_key: ,storage in partical file
}

pub async fn connect_into(target_url:&str, node_id:&str) {
    // println!("connect to system config service");
    let file_path = get_buckyos_system_etc_dir().join(format!("{}_identity.toml", node_id));
    let contents = std::fs::read_to_string(file_path.clone()).unwrap();
    let node_identity: NodeIdentityConfig = toml::from_str(&contents).unwrap();
    let device_doc_json = decode_json_from_jwt_with_default_pk(&node_identity.device_doc_jwt, &node_identity.owner_public_key).unwrap();
    let device_doc : DeviceConfig = serde_json::from_value(device_doc_json).unwrap();

    let now = SystemTime::now();
    let since_the_epoch = now.duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let timestamp = since_the_epoch.as_secs();

    let private_key_path = get_buckyos_system_etc_dir().join(format!("{}_private_key.pem",node_id));
    let private_key = std::fs::read_to_string(private_key_path.clone()).unwrap();
    let device_private_key: EncodingKey = EncodingKey::from_ed_pem(private_key.as_bytes()).unwrap();

    // TODO improve
    let device_session_token = kRPC::RPCSessionToken {
        token_type : kRPC::RPCSessionTokenType::JWT,
        nonce : None,
        userid : Some(device_doc.name.clone()),
        appid:Some("kernel".to_string()),
        exp:Some(timestamp + 3600*24*7),
        iss:Some(device_doc.name.clone()),
        token:None,
    };
    let device_session_token_jwt = device_session_token.generate_jwt(Some(device_doc.did.clone()),&device_private_key).unwrap();
    let syc_cfg_client: SystemConfigClient = SystemConfigClient::new(Some(target_url), Some(device_session_token_jwt.as_str()));
    // handle error

    // ping test connection
    if let Err(e) = syc_cfg_client.get("boot/config").await {
        println!("connect system config service failed {}", e.to_string());
        return;
    }
    // println!("boot config: {:?}", boot_config_result);


    println!("connect to system_config_service success");
    // handle input
    let mut rl = DefaultEditor::new().unwrap();
    loop {
        // 读取用户输入
        let readline = rl.readline("sys_config> ");
        match readline {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                // 解析输入的命令
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                match parts[0] {
                    "get" => {
                        if parts.len() != 2 {
                            println!("用法: get <key>");
                            continue;
                        }
                        let key = parts[1];
                        let result = syc_cfg_client.get(key).await;
                        match result {
                            Ok(value) => {
                                println!("value:");
                                println!("{}", value.0);
                                println!("version:");
                                println!("{}", value.1);
                            }
                            Err(err) => println!("错误: {}", err),
                        }
                    }
                    "set" => {
                        if parts.len() != 3 {
                            println!("用法: set <key> <value>");
                            continue;
                        }
                        let key = parts[1];
                        let value = parts[2];
                        let result = syc_cfg_client.set(key, value).await;
                        match result {
                            Ok(version) => println!("{} 已设置为 {}, version: {}", key, value, version),
                            Err(err) => println!("错误: {}", err),
                        }
                    }
                    "exit" => {
                        println!("退出程序。");
                        break;
                    }
                    _ => {
                        println!("未知命令: {}", parts[0]);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("接收到中断信号，退出程序。");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("接收到 EOF，退出程序。");
                break;
            }
            Err(err) => {
                println!("读取输入时发生错误: {}", err);
                break;
            }
        }
    }
}