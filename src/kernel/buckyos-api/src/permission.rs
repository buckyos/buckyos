use serde::{Deserialize, Serialize};

// permission的设置应该继承系统的RBAC定义
// App申请特定resource（用path表示）的权限
// App声明自己在运行过程中可能使用的系统功能（允许安装后动态调整）
pub const APP_PERMISSION_AICC: &str = "kapi/aicc";
pub const APP_PERMISSION_TASK_MGR: &str = "kapi/task-manager";
pub const APP_PERMISSION_WORKFLOW: &str = "kapi/workflow";
pub const APP_PERMISSION_MSG_CENTER: &str = "kapi/msg-center";
pub const APP_PERMISSION_KMSG_QUEUE: &str = "kapi/kmsg";
pub const APP_PERMISSION_REPO: &str = "kapi/repo-service";
// 以客户端身份主动访问网络时，可以使用更具体的scope_path限制域名/IP地址
pub const APP_PERMISSION_INTERNET: &str = "wan";
pub const APP_PERMISSION_LOCAL_NETWORK: &str = "lan";
// default/app/www 说明app想要申请短域名www，通过后会写入services/gateway/settings → shortcuts
pub const APP_PERMISSION_DEFAULT_WEB_APP: &str = "default/app/{}";
//安装后允许为所有用户提供服务
pub const APP_PERMISSION_FOR_ALL_USER: &str = "access/all";
//可以主动访问特定设备
pub const APP_PERMISSION_IOT_DEVICE: &str = "devices/iot";
pub const APP_PERMISSION_USER_HOME: &str = "user/home";
pub const APP_PERMISSION_LIBRARY: &str = "zone/library";
pub const APP_PERMISSION_ZONE_PUBLIC: &str = "zone/public";
pub const APP_PERMISSION_LOCATION: &str = "zone/location";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PermissionItem {
    pub scope_path: String, //APP_PERMISSION_XXX
    pub required: bool,     //对应用来说，是不是一个必须批准的权限（拒绝可能会影响App的正确工作)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>, // e.g. ["read","write"]
    pub exp: Option<u32>,   //权限的有效期，为None表示一直有效
}
