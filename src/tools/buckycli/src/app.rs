#[allow(dead_code, unused)]
use buckyos_api::*;
use serde_json::Value;

use crate::app;

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
    "tcp_ports": 80
}
 */

pub async fn create_app(app_config: &str) {
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

    let full_app_config = match build_app_service_config(&app_config) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to build app service config: {}", e);
            return;
        }
    };
    let config_value: Value = serde_json::from_str(&full_app_config).unwrap();
    let app_id = config_value.get("app_id").and_then(|v| v.as_str()).unwrap();

    let api_runtime = get_buckyos_api_runtime().unwrap();
    let syc_cfg_client = api_runtime.get_system_config_client().await.unwrap();
    let config_key = format!("users/devtest/apps/{}/config", app_id);
    match syc_cfg_client.set(&config_key, &full_app_config).await {
        Ok(_) => {
            println!("App service config set successfully for app_id: {}", app_id);
        }
        Err(e) => {
            eprintln!("Failed to set app service config: {}", e);
            return;
        }
    }
}

fn build_app_service_config(app_config: &serde_json::Value) -> Result<String, String> {
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
        "app_index": 2,
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
        "udp_ports": { }
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
    let mut data_mount_point = app_config
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
    let tcp_ports = app_config.get("tcp_ports").and_then(|v| v.as_i64());
    let tcp_ports = match tcp_ports {
        Some(port) => format!("\"www\": {}", port),
        None => " ".to_string(),
    };
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
                    }}
                }}
            }},
            "app_index": 2,
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
            }}
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
        data_mount_point,
        tcp_ports
    );
    return Ok(full_app_config);
}

//test build_app_service_config
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_build_app_service_config() {
        let app_config = r#"
        {
            "app_id": "test_app",
            "app_name": "Test App",
            "version": "0.1.0",
            "author": "Test Author",
            "description": "This is a test app",
            "docker_image": "https://test_docker_url",
            "data_mount_point": {
            },
            "tcp_ports": 80
        }
        "#;
        let app_config: serde_json::Value = serde_json::from_str(app_config).unwrap();
        println!("App Config: {:?}", app_config);
        let result = build_app_service_config(&app_config);
        assert!(result.is_ok());
        let full_app_config = result.unwrap();
        println!("Full App Config: {}", full_app_config);
        let _app_config: AppConfig = serde_json::from_str(&full_app_config).unwrap();
        let _test_parse_value: Value = serde_json::from_str(&full_app_config).unwrap();
    }
}
