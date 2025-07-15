use ::kRPC::*;
use buckyos_api::*;
use buckyos_kit::*;
use name_lib::*;
use serde_json::json;
use sysinfo::System;

const SERVICE_NAME: &str = "test-service";

const DEFAULT_SERVICE_CONFIG: &str = r#"
{
    "name":"test-service",
    "description":"just for test",
    "vendor_did":"did:bns:buckyos",
    "pkg_id":"test_service",
    "port":33333,
    "node_list":["ood1"],
    "service_type":"kernel",
    "state":"New",
    "instance":1
}
"#;

async fn remove_service(client: &kRPC) -> std::result::Result<(), String> {
    // 移除服务
    // 获取服务poditem配置
    println!("Will remove service: {}", SERVICE_NAME);
    let mut service_config = get_current_service_config(client).await.map_err(|e| {
        println!("Failed to get current service config: {}", e);
        return e.to_string();
    })?;

    //println!("Current service config: {:?}", service_config);

    // 设置service_config的state状态为 Removing
    service_config.state = "Removing".to_string();

    // 更新配置
    let _ = client
        .call(
            "sys_config_set",
            json!({ "key": format!("services/{}/config", SERVICE_NAME), "value": serde_json::to_string(&service_config).unwrap() }),
        )
        .await.map_err(|e| {
            println!("Failed to set services/{}/config: {}", SERVICE_NAME, e);
            return e.to_string();
        })?;

    Ok(())
}

async fn start_service(client: &kRPC) -> std::result::Result<(), String> {
    // 启动服务
    println!("Will start service: {}", SERVICE_NAME);
    // 获取rbac配置
    let result = client
        .call("sys_config_get", json!({ "key": "system/rbac/policy" }))
        .await
        .map_err(|e| {
            println!("Failed to get system/rbac/policy: {}", e);
            return e.to_string();
        })?;

    if result.is_null() {
        return Err("rbac policy not found".to_string());
    }

    let rbac_value = result.as_str().unwrap_or("");
    // 替换掉rbac里面的空格
    let rbac_value = rbac_value.replace(" ", "");
    // 查找有没有"g,test-service"的配置
    if !rbac_value.contains(&format!("g,{},", SERVICE_NAME)) {
        // 如果没有，添加一条rbac规则
        let rbac_rule = format!("\ng, {}, service", SERVICE_NAME);
        let _ = client
            .call(
                "sys_config_append",
                json!({ "key": "system/rbac/policy", "append_value": rbac_rule }),
            )
            .await
            .map_err(|e| {
                println!("Failed to append to system/rbac/policy: {}", e);
                return e.to_string();
            })?;
        println!("rbac policy updated with: {}", rbac_rule);
    } else {
        println!("rbac policy already contains g,{},service", SERVICE_NAME);
    }

    // 设置服务的配置
    let _ = client
        .call(
            "sys_config_set",
            json!({ "key": format!("services/{}/config", SERVICE_NAME), "value": DEFAULT_SERVICE_CONFIG }),
        )
        .await
        .map_err(|e| {
            println!("Failed to set services/{}/config: {}", SERVICE_NAME, e);
            return e.to_string();
        })?;

    Ok(())
}

async fn check_service_running() -> std::result::Result<bool, String> {
    let mut system = System::new_all();
    system.refresh_all();

    let target_name = "test_service";

    let process_exists = system
        .processes()
        .values()
        .any(|process| process.name() == target_name);

    if process_exists { Ok(true) } else { Ok(false) }
}

async fn get_current_service_config(
    client: &kRPC,
) -> std::result::Result<KernelServiceConfig, String> {
    let result = client
        .call(
            "sys_config_get",
            json!({ "key": format!("services/{}/config", SERVICE_NAME) }),
        )
        .await
        .map_err(|e| {
            println!("Failed to get services/{}/config: {}", SERVICE_NAME, e);
            return e.to_string();
        })?;

    if result.is_null() {
        return Err(format!("service {} not found", SERVICE_NAME));
    }

    let service_config: KernelServiceConfig = serde_json::from_str(result.as_str().unwrap())
        .map_err(|e| format!("failed to parse service config: {}", e))?;

    Ok(service_config)
}

async fn test() -> std::result::Result<(), String> {
    let etc_dir = get_buckyos_system_etc_dir();
    let bucky_cli_dir = etc_dir.join(".buckycli");
    println!("buckycli dir {:?}", bucky_cli_dir);

    if !bucky_cli_dir.exists() {
        println!("bucky_cli_dir not exists");
        return Err("bucky_cli_dir not exists".to_string());
    }

    let user_config_file = bucky_cli_dir.join("user_config.json");
    let user_private_key_file = bucky_cli_dir.join("user_private_key.pem");
    if !user_config_file.exists() {
        println!("user config file not exists");
        return Err("user config file not exists".to_string());
    }
    if !user_private_key_file.exists() {
        println!("user private key file not exists");
        return Err("user private key file not exists".to_string());
    }

    let private_key = load_private_key(&user_private_key_file).map_err(|e| {
        println!("Failed to load private key: {}", e);
        return e.to_string();
    })?;

    let user_name = "root".to_string(); // 使用root用户进行测试

    let (session_token_str, _real_session_token) = RPCSessionToken::generate_jwt_token(
        &user_name,
        "buckycli",
        Some(user_name.clone()),
        &private_key,
    )
    .map_err(|e| {
        println!("Failed to generate session token for admin + kernel: {}", e);
        return e.to_string();
    })?;

    let client = kRPC::new(
        "http://127.0.0.1:3200/kapi/system_config",
        Some(session_token_str),
    );

    // 先检查服务有没有运行，如果有，停止并移除
    if check_service_running().await? {
        println!("Service is running, stopping and removing it...");
        // 移除服务
        remove_service(&client).await?;
        println!("Waiting for service to be removed...");
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;

        // 检查服务是否已经停止
        if check_service_running().await? {
            let service_config = get_current_service_config(&client).await?;
            println!(
                "Service is still running after removal, current state: {}",
                service_config.state
            );
            return Err(format!(
                "Service is still running after removal, current state: {}",
                service_config.state
            ));
        } else {
            println!("Service has been removed successfully.");
        }
    }

    // 再创建服务
    start_service(&client).await?;
    println!("Waiting for service to start...");
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    // 检查服务是否运行
    if !check_service_running().await? {
        return Err("Service is not running after start".to_string());
    } else {
        let service_config = get_current_service_config(&client).await?;
        if service_config.state != "Deployed" {
            return Err(format!(
                "Service state is not Deployed, current state: {}",
                service_config.state
            ));
        }
        println!(
            "Service is running successfully. current state: {}",
            service_config.state
        );
    }

    // 再次停止服务，回到最初状态
    remove_service(&client).await?;
    println!("Waiting for service to be removed...");
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    if check_service_running().await? {
        println!("Service is still running after removal, please check!");
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    println!("test_create_service started");
    let result = test().await;
    if result.is_err() {
        println!("Test failed: {}", result.err().unwrap());
        std::process::exit(1);
    }
    println!("Test success");
    std::process::exit(0);
}
