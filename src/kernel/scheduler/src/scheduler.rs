/*
NodeScheduler — 纯函数式、平台无关的服务调度器
=================================================

## 设计理念

这是一个平台无关的调度器，基于一个确定性的系统状态（来自 system_config KV 存储），
执行一次调度，得到一组 SchedulerAction，然后等待执行后（系统状态发生改变），
再进入下一次调度。调度本身是幂等的，但会依赖"上一次调度快照"来判断变更。

构建的是 **纯函数式（Pure Functional）** 的调度器核心：
- 确定性（Determinism）：给定 Input (Snapshot)，必然得到 Output (Actions)。排查 Bug 只需要 Input 数据。
- 极高的测试覆盖率：不需要 Mock 网络或外部存储，只需要构造内存 Struct。
- 开发解耦：调度器只关心逻辑（本文件），执行器只关心"如何把动作落地"（system_config_agent.rs / service.rs）。

## 核心实体（已实现）

- `ServiceSpec`  — 被调度对象，描述一个服务的目标状态和资源需求
    - `ServiceSpecType`: Kernel（内核服务, owner=root）| Service（系统服务）| App（用户应用, 有 owner_user_id）
    - `ServiceSpecState`: New → Deployed | DeployFailed | Abnormal | Disable | Deleted
- `NodeItem`      — 资源载体，描述一个物理/逻辑节点的能力和状态
    - `NodeType`: OOD | Server | Desktop | Mobile | Sensor | IoTController
    - `NodeState`: New → Prepare → Ready → Abnormal | Unavailable | Removing → Deleted
    - 包含 CPU/Memory/GPU 资源、labels（用于亲和性匹配）、network_zone
- `UserItem`      — 用户信息（userid, user_type: Admin/User/Limited, 可选 res_pool_id）
- `ReplicaInstance` — ServiceSpec 在某个 Node 上的运行实例
    - `InstanceState`: Prepare | Running | Suspended | Deleted
    - 包含 last_update_time 用于存活检测（INSTANCE_ALIVE_TIME = 90s）
- `ServiceInfo`   — 调度器基于存活的 Instance 计算出的服务访问信息
    - SingleInstance（单实例）| RandomCluster（多实例集群，带权重）
- `OPTask`        — 运维任务定义（将在未来版本中被 Function Instance 取代，
    详见 doc/arch/使用function_instance实现分布式调度器.md）
- `SchedulerAction` — 调度器输出的动作枚举

## Schedule Loop（已实现，见 system_config_agent.rs::schedule_loop）

每 5 秒执行一轮调度：
1. 从 system_config 拉取全量系统状态（dump_configs_for_scheduler）
2. 通过 create_scheduler_by_system_config() 构造 NodeScheduler 实例
3. 从 system_config 加载上一次调度快照（system/scheduler/snapshot）
4. 调用 scheduler.schedule(last_snapshot) 得到 Vec<SchedulerAction>
5. 通过 schedule_action_to_tx_actions() 将 SchedulerAction 转换为 KV 事务操作
6. 附加处理：更新 RBAC 策略、更新 Node 的 gateway_config（路由规则）
7. 通过 exec_tx() 事务提交所有变更
8. 保存本次调度快照到 system/scheduler/snapshot

支持两种运行模式：
- Boot 模式（--boot）：仅执行一次调度，用于系统初始化
- Loop 模式：持续循环调度

## schedule() 四阶段流程（已实现）

Step1. resort_nodes() — 节点状态审查
    - New 节点 → Prepare（等待外部初始化完成后标记为 Ready）
    - Removing 节点 → Deleted
    - 小系统优化（节点数 ≤ 7）：如果 Step1 产生了动作，跳过 Step2，降低复杂度

Step2. schedule_spec_change() — ServiceSpec 实例化/反实例化（仅在 spec 发生变化时触发）
    - New → 过滤+打分选择最优节点，创建 ReplicaInstance，标记为 Deployed
    - Deleted → 回收所有关联的 ReplicaInstance
    - 节点过滤（filter_node_for_instance）：检查节点状态(Ready)、类型(OOD/Server)、
      容器支持、CPU/Memory/GPU 资源、node_affinity 标签匹配
    - 节点打分（score_node）：资源充足度 + 负载均衡 + 网络亲和性

Step3. （TODO）优化 instance 的资源使用

Step4. calc_service_infos() — 基于存活的 Instance 计算 ServiceInfo
    - 只有 Running 状态且 last_update_time 在 INSTANCE_ALIVE_TIME 内的 Instance 才计入
    - 启动期调度器会注入 bootstrap 假上报（创建 Instance 时设置当前时间戳），
      避免内核服务之间的启动依赖环
    - 仅在 ServiceInfo 发生变化时才产生 UpdateServiceInfo Action

## 执行器层（system_config_agent.rs + service.rs，非本文件）

SchedulerAction 的执行由外部模块负责：
- ChangeNodeStatus     → 更新 nodes/{node_id}/config 中的 state
- ChangeServiceStatus  → 更新 services/{spec_id}/spec 或 users/.../apps/.../spec 中的 state
- InstanceReplica      → 写入 nodes/{node_id}/config 中的 kernel 或 app 配置
- RemoveInstance       → 从 node config 中移除实例配置
- UpdateServiceInfo    → 更新 services/{spec_id}/info，供其他服务发现使用
- CreateOPTask         → 未来版本将重构为 Function Instance 调度
    （详见 doc/arch/使用function_instance实现分布式调度器.md）
- UpdateInstance       → （TODO: unimplemented for service type）

附加处理（在 schedule_loop 中执行）：
- 更新 RBAC 策略：根据 users/nodes/specs 的变化重新生成 system/rbac/policy
- 更新 Gateway 路由：根据 ServiceInfo 生成各 Node 的 gateway_config（process chain 规则）
    - Kernel/Service 服务 → 匹配 /kapi/{service_name} 路径
    - App 服务 → 匹配 host 前缀（子域名），支持 shortcut 快捷方式
    - 单实例本机 → 直接 forward 到 127.0.0.1:{port}
    - 其他情况 → 通过 buckyos-select 路由

## 工作原则

- 最小改动原则：尽量不改动已有的 Instance（可以增加，少删除或修改）
- 对系统做控制的方法：
    - 绝对不要直接修改 Instance 和 ServiceInfo，这些由调度器管理
    - 修改 ServiceSpec：调整服务的"目标状态"
    - 修改 Node：调整节点的"目标状态"

## 尚未实现的规划功能（仅供参考，不代表当前能力）

以下功能在早期设计文档中提及，但尚未在代码中实现：
- 系统级管理（重启/暂停/启动/关闭/备份/恢复）
- 完整的用户管理（删除/停用/启用用户及其服务）
- Node 日常体检、维护（排空/暂停）、替换、区域维护
- ServiceSpec Disable 状态的处理逻辑
- Instance 动态资源调整、优先级管理、自动错误隔离
- 运维 event 管理（异步决策）
- 树状资源组

## Function Instance（未来版本）

当前的 OPTask 原语将在未来版本中被 Function Instance（fun_instance）取代。
fun_instance 将调度原语从命令式的"在某节点执行某脚本"降级为纯函数调用
`exec(funid, params)`，使调度器保持纯函数式语义——只做函数求值分配，
不理解业务语义。可变状态通过版本化 `node_state(id, version)` 消解，
计算结果通过 Named Object 全局缓存实现跨任务复用。

本调度器的纯函数式设计（确定性输入→确定性输出）与 fun_instance 模型天然契合，
届时主要变化集中在调度原语和执行器层，调度核心逻辑保持不变。

详细设计见：doc/arch/使用function_instance实现分布式调度器.md

*/
#[warn(unused, unused_mut, dead_code)]
use anyhow::Result;
use buckyos_api::{ServiceInstanceState, ServiceState, BASE_APP_PORT};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

