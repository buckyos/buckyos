use std::collections::HashMap;

use ndn_lib::NamedObject;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionType {
    ExecPkg,// executable package
    Script(String),// script ,language type,content is the script content
    Operator,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRequirements {
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_duration: Option<String>,
    #[serde(default)]
    pub gpu_required: bool,
    #[serde(default)]
    pub max_cost_usdb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FunctionResultType {
    /// 单值结果，一次返回完整对象
    Fixed,
    /// 结果是一个 Named Object 的引用
    Object,
    /// 字节流
    Stream,
    /// 元素迭代器：结果是一个有限的结构化元素序列
    Iterator {
        element_schema: String,    // 每个元素的 schema 标识（或 $ref）
        seekable: bool,            // true = 可随机访问任意元素，false = 只能顺序消费
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionObject {
    pub func_type: FunctionType,
    pub content: String,
    pub is_pure: bool,
    pub timeout: Option<u64>,
    
    // resource_type => resource_value
    pub requirements: HashMap<String, u64>,
    //是输入亲和还是结果亲和
    //pub close_type: CloseType,

    pub params_type: Value,
    pub result_type: FunctionResultType,
    
}

impl NamedObject for FunctionObject {
    fn get_obj_type() -> &'static str {
        "func"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkParams {
    #[serde(rename = "type")]
    pub param_type: ThunkParamType,
    #[serde(default)]
    pub values: Value,
    #[serde(default)]
    pub obj_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThunkParamType {
    Fixed,
    Normal,
    CheckByRunner,
}



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkMetadata {
    pub run_id: String,
    pub node_id: String,
    pub attempt: u32,
    #[serde(default)]
    pub shard: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkObject {
    pub thunk_obj_id: String,
    pub fun_id: String,
    pub params: ThunkParams,
    pub idempotent: bool,
    pub resource_requirements: ResourceRequirements,
    pub metadata: ThunkMetadata,
}

impl NamedObject for ThunkObject {
    fn get_obj_type() -> &'static str {
        "thunk"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThunkMetrics {
    #[serde(default)]
    pub tokens_used: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub cost_usdb: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThunkExecutionStatus {
    Success,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkExecutionResult {
    pub thunk_obj_id: String,
    pub status: ThunkExecutionStatus,
    #[serde(default)]
    pub result_obj_id: Option<String>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub metrics: ThunkMetrics,
    #[serde(default)]
    pub side_effect_receipt: Option<Value>,
}
