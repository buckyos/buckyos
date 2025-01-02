mod app;
mod pod;

use std::collections::HashMap;
use std::process::exit;
use log::*;
use serde_json::json;
use serde_json::Value;
//use upon::Engine;

use name_lib::*;
use name_client::*;
use buckyos_kit::*;
use sys_config::SystemConfigClient;
use app::*;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn generate_app_config(user_name:&str) -> Result<HashMap<String,String>> {
    let mut init_list : HashMap<String,String> = HashMap::new();
    let config_str = serde_json::to_string(&json!({
        "app_id":"buckyos.home_station",
        "app_name" : "Home Station",
        "app_description" : "Home Station",
        "vendor_id" : "buckyos",
        "pkg_id" : "home_station",
        "username" : user_name.to_string(),
        "service_docker_images" : {
            "x86_server" : "filebrowser/filebrowser:s6"
        },
        "data_mount_point" : "/srv",
        "cache_mount_point" : "/database/",
        "local_cache_mount_point" : "/config/",
        "max_cpu_num" : Some(4),
        "max_cpu_percent" : Some(80),
        "memory_quota" : 1024*1024*1024*1, //1GB
        "host_name" : Some("home".to_string()),
        "port" : 20080,
        "org_port" : 80
    })).unwrap();
    init_list.insert(format!("users/{}/apps/{}/config",user_name.to_string(),"buckyos.home_station"),config_str);
    Ok(init_list)
}

async fn generate_ood_config(ood_name:&str,owner_name:&str) -> Result<HashMap<String,String>> {
    let mut init_list : HashMap<String,String> = HashMap::new();
    let config_str = serde_json::to_string(&json!({
        "is_running":true,
        "revision" : 0,
        "kernel" : {
            "verify_hub" : {
                "target_state":"Running",
                "pkg_id":"verify_hub",
                "operations":{
                    "status":{
                        "command":"status",
                        "params":[]
                    },
                    "start":{
                        "command":"start",
                        "params":[]
                    },
                    "stop":{
                        "command":"stop",
                        "params":[]
                    },
                }
            },
            "scheduler" : {
                "target_state":"Running",
                "pkg_id":"scheduler",
                "operations":{
                    "status":{
                        "command":"status",
                        "params":[]
                    },
                    "start":{
                        "command":"start",
                        "params":[]
                    },
                    "stop":{
                        "command":"stop",
                        "params":[]
                    },
                }
            }
        },
        "services":{
        },
        "apps":{
            format!("{}#buckyos.home_station",owner_name):{
                "target_state":"Running",
                "app_id":"buckyos.home_station",
                "username":owner_name,
            }
        }
    })).unwrap();
    //init ood config
    init_list.insert(format!("nodes/{}/config",ood_name),config_str);

    Ok(init_list)
}

async fn create_init_list_by_template() -> Result<HashMap<String,String>> {
    //load start_parms from active_service.
    let start_params_file_path = get_buckyos_system_etc_dir().join("start_config.json");
    println!("start_params_file_path:{}",start_params_file_path.to_string_lossy());
    info!("try load start_params from :{}",start_params_file_path.to_string_lossy());
    let start_params_str = tokio::fs::read_to_string(start_params_file_path).await?;
    let mut start_params:serde_json::Value = serde_json::from_str(&start_params_str)?;

    let mut template_type_str = "nat_ood_and_sn".to_string();
    //load template by zone_type from start params.
    let template_type = start_params.get("zone_type");
    if template_type.is_some() {
        template_type_str = template_type.unwrap().as_str().unwrap().to_string();
    }

    let template_file_path = get_buckyos_system_etc_dir().join("scheduler").join(format!("{}.template.toml",template_type_str));
    let template_str = tokio::fs::read_to_string(template_file_path).await?;

    //generate dynamic params
    let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
    start_params["verify_hub_key"] = json!(private_key_pem);
    start_params["verify_hub_public_key"] = json!(public_key_jwk.to_string());
    start_params["BUCKYOS_ROOT"] = json!(get_buckyos_root_dir().to_string_lossy().to_string());

    let mut engine = upon::Engine::new();
    engine.add_template("config", &template_str)?;
    let result = engine
        .template("config")
        .render(&start_params)
        .to_string()?;

    if result.find("{{").is_some() {
        return Err("template contains unescaped double curly braces".into());
    }

    //wwrite result to file
    //let result_file_path = get_buckyos_system_etc_dir().join("scheduler_boot.toml");
    //tokio::fs::write(result_file_path, result.clone()).await?;

    let config: HashMap<String, String> = toml::from_str(&result)?;

    Ok(config)
}