const SMALL_SYSTEM_NODE_COUNT: usize = 7;
// INSTANCE_ALIVE_TIME 不是“请求级”可用性 SLA，而是调度器层面的存活证明窗口。
// 调度器只关心“实例第一次被判定掉线”的时刻，因此允许真实访问失败与被宣告下线之间存在误差。
// 启动期调度器也会主动构造一次“假上报”来打破内核服务之间的启动依赖环。
const INSTANCE_ALIVE_TIME: u64 = 90;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum ServiceSpecType {
    Kernel,  //kernel service 无owner_user_id
    Service, //系统服务
    App,     // 无状态的app服务，有owner_user_id
}

impl From<String> for ServiceSpecType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "kernel" => ServiceSpecType::Kernel,
            "service" => ServiceSpecType::Service,
            "frame" => ServiceSpecType::Service,
            "app" => ServiceSpecType::App,
            _ => ServiceSpecType::App,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum ServiceSpecState {
    New,          //刚刚添加是这个状态， 调度器会试图将这个状态的Spec，变成Deployed
    Deployed,     // 调度器会努力保持有足够多的Instance
    DeployFailed, //调度器标记为部署失败，所有的Instance都会被删除
    Abnormal,     //调度器标记为异常，不会构造ServiceInfo,也不会主动回收Instance
    Disable,      //运维手工Disable,所有的Instance都会被Disable（不会回收）
    //Removing,// 调度器处理Deleted时，发根据某些逻辑需要做一些长时间的处理
    Deleted, //运维手工调用删除，调度器会尝试回收所有相关的Instance
}

