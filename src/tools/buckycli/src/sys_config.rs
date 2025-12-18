use buckyos_api::*;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

pub async fn get_config(key: &str) {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.get(key).await;
    match result {
        Ok(value) => {
            // println!("value:");
            println!("{}", value.value);
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
        Ok(version) => println!("{} set to {}, version: {}", key, value, version),
        Err(err) => println!("config set error: {}", err),
    }
}

pub async fn append_config(key: &str, value: &str) {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.append(key, value).await;
    match result {
        Ok(version) => println!("{} appended {}, version: {}", key, value, version),
        Err(err) => println!("config append error: {}", err),
    }
}

pub async fn list_config(key: &str) {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.list(key).await;
    match result {
        Ok(value) => value.iter().for_each(|item| {
            println!("{}", item);
        }),
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
    // Read user input
    let readline = rl.readline("sys_config> ");
    match readline {
        Ok(line) => {
            let _ = rl.add_history_entry(line.as_str());
            // Parse input command
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                match parts[0] {
                    "get" => {
                        if parts.len() != 2 {
                            println!("Usage: get <key>");
                            continue;
                        }
                        let key = parts[1];
                        let result = syc_cfg_client.get(key).await;
                        match result {
                            Ok(value) => {
                                // println!("value:");
                                println!("{}", value.value);
                                // println!("version:");
                                // println!("{}", value.1);
                            }
                            Err(err) => println!("config get error: {}", err),
                        }
                    }
                    "set" => {
                        // TODO ask if overwrite
                        if parts.len() != 3 {
                            println!("Usage: set <key> <value>");
                            continue;
                        }
                        let key = parts[1];
                        let value = parts[2];
                        let result = syc_cfg_client.set(key, value).await;
                        match result {
                            Ok(version) => {
                                println!("{} set to {}, version: {}", key, value, version)
                            }
                            Err(err) => println!("config set error: {}", err),
                        }
                    }
                    "list" => {
                        let key = if parts.len() > 1 { parts[1] } else { "" };
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
                        if parts.len() != 2 {
                            println!("Usage: del <key>");
                            continue;
                        }
                        let key = parts[1];
                        let result = syc_cfg_client.delete(key).await;
                        match result {
                            Ok(version) => println!("key [{}] deleted, version: {}", key, version),
                            Err(err) => println!("config delete error: {}", err),
                        }
                    }
                    "set_jpath" | "set_jsonpath" => {
                        if parts.len() != 4 {
                            println!("Usage: jsonpath <key>/<json_path> <value>");
                            continue;
                        }
                        let key = parts[1];
                        let json_path = parts[2];
                        let value = parts[3];
                        let result = syc_cfg_client.set_by_json_path(key, json_path, value).await;
                        match result {
                            Ok(version) => println!("key [{}] set, version: {}", key, version),
                            Err(err) => println!("config set by json path error: {}", err),
                        }
                    }
                    "exit" => {
                        println!("Exiting program.");
                        break;
                    }
                    _ => {
                        println!("Unknown command: {}", parts[0]);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Received interrupt signal, exiting program.");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("Received EOF, exiting program.");
                break;
            }
            Err(err) => {
                println!("Error reading input: {}", err);
                break;
            }
        }
    }
}
