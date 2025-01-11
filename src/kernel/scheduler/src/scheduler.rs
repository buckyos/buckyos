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

use anyhow::Result;
use std::{collections::HashMap, sync::Arc};
use log::*;

#[derive(Clone)]
pub enum PodItemType {
    Service, //无状态的系统服务
    App,// 无状态的app服务
}
#[derive(Clone)]
pub enum PodItemStatus {
    New,
    Deploying,
    Deployed,
    DeployFailed,
    Abnormal,
    Unavailable,
    Removing,
    Deleted,
}
#[derive(Clone)]
pub struct PodItem {
    pub id: String,
    pub pod_type: PodItemType,
    pub status: PodItemStatus,

    pub required_cpu: f64,
    pub required_memory: f64,
    // 亲和性规则
    pub node_affinity: Option<String>,
    pub network_affinity: Option<String>,
}
#[derive(Clone)]
pub enum OPTaskBody {
    NodeInitBaseService,
}

#[derive(Clone)]
pub enum OPTaskStatus {
    New,
    Running,
    Done,
    Failed,
}
#[derive(Clone)]
pub struct OPTask {
    pub id: String,
    pub creator_id: Option<String>,//为None说明创建者是Scheduler
    pub owner_id: String,// owner id 可以是node_id,也可以是pod_id
    pub status: OPTaskStatus,
    pub create_time: u64,
    pub create_step_id: u64,
    pub body: OPTaskBody,
}

#[derive(Clone)]
pub struct NodeResource {
    pub total_capacity: u64,
    pub used_capacity: u64,
}

#[derive(Clone)]
pub struct NodeItem {
    pub id: String,
    //pub name: String,
    // 节点标签，用于亲和性匹配
    pub labels: Vec<String>,
    pub network_zone: String,

    //Node的可变状态
    pub status: NodeStatus,
    pub available_cpu: f64,
    pub available_memory: f64,
    pub current_load: f64,
    pub resources: HashMap<String, NodeResource>,
    pub op_tasks: Vec<OPTask>,
}

#[derive(PartialEq,Clone)]
pub enum NodeStatus {
    New,
    Prepare,//new->ready的准备阶段
    Ready,//正常可用
    Abnormal,//存在异常
    Unavailable,//不可用
    Removing,//正在移除
    Deleted,//已删除
}

#[derive(Clone)]
pub struct PodInstance {
    pub node_id:String,
    pub pod_id:String,
    pub res_limits: HashMap<String,f64>,
}

#[derive(Clone)]
pub enum SchedulerAction {
    ChangeNodeStatus(String,NodeStatus),
    CreateOPTask(OPTask),
    ChangePodStatus(String,PodItemStatus),
    InstancePod(PodInstance),
    UpdatePodInstance(String,PodInstance),
    RemovePodInstance(String),
}

pub struct PodScheduler {
    schedule_step_id:u64,
    last_schedule_time:u64,
    nodes:Vec<NodeItem>,
    pods:Vec<PodItem>,
    // 系统里所有的PodInstance,key是pod_id
    pod_instances:HashMap<String,PodInstance>,
}


impl PodScheduler {
    pub fn new(step_id:u64,
        last_schedule_time:u64,
        nodes:Vec<NodeItem>,
        pods:Vec<PodItem>,
        pod_instances:HashMap<String,PodInstance>) -> Self {
        Self { 
            schedule_step_id:step_id,
            last_schedule_time,
            nodes,
            pods,
            pod_instances
        }
    }

    fn resort_nodes(&mut self)->Result<Vec<SchedulerAction>> {
        let mut node_actions = Vec::new();
        // 根据Node的status进行排序
        for node in &self.nodes {
            // 1. 检查已有op tasks的完成情况，该部分可能会对可用Node进行一些标记
            self.check_node_op_tasks(node, &mut node_actions)?;
            
            // 2. 处理新node的初始化
            if node.status == NodeStatus::New {
                node_actions.push(SchedulerAction::ChangeNodeStatus(
                    node.id.clone(), 
                    NodeStatus::Prepare
                ));
                
                // 创建node初始化任务
                node_actions.push(SchedulerAction::CreateOPTask(OPTask {
                    id: format!("init-{}", node.id),
                    creator_id: None,
                    owner_id: node.id.clone(),
                    status: OPTaskStatus::New,
                    create_time: self.last_schedule_time,
                    create_step_id: self.schedule_step_id,
                    body: OPTaskBody::NodeInitBaseService,
                }));
            }
        }

        Ok(node_actions)
    }