async fn do_boot_scheduler() -> Result<()> {
    let mut init_list : HashMap<String,String> = HashMap::new();
    let zone_config_str = std::env::var("BUCKY_ZONE_CONFIG");

    if zone_config_str.is_err() {
        warn!("BUCKY_ZONE_CONFIG is not set, use default zone config");
        return Err("BUCKY_ZONE_CONFIG is not set".into());
    }

    info!("zone_config_str:{}",zone_config_str.as_ref().unwrap());
    let mut zone_config:ZoneConfig = serde_json::from_str(&zone_config_str.unwrap()).unwrap();
    let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN");

    if rpc_session_token_str.is_err() {
        return Err("SCHEDULER_SESSION_TOKEN is not set".into());
    }

    let rpc_session_token = rpc_session_token_str.unwrap();
    let system_config_client = SystemConfigClient::new(None,Some(rpc_session_token.as_str()));
    let boot_config = system_config_client.get("boot/config").await;
    if boot_config.is_ok() {
        return Err("boot/config already exists, boot scheduler failed".into());
    }

    let init_list = create_init_list_by_template().await
        .map_err(|e| {
            error!("create_init_list_by_template failed: {:?}", e);
            e
        })?;

    info!("use init list from template to do boot scheduler");
    //write to system_config
    for (key,value) in init_list.iter() {
        system_config_client.create(key,value).await?;
    }
    info!("boot scheduler success");
    return Ok(());
}