impl Default for ServiceSpecState {
    fn default() -> Self {
        ServiceSpecState::New
    }
}

impl fmt::Display for ServiceSpecState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            ServiceSpecState::New => "New",
            ServiceSpecState::Deployed => "Deployed",
            ServiceSpecState::DeployFailed => "DeployFailed",
            ServiceSpecState::Abnormal => "Abnormal",
            ServiceSpecState::Disable => "Disable",
            ServiceSpecState::Deleted => "Deleted",
        };
        write!(f, "{value}")
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum InstanceState {
    Prepare,
    Running,
    Suspended,
    Deleted,
}

impl From<String> for InstanceState {
    fn from(s: String) -> Self {
        let s = s.to_lowercase();
        match s.as_str() {
            "prepare" => InstanceState::Prepare,
            "running" => InstanceState::Running,
            "suspended" => InstanceState::Suspended,
            "deleted" => InstanceState::Deleted,
            _ => InstanceState::Prepare,
        }
    }
}

impl From<ServiceInstanceState> for InstanceState {
    fn from(state: ServiceInstanceState) -> Self {
        match state {
            ServiceInstanceState::Started => InstanceState::Running,
            _ => InstanceState::Suspended,
        }
    }
}

impl ToString for InstanceState {
    fn to_string(&self) -> String {
        match self {
            InstanceState::Prepare => "prepare".to_string(),
            InstanceState::Running => "running".to_string(),
            InstanceState::Suspended => "suspended".to_string(),
            InstanceState::Deleted => "deleted".to_string(),
        }
    }
}

impl From<String> for ServiceSpecState {
    fn from(s: String) -> Self {
        match s.as_str() {
            "New" => ServiceSpecState::New,
            "Deployed" => ServiceSpecState::Deployed,
            "DeployFailed" => ServiceSpecState::DeployFailed,
            "Abnormal" => ServiceSpecState::Abnormal,
            "Unavailable" => ServiceSpecState::Disable,
            "Deleted" => ServiceSpecState::Deleted,
            _ => ServiceSpecState::Disable,
        }
    }
}

