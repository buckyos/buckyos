use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/*
App申请的权限分类

申请多用户权限
申请短域名权限（用friend-name做短域名）


访问用户信息权限
文件系统权限
系统服务接口权限(app在使用时需要访问OOD的信息)
    kservice-name, method-name

公网访问权限
    允许访问Known Node (用户默认的社交分组)
    对特定IP地址有权限
    对特定IP地址无权限


*/




#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GrantMode {
    All,
    Subset,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PermissionRequest {
    pub scope: String,              // e.g. "fs.home"
    pub grant: GrantMode,           // all | subset
    pub required: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,       // e.g. ["read","write"]

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<PermissionItem>, // required when grant=subset


    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PermissionItem {
    pub id: String, // e.g. "public"

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,


    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<serde_json::Value>,
}

