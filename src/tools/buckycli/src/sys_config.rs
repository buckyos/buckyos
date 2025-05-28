
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use buckyos_api::*;

pub async fn get_config(key: &str) {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.get(key).await;
    match result {
        Ok(value) => {
            // println!("value:");
            println!("{}", value.0);
            // println!("version:");
            // println!("{}", value.1);
        }
        Err(err) => println!("config get error: {}", err),
    }
}

pub async fn set_config(key: &str, value: &str) {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.set(key, value).await;
    match result {
        Ok(version) => println!("{} 已设置为 {}, version: {}", key, value, version),
        Err(err) => println!("config set error: {}", err),
    }
}

pub async fn list_config(key: &str) {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.list(key).await;
    match result {
        Ok(value) => {
            value.iter().for_each(|item| {
                println!("{}", item);
            })
        },
        Err(err) => println!("config list error: {}", err),
    }
}





pub async fn connect_into() {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();

    // ping test connection
    if let Err(e) = syc_cfg_client.get("boot/config").await {
        println!("connect system config service failed {}", e.to_string());
        return;
    }
    
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
                                // println!("value:");
                                println!("{}", value.0);
                                // println!("version:");
                                // println!("{}", value.1);
                            }
                            Err(err) => println!("config get error: {}", err),
                        }
                    }
                    "set" => {
                        // TODO 询问是否覆盖
                        if parts.len() != 3 {
                            println!("用法: set <key> <value>");
                            continue;
                        }
                        let key = parts[1];
                        let value = parts[2];
                        let result = syc_cfg_client.set(key, value).await;
                        match result {
                            Ok(version) => println!("{} 已设置为 {}, version: {}", key, value, version),
                            Err(err) => println!("config set error: {}", err),
                        }
                    }
                    "list" => {
let key =                         if parts.len() > 1 { parts[1] } else { "" };
                        let result = syc_cfg_client.list(key).await;
                        match result {
                            Ok(value) => {
                                value.iter().for_each(|item| {
                                    println!("{}", item);
                                });
                            }
                            Err(err) => println!("config list error: {}", err),
                        }
                    }
                    "del" => {
                        if parts.len()!= 2 {
                            println!("用法: del <key>");
                            continue;
                        }
                        let key = parts[1];
                        let result = syc_cfg_client.delete(key).await;
                        match result {
                            Ok(version) => println!("key [{}] 已删除, version: {}", key, version),
                            Err(err) => println!("config delete error: {}", err),
                        }
                    }
                    "set_jpath" | "set_jsonpath" => {
                        if parts.len()!= 4 {
                            println!("用法: jsonpath <key>/<json_path> <value>");
                            continue;
                        }
                        let key = parts[1];
                        let json_path = parts[2];
                        let value = parts[3];
                        let result = syc_cfg_client.set_by_json_path(key, json_path, value).await;
                        match result {
                            Ok(version) => println!("key [{}] 已设置, version: {}", key, version),
                            Err(err) => println!("config set by json path error: {}", err),
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