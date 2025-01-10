/*
实例化:
    将一个容器/容器Group (PodItem) 绑定到合适的节点上运行(实例化/部署) 

资源调度:针对PodInstance,调度器可以
    调整实例的资源控制
    启动/关闭实例

运维操作:
    迁移PodInstance

无状态PodItem可以实现无运维迁移
    删除实例
    重新实例化
    调度器实现重新部署


扩容添加Node
    

维护替换添加Node
    目的是添加一个新的Node,来替换旧的Node
    新Node的nodeid与旧的node有一定的近视之处



删除Node(系统d动态缩容)
   3节点以下是无法缩容的,是在需要,用备份数据->新建zone->恢复数据的方式实现



*/

use anyhow::Result;
use std::collections::HashMap;

pub enum PodItemType {
    Service, //无状态的系统服务
    App,// 无状态的app服务
}

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

pub enum OPTaskBody {
    NodeInitBaseService,
}


pub enum OPTaskStatus {
    New,
    Running,
    Done,
    Failed,
}

pub struct OPTask {
    pub id: String,
    pub creator_id: Option<String>,//为None说明创建者是Scheduler
    pub owner_id: String,// owner id 可以是node_id,也可以是pod_id
    pub status: OPTaskStatus,
    pub create_time: u64,
    pub create_step_id: u64,
    pub body: OPTaskBody,
}


pub struct NodeResource {
    pub total_capacity: u64,
    pub used_capacity: u64,
}

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

#[derive(PartialEq)]
pub enum NodeStatus {
    New,
    Prepare,//new->ready的准备阶段
    Ready,//正常可用
    Abnormal,//存在异常
    Unavailable,//不可用
    Removing,//正在移除
    Deleted,//已删除
}

pub struct PodInstance {
    pub node_id:String,
    pub pod_id:String,
    pub res_limits: HashMap<String,f64>,
}

pub struct PodScheduler {
    schedule_step_id:u64,
    last_schedule_time:u64,
    nodes:Vec<NodeItem>,
    // 系统里所有的PodInstance,key是pod_id
    pod_instances:HashMap<String,PodInstance>,
}

pub enum SchedulerAction {
    ChangeNodeStatus(String,NodeStatus),
    CreateOPTask(OPTask),
    ChangePodStatus(String,PodItemStatus),
    InstancePod(PodInstance),
    UpdatePodInstance(String,PodInstance),
    RemovePodInstance(String),
}

impl PodScheduler {
    pub fn new(step_id:u64,
        last_schedule_time:u64,
        nodes:Vec<NodeItem>,
        pod_instances:HashMap<String,PodInstance>) -> Self {
        Self { 
            schedule_step_id:step_id,
            last_schedule_time,
            nodes,
            pod_instances
        }
    }

    pub fn schedule(&self, pods: &[PodItem])->Result<Vec<SchedulerAction>> {
        let mut actions = Vec::new();
        
        // 第一阶段：扫描所有Node
        for node in &self.nodes {
            // 1. 检查已有op tasks的完成情况
            self.check_node_op_tasks(node, &mut actions)?;
            
            // 2. 处理新node的初始化
            if node.status == NodeStatus::New {
                actions.push(SchedulerAction::ChangeNodeStatus(
                    node.id.clone(), 
                    NodeStatus::Prepare
                ));
                
                // 创建node初始化任务
                actions.push(SchedulerAction::CreateOPTask(OPTask {
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

        // 获取可用的node列表
        let available_nodes: Vec<&NodeItem> = self.nodes.iter()
            .filter(|n| n.status == NodeStatus::Ready)
            .collect();

        // 第二阶段：扫描所有Pod和PodInstance
        // 1. 优先处理未实例化的Pod
        for pod in pods {
            if !self.pod_instances.contains_key(&pod.id) {
                // Pod尚未实例化
                match pod.status {
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

        // 2. 处理已有实例的Pod
        for (pod_id, instance) in &self.pod_instances {
            // 找到对应的Pod配置
            if let Some(pod) = pods.iter().find(|p| p.id == *pod_id) {
                match pod.status {
                    PodItemStatus::Removing => {
                        // 处理正在删除的Pod
                        actions.push(SchedulerAction::RemovePodInstance(pod_id.clone()));
                        actions.push(SchedulerAction::ChangePodStatus(
                            pod_id.clone(), 
                            PodItemStatus::Deleted
                        ));
                    },
                    _ => {
                        // 检查实例是否需要迁移或更新
                        let node = self.nodes.iter()
                            .find(|n| n.id == instance.node_id);
                        
                        match node {
                            Some(node) if node.status == NodeStatus::Ready => {
                                // Pod实例正常，检查资源限制是否需要更新
                                self.check_resource_limits(instance, node, &mut actions)?;
                            },
                            _ => {
                                // Pod需要迁移或重新部署
                                actions.push(SchedulerAction::RemovePodInstance(pod_id.clone()));
                                
                                // 尝试在新节点上重新部署
                                if let Ok(new_instances) = self.instance_pod(pod, &available_nodes) {
                                    for new_instance in new_instances {
                                        actions.push(SchedulerAction::InstancePod(new_instance));
                                    }
                                } else {
                                    actions.push(SchedulerAction::ChangePodStatus(
                                        pod_id.clone(), 
                                        PodItemStatus::Abnormal
                                    ));
                                }
                            }
                        }
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

    pub fn instance_pod(&self, pod_item: &PodItem, node_list: &Vec<&NodeItem>) -> Result<Vec<PodInstance>> {
        
        // 1. 过滤阶段
        let candidate_nodes : Vec<&&NodeItem> = node_list.iter()
            .filter(|node| self.filter_node(node, pod_item))
            .collect();

        if candidate_nodes.is_empty() {
            return Err(anyhow::anyhow!("No suitable node found for pod"));
        }

        // 2. 打分阶段
        let mut scored_nodes: Vec<(f64, &&NodeItem)> = candidate_nodes.iter()
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

        let result = scheduler.instance_pod(&pod).unwrap();
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

        assert!(scheduler.instance_pod(&pod).is_err());
    }
}