async fn do_one_ood_schedule(input_config: &HashMap<String, String>) -> Result<(HashMap<String, JsonValueAction>,Value)> {
    let mut result_config: HashMap<String, JsonValueAction> = HashMap::new();
    let mut device_list: HashMap<String, DeviceInfo> = HashMap::new();
    for (key, value) in input_config.iter() {
        if key.starts_with("devices/") && key.ends_with("/info") {
            let device_name = key.split('/').nth(1).unwrap();
            let device_info:DeviceInfo = serde_json::from_str(value)
                .map_err(|e| {
                    error!("serde_json::from_str failed: {:?}", e);
                    e
                })?;
            device_list.insert(device_name.to_string(), device_info);
        }
    }
    let mut all_app_http_port:HashMap<String,u16> = HashMap::new();

    // deploy all app configurations
    for (key, value) in input_config.iter() {
        if key.starts_with("users/") && key.ends_with("/config") {
            let parts: Vec<&str> = key.split('/').collect();
            if parts.len() >= 4 && parts[2] == "apps" {
                let user_name = parts[1];
                let app_id = parts[3];

                // Install app for user
                let app_config = deploy_app_service(user_name, app_id,&device_list, &input_config).await;
                if app_config.is_err() {
                    error!("do_one_ood_schedule Failed to install app {} for user {}: {:?}", app_id, user_name, app_config.err().unwrap());
                    return Err("do_one_ood_schedule Failed to install app".into());
                }
                let (app_config,http_port) = app_config.unwrap();
                extend_json_action_map(&mut result_config, &app_config);
                if http_port.is_some() {
                    all_app_http_port.insert(format!("{}.{}",app_id,user_name),http_port.unwrap());
                }
            }
        }
    }

    //结合系统的快捷方式配置,设置nodes/gateway 配置,目前所有节点的gateway配置都是一样的
    let mut real_base_gateway_config : Value = json!({});
    let base_gateway_config = input_config.get("services/gateway/base_config");
    if base_gateway_config.is_some() {
        let base_gateway_config = base_gateway_config.unwrap();
        let base_gateway_config = serde_json::from_str(base_gateway_config);
        if base_gateway_config.is_ok() {
            real_base_gateway_config = base_gateway_config.unwrap();
        } else {
            error!("serde_json::from_str failed: {:?}", base_gateway_config.err().unwrap());
        }
    } else {
        error!("services/gateway/base_config is not set");
    }

    let gateway_setting = input_config.get("services/gateway/setting");
    if gateway_setting.is_some() {
        info!("gateway_setting:{}",gateway_setting.unwrap());
        let gateway_setting = gateway_setting.unwrap();
        let gateway_setting = serde_json::from_str(gateway_setting);
        if gateway_setting.is_ok() {
            let gateway_setting:Value = gateway_setting.unwrap();
            let shortcuts = gateway_setting.get("shortcuts");
            if shortcuts.is_some() {
                let shortcuts = shortcuts.unwrap();
                for (short_name,value) in shortcuts.as_object().unwrap() {
                    let short_type = value.get("type");
                    let short_owner = value.get("user_id");
                    let short_app_id = value.get("app_id");
                    if short_type.is_some() && short_owner.is_some() && short_app_id.is_some() {
                        //let short_type = short_type.unwrap();
                        let short_owner = short_owner.unwrap().as_str().unwrap();
                        let short_app_id = short_app_id.unwrap().as_str().unwrap();
                        let short_app_key = format!("{}.{}",short_app_id,short_owner);
                        if all_app_http_port.contains_key(&short_app_key) {
                            let http_port = all_app_http_port.get(&short_app_key).unwrap();
                            info!("find shortcut to app_service :{} http_port:{},modify gateway_config",short_app_key,http_port);
                            let app_gateway_config = json!({
                                "routes":{
                                    "/":{
                                        "upstream":format!("http://127.0.0.1:{}",http_port)
                                    }
                                }
                            });
                            if short_name == "www" {
                                let default_upstream = json!({
                                    "/":{
                                        "upstream":format!("http://127.0.0.1:{}",http_port)
                                    }
                                });

                                set_json_by_path(&mut real_base_gateway_config,"/servers/main_http_server/hosts/*/routes",Some(&default_upstream));

                            }
                            set_json_by_path(&mut real_base_gateway_config,format!("/servers/main_http_server/hosts/{}.*",short_name).as_str(),Some(&app_gateway_config));
                        } else {
                            warn!("find shortcut to app_service :{} not found",short_app_key);
                        }
                    }
                }
            }
        }
    }


    Ok((result_config,real_base_gateway_config))
}

fn path_is_gateway_config(path: &str) -> bool {
    if path.starts_with("nodes") && path.ends_with("gateway") {
        return true;
    }
    return false;
}

