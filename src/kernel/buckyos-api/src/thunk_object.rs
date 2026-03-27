use std::collections::HashMap;

use ndn_lib::{NamedObject, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionType {
    ExecPkg,// executable package
    Script(String),// script ,language type,content is the script content
    Operator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FunctionParamType {
    Fixed(String),//参数是传统的类型(和JsonValue Type)，必须保存在ThunkParams中
    ObjId(String),//参数是ObjId(Obj类型)，在运行前需要先确认该参数在NamedStore中存在
    CheckByRunner(String),//参数是类型是ObjId,但是由Runner在运行期处理检查
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
    Fixed(String),
    /// 结果是一个 Named Object 的引用
    Object(String),
    /// 字节流
    Stream(String),
    /// 元素迭代器：结果是一个有限的结构化元素序列
    Iterator {
        element_schema: String,    // 容器的类型（element的访问前缀）
        seekable: bool,            // true = 可随机访问任意元素，false = 只能顺序消费
    },
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AffinityType {
    Input,
    Result,
    Custom(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionObject {
    pub func_type: FunctionType,
    pub content: String,
    pub is_pure: bool,
    pub timeout: Option<u64>,
    
    // resource_type => resource_value,0表示只要有就可以，没有最小值要求
    // resourc_type也可以是一个自定义的资源路径，比如 /data/mydata 说明node需要持有这个路径的数据才可以（传统的tag系统），
    //此时resource_value是0
    pub requirements: HashMap<String, u64>,
    //如果有多个节点满足资源需求，则根据best_run_weight的资源权重来评分，选择得分最高的节点
    pub best_run_weight: HashMap<String, u64>,
    pub affinity_type: AffinityType,

    //param_name => param_type
    pub params_type: HashMap<String, FunctionParamType>,//参数类型
    pub result_type: FunctionResultType,
    
}

impl NamedObject for FunctionObject {
    fn get_obj_type() -> &'static str {
        "func"
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkObject {
    pub fun_id: ObjId,
    pub params: HashMap<String, Value>,
    pub metadata: Value,//metadata is a json object
}

impl NamedObject for ThunkObject {
    fn get_obj_type() -> &'static str {
        "thunk"
    }
}


#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThunkExecutionStatus {
    Waiting,//waiting for the runner to dispatch
    Dispatched,//dispatched to the runner id
    Success,
    Failed,//error message
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkExecutionResult {
    pub thunk_obj_id: ObjId,
    pub task_id: String,

    pub status: ThunkExecutionStatus,
    //for object result, the result_obj_id is the obj_id of the result
    #[serde(default)]
    pub result_obj_id: Option<ObjId>,
    //for fixed result, the result is the result value
    #[serde(default)]
    pub result: Option<Value>,
    //for iterator result, the result_url is the url of the result
    #[serde(default)]
    pub result_url: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub metrics: Value,
}
