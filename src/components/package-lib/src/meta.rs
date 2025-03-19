use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageMeta {
    pub pkg_name: String,
    pub version: String,
    pub description: String,
    pub pub_time: u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub deps: HashMap<String, String>,     //key = pkg_name,value = version_req_str,like ">1.0.0-alpha"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>, //pkg的分类,app,pkg,agent等
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>, //有些pkg不需要下载
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_url: Option<String>, //发布时的URL,可以不写
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size: Option<u64>, //有些pkg不需要下载

    #[serde(flatten)]
    pub extra_info: HashMap<String, Value>,

}

pub struct PackageMetaNode {
    pub meta_jwt:String,
    pub pkg_name:String,
    pub version:String,
    pub tag:Option<String>,
    pub author:String,
    pub author_pk:String,
}