impl From<ServiceState> for ServiceSpecState {
    fn from(state: ServiceState) -> Self {
        match state {
            ServiceState::New => ServiceSpecState::New,
            ServiceState::Running => ServiceSpecState::Deployed,
            ServiceState::Stopped => ServiceSpecState::Disable,
            ServiceState::Stopping => ServiceSpecState::Disable,
            ServiceState::Restarting => ServiceSpecState::Abnormal,
            ServiceState::Updating => ServiceSpecState::Abnormal,
            ServiceState::Deleted => ServiceSpecState::Deleted,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum UserType {
    Admin,
    User,
    Limited,
}

impl From<String> for UserType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "admin" => UserType::Admin,
            "user" => UserType::User,
            "limited" => UserType::Limited,
            _ => UserType::User,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserItem {
    pub userid: String,
    pub user_type: UserType,
    pub res_pool_id: Option<String>, //资源池id，为None时表示未指定资源池
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub id: String, //format!("{}@{}", app_id, owner_id)
    pub app_id: String,
    pub app_index: u16,
    pub owner_id: String, //kernel service的owner_id是root
    pub spec_type: ServiceSpecType,
    pub state: ServiceSpecState,
    pub best_instance_count: u32,
    pub need_container: bool,

    pub required_cpu_mhz: u32,
    pub required_memory: u64,
    pub required_gpu_tflops: f32,
    pub required_gpu_mem: u64,
    // 亲和性规则
    pub node_affinity: Option<String>,
    pub network_affinity: Option<String>,

    pub service_ports_config: HashMap<String, u16>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum OPTaskBody {
    NodeInitBaseService,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum OPTaskState {
    New,
    Running,
    Done,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OPTask {
    pub id: String,
    pub creator_id: Option<String>, //为None说明创建者是Scheduler
    pub body: OPTaskBody,
    pub create_time: u64,
    pub create_step_id: u64,
    pub max_timeout_sec: u64,
    pub status: OPTaskState,
    pub start_time: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeResource {
    pub total_capacity: u64,
    pub used_capacity: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeType {
    OOD,
    Server,
    Desktop,       //PC + lattop
    Mobile,        //phone + tablet
    Sensor,        //sensor,
    IoTController, //iot controller
    UnknownClient(String),
}

impl From<String> for NodeType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "ood" => NodeType::OOD,
            "server" => NodeType::Server,
            "desktop" => NodeType::Desktop,
            "mobile" => NodeType::Mobile,
            "sensor" => NodeType::Sensor,
            "controller" => NodeType::IoTController,
            _ => NodeType::UnknownClient(s),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeItem {
    pub id: String,
    pub node_type: NodeType,
    //pub name: String,
    // 节点标签，用于亲和性匹配
    pub labels: Vec<String>,
    pub network_zone: String,
    pub support_container: bool,

    //Node的可变状态
    pub state: NodeState,

    pub available_cpu_mhz: u32,    //available mhz
    pub total_cpu_mhz: u32,        //total mhz
    pub available_memory: u64,     //bytes
    pub total_memory: u64,         //bytes
    pub available_gpu_memory: u64, //bytes
    pub total_gpu_memory: u64,     //bytes
    pub gpu_tflops: f32,           //tflops

    pub resources: HashMap<String, NodeResource>,
    pub op_tasks: Vec<OPTask>,
}

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize)]
pub enum NodeState {
    New,
    Prepare,     //new->ready的准备阶段
    Ready,       //正常可用
    Abnormal,    //存在异常
    Unavailable, //不可用
    Removing,    //正在移除
    Deleted,     //已删除
}

impl fmt::Display for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            NodeState::New => "New",
            NodeState::Prepare => "Prepare",
            NodeState::Ready => "Ready",
            NodeState::Abnormal => "Abnormal",
            NodeState::Unavailable => "Unavailable",
            NodeState::Removing => "Removing",
            NodeState::Deleted => "Deleted",
        };
        write!(f, "{}", status_str)
    }
}

impl From<String> for NodeState {
    fn from(s: String) -> Self {
        match s.as_str() {
            "New" => NodeState::New,
            "Prepare" => NodeState::Prepare,
            "Ready" => NodeState::Ready,
            "Abnormal" => NodeState::Abnormal,
            "Unavailable" => NodeState::Unavailable,
            "Removing" => NodeState::Removing,
            "Deleted" => NodeState::Deleted,
            _ => NodeState::New,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ReplicaInstance {
    pub node_id: String,
    pub spec_id: String, //service_name or app_id
    pub res_limits: HashMap<String, f64>,
    pub instance_id: String,
    // last_update_time 表示“最近一次可接受的存活证明时间”。
    // 它既可能来自真实 runtime 心跳，也可能来自启动期调度器注入的 bootstrap 假上报。
    // 这里刻意不把它等同于“服务自行声明的心跳时间”。
    pub last_update_time: u64,
    pub state: InstanceState,
    pub service_ports: HashMap<String, u16>,
}

impl ReplicaInstance {
    pub fn is_app_instance(&self) -> bool {
        self.spec_id.contains("@")
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum ServiceInfo {
    SingleInstance(ReplicaInstance),
    RandomCluster(HashMap<String, (u32, ReplicaInstance)>),
}

#[derive(Clone, Debug)]
pub enum SchedulerAction {
    ChangeNodeStatus(String, NodeState),
    CreateOPTask(OPTask),
    ChangeServiceStatus(String, ServiceSpecState),
    InstanceReplica(ReplicaInstance),
    UpdateInstance(String, ReplicaInstance),
    RemoveInstance(String, String, String), //value is spec_id,instance_id,node_id
    UpdateServiceInfo(String, ServiceInfo),
}
//spec_id@owner_id@node_id
pub fn parse_instance_id(instance_id: &str) -> Result<(String, String, String)> {
    let parts: Vec<&str> = instance_id.split('@').collect();
    if parts.len() == 3 {
        return Ok((
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
        ));
    }
    if parts.len() == 2 {
        return Ok((
            parts[0].to_string(),
            "root".to_string(),
            parts[1].to_string(),
        ));
    }
    Err(anyhow::anyhow!(
        "Invalid instance_id format: {}",
        instance_id
    ))
}

pub fn parse_spec_id(spec_id: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = spec_id.split('@').collect();
    if parts.len() == 2 {
        return Ok((parts[0].to_string(), parts[1].to_string()));
    }
    return Ok((spec_id.to_string(), "root".to_string()));
}

// app_id@user_id
// pub fn parse_app_instance_id(spec_id: &str) -> Result<(String, String)> {
//     let parts: Vec<&str> = spec_id.split('@').collect();
//     if parts.len() != 3 {
//         return Err(anyhow::anyhow!("Invalid spec_id format: {}", spec_id));
//     }
//     Ok((parts[0].to_string(), parts[1].to_string()))
// }

pub fn create_replica_instance_id(spec: &ServiceSpec, node_id: &str) -> String {
    format!("{}@{}", spec.id, node_id)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeScheduler {
    schedule_step_id: u64,
    pub users: HashMap<String, UserItem>,
    pub default_user_id: String,
    pub nodes: HashMap<String, NodeItem>,
    pub specs: HashMap<String, ServiceSpec>,

    pub replica_instances: HashMap<String, ReplicaInstance>,
    pub service_infos: HashMap<String, ServiceInfo>,
    pub schedule_time: u64,
}

impl NodeScheduler {
    pub fn new_empty(step_id: u64) -> Self {
        Self {
            schedule_step_id: step_id,
            users: HashMap::new(),
            default_user_id: "".to_string(),
            nodes: HashMap::new(),
            specs: HashMap::new(),
            replica_instances: HashMap::new(),
            service_infos: HashMap::new(),
            schedule_time: buckyos_get_unix_timestamp(),
        }
    }

    pub fn new(
        step_id: u64,
        users: HashMap<String, UserItem>,
        nodes: HashMap<String, NodeItem>,
        specs: HashMap<String, ServiceSpec>,
        replica_instances: HashMap<String, ReplicaInstance>,
        service_infos: HashMap<String, ServiceInfo>,
    ) -> Self {
        let now = buckyos_get_unix_timestamp();
        Self {
            schedule_step_id: step_id,
            default_user_id: "".to_string(),
            users,
            nodes,
            specs,
            replica_instances,
            service_infos,
            schedule_time: now,
        }
    }

    pub fn add_user(&mut self, user: UserItem) {
        if self.default_user_id.is_empty() {
            self.default_user_id = user.userid.clone();
        }
        self.users.insert(user.userid.clone(), user);
    }

    pub fn add_node(&mut self, node: NodeItem) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn add_service_spec(&mut self, spec: ServiceSpec) {
        self.specs.insert(spec.id.clone(), spec);
    }

    pub fn get_service_spec(&self, spec_id: &str) -> Option<&ServiceSpec> {
        self.specs.get(spec_id)
    }

    pub fn get_replica_instance(&self, instance_id: &str) -> Option<&ReplicaInstance> {
        self.replica_instances.get(instance_id)
    }

    pub fn add_replica_instance(&mut self, instance: ReplicaInstance) {
        let key = instance.instance_id.clone();
        self.replica_instances.insert(key, instance);
    }

    #[cfg(test)]
    pub fn remove_service_spec(&mut self, spec_id: &str) {
        self.specs.remove(spec_id);
    }

    #[cfg(test)]
    pub fn update_service_spec_state(&mut self, spec_id: &str, state: ServiceSpecState) {
        if let Some(spec) = self.specs.get_mut(spec_id) {
            spec.state = state;
        }
    }

    pub fn schedule(
        &mut self,
        last_snapshot: Option<&NodeScheduler>,
    ) -> Result<Vec<SchedulerAction>> {
        let mut actions = Vec::new();
        info!("-------------NODE--------------");
        for (node_id, node) in self.nodes.iter() {
            info!("- {}:{:?}", node_id, node);
        }
        info!("-------------SERVICE_SPEC--------------");
        for (spec_id, spec) in self.specs.iter() {
            info!("- {}:{:?}", spec_id, spec);
        }

        if self.nodes.is_empty() {
            return Err(anyhow::anyhow!("No nodes found"));
        }
        // Step1. review node (资源池)
        let is_small_system = self.nodes.len() <= SMALL_SYSTEM_NODE_COUNT;
        let node_actions = self.resort_nodes()?;
        actions.extend(node_actions);
        if !actions.is_empty() && is_small_system {
            // 为了降低复杂度,在系统规模较小时,如果第一个阶段存在动作,则跳过第二阶段,防止复杂度的重叠.
            // TODO:是否会带来问题?
            info!("Small system, skip review replica instance when have node actions");
            return Ok(actions);
        }

        // Step2. 处理service_spec的实例化与反实例化
        if self.is_spec_changed(last_snapshot) {
            debug!("spec changed, schedule spec change");
            let spec_actions = self.schedule_spec_change()?;
            actions.extend(spec_actions);
        }
        // Step3. 优化instance的资源使用

        // Step4. 计算service_info
        let service_spec_actions = self.calc_service_infos(last_snapshot)?;
        actions.extend(service_spec_actions);
        info!("-------------RESULT ACTIONS--------------");
        for action in actions.iter() {
            info!("- {:?}", action);
        }
        Ok(actions)
    }

    fn calc_service_infos(
        &mut self,
        last_snapshot: Option<&NodeScheduler>,
    ) -> Result<Vec<SchedulerAction>> {
        let now = buckyos_get_unix_timestamp();
        let mut actions = Vec::new();
        let last_service_infos = last_snapshot.map(|snapshot| &snapshot.service_infos);
        for (spec_id, spec) in self.specs.iter() {
            let mut info_map = HashMap::new();
            // if spec.spec_type == ServiceSpecType::App {
            //     self.service_infos.remove(spec_id);
            //     continue;
            // }

            for instance in self.replica_instances.values() {
                //info!("spec_id:{} instance:{:?}", spec_id, instance);
                if instance.state == InstanceState::Running && instance.spec_id == *spec_id {
                    // 调度器只基于“最近一次存活证明”决定是否继续对外发布 service_info。
                    // 这里允许存在检测误差：实例真实失联与调度器宣告下线之间不要求严格同步。（也无法做到)
                    // 对应用来说，调度器返回服务可用时，如果访问失败可以尝试重试
                    // 如果调度器返回服务不可用，应用使用服务的接口应直接返回失败
                    if now - instance.last_update_time < INSTANCE_ALIVE_TIME {
                        info_map.insert(instance.instance_id.clone(), (100, instance.clone()));
                    } else {
                        warn!(
                            "spec_id:{} instance:{} is not alive",
                            spec_id, instance.instance_id
                        );
                    }
                }
            }

            if info_map.is_empty() {
                warn!("spec_id:{} NO running instance", spec_id);
            }

            let new_info = if info_map.len() == 1 {
                let (_, instance) = info_map.values().next().unwrap();
                ServiceInfo::SingleInstance(instance.clone())
            } else {
                ServiceInfo::RandomCluster(info_map)
            };

            let old_info = last_service_infos.and_then(|infos| infos.get(spec_id));
            let is_need_update = match old_info {
                Some(old) => old != &new_info,
                None => true,
            };
            if is_need_update {
                actions.push(SchedulerAction::UpdateServiceInfo(
                    spec_id.clone(),
                    new_info.clone(),
                ));
                info!("spec_id:{} calc new service info: {:?}", spec_id, new_info);
            }
            self.service_infos.insert(spec_id.clone(), new_info);
        }
        Ok(actions)
    }

    pub fn resort_nodes(&mut self) -> Result<Vec<SchedulerAction>> {
        let mut node_actions = Vec::new();
        // TOD: 根据Node的status进行排序
        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
        for node_id in node_ids {
            if let Some(node) = self.nodes.get(&node_id) {
                // 1. 检查已有op tasks的完成情况，该部分可能会对可用Node进行一些标记
                self.check_node_op_tasks(node, &mut node_actions)?;
            }

            if let Some(node_mut) = self.nodes.get_mut(&node_id) {
                // 2. 处理新node的初始化
                match node_mut.state {
                    NodeState::New => {
                        // 由调度器控制node进入初始化准备状态
                        node_mut.state = NodeState::Prepare;
                        node_actions.push(SchedulerAction::ChangeNodeStatus(
                            node_mut.id.clone(),
                            NodeState::Prepare,
                        ));
                    }
                    NodeState::Removing => {
                        // TODO:由调度器控制node 处理移除任务
                        node_mut.state = NodeState::Deleted;
                        node_actions.push(SchedulerAction::ChangeNodeStatus(
                            node_mut.id.clone(),
                            NodeState::Deleted,
                        ));
                    }
                    _ => {}
                }
            }
        }

        Ok(node_actions)
    }

    pub fn schedule_spec_change(&mut self) -> Result<Vec<SchedulerAction>> {
        let mut scheduler_actions = Vec::new();
        // 影子资源账本：每次分配后扣减可用资源，防止同一轮调度中多个 spec 被分配到同一节点导致超卖
        let mut shadow_nodes: Vec<NodeItem> = self
            .nodes
            .values()
            .filter(|node| node.state == NodeState::Ready)
            .cloned()
            .collect();

        let spec_ids: Vec<String> = self.specs.keys().cloned().collect();

        for spec_id in spec_ids {
            let spec_snapshot = match self.specs.get(&spec_id) {
                Some(spec) => spec.clone(),
                None => continue,
            };
            match spec_snapshot.state {
                ServiceSpecState::New => {
                    let new_instances =
                        self.create_replica_instance(&spec_snapshot, &shadow_nodes)?;
                    for instance in &new_instances {
                        if let Some(node) = shadow_nodes.iter_mut().find(|n| n.id == instance.node_id) {
                            node.available_cpu_mhz = node
                                .available_cpu_mhz
                                .saturating_sub(spec_snapshot.required_cpu_mhz);
                            node.available_memory = node
                                .available_memory
                                .saturating_sub(spec_snapshot.required_memory);
                            node.available_gpu_memory = node
                                .available_gpu_memory
                                .saturating_sub(spec_snapshot.required_gpu_mem);
                        }
                    }
                    for instance in new_instances {
                        self.replica_instances
                            .insert(instance.instance_id.clone(), instance.clone());
                        scheduler_actions.push(SchedulerAction::InstanceReplica(instance));
                    }
                    //TODO:现在没有部署中的状态
                    if let Some(spec_mut) = self.specs.get_mut(&spec_id) {
                        spec_mut.state = ServiceSpecState::Deployed;
                    }
                    scheduler_actions.push(SchedulerAction::ChangeServiceStatus(
                        spec_id.clone(),
                        ServiceSpecState::Deployed,
                    ));
                }
                ServiceSpecState::Deleted => {
                    let instance_ids: Vec<String> = self
                        .replica_instances
                        .iter()
                        .filter(|(_, instance)| instance.spec_id == spec_id)
                        .map(|(instance_id, _)| instance_id.clone())
                        .collect();
                    if instance_ids.is_empty() {
                        warn!(
                            "spec_id:{} instance not found,no instance uninstance",
                            spec_id
                        );
                    }
                    for instance_id in instance_ids {
                        if let Some(instance) = self.replica_instances.remove(&instance_id) {
                            info!(
                                "will remove instance: {} @ node: {}, spec_id:{}",
                                instance.instance_id, &instance.node_id, &instance.spec_id
                            );
                            scheduler_actions.push(SchedulerAction::RemoveInstance(
                                instance.spec_id.clone(),
                                instance.instance_id.clone(),
                                instance.node_id.clone(),
                            ));
                        }
                    }
                    //TODO:现在没有删除中的状态
                    if let Some(spec_mut) = self.specs.get_mut(&spec_id) {
                        spec_mut.state = ServiceSpecState::Deleted;
                    }
                    scheduler_actions.push(SchedulerAction::ChangeServiceStatus(
                        spec_id.clone(),
                        ServiceSpecState::Deleted,
                    ));
                }
                _ => {}
            }
        }
        Ok(scheduler_actions)
    }

    fn is_spec_changed(&self, last_snapshot: Option<&NodeScheduler>) -> bool {
        let Some(last) = last_snapshot else {
            return true;
        };

        if self.specs.len() != last.specs.len() {
            return true;
        }

        for (spec_id, spec) in &self.specs {
            match last.specs.get(spec_id) {
                None => return true,
                Some(last_spec) => {
                    if spec.state != last_spec.state
                        || spec.required_cpu_mhz != last_spec.required_cpu_mhz
                        || spec.required_memory != last_spec.required_memory
                        || spec.node_affinity != last_spec.node_affinity
                        || spec.network_affinity != last_spec.network_affinity
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    // 辅助函数
    fn check_node_op_tasks(
        &self,
        node: &NodeItem,
        actions: &mut Vec<SchedulerAction>,
    ) -> Result<()> {
        // TODO: 实现检查node的op tasks完成情况的逻辑
        Ok(())
    }

    fn check_resource_limits(
        &self,
        instance: &ReplicaInstance,
        node: &NodeItem,
        actions: &mut Vec<SchedulerAction>,
    ) -> Result<()> {
        // TODO: 实现检查和更新资源限制的逻辑
        Ok(())
    }

    fn find_new_placement(
        &self,
        spec_id: &str,
        available_nodes: &[&NodeItem],
    ) -> Result<Vec<ReplicaInstance>> {
        // TODO: 实现查找新的部署位置的逻辑
        Ok(vec![])
    }

    fn alloc_replica_instance_port(
        &self,
        app_index: u16,
        service_name: &str,
        expose_port: Option<u16>,
    ) -> u16 {
        //TODO：调度器应该记录所有instance使用过的port,并保证不会返回已经使用过的port
        if service_name == "www" {
            if app_index == 0 {
                if expose_port.is_some() {
                    return expose_port.unwrap();
                }
                warn!(
                    "alloc_replica_instance_port: service_name: {} alloc instance port failed!",
                    service_name
                );
                return 0;
            }
            return app_index * 16 + BASE_APP_PORT;
        }
        if expose_port.is_some() {
            return expose_port.unwrap();
        }
        warn!(
            "alloc_replica_instance_port: service_name: {} alloc instance port failed!",
            service_name
        );
        return 0;
    }

    fn create_replica_instance(
        &self,
        service_spec: &ServiceSpec,
        node_list: &Vec<NodeItem>,
    ) -> Result<Vec<ReplicaInstance>> {
        // 1. 过滤阶段
        let candidate_nodes: Vec<&NodeItem> = node_list
            .iter()
            .filter(|node| self.filter_node_for_instance(node, service_spec))
            .collect();

        if candidate_nodes.is_empty() {
            return Err(anyhow::anyhow!("No suitable node found for service_spec"));
        }
        let candidate_node_count: u32 = candidate_nodes.len() as u32;

        // 2. 打分阶段
        let mut scored_nodes: Vec<(f64, &NodeItem)> = candidate_nodes
            .iter()
            .map(|node| (self.score_node(node, service_spec), *node))
            .collect();

        let instance_count: u32 =
            std::cmp::min(service_spec.best_instance_count, candidate_node_count);
        // 按分数降序排序
        scored_nodes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        // 选择得分最高的instance_count个节点
        let selected_nodes: Vec<(f64, &NodeItem)> = scored_nodes
            .iter()
            .take(instance_count as usize)
            .map(|&(score, node)| (score, node))
            .collect();

        let mut instances = Vec::new();
        let instance_id_uuid = uuid::Uuid::new_v4();

        //alloc instance service ports
        let mut service_ports = HashMap::new();
        for (service_name, expose_port) in service_spec.service_ports_config.iter() {
            let service_port = self.alloc_replica_instance_port(
                service_spec.app_index,
                service_name,
                Some(*expose_port),
            );
            service_ports.insert(service_name.clone(), service_port);
        }
        //TODO: 将port alloc的逻辑，放到调度器内部?
        for (_, node) in selected_nodes.iter() {
            instances.push(ReplicaInstance {
                node_id: node.id.clone(),
                spec_id: service_spec.id.clone(),
                res_limits: HashMap::new(),
                instance_id: create_replica_instance_id(service_spec, node.id.as_str()),
                // 启动期第一次调度时，很多服务还不能可靠依赖 service_info 上报链路。
                // 为了避免循环依赖，调度器在实例化时先注入一次 bootstrap 存活证明，
                // 让 service_info 能先被构造出来；后续再由真实心跳接管该时间戳。
                last_update_time: buckyos_get_unix_timestamp(),
                state: InstanceState::Running,
                service_ports: service_ports.clone(),
            });
        }
        Ok(instances)
    }

    //关键函数:根据service_spec的配置,过滤符合条件的node
    fn filter_node_for_instance(&self, node: &NodeItem, spec: &ServiceSpec) -> bool {
        // 1. 检查节点状态
        if node.state != NodeState::Ready {
            return false;
        }

        if node.node_type != NodeType::OOD && node.node_type != NodeType::Server {
            return false;
        }

        if spec.need_container && !node.support_container {
            return false;
        }

        // 2. 检查可用资源是否充足（available 而非 total，配合影子账本防止同轮超卖）
        if node.available_cpu_mhz < spec.required_cpu_mhz
            || node.available_memory < spec.required_memory
        {
            return false;
        }

        if node.available_gpu_memory < spec.required_gpu_mem
            || node.gpu_tflops < spec.required_gpu_tflops as f32
        {
            return false;
        }

        // 3. 检查亲和性
        if let Some(affinity) = &spec.node_affinity {
            if !node.labels.contains(affinity) {
                return false;
            }
        }

        true
    }

    fn score_node(&self, node: &NodeItem, spec: &ServiceSpec) -> f64 {
        let mut score = 0.0;

        // 1. 资源充足度评分 (分配后剩余比例越高，得分越高)
        if node.total_cpu_mhz > 0 {
            let cpu_ratio =
                (node.available_cpu_mhz as f64 - spec.required_cpu_mhz as f64) / node.total_cpu_mhz as f64;
            let mem_ratio =
                (node.available_memory as f64 - spec.required_memory as f64) / node.total_memory.max(1) as f64;
            score += (cpu_ratio + mem_ratio) / 2.0 * 100.0;
        }

        // 2. 负载均衡评分 (已用比例越低，得分越高——倾向 LeastAllocated / Spreading)
        if node.total_cpu_mhz > 0 {
            let idle_ratio = node.available_cpu_mhz as f64 / node.total_cpu_mhz as f64;
            score += idle_ratio * 50.0;
        }

        // 3. 网络亲和性评分
        if let Some(network_affinity) = &spec.network_affinity {
            if node.network_zone == *network_affinity {
                score += 30.0;
            }
        }

        score
    }
}
