/*
通用调度器被称作PodScheduler,其核心逻辑如下:

定义被调度对象 (ServiceSpec)
定义可使用的资源载体 (Node)
定义调度任务，实现Instance的迁移
实现Node调度算法
    识别新Node，并加入系统资源池
    排空Node，系统可用资源减少
    释放Node，Node在系统中删除

实现ServiceSpec调度算法
    实例化调度算法：分配资源：构造Instance，将未部署的ServiceSpec绑定到Node上
    反实例化：及时释放资源
    动态调整：根据运行情况，系统资源剩余情况，相对动态的调整ServiceSpec能用的资源
    动态调整也涉及到实例的迁移
*/
#[warn(unused, unused_mut, dead_code)]
use anyhow::Result;
use buckyos_kit::buckyos_get_unix_timestamp;

use log::*;
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

const SMALL_SYSTEM_NODE_COUNT: usize = 7;
const POD_INSTANCE_ALIVE_TIME: u64 = 90;

#[derive(Clone, PartialEq, Debug)]
pub enum ServiceSpecType {
    Kernel, //kernel service
    Service, //无状态的系统服务
    App,     // 无状态的app服务
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

#[derive(Clone, PartialEq, Debug)]
pub enum ServiceSpecState {
    New,
    Deploying,
    Deployed,
    DeployFailed,
    Abnormal,
    Unavailable,
    Removing,
    Deleted,
}

#[derive(Clone, PartialEq, Debug)]
pub enum InstanceState {
    New,
    Running,
}

impl From<String> for InstanceState {
    fn from(s: String) -> Self {
        let s = s.to_lowercase();
        match s.as_str() {
            "new" => InstanceState::New,
            "running" => InstanceState::Running,
            _ => InstanceState::New,
        }
    }
}

impl ToString for InstanceState {
    fn to_string(&self) -> String {
        match self {
            InstanceState::New => "new".to_string(),
            InstanceState::Running => "running".to_string(),
        }
    }
}

impl fmt::Display for ServiceSpecState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            ServiceSpecState::New => "New",
            ServiceSpecState::Deploying => "Deploying",
            ServiceSpecState::Deployed => "Deployed",
            ServiceSpecState::DeployFailed => "DeployFailed",
            ServiceSpecState::Abnormal => "Abnormal",
            ServiceSpecState::Unavailable => "Unavailable",
            ServiceSpecState::Removing => "Removing",
            ServiceSpecState::Deleted => "Deleted",
        };
        write!(f, "{}", status_str)
    }
}