    pub fn schedule(&mut self)->Result<Vec<SchedulerAction>> {
        if self.nodes.is_empty() {
            return Err(anyhow::anyhow!("No nodes found"));
        }

        let is_small_system = self.nodes.len() <= 7;
        let mut actions = self.resort_nodes()?;
        if !actions.is_empty() && is_small_system {
            // 为了降低复杂度,在系统规模较小时,如果第一个阶段存在动作,则跳过第二阶段,防止复杂度的重叠.
            // TODO:是否会带来问题?
            info!("Small system, skip schedule pod instance when have node actions");
            return Ok(actions);
        }


        let available_nodes: Vec<NodeItem> = self.nodes.iter()
            .filter(|n| n.status == NodeStatus::Ready)
            .cloned()
            .collect();

        // 扫描所有Pod和PodInstance
		//实例化调度算法：分配资源：构造PodInstance，将未部署的Pod绑定到Node上
		//反实例化：及时释放资源
		//动态调整：根据运行情况，系统资源剩余情况，相对动态的调整Pod能用的资源
		//动态调整也涉及到实例的迁移

        for pod in self.pods.iter() {
            if !self.pod_instances.contains_key(&pod.id) {
                // Pod尚未实例化
                match pod.status {
                    PodItemStatus::Removing => {
                        //首次看到，执行反实例化操作、构造必要的OP
                        //二次看到，判断相关Node的状态里是否已经上报删除完成
                        //如果看到完整，则将状态设置为deleted    
                    },
                    PodItemStatus::New  => {
                        // 尝试实例化Pod
                        match self.instance_pod(pod, &available_nodes) {
                            Ok(new_instances) => {
                                for instance in new_instances {
                                    actions.push(SchedulerAction::InstancePod(instance));
                                }
                                actions.push(SchedulerAction::ChangePodStatus(
                                    pod.id.clone(), 
                                    PodItemStatus::Deploying
                                ));
                            },
                            Err(_) => {
                                // 无法找到合适的节点，标记Pod状态为异常
                                actions.push(SchedulerAction::ChangePodStatus(
                                    pod.id.clone(), 
                                    PodItemStatus::DeployFailed
                                ));
                            }
                        }
                    },
                    _ => {} // 其他状态的Pod暂不处理
                }
            }
        }

        // 2. 动态挑战已有的PodInstance:根据运行情况，系统资源剩余情况，相对动态的调整Pod能用的资源
        //    动态调整也涉及到实例的自动迁移
        for (pod_id, instance) in &self.pod_instances {
            // 找到对应的Pod配置
            if let Some(pod) = self.pods.iter().find(|p| p.id == *pod_id) {
                match pod.status {
                    PodItemStatus::Deployed => {
                        // 检查处于改状态的时间
                        //self.check_resource_limits(instance, node, &mut actions)?;
                    },
                    _ => {
                        // 其他状态的Pod暂不处理
                        unimplemented!()
                    }
                }
            }
        }

        Ok(actions)
    }

    // 辅助函数
    fn check_node_op_tasks(&self, node: &NodeItem, actions: &mut Vec<SchedulerAction>) -> Result<()> {
        // TODO: 实现检查node的op tasks完成情况的逻辑
        Ok(())
    }

    fn check_resource_limits(&self, 
        instance: &PodInstance, 
        node: &NodeItem,
        actions: &mut Vec<SchedulerAction>) -> Result<()> {
        // TODO: 实现检查和更新资源限制的逻辑
        Ok(())
    }

    fn find_new_placement(&self, 
        pod_id: &str,
        available_nodes: &[&NodeItem]) -> Result<Vec<PodInstance>> {
        // TODO: 实现查找新的部署位置的逻辑
        Ok(vec![])
    }

    pub fn instance_pod(&self, pod_item: &PodItem, node_list: &Vec<NodeItem>) -> Result<Vec<PodInstance>> {
        
        // 1. 过滤阶段
        let candidate_nodes : Vec<&NodeItem> = node_list.iter()
            .filter(|node| self.filter_node(node, pod_item))
            .collect();

        if candidate_nodes.is_empty() {
            return Err(anyhow::anyhow!("No suitable node found for pod"));
        }

        // 2. 打分阶段
        let mut scored_nodes: Vec<(f64, &NodeItem)> = candidate_nodes.iter()
            .map(|node| (self.score_node(node, pod_item), *node))
            .collect();

        // 按分数降序排序
        scored_nodes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        // 选择得分最高的节点
        let selected_node = scored_nodes[0].1;
        
        Ok(vec![PodInstance {
            node_id: selected_node.id.clone(),
            pod_id: pod_item.id.clone(),
            res_limits: HashMap::new(),
        }])
    }

