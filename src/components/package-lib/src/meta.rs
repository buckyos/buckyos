use serde::{Deserialize, Serialize};
use std::collections::HashMap;


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageMeta {
    pub pkg_name: String,
    pub version: String,
    pub category: Option<String>, //pkg的分类,app,pkg,agent等
    pub author: String,
    pub chunk_id: Option<String>, //有些pkg不需要下载
    pub chunk_url: Option<String>, //发布时的URL,可以不写
    pub deps: HashMap<String, String>,     //key = pkg_name,value = version_expr,like ">1.0.0-alpha"
    pub pub_time: i64,
}