impl From<String> for ServiceSpecState {
    fn from(s: String) -> Self {
        match s.as_str() {
            "New" => ServiceSpecState::New,
            "Deploying" => ServiceSpecState::Deploying,
            "Deployed" => ServiceSpecState::Deployed,
            "DeployFailed" => ServiceSpecState::DeployFailed,
            "Abnormal" => ServiceSpecState::Abnormal,
            "Unavailable" => ServiceSpecState::Unavailable,
            "Removing" => ServiceSpecState::Removing,
            "Deleted" => ServiceSpecState::Deleted,
            _ => ServiceSpecState::Unavailable,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
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

pub struct UserItem {
    pub userid: String,
    pub user_type: UserType,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ServiceSpec {
    pub id: String,
    pub app_id:String,
    pub owner_id:String,
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
    pub default_service_port: u16,
}
#[derive(Clone, Debug)]
pub enum OPTaskBody {
    NodeInitBaseService,
}

#[derive(Clone, Debug)]
pub enum OPTaskState {
    New,
    Running,
    Done,
    Failed,
}
#[derive(Clone, Debug)]
pub struct OPTask {
    pub id: String,
    pub creator_id: Option<String>, //为None说明创建者是Scheduler
    pub owner_id: String,           // owner id 可以是node_id,也可以是pod_id
    pub status: OPTaskState,
    pub create_time: u64,
    pub create_step_id: u64,
    pub body: OPTaskBody,
}

#[derive(Clone, Debug)]
pub struct NodeResource {
    pub total_capacity: u64,
    pub used_capacity: u64,
}

#[derive(Clone, Debug,PartialEq)]
pub enum NodeType {
    OOD,
    Server,
    Desktop,//PC + lattop
    Mobile,//phone + tablet
    Sensor,//sensor,
    IoTController,//iot controller
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

#[derive(Clone, Debug)]
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

#[derive(PartialEq, Clone, Debug)]
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



#[derive(Clone,PartialEq,Debug)]
pub struct ReplicaInstance {
    pub node_id: String,
    pub spec_id: String,//service_name or app_id
    pub res_limits: HashMap<String, f64>,
    pub instance_id: String,//format!("{}-{}", service_name, instance_node_id
    pub last_update_time: u64,
    pub state : InstanceState,
    pub service_port: u16,
}

#[derive(Clone,PartialEq)]
pub enum ServiceInfo {
    //instance_id: format!("{}_{}", service_spec.id, instance_id_uuid) => (weight,Instance)
    RandomCluster(HashMap<String,(u32,ReplicaInstance)>),
}

#[derive(Clone)]
pub enum SchedulerAction {
    ChangeNodeStatus(String, NodeState),
    CreateOPTask(OPTask),
    ChangeServiceStatus(String, ServiceSpecState),
    InstanceReplica(ReplicaInstance),
    UpdateInstance(String, ReplicaInstance),
    RemoveInstance(String,String,String), //value is spec_id,instance_id,node_id
    UpdateServiceInfo(String, ServiceInfo),
}
//spec_id@node_id
pub fn parse_instance_id(instance_id: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = instance_id.split('@').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid instance_id format: {}",
            instance_id
        ));
    }
    //return parts1's before '_'
    let part2 = parts[1].split('_').nth(0).unwrap().to_string();
    Ok((parts[0].to_string(), part2))
}

// app_id@user_id
pub fn parse_app_service_id(spec_id: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = spec_id.split('@').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!("Invalid spec_id format: {}", spec_id));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

pub struct NodeScheduler {
    schedule_step_id: u64,
    last_schedule_time: u64,
    pub users: HashMap<String, UserItem>,
    pub nodes: HashMap<String, NodeItem>,
    pub specs: HashMap<String, ServiceSpec>,
    // 系统里所有的Instance,key是instance_id (podid@nodeid)
    replica_instances: HashMap<String, ReplicaInstance>,
    service_infos: HashMap<String, ServiceInfo>,//current pod_infos

    last_specs: HashMap<String, ServiceSpec>,
}

impl NodeScheduler {
    pub fn new_empty(step_id: u64, last_schedule_time: u64) -> Self {
        Self {
            schedule_step_id: step_id,
            last_schedule_time,
            users: HashMap::new(),
            nodes: HashMap::new(),
            specs: HashMap::new(),
            replica_instances: HashMap::new(),
            service_infos: HashMap::new(),
            last_specs: HashMap::new(),
        }
    }

    pub fn new(
        step_id: u64,
        last_schedule_time: u64,
        users: HashMap<String, UserItem>,
        nodes: HashMap<String, NodeItem>,
        specs: HashMap<String, ServiceSpec>,
        replica_instances: HashMap<String, ReplicaInstance>,
        last_specs: HashMap<String, ServiceSpec>,
        service_infos: HashMap<String, ServiceInfo>,
    ) -> Self {
        Self {
            schedule_step_id: step_id,
            last_schedule_time,
            users,
            nodes,
            specs,
            replica_instances,
            service_infos,
            last_specs,
        }
    }

    pub fn add_user(&mut self, user: UserItem) {
        self.users.insert(user.userid.clone(), user);
    }

    pub fn add_node(&mut self, node: NodeItem) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn add_service_spec(&mut self, pod: ServiceSpec) {
        self.specs.insert(pod.id.clone(), pod);
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

    pub fn schedule(&mut self) -> Result<Vec<SchedulerAction>> {
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

        // Step0. 根据运行中的instance，更新service_info
  
        

        // Step1. review node (资源池)
        let is_small_system = self.nodes.len() <= SMALL_SYSTEM_NODE_COUNT;
        let node_actions = self.resort_nodes()?;
        actions.extend(node_actions);
        if !actions.is_empty() && is_small_system {
            // 为了降低复杂度,在系统规模较小时,如果第一个阶段存在动作,则跳过第二阶段,防止复杂度的重叠.
            // TODO:是否会带来问题?
            info!("Small system, skip schedule pod instance when have node actions");
            return Ok(actions);
        }

        // Step2. 处理service_spec的实例化与反实例化
        if self.is_pod_changed() {
            let pod_actions = self.schedule_pod_change()?;
            actions.extend(pod_actions);
        }
        // Step3. 优化instance的资源使用

        let pod_service_actions =  self.calc_pod_service_infos()?;
        actions.extend(pod_service_actions);
        Ok(actions)
    }

    fn calc_pod_service_infos(&mut self) -> Result<Vec<SchedulerAction>> {
        let now = buckyos_get_unix_timestamp();
        let mut actions = Vec::new();
        for (spec_id, spec) in self.specs.iter() {
            let mut info_map = HashMap::new();
            if spec.spec_type == ServiceSpecType::App {
                continue;
            }

            for instance in self.replica_instances.values() {
                //info!("spec_id:{} instance:{:?}", spec_id, instance);
                if instance.state == InstanceState::Running && 
                   instance.spec_id == *spec_id {
                    if now - instance.last_update_time < POD_INSTANCE_ALIVE_TIME {
                        info_map.insert(instance.instance_id.clone(), (100,instance.clone()));
                    } else {
                        warn!("spec_id:{} instance:{} is not alive", spec_id, instance.instance_id);
                    }
                }
            }

            if info_map.is_empty() {
                warn!("spec_id:{} NO running instance", spec_id);
            }

            let new_info = ServiceInfo::RandomCluster(info_map);
            let old_info = self.service_infos.get(spec_id);
            let mut is_need_update = false;
            if old_info.is_none() {
                is_need_update = true;
            } else {
                let old_info = old_info.unwrap();
                if old_info != &new_info {
                    is_need_update = true;
                }
            }
            if is_need_update {
                actions.push(SchedulerAction::UpdateServiceInfo(spec_id.clone(), new_info));
                info!("spec_id:{} calc new service info", spec_id);
            }
            
        }
        Ok(actions)
    }

    pub fn resort_nodes(&mut self) -> Result<Vec<SchedulerAction>> {
        let mut node_actions = Vec::new();
        // TOD: 根据Node的status进行排序
        for node in self.nodes.values() {
            // 1. 检查已有op tasks的完成情况，该部分可能会对可用Node进行一些标记
            self.check_node_op_tasks(node, &mut node_actions)?;

            // 2. 处理新node的初始化
            match node.state {
                NodeState::New => {
                    // 由调度器控制node进入初始化准备状态
                    node_actions.push(SchedulerAction::ChangeNodeStatus(
                        node.id.clone(),
                        NodeState::Prepare,
                    ));
                }
                NodeState::Removing => {
                    // TODO:由调度器控制node 处理移除任务
                    //
                    node_actions.push(SchedulerAction::ChangeNodeStatus(
                        node.id.clone(),
                        NodeState::Deleted,
                    ));
                }
                _ => {}
            }
        }

        Ok(node_actions)
    }

    pub fn schedule_pod_change(&mut self) -> Result<Vec<SchedulerAction>> {
        let mut pod_actions = Vec::new();
        let valid_nodes: Vec<NodeItem> = self.nodes.values().cloned().collect();
        let valid_nodes: Vec<NodeItem> = valid_nodes
            .iter()
            .filter(|node| node.state == NodeState::Ready)
            .cloned()
            .collect();

        for (spec_id, spec) in &self.specs {
            match spec.state {
                ServiceSpecState::New => {
                    let new_instances = self.instance_pod(spec, &valid_nodes)?;
                    for instance in new_instances {
                        pod_actions.push(SchedulerAction::InstanceReplica(instance));
                    }
                    //TODO:现在没有部署中的状态
                    pod_actions.push(SchedulerAction::ChangeServiceStatus(
                        spec_id.clone(),
                        ServiceSpecState::Deployed,
                    ));
                }
                ServiceSpecState::Removing => {
                    let mut is_moved = false;
                    for instance in self.replica_instances.values() {
                        if instance.spec_id == format!("{}@{}", spec.app_id, spec.owner_id) {
                            info!("will remove instance: {},spec_id:{}", instance.instance_id, &instance.spec_id);
                            is_moved = true;
                            pod_actions.push(SchedulerAction::RemoveInstance(
                                instance.spec_id.clone(),
                                instance.instance_id.clone(),
                                instance.node_id.clone(),
                            ));
                        }
                    }
                    if !is_moved {
                        warn!("spec_id:{} instance not found,no instance uninstance", spec_id);
                    }
                    //TODO:现在没有删除中的状态
                    pod_actions.push(SchedulerAction::ChangeServiceStatus(
                        spec_id.clone(),
                        ServiceSpecState::Deleted,
                    ));
                }
                _ => {}
            }
        }
        Ok(pod_actions)
    }

    fn is_pod_changed(&self) -> bool {
        if self.specs.len() != self.last_specs.len() {
            return true;
        }

        for (spec_id, spec) in &self.specs {
            match self.last_specs.get(spec_id) {
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

    fn instance_pod(
        &self,
        service_spec: &ServiceSpec,
        node_list: &Vec<NodeItem>,
    ) -> Result<Vec<ReplicaInstance>> {
        // 1. 过滤阶段
        let candidate_nodes: Vec<&NodeItem> = node_list
            .iter()
            .filter(|node| self.filter_node_for_pod_instance(node, service_spec))
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

        let instance_count: u32 = std::cmp::min(service_spec.best_instance_count, candidate_node_count);
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
        for (_, node) in selected_nodes.iter() {
            instances.push(ReplicaInstance {
                node_id: node.id.clone(),
                spec_id: service_spec.id.clone(),
                res_limits: HashMap::new(),
                instance_id: format!("{}_{}", service_spec.id, instance_id_uuid),
                last_update_time: 0,
                state: InstanceState::Running,
                service_port: service_spec.default_service_port,
            });
        }
        Ok(instances)
    }

    //关键函数:根据service_spec的配置,过滤符合条件的node
    fn filter_node_for_pod_instance(&self, node: &NodeItem, pod: &ServiceSpec) -> bool {
        // 1. 检查节点状态
        if node.state != NodeState::Ready {
            return false;
        }

        if node.node_type != NodeType::OOD && node.node_type != NodeType::Server {
            return false;
        }

        if pod.need_container && !node.support_container {
            return false;
        }

        // 2. 检查资源是否充足
        if node.total_cpu_mhz < pod.required_cpu_mhz || node.total_memory < pod.required_memory {
            return false;
        }

        if node.total_gpu_memory < pod.required_gpu_mem
            || node.gpu_tflops < pod.required_gpu_tflops as f32
        {
            return false;
        }

        // 3. 检查亲和性
        if let Some(affinity) = &pod.node_affinity {
            if !node.labels.contains(affinity) {
                return false;
            }
        }

        true
    }

    //关键函数:根据service_spec的配置,对node进行打分
    fn score_node(&self, node: &NodeItem, pod: &ServiceSpec) -> f64 {
        let mut score = 0.0;

        // 1. 资源充足度评分
        let cpu_score = (node.available_cpu_mhz - pod.required_cpu_mhz) / node.total_cpu_mhz;
        let memory_score = (node.available_memory - pod.required_memory) / node.total_memory;
        score += (cpu_score as f64 + memory_score as f64) / 2.0 * 100.0;

        // 2. 节点负载均衡评分
        let load_score = 1.0 - (node.available_cpu_mhz / node.total_cpu_mhz) as f64;
        score += load_score * 50.0;

        // 3. 网络亲和性评分
        if let Some(network_affinity) = &pod.network_affinity {
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