async fn schedule_loop() -> Result<()> {
    let mut loop_step = 0;
    let is_running = true;
    info!("schedule loop start...");
    loop {
        if !is_running {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
        loop_step += 1;
        info!("schedule loop step:{}.", loop_step);
        let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN");
        if rpc_session_token_str.is_err() {
            return Err("SCHEDULER_SESSION_TOKEN is not set".into());
        }

        let rpc_session_token = rpc_session_token_str.unwrap();
        let system_config_client = SystemConfigClient::new(None,Some(rpc_session_token.as_str()));
        let input_config = system_config_client.dump_configs_for_scheduler().await;
        if input_config.is_err() {
            error!("dump_configs_for_scheduler failed: {:?}", input_config.err().unwrap());
            continue;
        }
        let input_config = input_config.unwrap();
        //cover value to hashmap
        let input_config = serde_json::from_value(input_config);
        if input_config.is_err() {
            error!("serde_json::from_value failed: {:?}", input_config.err().unwrap());
            continue;
        }
        let input_config = input_config.unwrap();
        // According to the configuration of OOD, it is 1 OOD, 2 OOD, 3 OOD, 3-7 OOD,
        // 7OOD or above multi -OOD general -purpose scheduling loop
        let schedule_result = do_one_ood_schedule(&input_config).await;
        if schedule_result.is_err() {
            error!("do_one_ood_schedule failed: {:?}", schedule_result.err().unwrap());
            continue;
        }
        let (schedule_result,base_gateway_config) = schedule_result.unwrap();

        //write to system_config
        for (path,value) in schedule_result.iter() {
            match value {
                JsonValueAction::Create(value) => {
                   //TODO:
                   unimplemented!();
                }
                JsonValueAction::Update(value) => {
                    system_config_client.set(path,value).await?;
                }
                JsonValueAction::SetByPath(value) => {
                    let mut old_value:Value;
                    if path_is_gateway_config(path) {
                        old_value = base_gateway_config.clone();
                    } else {
                        let remote_value = input_config.get(path).unwrap();
                        old_value = serde_json::from_str(remote_value).unwrap();
                    }
                    for (sub_path,sub_value) in value.iter() {
                        set_json_by_path(&mut old_value,sub_path,Some(sub_value));
                    }
                    system_config_client.set(path,old_value.to_string().as_str()).await?;
                }
                JsonValueAction::Remove => {
                    system_config_client.delete(path).await?;
                }
            }
        }
    }
    Ok(())

}


async fn service_main(is_boot:bool) -> Result<i32> {
    init_logging("scheduler");
    info!("Starting scheduler service............................");
    init_global_buckyos_value_by_env("SCHEDULER");
    let _ =init_default_name_client().await;
    if is_boot {
        info!("do_boot_scheduler,scheduler run once");
        do_boot_scheduler().await.map_err(|e| {
            error!("do_boot_scheduler failed: {:?}", e);
            e
        })?;
        return Ok(0);
    }

    info!("Enter schedule loop.");
    schedule_loop().await.map_err(|e| {
        error!("schedule_loop failed: {:?}", e);
        e
    })?;
    return Ok(0);
}

#[tokio::main]
async fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    let mut is_boot = false;
    if args.len() > 1 {
        if args[1] == "--boot" {
            is_boot = true;
        }
    }

    let ret = service_main(is_boot).await;
    if ret.is_err() {
        println!("service_main failed: {:?}", ret);
        exit(-1);
    }
    exit(ret.unwrap());
}

#[cfg(test)]
mod test {
    use tokio::test;
    use super::*;
    #[tokio::test]
    async fn test_schedule_loop() {
        service_main(true).await;
    }

