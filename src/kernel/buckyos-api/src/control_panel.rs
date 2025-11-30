use std::ops::Deref;
//system control panel client

use name_lib::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::json;
use log::*;
use ::kRPC::*; 
use crate::system_config::*;
use crate::app_mgr::*;
use package_lib::PackageMeta;
use crate::KVAction;

pub const SERVICE_INSTANCE_INFO_UPDATE_INTERVAL: u64 = 30;

#[derive(Serialize, Deserialize, Clone)]
#[serde(try_from = "String", into = "String")]
pub enum UserState {
    Active,
    Suspended(String),//suspend reason
    Deleted,//delete reason
    Banned(String),//ban reason
}

impl TryFrom<String> for UserState {
    type Error = &'static str;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        let split_result = value.split(":").collect::<Vec<&str>>();
        let state_str = split_result[0];
        let reason = split_result.get(1).unwrap_or(&"");
        match state_str {
            "active" => Ok(UserState::Active),
            "suspended" => Ok(UserState::Suspended(reason.to_string())),
            "deleted" => Ok(UserState::Deleted),
            "banned" => Ok(UserState::Banned(reason.to_string())),
            _ => Err("Invalid user state"),
        }
    }
}

impl Into<String> for UserState {
    fn into(self) -> String {
        match self {
            UserState::Active => "active".to_string(),
            UserState::Suspended(reason) => format!("suspended:{}", reason),
            UserState::Deleted => "deleted".to_string(),
            UserState::Banned(reason) => format!("banned:{}", reason),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum UserType {
    Admin,
    User,
    Root,
    Limited,
    Guest,
}

//user did -> UserSettings
#[derive(Serialize, Deserialize)]
pub struct UserSettings {
    //rename to type
    #[serde(rename = "type")]
    pub user_type:UserType,
    pub username:String,//友好名称
    pub password:String,
    pub state: UserState,
    pub res_pool_id:String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum NodeState {
    Running,
    Stopped,
    Starting,
    Stopping,
    Restarting,
    Initializing,
    Removed,
    Maintenance,
    Repalcing,
}

//load from SystemConfig,node的配置分为下面几个部分
// 固定的硬件配置，一般只有硬件改变或损坏才会修改
// 系统资源情况，（比如可用内存等），改变密度很大。这一块暂时不用etcd实现，而是用专门的监控服务保存
// RunItem的配置。这块由调度器改变，一旦改变,node_daemon就会产生相应的控制命令
// Task(Cmd)配置，暂时不实现
#[derive(Serialize, Deserialize)]
pub struct NodeConfig {
    //pub pure_version: u64,
    pub kernel: HashMap<String, KernelServiceInstanceConfig>,
    pub apps: HashMap<String, AppServiceInstanceConfig>,
    pub frame_services: HashMap<String, FrameServiceInstanceConfig>,
    pub state:NodeState,
}

impl NodeConfig {
    pub fn is_running(&self) -> bool {
        match self.state {
            NodeState::Running => true,
            _ => false,
        }
    }
}

pub struct ControlPanelClient {
    system_config_client: SystemConfigClient,
}

impl ControlPanelClient {
    pub fn new(system_config_client: SystemConfigClient) -> Self {
        Self { system_config_client }
    }

    //return (rbac_model,rbac_policy)
    pub async fn load_rbac_config(&self) -> Result<(String,String)> {
        let rbac_model_path = "system/rbac/model";
        let rbac_model_result = self.system_config_client.get(rbac_model_path).await;
        if rbac_model_result.is_err() {
            return Err(RPCErrors::ReasonError("rbac model not found".to_string()));
        }
        let rbac_model_result = rbac_model_result.unwrap();

        let rbac_policy_path = "system/rbac/policy";
        let rbac_policy_result = self.system_config_client.get(rbac_policy_path).await;
        if rbac_policy_result.is_err() {
            return Err(RPCErrors::ReasonError("rbac policy not found".to_string()));
        }
        let rbac_policy_result = rbac_policy_result.unwrap();
        return Ok((rbac_model_result.value,rbac_policy_result.value));   
    }

    pub async fn load_zone_config(&self) -> Result<ZoneConfig> {
        let zone_config_path = "boot/config";
        let zone_config_result = self.system_config_client.get(zone_config_path).await;
        if zone_config_result.is_err() {
            return Err(RPCErrors::ReasonError(format!("get boot config(Zone config) failed:{}",zone_config_result.err().unwrap())));
        }
        let zone_config_result = zone_config_result.unwrap();
        let zone_config:ZoneConfig = serde_json::from_str(&zone_config_result.value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(zone_config)
    }

    pub async fn get_device_info(&self,device_id:&str) -> Result<DeviceInfo> {
        let device_info_path = format!("devices/{}/info",device_id);
        let get_result = self.system_config_client.get(device_info_path.as_str()).await;
        if get_result.is_err() {
            return Err(RPCErrors::ReasonError("Device info not found".to_string()));
        }

        let get_result = get_result.unwrap();
        let device_info:DeviceInfo= serde_json::from_str(&get_result.value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(device_info)
    }
    
    pub async fn get_device_config(&self,device_id:&str) -> Result<DeviceConfig> {
        let device_doc_path = format!("devices/{}/doc",device_id);
        let get_result = self.system_config_client.get(device_doc_path.as_str()).await;
        if get_result.is_err() {
            return Err(RPCErrors::ReasonError("Trust key  not found".to_string()));
        }

        let get_result = get_result.unwrap();    
        let device_doc: EncodedDocument = EncodedDocument::from_str(get_result.value.clone())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        let device_doc: DeviceConfig = DeviceConfig::decode(&device_doc, None)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

        Ok(device_doc)
    }

    pub async fn add_user(&self, user_config:&OwnerConfig,is_admin:bool) -> Result<u64> {
        //0. check user_config.name is valid
        //1. create users/{user_id}/doc
        //2. create users/{user_id}/settings
        //3. add user to rbac group
        let user_id = user_config.name.clone();
        let user_doc_path = format!("users/{}/doc", user_id);
        let user_settings_path = format!("users/{}/settings", user_id);
        
        // 将用户配置序列化为 JSON 字符串
        let user_doc_str = serde_json::to_string(user_config)
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to serialize user config: {}", e)))?;
            
        // 创建默认用户设置
        let default_settings = json!({
            "theme": "light",
            "language": "en",
            "notifications": true
        });
        let settings_str = serde_json::to_string(&default_settings)
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to serialize settings: {}", e)))?;

        // 准备事务操作
        let mut tx_actions = HashMap::new();
        
        // 1. 创建用户文档
        tx_actions.insert(user_doc_path, KVAction::Create(user_doc_str));
        
        // 2. 创建用户设置
        tx_actions.insert(user_settings_path, KVAction::Create(settings_str));
        
        // // 3. 添加用户到 RBAC 组 => move to scheduler
        // let rbac_policy = if is_admin {
        //     format!("\ng, {}, admin", user_id)
        // } else {
        //     format!("\ng, {}, user", user_id)
        // };
        // tx_actions.insert("system/rbac/policy".to_string(), KVAction::Append(rbac_policy));
        
        // 执行事务
        self.system_config_client.exec_tx(tx_actions, None).await
            .map_err(|e| RPCErrors::ReasonError(format!("Failed to execute user creation transaction: {}", e)))?;

        info!("Successfully added user {} with admin={}", user_id, is_admin);
        Ok(0)
    }

    //TODO: help app installer dev easy to generate right app-index
    pub async fn install_app_service(&self,user_id:&str,app_config:&AppServiceSpec,shortcut:Option<String>) -> Result<u64> {
        // TODO: if you want install a web-client-app, use another function
        //1. create users/{user_id}/apps/{appid}/config
        let app_id = app_config.app_doc.pkg_name.as_str();
        let app_config_str = serde_json::to_string(app_config).unwrap();
        self.system_config_client.create(format!("users/{}/apps/{}/config",user_id,app_id).as_str(),app_config_str.as_str()).await
            .map_err(|e| RPCErrors::ReasonError(format!("install app service failed, err:{}", e)))?;
        //2. update rbac
        self.system_config_client.append("system/rbac/policy",format!("\ng, {}, app",app_id).as_str()).await
            .map_err(|e| RPCErrors::ReasonError(format!("install app service failed, err:{}", e)))?;
        //3. update gateway shortcuts
        if shortcut.is_some() {
            let short_name = shortcut.unwrap();
            let short_json_path = format!("/shortcuts/{}",short_name.as_str());
            let short_json_value = json!({
                "type":"app",
                "user_id":user_id,
                "app_id":app_id
            });
            let short_json_value_str = serde_json::to_string(&short_json_value).unwrap();

            self.system_config_client.set_by_json_path("services/gateway/settings",
                short_json_path.as_str(),short_json_value_str.as_str()).await
                .map_err(|e| RPCErrors::ReasonError(format!("install app service failed, err:{}", e)))?;

            info!("set shortcut {} for user {}'s app {} success!",short_name,user_id,app_id);
        }

        info!("install app service {} for user {} success!",app_id,user_id);
        Ok(0)
    }

    pub async fn get_user_list(&self) -> Result<Vec<String>> {
        let user_list = self.system_config_client.list("users").await;
        if user_list.is_err() {
            return Err(RPCErrors::ReasonError("user list not found".to_string()));
        }
        Ok(user_list.unwrap())
    }

    pub async fn get_app_list(&self) -> Result<Vec<AppServiceSpec>> {
        let user_list = self.get_user_list().await;
        if user_list.is_err() {
            return Err(RPCErrors::ReasonError("user list not found".to_string()));
        }
        let user_list = user_list.unwrap();
        let mut result_app_list = Vec::new();
        for user_id in user_list {
            let app_list = self.system_config_client.list(format!("users/{}/apps",user_id).as_str()).await;
            if app_list.is_err() {
                return Err(RPCErrors::ReasonError("app list not found".to_string()));
            }
            for app_id in app_list.unwrap() {
                let app_config = self.system_config_client.get(format!("users/{}/apps/{}/config",user_id,app_id).as_str()).await;
                if app_config.is_err() {
                    return Err(RPCErrors::ReasonError("app config not found".to_string()));
                }
                let app_config = app_config.unwrap();
                let app_config:AppServiceSpec = serde_json::from_str(&app_config.value)
                    .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
                result_app_list.push(app_config);
            }
        }
        Ok(result_app_list)
    }

    pub async fn update_service_instance_info(&self,
        service_name:&str,node_name:&str,
        instance_info:&ServiceInstanceReportInfo) -> Result<u64> {
        let service_info_path = format!("services/{}/instances/{}",service_name,node_name);
        let service_info_str = serde_json::to_string(instance_info).unwrap();
        self.system_config_client.set(service_info_path.as_str(),service_info_str.as_str()).await
            .map_err(|e| RPCErrors::ReasonError(format!("update service instance info {}@{} failed, err:{}", service_name,node_name, e)))?;
        Ok(0)
    }

    pub async fn get_services_info(&self,service_name:&str) -> Result<ServiceInfo> {
        let service_info_path = format!("services/{}/info",service_name);
        let service_info = self.system_config_client.get(service_info_path.as_str()).await;
        if service_info.is_err() {
            return Err(RPCErrors::ServiceNotValid("service info not found".to_string()));
        }
        let service_info = service_info.unwrap();
        let service_info:ServiceInfo = serde_json::from_str(&service_info.value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(service_info)
    }
    // TODO: move to scheduler_service
    // pub async fn get_valid_app_index(&self,_user_id:&str) -> Result<u64> {
    //     unimplemented!();
    // }

    pub async fn remove_app(&self,_appid:&str) -> Result<u64> {
        unimplemented!();
    }

    //disable means stop app service
    pub async fn stop_app(&self,_appid:&str) -> Result<u64> {
        unimplemented!();
    }

    pub async fn start_app(&self,_appid:&str) -> Result<u64> {
        unimplemented!();
    }

}


