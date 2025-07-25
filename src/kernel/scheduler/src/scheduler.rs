/*
通用调度器被称作PodScheduler,其核心逻辑如下:

定义被调度对象 (Pod)
定义可使用的资源载体 (Node)
定义调度任务，实现Instance的迁移
实现Node调度算法
    识别新Node，并加入系统资源池
    排空Node，系统可用资源减少
    释放Node，Node在系统中删除

实现Pod调度算法
    实例化调度算法：分配资源：构造PodInstance，将未部署的Pod绑定到Node上
    反实例化：及时释放资源
    动态调整：根据运行情况，系统资源剩余情况，相对动态的调整Pod能用的资源
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
pub enum PodItemType {
    Kernel, //kernel service
    Service, //无状态的系统服务
    App,     // 无状态的app服务
}

impl From<String> for PodItemType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "kernel" => PodItemType::Kernel,
            "service" => PodItemType::Service,
            "frame" => PodItemType::Service,
            "app" => PodItemType::App,
            _ => PodItemType::App,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum PodItemState {
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
pub enum PodInstanceState {
    New,
    Running,
}

impl From<String> for PodInstanceState {
    fn from(s: String) -> Self {
        let s = s.to_lowercase();
        match s.as_str() {
            "new" => PodInstanceState::New,
            "running" => PodInstanceState::Running,
            _ => PodInstanceState::New,
        }
    }
}

impl ToString for PodInstanceState {
    fn to_string(&self) -> String {
        match self {
            PodInstanceState::New => "new".to_string(),
            PodInstanceState::Running => "running".to_string(),
        }
    }
}

impl fmt::Display for PodItemState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            PodItemState::New => "New",
            PodItemState::Deploying => "Deploying",
            PodItemState::Deployed => "Deployed",
            PodItemState::DeployFailed => "DeployFailed",
            PodItemState::Abnormal => "Abnormal",
            PodItemState::Unavailable => "Unavailable",
            PodItemState::Removing => "Removing",
            PodItemState::Deleted => "Deleted",
        };
        write!(f, "{}", status_str)
    }
}

impl From<String> for PodItemState {
    fn from(s: String) -> Self {
        match s.as_str() {
            "New" => PodItemState::New,
            "Deploying" => PodItemState::Deploying,
            "Deployed" => PodItemState::Deployed,
            "DeployFailed" => PodItemState::DeployFailed,
            "Abnormal" => PodItemState::Abnormal,
            "Unavailable" => PodItemState::Unavailable,
            "Removing" => PodItemState::Removing,
            "Deleted" => PodItemState::Deleted,
            _ => PodItemState::Unavailable,
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
pub struct PodItem {
    pub id: String,
    pub app_id:String,
    pub owner_id:String,
    pub pod_type: PodItemType,
    pub state: PodItemState,
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
pub struct PodInstance {
    pub node_id: String,
    pub pod_id: String,
    pub res_limits: HashMap<String, f64>,
    pub instance_id: String,
    pub last_update_time: u64,
    pub state : PodInstanceState,
    pub service_port: u16,
}

#[derive(Clone,PartialEq)]
pub enum PodInfo {
    //instance_id: format!("{}_{}", pod_item.id, instance_id_uuid) => (weight,PodInstance)
    RandomCluster(HashMap<String,(u32,PodInstance)>),
}

#[derive(Clone)]
pub enum SchedulerAction {
    ChangeNodeStatus(String, NodeState),
    CreateOPTask(OPTask),
    ChangePodStatus(String, PodItemState),
    InstancePod(PodInstance),
    UpdatePodInstance(String, PodInstance),
    RemovePodInstance(String), //value is pod_id@node_id
    UpdatePodServiceInfo(String, PodInfo),
}
//pod_id@node_id
pub fn parse_instance_id(instance_id: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = instance_id.split('@').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid instance_id format: {}",
            instance_id
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

// app_id@user_id
pub fn parse_app_pod_id(pod_id: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = pod_id.split('@').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!("Invalid pod_id format: {}", pod_id));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

pub struct PodScheduler {
    schedule_step_id: u64,
    last_schedule_time: u64,
    pub users: HashMap<String, UserItem>,
    pub nodes: HashMap<String, NodeItem>,
    pub pods: HashMap<String, PodItem>,
    // 系统里所有的PodInstance,key是instance_id (podid@nodeid)
    pod_instances: HashMap<String, PodInstance>,
    pod_infos: HashMap<String, PodInfo>,//current pod_infos

    last_pods: HashMap<String, PodItem>,
}

impl PodScheduler {
    pub fn new_empty(step_id: u64, last_schedule_time: u64) -> Self {
        Self {
            schedule_step_id: step_id,
            last_schedule_time,
            users: HashMap::new(),
            nodes: HashMap::new(),
            pods: HashMap::new(),
            pod_instances: HashMap::new(),
            pod_infos: HashMap::new(),
            last_pods: HashMap::new(),
        }
    }

    pub fn new(
        step_id: u64,
        last_schedule_time: u64,
        users: HashMap<String, UserItem>,
        nodes: HashMap<String, NodeItem>,
        pods: HashMap<String, PodItem>,
        pod_instances: HashMap<String, PodInstance>,
        last_pods: HashMap<String, PodItem>,
        pod_infos: HashMap<String, PodInfo>,
    ) -> Self {
        Self {
            schedule_step_id: step_id,
            last_schedule_time,
            users,
            nodes,
            pods,
            pod_instances,
            pod_infos,
            last_pods,
        }
    }

    pub fn add_user(&mut self, user: UserItem) {
        self.users.insert(user.userid.clone(), user);
    }

    pub fn add_node(&mut self, node: NodeItem) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn add_pod(&mut self, pod: PodItem) {
        self.pods.insert(pod.id.clone(), pod);
    }

    pub fn get_pod_item(&self, pod_id: &str) -> Option<&PodItem> {
        self.pods.get(pod_id)
    }

    pub fn get_pod_instance(&self, pod_id: &str) -> Option<&PodInstance> {
        self.pod_instances.get(pod_id)
    }

    pub fn add_pod_instance(&mut self, instance: PodInstance) {
        let key = format!("{}@{}", instance.pod_id, instance.node_id);
        self.pod_instances.insert(key, instance);
    }

    #[cfg(test)]
    pub fn remove_pod(&mut self, pod_id: &str) {
        self.pods.remove(pod_id);
    }

    #[cfg(test)]
    pub fn update_pod_state(&mut self, pod_id: &str, state: PodItemState) {
        if let Some(pod) = self.pods.get_mut(pod_id) {
            pod.state = state;
        }
    }

    pub fn schedule(&mut self) -> Result<Vec<SchedulerAction>> {
        let mut actions = Vec::new();
        info!("-------------NODE--------------");
        for (node_id, node) in self.nodes.iter() {
            info!("- {}:{:?}", node_id, node);
        }
        info!("-------------POD--------------");
        for (pod_id, pod) in self.pods.iter() {
            info!("- {}:{:?}", pod_id, pod);
        }

        if self.nodes.is_empty() {
            return Err(anyhow::anyhow!("No nodes found"));
        }

        // Step0. 根据运行中的pod_instance，更新service_info
  
        

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

        // Step2. 处理pod的实例化与反实例化
        if self.is_pod_changed() {
            let pod_actions = self.schedule_pod_change()?;
            actions.extend(pod_actions);
        }
        // Step3. 优化pod_instance的资源使用

        let pod_service_actions =  self.calc_pod_service_infos()?;
        actions.extend(pod_service_actions);
        Ok(actions)
    }

    fn calc_pod_service_infos(&mut self) -> Result<Vec<SchedulerAction>> {
        let now = buckyos_get_unix_timestamp();
        let mut actions = Vec::new();
        for (pod_id, pod) in self.pods.iter() {
            let mut info_map = HashMap::new();
            if pod.pod_type == PodItemType::App {
                continue;
            }

            for instance in self.pod_instances.values() {
                //info!("pod_id:{} instance:{:?}", pod_id, instance);
                if instance.state == PodInstanceState::Running && 
                   instance.pod_id == *pod_id {
                    if now - instance.last_update_time < POD_INSTANCE_ALIVE_TIME {
                        info_map.insert(instance.instance_id.clone(), (100,instance.clone()));
                    } else {
                        warn!("pod_id:{} instance:{} is not alive", pod_id, instance.instance_id);
                    }
                }
            }

            if info_map.is_empty() {
                warn!("pod_id:{} NO running instance", pod_id);
            }

            let new_info = PodInfo::RandomCluster(info_map);
            let old_info = self.pod_infos.get(pod_id);
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
                actions.push(SchedulerAction::UpdatePodServiceInfo(pod_id.clone(), new_info));
                info!("pod_id:{} calc new service info", pod_id);
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

        for (pod_id, pod) in &self.pods {
            match pod.state {
                PodItemState::New => {
                    let new_instances = self.instance_pod(pod, &valid_nodes)?;
                    for instance in new_instances {
                        pod_actions.push(SchedulerAction::InstancePod(instance));
                    }
                    //TODO:现在没有部署中的状态
                    pod_actions.push(SchedulerAction::ChangePodStatus(
                        pod_id.clone(),
                        PodItemState::Deployed,
                    ));
                }
                PodItemState::Removing => {
                    for instance in self.pod_instances.values() {
                        if instance.pod_id == *pod_id {
                            pod_actions.push(SchedulerAction::RemovePodInstance(format!(
                                "{}@{}",
                                pod_id, instance.node_id
                            )));
                        }
                    }
                    //TODO:现在没有删除中的状态
                    pod_actions.push(SchedulerAction::ChangePodStatus(
                        pod_id.clone(),
                        PodItemState::Deleted,
                    ));
                }
                _ => {}
            }
        }
        Ok(pod_actions)
    }

    fn is_pod_changed(&self) -> bool {
        if self.pods.len() != self.last_pods.len() {
            return true;
        }

        for (pod_id, pod) in &self.pods {
            match self.last_pods.get(pod_id) {
                None => return true,
                Some(last_pod) => {
                    if pod.state != last_pod.state
                        || pod.required_cpu_mhz != last_pod.required_cpu_mhz
                        || pod.required_memory != last_pod.required_memory
                        || pod.node_affinity != last_pod.node_affinity
                        || pod.network_affinity != last_pod.network_affinity
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
        instance: &PodInstance,
        node: &NodeItem,
        actions: &mut Vec<SchedulerAction>,
    ) -> Result<()> {
        // TODO: 实现检查和更新资源限制的逻辑
        Ok(())
    }

    fn find_new_placement(
        &self,
        pod_id: &str,
        available_nodes: &[&NodeItem],
    ) -> Result<Vec<PodInstance>> {
        // TODO: 实现查找新的部署位置的逻辑
        Ok(vec![])
    }

    fn instance_pod(
        &self,
        pod_item: &PodItem,
        node_list: &Vec<NodeItem>,
    ) -> Result<Vec<PodInstance>> {
        // 1. 过滤阶段
        let candidate_nodes: Vec<&NodeItem> = node_list
            .iter()
            .filter(|node| self.filter_node_for_pod_instance(node, pod_item))
            .collect();

        if candidate_nodes.is_empty() {
            return Err(anyhow::anyhow!("No suitable node found for pod"));
        }
        let candidate_node_count: u32 = candidate_nodes.len() as u32;

        // 2. 打分阶段
        let mut scored_nodes: Vec<(f64, &NodeItem)> = candidate_nodes
            .iter()
            .map(|node| (self.score_node(node, pod_item), *node))
            .collect();

        let instance_count: u32 = std::cmp::min(pod_item.best_instance_count, candidate_node_count);
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
            instances.push(PodInstance {
                node_id: node.id.clone(),
                pod_id: pod_item.id.clone(),
                res_limits: HashMap::new(),
                instance_id: format!("{}_{}", pod_item.id, instance_id_uuid),
                last_update_time: 0,
                state: PodInstanceState::Running,
                service_port: pod_item.default_service_port,
            });
        }
        Ok(instances)
    }

    //关键函数:根据pod_item的配置,过滤符合条件的node
    fn filter_node_for_pod_instance(&self, node: &NodeItem, pod: &PodItem) -> bool {
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

    //关键函数:根据pod_item的配置,对node进行打分
    fn score_node(&self, node: &NodeItem, pod: &PodItem) -> f64 {
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
