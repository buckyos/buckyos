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
*/

use anyhow::Result;
use std::collections::HashMap;

pub struct PodItem {
    pub id: String,
    pub required_cpu: f64,
    pub required_memory: f64,
    // 亲和性规则
    pub node_affinity: Option<String>,
    pub network_affinity: Option<String>,
}

pub struct NodeItem {
    pub id: String,
    pub available_cpu: f64,
    pub available_memory: f64,
    pub status: NodeStatus,
    // 节点标签，用于亲和性匹配
    pub labels: Vec<String>,
    pub current_load: f64,
    pub network_zone: String,
    pub is_healthy: bool,
}

#[derive(PartialEq)]
pub enum NodeStatus {
    Ready,
    NotReady,
}

pub struct PodInstance {
    pub node_id:String,
    pub pod_id:String,
}

pub struct PodScheduler {
    nodes:Vec<NodeItem>,
    pod_instances:HashMap<String,PodInstance>,
}

impl PodScheduler {
    pub fn new(nodes:Vec<NodeItem>) -> Self {
        Self { 
            nodes,
            pod_instances:HashMap::new() 
        }
    }
}

impl PodScheduler {
    pub fn instance_pod(&self, pod_item: &PodItem) -> Result<Vec<PodInstance>> {
        
        // 1. 过滤阶段
        let candidate_nodes: Vec<&NodeItem> = self.nodes.iter()
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
        let health_score = if node.is_healthy { 20.0 } else { 0.0 };
        score += health_score;

        // 可以添加更多评分规则
        // - 节点存储评分 (关键设计!)
        // - 节点地理位置评分 (异地机房)


        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_node(
        id: &str,
        cpu: f64,
        memory: f64,
        labels: Vec<String>,
        load: f64,
        network_zone: &str,
        is_healthy: bool,
    ) -> NodeItem {
        NodeItem {
            id: id.to_string(),
            available_cpu: cpu,
            available_memory: memory,
            status: NodeStatus::Ready,
            labels,
            current_load: load,
            network_zone: network_zone.to_string(),
            is_healthy,
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
        };

        let scheduler = PodScheduler::new(vec![
                create_test_node(
                    "node1",
                    4.0,
                    8.0,
                    vec!["gpu".to_string()],
                    0.5,
                    "zone1",
                    true,
                ),
                create_test_node(
                    "node2",
                    1.0, // insufficient CPU
                    8.0,
                    vec!["gpu".to_string()],
                    0.5,
                    "zone1",
                    true,
                ),
                create_test_node(
                    "node3",
                    4.0,
                    8.0,
                    vec![], // missing required label
                    0.5,
                    "zone1",
                    true,
                ),
            ],
        );

        assert!(scheduler.filter_node(&scheduler.nodes[0], &pod));
        assert!(!scheduler.filter_node(&scheduler.nodes[1], &pod));
        assert!(!scheduler.filter_node(&scheduler.nodes[2], &pod));
    }

    #[test]
    fn test_instance_pod() {
        let pod = PodItem {
            id: "test-pod".to_string(),
            required_cpu: 2.0,
            required_memory: 4.0,
            node_affinity: None,
            network_affinity: Some("zone1".to_string()),
        };

        let scheduler = PodScheduler::new(vec![
                create_test_node(
                    "node1",
                    4.0,
                    8.0,
                    vec![],
                    0.8, // high load
                    "zone1",
                    true,
                ),
                create_test_node(
                    "node2",
                    4.0,
                    8.0,
                    vec![],
                    0.2, // low load - should be selected
                    "zone1",
                    true,
                ),
            ],
        );

        let result = scheduler.instance_pod(&pod).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node_id, "node2");
        assert_eq!(result[0].pod_id, "test-pod");
    }

    #[test]
    fn test_no_suitable_node() {
        let pod = PodItem {
            id: "test-pod".to_string(),
            required_cpu: 8.0, // requires more CPU than available
            required_memory: 4.0,
            node_affinity: None,
            network_affinity: None,
        };

        let scheduler = PodScheduler::new(vec![create_test_node(
            "node1",
            4.0,
            8.0,
            vec![],
            0.5,
            "zone1",
            true,
        )]);

        assert!(scheduler.instance_pod(&pod).is_err());
    }
}