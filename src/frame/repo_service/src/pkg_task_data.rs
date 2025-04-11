
/*

里创建发布任务，在任务状态中写入pkg_list，初始发布任务的状态为 已经收到请求
开始执行任务，检查pkg_list的各种deps是否已经在当前index-meta-db中存在了,检查失败在发布任务中写入错误信息
获得所有待下载的chunklist,保存在任务数据中
尝试下载pkg的chunk,失败在发布任务中写入错误信息，下载完成会更新任务的百分比进度信息
下载完成后，标识任务为 发布完成等待审核 

## handle_merge_wait_pub_to_source_pkg
合并 `发布完成等待审核 ` 发布任务里包含的pkg_list到local-wait-meta
 
*/


use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubPkgTaskData {
    //pkg_name -> pkg_meta_jwt
    pub pkg_list: HashMap<String,String>,
    pub author_name: String,
    pub author_pk: jsonwebtoken::jwk::Jwk,
    pub author_repo_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPkgTaskData {
    //pkg_id -> will_install_chunk_id
    pub pkg_list: HashMap<String,String>,

}