    fn filter_node(&self, node: &NodeItem, pod: &PodItem) -> bool {
        // 1. 检查节点状态
        if node.status != NodeStatus::Ready {
            return false;
        }

        // 2. 检查资源是否充足
        if node.available_cpu < pod.required_cpu || 
           node.available_memory < pod.required_memory {
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

    fn score_node(&self, node: &NodeItem, pod: &PodItem) -> f64 {
        let mut score = 0.0;

        // 1. 资源充足度评分
        let cpu_score = (node.available_cpu - pod.required_cpu) / node.available_cpu;
        let memory_score = (node.available_memory - pod.required_memory) / node.available_memory;
        score += (cpu_score + memory_score) / 2.0 * 100.0;

        // 2. 节点负载均衡评分
        let load_score = 1.0 - node.current_load;
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

#[cfg(test)]
mod tests {
    use buckyos_kit::buckyos_get_unix_timestamp;

    use super::*;

    use std::collections::HashMap;

    fn create_test_node(
        id: &str,
        cpu: f64,
        memory: f64,
        labels: Vec<String>,
        load: f64,
        network_zone: &str,
    ) -> NodeItem {
        NodeItem {
            id: id.to_string(),
            available_cpu: cpu,
            available_memory: memory,
            status: NodeStatus::Ready,
            labels,
            current_load: load,
            network_zone: network_zone.to_string(),
            resources: HashMap::new(),
            op_tasks: vec![],
        }
    }

    #[test]
    fn test_filter_node() {
        let pod = PodItem {
            id: "test-pod".to_string(),
            required_cpu: 2.0,
            required_memory: 4.0,
            node_affinity: Some("gpu".to_string()),
            network_affinity: Some("zone1".to_string()),
            pod_type: PodItemType::Service,
            status: PodItemStatus::New,
        };

        let now = buckyos_get_unix_timestamp();
        let scheduler = PodScheduler::new(1,now,
            vec![
                create_test_node(
                    "node1",
                    4.0,
                    8.0,
                    vec!["gpu".to_string()],
                    0.5,
                    "zone1"

                ),
                create_test_node(
                    "node2",
                    1.0, // insufficient CPU
                    8.0,
                    vec!["gpu".to_string()],
                    0.5,
                    "zone1"
       
                ),
                create_test_node(
                    "node3",
                    4.0,
                    8.0,
                    vec![], // missing required label
                    0.5,
                    "zone1"
                ),
            ],
            HashMap::new()
        );

        assert!(scheduler.filter_node(&scheduler.nodes[0], &pod));
        assert!(!scheduler.filter_node(&scheduler.nodes[1], &pod));
        assert!(!scheduler.filter_node(&scheduler.nodes[2], &pod));
    }

    #[test]
    fn test_instance_pod() {
        let now = buckyos_get_unix_timestamp();

        let pod = PodItem {
            id: "test-pod".to_string(),
            required_cpu: 2.0,
            required_memory: 4.0,
            node_affinity: None,
            network_affinity: Some("zone1".to_string()),
            pod_type: PodItemType::Service,
            status: PodItemStatus::New,
        };

        let scheduler = PodScheduler::new(1,now,
            vec![
                create_test_node(
                    "node1",
                    4.0,
                    8.0,
                    vec![],
                    0.8, // high load
                    "zone1"
                ),
                create_test_node(
                    "node2",
                    4.0,
                    8.0,
                    vec![],
                    0.2, // low load - should be selected
                    "zone1"
                ),
            ],
            HashMap::new()
        );

        let result = scheduler.instance_pod(&pod, &scheduler.nodes).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node_id, "node2");
        assert_eq!(result[0].pod_id, "test-pod");
    }

    #[test]
    fn test_no_suitable_node() {
        let now = buckyos_get_unix_timestamp();
        let pod = PodItem {
            id: "test-pod".to_string(),
            required_cpu: 8.0, // requires more CPU than available
            required_memory: 4.0,
            node_affinity: None,
            network_affinity: None,
            pod_type: PodItemType::Service,
            status: PodItemStatus::New,
        };

        let scheduler = PodScheduler::new(1,now,
            vec![create_test_node(
            "node1",
            4.0,
            8.0,
            vec![],
            0.5,
            "zone1"
        )],
        HashMap::new());

        assert!(scheduler.instance_pod(&pod, &scheduler.nodes).is_err());
    }
}