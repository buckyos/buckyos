#[allow(dead_code, unused)]
use buckyos_api::*;
use serde_json::json;
use serde_json::Value;

/* app_config.json example:
{
    "app_id": "test_app",
    "app_name": "Test App",
    "version": "0.1.0",
    "author": "Test Author",
    "description": "This is a test app",
    "docker_image": "image name",
    "data_mount_point": {
        "/srv/": "home/"
    },
    "tcp_ports": {
        "www": 80,
    }
}
 */

pub async fn create_app(app_config: &str) {
    let api_runtime = get_buckyos_api_runtime()
        .map_err(|e| {
            eprintln!("Failed to get BuckyOS API runtime: {}", e);
            return;
        })
        .unwrap();

    // 从文件读取app_config, 解析app_id, pkg_name, version, app_name, description等信息
    let result = std::fs::File::open(app_config);
    let file = match result {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Failed to open app config file: {}", e);
            return;
        }
    };
    let result = serde_json::from_reader(file);
    let app_config: serde_json::Value = match result {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to parse app config file: {}", e);
            return;
        }
    };
    println!("Parsed app config: {:?}", app_config);

    let user_id = api_runtime.user_id.clone();
    if user_id.is_none() {
        eprintln!("User ID is not set in the BuckyOS API runtime.");
        return;
    }
    let user_id = user_id.unwrap();

    let full_app_config = match build_app_service_config(&app_config).await {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to build app service config: {}", e);
            return;
        }
    };
    let config_value: Value = serde_json::from_str(&full_app_config).unwrap();
    let app_id = config_value.get("app_id").and_then(|v| v.as_str()).unwrap();

    match is_app_exist(app_id).await {
        Ok(exists) => {
            if exists {
                eprintln!("App {} already exists, please remove it first.", app_id);
                return;
            }
        }
        Err(e) => {
            eprintln!("Failed to check if app {} exists: {}", app_id, e);
            return;
        }
    }

    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let config_key = format!("users/{}/apps/{}/config", user_id, app_id);
    match syc_cfg_client.set(&config_key, &full_app_config).await {
        Ok(_) => {
            println!("App service config set successfully for app_id: {}", app_id);
        }
        Err(e) => {
            eprintln!("Failed to set app service config: {}", e);
            return;
        }
    }
    //2. update gateway shortcuts
    let mut app_url = String::new();
    let tcp_port = app_config
        .get("tcp_ports")
        .and_then(|v| v.as_object())
        .and_then(|v| v.get("www"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if tcp_port > 0 {
        let short_json_path = format!("/shortcuts/{}", app_id);
        let short_json_value = json!({
            "type": "app",
            "user_id": user_id,
            "app_id": app_id
        });
        let short_json_value_str = serde_json::to_string(&short_json_value).unwrap();
        match syc_cfg_client
            .set_by_json_path(
                "services/gateway/settings",
                short_json_path.as_str(),
                short_json_value_str.as_str(),
            )
            .await
        {
            Ok(_) => {
                println!(
                    "Gateway shortcut created successfully for app_id: {}",
                    app_id
                );
                let user_zone_host = api_runtime.zone_id.to_host_name();
                app_url = format!("{}.{}", app_id, user_zone_host);
            }
            Err(e) => {
                eprintln!("Failed to create gateway shortcut: {}", e);
                return;
            }
        }
    } else {
        println!("No TCP port specified, skipping gateway shortcut creation.");
    }
    //3. update rbac
    let rbac = match syc_cfg_client.get("system/rbac/policy").await {
        Ok(policy) => policy,
        Err(e) => {
            eprintln!("Failed to get RBAC policy: {}", e);
            return;
        }
    };
    if rbac.value.is_empty() {
        eprintln!("RBAC policy is empty.");
        return;
    } else {
        let app_rbac = format!("g, {}, app", app_id);
        if rbac.value.contains(&app_rbac) {
            println!("RBAC policy already contains: {}", app_rbac);
        } else {
            println!("Adding RBAC policy: {}", app_rbac);
            match syc_cfg_client
                .append("system/rbac/policy", app_rbac.as_str())
                .await
            {
                Ok(_) => {
                    println!("RBAC policy added successfully for app_id: {}", app_id);
                }
                Err(e) => {
                    eprintln!("Failed to add RBAC policy: {}", e);
                    return;
                }
            }
        }
    }

    if app_url.is_empty() {
        println!("App {} created successfully, but no gateway shortcut created due to no TCP port specified.", app_id);
    } else {
        println!(
            "App {} created successfully, access it at: http://{}",
            app_id, app_url
        );
    }
}

async fn is_app_exist(app_id: &str) -> Result<bool, String> {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.list("/users").await;
    let users = result.map_err(|e| format!("Failed to list users: {}", e))?;
    if users.is_empty() {
        return Ok(false);
    } else {
        for user in users {
            let apps_key = format!("users/{}/apps/{}", user, app_id);
            let result = syc_cfg_client.list(&apps_key).await;
            let keys =
                result.map_err(|e| format!("Failed to list keys for app {}: {}", apps_key, e))?;
            if !keys.is_empty() {
                println!("App {} exists for user {}", app_id, user);
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub async fn delete_app(app_id: &str) {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.list("/users").await;
    let users = match result {
        Ok(users) => users,
        Err(e) => {
            eprintln!("Failed to list users: {}", e);
            return;
        }
    };
    if users.is_empty() {
        eprintln!("No users found in the system.");
        return;
    }
    let mut config_key = String::new();
    let mut config_content = String::new();
    for user in users {
        let app_key = format!("/users/{}/apps/{}/config", user, app_id);
        if let Ok(content) = syc_cfg_client.get(&app_key).await {
            println!("App {} found for user {}", app_id, user);
            config_key = app_key;
            config_content = content.value;
            break;
        }
    }
    if config_key.is_empty() || config_content.is_empty() {
        eprintln!("App {} not found for any user.", app_id);
        return;
    }
    let mut app_config: serde_json::Value = match serde_json::from_str(&config_content) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to parse app config: {}", e);
            return;
        }
    };
    // set state to "PodItemState::Removing"
    app_config["state"] = "Removing".into();
    let app_config_str = match serde_json::to_string(&app_config) {
        Ok(config_str) => config_str,
        Err(e) => {
            eprintln!("Failed to serialize app config: {}", e);
            return;
        }
    };
    // update app config
    match syc_cfg_client.set(&config_key, &app_config_str).await {
        Ok(_) => {
            println!("App {} deleted successfully.", app_id);
        }
        Err(e) => {
            eprintln!("Failed to delete app {}: {}", app_id, e);
        }
    }
}

async fn build_app_service_config(app_config: &serde_json::Value) -> Result<String, String> {
    // 检查app_config是否包含必要字段
    if !app_config.is_object() {
        return Err("Invalid app config format, expected a JSON object.".into());
    }
    if !app_config.get("app_id").is_some() {
        return Err("Missing 'app_id' in app config.".into());
    }
    if !app_config.get("docker_image").is_some() {
        return Err("Missing 'docker_image' in app config.".into());
    }
    let cur_app_count = get_app_count().await.map_err(|e| e.to_string())?;
    /*
    let full_app_config = r#"
    {
        "app_id": "{}",
        "app_doc": {
            "pkg_name": "{}",
            "version": "*",
            "tag": "latest",
            "app_name": "{}",
            "description": {
                "detail": "{}"
            },
            "author": {},
            "pub_time": 0,
            "exp": 0,
            "pkg_list": {
                "amd64_docker_image": {
                    "docker_image_name": "{}",
                }
            },
            "deps": {
            },
            "install_config": {
                "data_mount_point": [
                    "/srv/"
                ],
                "cache_mount_point": [ ],
                "local_cache_mount_point": [ ],
                "tcp_ports": {
                    "www": 80
                },
                "udp_ports": { }
            }
        },
        "app_index": {},
        "enable": true,
        "state": "New",
        "instance": 1,
        "data_mount_point": [
            "/srv/": "home/",
        ],
        "cache_mount_point": [ ],
        "local_cache_mount_point": [ ],
        "max_cpu_num": 2,
        "max_cpu_percent": 20,
        "memory_quota": 1073741824,
        "tcp_ports": {
            "www": 80
        },
        "udp_ports": { },
    }"#;
    */

    //通过app_config, 填充full_app_config
    let app_id = app_config
        .get("app_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown_app_id")
        .to_string();
    let app_name = app_config
        .get("app_name")
        .and_then(|v| v.as_str())
        .unwrap_or(app_id.as_str())
        .to_string();
    let version = app_config
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.1")
        .to_string();
    let author = app_config
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let description = app_config
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("No description provided")
        .to_string();
    let docker_image = app_config
        .get("docker_image")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown_docker_image")
        .to_string();
    let data_mount_point = app_config
        .get("data_mount_point")
        .and_then(|v| v.as_object())
        .map(|v| {
            v.iter()
                .map(|(k, v)| format!("\"{}\": \"{}\"", k, v.as_str().unwrap()))
                .collect::<Vec<String>>()
                .join(", ")
        })
        .unwrap_or_else(|| "".to_string());
    let data_mount_point_config = app_config
        .get("data_mount_point")
        .and_then(|v| v.as_object())
        .map(|v| {
            v.iter()
                .map(|(k, _v)| format!("\"{}\"", k))
                .collect::<Vec<String>>()
                .join(", ")
        })
        .unwrap_or("".to_string());
    let tcp_ports = app_config
        .get("tcp_ports")
        .and_then(|v| v.as_object())
        .map(|v| {
            v.iter()
                .map(|(k, v)| format!("\"{}\": {}", k, v.as_i64().unwrap_or(0)))
                .collect::<Vec<String>>()
                .join(", ")
        })
        .unwrap_or("".to_string());
    let udp_ports = app_config
        .get("udp_ports")
        .and_then(|v| v.as_object())
        .map(|v| {
            v.iter()
                .map(|(k, v)| format!("\"{}\": {}", k, v.as_i64().unwrap_or(0)))
                .collect::<Vec<String>>()
                .join(", ")
        })
        .unwrap_or("".to_string());
    let container_param = app_config
        .get("container_param")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let full_app_config = format!(
        r#"
        {{
            "app_id": "{}",
            "app_doc": {{
                "pkg_name": "{}",
                "version": "{}",
                "tag": "latest",
                "app_name": "{}",
                "description": {{
                    "detail": "{}"
                }},
                "author": "{}",
                "pub_time": 0,
                "exp": 0,
                "pkg_list": {{
                    "amd64_docker_image": {{
                        "docker_image_name": "{}",
                        "pkg_id": "{}#{}"
                    }}
                }},
                "deps": {{}},
                "install_config": {{
                    "data_mount_point": [{}],
                    "cache_mount_point": [],
                    "local_cache_mount_point": [],
                    "tcp_ports": {{
                        {}
                    }},
                    "udp_ports": {{
                        {}
                    }}
                }},
                "container_param": "{}"
            }},
            "app_index": {},
            "enable": true,
            "state": "New",
            "instance": 1,
            "data_mount_point": {{{}}},
            "cache_mount_point": [],
            "local_cache_mount_point" : [],
            "max_cpu_num": 2,
            "max_cpu_percent": 20,
            "memory_quota": 1073741824,
            "tcp_ports": {{
                {}
            }},
            "udp_ports": {{
                {}
            }},
            "container_param": "{}"
        }}"#,
        app_id,
        app_id,
        version,
        app_name,
        description,
        author,
        docker_image,
        app_id,
        version,
        data_mount_point_config,
        tcp_ports,
        udp_ports,
        container_param,
        cur_app_count + 1,
        data_mount_point,
        tcp_ports,
        udp_ports,
        container_param
    );
    return Ok(full_app_config);
}

async fn get_app_count() -> Result<u64, String> {
    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let result = syc_cfg_client.list("/users").await;
    let users = result.map_err(|e| format!("Failed to list users: {}", e))?;
    if users.is_empty() {
        return Ok(0);
    }
    println!("list uesrs: {:?}", users);
    let mut app_count = 0;
    for user in users {
        let user_apps_key = format!("/users/{}/apps", user);
        let result = syc_cfg_client.list(&user_apps_key).await;
        let apps = result.map_err(|e| format!("Failed to list apps for user {}: {}", user, e))?;
        println!("list apps for user {}: {:?}", user, apps);
        app_count += apps.len() as u64;
    }
    Ok(app_count)
}

//test build_app_service_config
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_build_app_service_config() {
        let user_id;
        match init_buckyos_api_runtime("buckycli", None, BuckyOSRuntimeType::AppClient).await {
            Ok(mut runtime) => match runtime.login().await {
                Ok(_) => {
                    user_id = runtime.user_id.clone().unwrap();
                    println!("user id {:?}", runtime.user_id);
                    println!("user config {:?}", runtime.user_config);
                    set_buckyos_api_runtime(runtime);
                }
                Err(e) => {
                    println!("Failed to login: {}", e);
                    return;
                }
            },
            Err(e) => {
                println!("Failed to init buckyos runtime: {}", e);
                return;
            }
        }

        let app_config_1 = r#"
        {
            "app_id": "n8n",
            "app_name": "n8n",
            "version": "*",
            "author": "n8nio",
            "description": "This is n8n",
            "docker_image": "docker.n8n.io/n8nio/n8n",
            "data_mount_point": {
                "/home/node/.n8n/" :  "n8n/data/"
            },
            "tcp_ports": {
                "www": 80
            },
            "container_param": "-e N8N_SECURE_COOKIE=false -e N8N_PORT=80"
        }
        "#;
        let app_config: serde_json::Value = serde_json::from_str(app_config_1).unwrap();
        println!("App Config: {:?}", app_config);
        let result = build_app_service_config(&app_config).await;
        assert!(result.is_ok());
        let full_app_config = result.unwrap();
        println!("Full App Config: {}", full_app_config);
        let _app_config: AppConfig = serde_json::from_str(&full_app_config).unwrap();
        let _test_parse_value: Value = serde_json::from_str(&full_app_config).unwrap();

        let app_config_2 = r#"
        {
            "app_id": "home-assistant",
            "app_name": "Home Assistant",
            "version": "*",
            "author": "home-assistant",
            "description": "This is home-assistant",
            "docker_image": "homeassistant/home-assistant",
            "data_mount_point": {
                "/config":  "homeassistant/config/"
            },
            "container_param": "-v /run/dbus:/run/dbus:ro -v /etc/localtime:/etc/localtime:ro --network=host --privileged"
        }
        "#;
        let app_config: serde_json::Value = serde_json::from_str(app_config_2).unwrap();
        println!("App Config: {:?}", app_config);
        let result = build_app_service_config(&app_config).await;
        assert!(result.is_ok());
        let full_app_config = result.unwrap();
        println!("Full App Config: {}", full_app_config);
        let _app_config: AppConfig = serde_json::from_str(&full_app_config).unwrap();
        let _test_parse_value: Value = serde_json::from_str(&full_app_config).unwrap();
    }
}
