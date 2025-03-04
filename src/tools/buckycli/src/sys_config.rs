use jsonwebtoken::EncodingKey;
use name_lib::{DeviceConfig};
use sys_config::{SystemConfigClient};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use crate::util::{get_device_token_jwt, get_device_doc, load_device_private_key};


pub async fn connect_into(target_url:&str, node_id:&str) {
    // 读取node配置和密钥信息
    let device_doc: DeviceConfig = get_device_doc(node_id).unwrap();
    let device_private_key: EncodingKey = load_device_private_key(node_id).unwrap();
    // 生成对应jwt
    let device_session_token_jwt = get_device_token_jwt(&device_private_key, &device_doc).unwrap();

    // init client
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