    #[tokio::test]
    async fn test_template() {
        let start_params = create_init_list_by_template().await.unwrap();
        for (key,value) in start_params.iter() {
            let json_value= serde_json::from_str(value);
            if json_value.is_ok() {
                let json_value:serde_json::Value = json_value.unwrap();
                println!("{}:\t{:?}",key,json_value);
            } else {
                println!("{}:\t{}",key,value);
            }
        }
        //println!("start_params:{}",serde_json::to_string(&start_params).unwrap());
    }
    #[tokio::test]
    async fn test_simple_schedule() {

        let input_config_str = r#"
# users : users/{{user_id}}/info , user_id is never changed, user_name can changed. User root cann't be deleted and always exists
"users/root/info" = '{"type":"root","username":"{{user_name}}","password":"{{admin_password_hash}}"}'

# devices,set & update by register_device_doc@node_daemon
#"devices/ood1/doc" = "ood1_doc"
# devices,set & update by update_device_info@node_daemon
"devices/ood1/info" = """
{
    "hostname":"ood1",
    "device_type":"ood",
    "arch":"aarch64"
}
"""

# system settings

"system/verify_hub/key" = """
{{verify_hub_key}}
"""
# frames & services
"services/verify_hub/info" = """
{
    "port":3300,
    "node_list":["ood1"],
    "type":"kernel"
}
"""
"services/verify_hub/setting" = """
{
    "trust_keys" : []
}
"""
"services/scheduler/info" = """
{
    "port":3400,
    "node_list":["ood1"],
    "type":"kernel"
}
"""
# info for zone-gateway
"services/gateway/info" = """
{
    "port":3100,
    "node_list":["ood1"],
    "type":"kernel"
}
"""
"services/gateway/setting" = """
{
    "shortcuts": {
        "www": {
            "type":"app",
            "user_id":"root",
            "app_id":"home-station"
        },
        "sys": {
            "type":"app",
            "user_id":"root",
            "app_id":"control-panel"
        },
        "test":{
            "type":"app",
            "user_id":"root",
            "app_id":"sys-test"
        }
    }
}
"""

"services/gateway/base_config" = """
{
    "device_key_path":"/opt/buckyos/etc/node_private_key.pem",
    "servers":{
        "main_http_server":{
            "type":"cyfs-warp",
            "bind":"0.0.0.0",
            "http_port":80,
            "tls_port":443,
            "hosts": {
                "*": {
                    "enable_cors":true,
                    "routes": {
                        "/kapi/system_config":{
                            "upstream":"http://127.0.0.1:3200"
                        },
                        "/kapi/verify_hub":{
                            "upstream":"http://127.0.0.1:3300"
                        }
                    }
                },
                "sys.*": {
                    "enable_cors":true,
                    "routes": {
                        "/":{
                            "local_dir":"/opt/buckyos/bin/control_panel"
                        },
                        "/kapi/system_config":{
                            "upstream":"http://127.0.0.1:3200"
                        },
                        "/kapi/verify_hub":{
                            "upstream":"http://127.0.0.1:3300"
                        }
                    }
                }
            }
        }
    },
    "dispatcher" : {
        "tcp://0.0.0.0:80":{
            "type":"server",
            "id":"main_http_server"
        },
        "tcp://0.0.0.0:443":{
            "type":"server",
            "id":"main_http_server"
        }
    }
}
"""

# install apps
"users/root/apps/home-station/config" = """
{
    "app_id":"home-station",
    "app_info" : {
        "name" : "Home Station",
        "description" : "Home Station",
        "vendor_did" : "did:bns:buckyos",
        "pkg_id" : "home-station",
        "pkg_list" : {
            "amd64_docker_image" : {
                "pkg_id":"home-station-x86-img",
                "docker_image_name":"filebrowser/filebrowser:s6"
            },
            "aarch64_docker_image" : {
                "pkg_id":"home-station-arm64-img",
                "docker_image_name":"filebrowser/filebrowser:s6"
            },
            "web_pages" :{
                "pkg_id" : "home-station-web-page"
            }
        }
    },
    "app_index" : 0,
    "enable" : true,
    "instance" : 1,
    "deployed" : false,

    "data_mount_point" : "/srv",
    "cache_mount_point" : "/database/",
    "local_cache_mount_point" : "/config/",
    "max_cpu_num" : 4,
    "max_cpu_percent" : 80,
    "memory_quota" : 1073741824,
    "tcp_ports" : {
        "www":80
    }
}
"""
# node config
"nodes/ood1/config" = """
{
    "is_running":true,
    "revision" : 0,
    "gateway" : {
    },
    "kernel":{
        "verify_hub" : {
            "target_state":"Running",
            "pkg_id":"verify_hub",
            "operations":{
                "status":{
                    "command":"status",
                    "params":[]
                },
                "start":{
                    "command":"start",
                    "params":[]
                },
                "stop":{
                    "command":"stop",
                    "params":[]
                }
            }
        },
        "scheduler" : {
            "target_state":"Running",
            "pkg_id":"scheduler",
            "operations":{
                "status":{
                    "command":"status",
                    "params":[]
                },
                "start":{
                    "command":"start",
                    "params":[]
                },
                "stop":{
                    "command":"stop",
                    "params":[]
                }
            }
        }
    },
    "services":{

    },
    "apps":{
        "0":{
            "target_state":"Running",
            "app_id":"home-station",
            "username":"root"
            "service_docker_images" : "filebrowser/filebrowser:s6",
            "data_mount_point" : "/srv",
            "cache_mount_point" : "/database/",
            "local_cache_mount_point" : "/config/",
            "extra_mounts" : {
                "/opt/buckyos/data/root/home-station/:/srv",
            },
            "max_cpu_num" : 4,
            "max_cpu_percent" : 80,
            "memory_quota" : 1073741824,
            "tcp_ports" : {
                "20000":80
            }
        }
    }
}
""""
"nodes/ood1/gateway" = """
{
    "device_key_path":"/opt/buckyos/etc/node_private_key.pem",
    "servers":{
        "main_http_server":{
            "type":"cyfs-warp",
            "bind":"0.0.0.0",
            "http_port":80,
            "tls_port":443,
            "hosts": {
                "*": {
                    "enable_cors":true,
                    "routes": {
                        "/kapi/system_config":{
                            "upstream":"http://127.0.0.1:3200"
                        },
                        "/kapi/verify_hub":{
                            "upstream":"http://127.0.0.1:3300"
                        },
                        "/":{
                            "upstream":"http://127.0.0.1:20000"
                        }
                    }
                },
                "sys.*":{
                    "routes":{
                        "/":{
                            "local_dir":"{{BUCKYOS_ROOT}}/bin/control_panel"
                        }
                    }
                },
                "test.*":{
                    "routes":{
                        "/":{
                            "local_dir":"{{BUCKYOS_ROOT}}/bin/sys_test"
                        }
                    }
                }
            }
        }
    },
    "dispatcher" : {
        "tcp://0.0.0.0:80":{
            "type":"server",
            "id":"main_http_server"
        },
        "tcp://0.0.0.0:443":{
            "type":"server",
            "id":"main_http_server"
        }
    }
}
"""

"system/rbac/model" = """
[request_definition]
r = sub,obj,act

[policy_definition]
p = sub, obj, act, eft

[role_definition]
g = _, _ # sub, role

[policy_effect]
e = priority(p.eft) || deny

[matchers]
m = (g(r.sub, p.sub) || r.sub == p.sub) && ((r.sub == keyGet3(r.obj, p.obj, p.sub) || keyGet3(r.obj, p.obj, p.sub) =="") && keyMatch3(r.obj,p.obj)) && regexMatch(r.act, p.act)
"""
"system/rbac/policy" = """
p, kernel, kv://*, read|write,allow
p, kernel, dfs://*, read|write,allow

p, owner, kv://*, read|write,allow
p, owner, dfs://*, read|write,allow

p, user, kv://*, read,allow
p, user, dfs://public/*,read|write,allow
p, user, dfs://homes/{user}/*, read|write,allow
p, app,  dfs://homes/*/apps/{app}/*, read|write,allow

p, limit, dfs://public/*, read,allow
p, guest, dfs://public/*, read,allow

g, node_daemon, kernel
g, ood01,ood
g, alice, user
g, bob, user
g, app1, app
g, app2, app
"""

"boot/config" = """
{
    "did":"did:ens:{{user_name}}",
    "oods":["ood1"],
    "sn":"{{sn_host}}",
    "verify_hub_info":{
        "port":3300,
        "node_name":"ood1",
        "public_key":{{verify_hub_public_key}}
    }
}
"""
        "#;
        buckyos_kit::init_logging("scheduler");
        let input_config: HashMap<String, String> = toml::from_str(input_config_str).unwrap();
        let schedule_result = do_one_ood_schedule(&input_config).await;
        let schedule_result = schedule_result.unwrap();
        println!("schedule_result:{}",serde_json::to_string(&schedule_result).unwrap());
    }
}
