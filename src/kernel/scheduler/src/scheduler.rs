/*
这是一个平台无关的调度器基于一个确定性的系统状态（通常来自etcd上的信息），执行一次调度，得到一组新状态+OPTask，然后等待执行后（系统状态发生改变），
再进入下一次调度。调度本身是幂等的（一个调度结果不会依赖某个隐藏的状态），但大部分时候，调度器通常会依赖“上一次调度行为的具体操作结果”
这个调度器将易于构造海量的测试。在这个基础只上，另一个团队可以专注于执行器的开发，专心完成调度器给定的指令，并构建调度器所依赖的系统状态。

该设计最重要的一点就是定义参与调度的实体和可执行的调度动作，并坚持less is more，很多功能的添加只会让系统变的更复杂更不稳定。
基于长期的工业DFS和CDN的开发经验，调度器绝对不应该去管理这两个设施。这两个设施作为系统可用性和可靠性的基石，有另外的分布式模式。

通用调度器被称作NodeScheduler,其核心逻辑如下:

定义被调度对象 (ServiceSpec)
定义可使用的资源载体 (Node)
定义用户
定义树状的资源组
定义调度任务(OPTask),实现一些必要的Node和ReplicaInstance的维护工作
通过对上诉对象的修改，定义运维可以执行的”改变系统的操作“

系统级管理
    重启系统
    暂停系统（通常保留OPTask的执行能力）
    启动系统
    关闭系统
    系统备份（不停机）
    系统恢复
        新系统恢复
        系统先暂停，再执行恢复（引导至恢复模式？）

用户管理
    添加用户
    调整用户的资源池
    删除用户：释放属于用户的所有服务（不会立刻执行）
    停用用户：停止属于用户的所有服务
    启用用户：恢复属于用户的所有服务

实现Node管理调度算法
    识别新添加的Node，加入系统资源池。
        通过可扩展的OPTask，实现特定节点的初始化运维工作，减少集群维护负担（所有的Node只需要完成最最基本的初始化工作就可以接入集群，上架简单）
    Node 日常体检
        可以指定计划任务，在合适的时机执行深度的体检OPTask(一般是做数据健康检查或性能测试)。
    Node需要维护（暂停）操作，Node上的数据不会丢失
        排空Node，Instance有序迁移或直接释放，系统可用资源减少
        下发运维任务，并等待完成（比如升级内核）
        Node断电，进行物理维护
        Node从维护状态恢复 （此时单Node的资源可能发生了改变）
    删除(是否)Node，Node在系统中删除
        在删除前会根据Node的能力提示是否会导致系统不可用
    替换Node：
        先临时将Node上的数据通过OPTask迁移到其他(1个或多个)Node上
        再通过物理操作更换Node
        更换后，通过OPTask将数据迁移回新Node
    区域维护：
        当Node有物理分区(lanid)时，可以基于区域对Node进行隔离维护

实现ServiceSpec 管理调度管理
    添加ServiceSpec，实例化调度算法：
        分配资源：构造Instance，将未部署的ServiceSpec绑定到Node上更新ServiceSpec,调度器调整instance
    删除ServiceSpec,调度器及时释放instance
    暂停ServiceSpec，调度器及时停止instance的运行(释放资源)
    允许导入Spec拓扑图，对Spec的实例化进行具体的指导。

Instance管理 （自动负载均衡）
    动态调整：根据运行情况，系统资源剩余情况，相对动态的调整ServiceSpec能用的资源
    优先级管理：根据运行情况，临时暂停部分instance的运行，保证高优先级instance的运行
    自动错误隔离：当系统发现大量错误时，能自动隔离错误节点，保证系统的稳定性

运维event管理（异步调度）
    允许调度器在某些情况下下发运维event，并等待完成人的决策
    人的决策完成后，本次调度基于决策结果继续。
    一些重要决策，默认是倒计时决策，当人没有在规定的时间里否决该决策时，该决策自动通过。


状态管理：
    调度器所依赖的数据保存在etcd上，调度器不需要管理etcd的可靠性
    大部分Replica是无状态的，只在Node上保存Cache数据
        LocalCache: 保存在Node上的缓存数据，可以在磁盘空间不足的时候释放
    有状态的ReplicaInstance的主要
        Data：保存在DFS上，起可靠性和热点迁移性不需要调度器考虑
        LocalData: 保存在Node上的数据，当ReplicaInstance被迁移时，需要转移到新的Node上。但从原理上说，大部分ReplicaInstance的LocalData是可以从其他Instance恢复的，并不强依赖LocalData的迁移


构建的是**纯函数式（Pure Functional）**的调度器核心。
这种架构的核心优势在于：

- 确定性（Determinism）：给定 Input (Snapshot)，必然得到 Output (Actions)。排查 Bug 只需要 Input 数据。
- 极高的测试覆盖率：不需要 Mock ETCD，不需要 Mock 网络，只需要构造内存 Struct。
- 开发解耦：调度器团队只关心逻辑，执行器团队只关心“如何把动作落地”。

schedule loops:
收集系统信息可以得到 系统当前状态（当前快照）
执行一次调度（上一次调度成功的系统状态，当前系统状态，得到 Action + OPTaskList
    Action是对系统状态的立刻改变（构造Action时，通常也会立刻修改调度器的内部状态）：在一个正常的系统里，这个改变会立刻成功
        `完成后调度器的状态` 等于 `SystemConfig Apply Action后->重新构造调度的状态`
    OPTask是一个需要时间的，可跟踪的Task,在调度器看来，其状态也是系统当前状态的一部分
调度完成后，系统需要立刻用事务执行Action，并保存执行完成后的系统快照

调度器的工作核心基本是围绕Instance来的
    - 按优先级ReviewInstance的资源占用
    - 确保所有的，运行中的ServiceSpec都有合适的Instance在运行（至少1个）
    - 及时发现Instance的负载变高，并反应到ServiceInfo的权重上去：是系统计算权重，还是给出原始信息让应用有机会自己算？
    - 及时发现Instance的故障，并反应到ServiceInfo上去

一些工作原则：
    - 最小改动原则，尽量不改动已有的Instance（可以增加，少删除或修改)


基于上述设计，对系统做控制的方法
    - 绝对不要修改Instance和ServiceInfo,这个都是调度器来修改的
    - 修改ServiceSpec: 在系统中调整服务的“目标状态“（注意不删除原则，只是修改状态）
    - 修改Node：在系统中调整Node的“目标状态“（注意不删除原则，只是修改状态）
        - 增加/减少 Node
        - 调整Node和逻辑Device的关系
        - 调整Node的目标状态


ServiceInfo:
- 大型系统中的流量控制，通常都是由运维手工操作完成的，因此调度器不会去尝试构造流量控制的逻辑
- 系统使用最短路径进行访问，因此调度器只需要说明“服务的EndPoint”在哪，非调度器逻辑就会为所有的访问者，按固定规则创建访问路径
    - 调度器专注于基于 Instance(with Report Info) 来构造ServiceInfo
    - 外部逻辑根据ServiceInfo来完成确定的访问控制


基于调度器状态机的一些复杂场景思考
- 用户反复的添加/删除 同一个id ,单有不同配置的ServiceSpec:
在下一个调度器触发时，调度器只会按其触发的那一瞬间的状态 结合上一次调度时的状态 来进行处理，而无视中间状态
UI在做操作后，可以看到的状态是“New”（等待部署） 和 “Deleted”（已删除）

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
        let valid_nodes: Vec<NodeItem> = self
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
                        self.create_replica_instance(&spec_snapshot, &valid_nodes)?;
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

        // 2. 检查资源是否充足
        if node.total_cpu_mhz < spec.required_cpu_mhz || node.total_memory < spec.required_memory {
            return false;
        }

        if node.total_gpu_memory < spec.required_gpu_mem
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

    //关键函数:根据service_spec的配置,对node进行打分
    fn score_node(&self, node: &NodeItem, spec: &ServiceSpec) -> f64 {
        let mut score = 0.0;

        // 1. 资源充足度评分
        let cpu_score = (node.available_cpu_mhz - spec.required_cpu_mhz) / node.total_cpu_mhz;
        let memory_score = (node.available_memory - spec.required_memory) / node.total_memory;
        score += (cpu_score as f64 + memory_score as f64) / 2.0 * 100.0;

        // 2. 节点负载均衡评分
        let load_score = 1.0 - (node.available_cpu_mhz / node.total_cpu_mhz) as f64;
        score += load_score * 50.0;

        // 3. 网络亲和性评分
        if let Some(network_affinity) = &spec.network_affinity {
            if node.network_zone == *network_affinity {
                score += 30.0;
            }
        }

        // 4. 节点健康评分
        //let health_score = if node.is_healthy { 20.0 } else { 0.0 };
        //score += health_score;

        // 可以添加更多评分规则
        // - 节点存储评分 (关键设计!)
        // - 节点地理位置评分 (异地机房)

        score
    }
}
