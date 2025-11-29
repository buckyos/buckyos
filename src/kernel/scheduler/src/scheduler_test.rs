use buckyos_kit::buckyos_get_unix_timestamp;
use std::collections::HashMap;

use crate::scheduler::*;

fn create_test_node(
    id: &str,
    cpu: u32,
    memory: u64,
    labels: Vec<String>,
    load: f64,
    state: NodeState,
    network_zone: &str,
) -> NodeItem {
    NodeItem {
        id: id.to_string(),
        node_type: NodeType::OOD,
        total_cpu_mhz: cpu,
        available_cpu_mhz: cpu,
        total_memory: memory,
        available_memory: memory,
        total_gpu_memory: 0,
        available_gpu_memory: 0,
        gpu_tflops: 0.0,
        state,
        labels,
        network_zone: network_zone.to_string(),
        support_container: true,
        resources: HashMap::new(),
        op_tasks: vec![],
    }
}

// test node state change
// New -> Prepare
// Removing -> Deleted
#[test]
fn test_node_state_change() {
    let mut scheduler = NodeScheduler::new_empty(1, buckyos_get_unix_timestamp());

    let node1 = create_test_node(
        "node1",
        1000,
        1024 * 1024 * 256,
        vec![],
        0.0,
        NodeState::New,
        "zone1",
    );
    let node2 = create_test_node(
        "node2",
        2000,
        1024 * 1025 * 256,
        vec![],
        0.0,
        NodeState::Removing,
        "zone2",
    );
    let node3 = create_test_node(
        "node3",
        3000,
        1024 * 1026 * 256,
        vec![],
        0.0,
        NodeState::Ready,
        "zone3",
    );

    scheduler.add_node(node1);
    scheduler.add_node(node2);
    scheduler.add_node(node3);

    let actions = scheduler.resort_nodes().unwrap();
    assert_eq!(actions.len(), 2);
            if let SchedulerAction::ChangeNodeStatus(node_id, new_state) = &actions[0] {
            if node_id == "node1" {
                assert_eq!(new_state, &NodeState::Prepare);
            } else if node_id == "node2" {
                assert_eq!(new_state, &NodeState::Deleted);
            } else {
                panic!("Unexpected node id: {}", node_id);
            }
        }
}

// test create service_spec instance
#[test]
fn test_create_pod_instance() {
    let now = buckyos_get_unix_timestamp();
    let mut scheduler = NodeScheduler::new_empty(1, now);

    let pod = ServiceSpec {
        id: "pod1".to_string(),
        app_id: "pod1".to_string(),
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Service,
        state: ServiceSpecState::New,
        best_instance_count: 2,
        need_container: false,
        required_cpu_mhz: 100,
        required_memory: 1024 * 1024 * 512,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        default_service_port: 80,
    };

    scheduler.add_service_spec(pod);

    // create nodes
    let node1 = create_test_node(
        "node1",
        1000,
        1024 * 1024 * 256,
        vec![],
        0.0,
        NodeState::Ready,
        "zone1",
    );
    let node2 = create_test_node(
        "node2",
        2000,
        1024 * 1024 * 1024,
        vec![],
        0.0,
        NodeState::Ready,
        "zone2",
    );

    scheduler.add_node(node1);
    scheduler.add_node(node2);

    let actions = scheduler.schedule_spec_change().unwrap();
    assert_eq!(actions.len(), 2);
    for action in &actions {
        match action {
            SchedulerAction::InstanceReplica(instance) => {
                assert_eq!(instance.spec_id, "pod1");
                assert_eq!(instance.node_id, "node2");
            }
            SchedulerAction::ChangeServiceStatus(spec_id, new_state) => {
                assert_eq!(spec_id, "pod1");
                assert_eq!(new_state, &ServiceSpecState::Deployed);
            }
            _ => panic!("Unexpected action"),
        }
    }
}

// test service_spec state change: New -> Deployed, Removing -> Deleted
#[test]
fn test_pod_state_change() {
    let now = buckyos_get_unix_timestamp();
    let mut scheduler = NodeScheduler::new_empty(1, now);

    let pod1 = ServiceSpec {
        id: "pod1".to_string(),
        app_id: "pod1".to_string(),
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Service,
        state: ServiceSpecState::New,
        best_instance_count: 1,
        need_container: false,
        required_cpu_mhz: 100,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        default_service_port: 80,
    };
    let pod2 = ServiceSpec {
        id: "pod2".to_string(),
        app_id: "pod2".to_string(),
        owner_id: "user1".to_string(),
        spec_type: ServiceSpecType::App,
        state: ServiceSpecState::Removing,
        best_instance_count: 1,
        need_container: false,
        required_cpu_mhz: 200,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        default_service_port: 80,
    };

    scheduler.add_service_spec(pod1);
    scheduler.add_service_spec(pod2);

    // create node for pod1
    let node1 = create_test_node(
        "node1",
        1000,
        1024 * 1024 * 256,
        vec![],
        0.0,
        NodeState::Ready,
        "zone1",
    );
    scheduler.add_node(node1);

    let actions = scheduler.schedule_spec_change().unwrap();
    assert_eq!(actions.len(), 3);
    for action in &actions {
        match action {
            SchedulerAction::InstanceReplica(instance) => {
                assert_eq!(instance.spec_id, "pod1");
                assert_eq!(instance.node_id, "node1");
                //add pod instance
                scheduler.add_replica_instance(instance.clone());
            }
            SchedulerAction::ChangeServiceStatus(spec_id, new_state) => {
                if spec_id == "pod1" {
                    assert_eq!(new_state, &ServiceSpecState::Deployed);
                } else if spec_id == "pod2" {
                    assert_eq!(new_state, &ServiceSpecState::Deleted);
                    // remove service_spec
                    scheduler.remove_service_spec(spec_id);
                } else {
                    panic!("Unexpected spec id: {}", spec_id);
                }
            }
            _ => panic!("Unexpected action"),
        }
    }

    /*
    改变pod1的状态为Removing
    */
    scheduler.update_service_spec_state("pod1", ServiceSpecState::Removing);
    let actions = scheduler.schedule_spec_change().unwrap();
    println!("test_pod_state_change actions: {:?}", actions);
    assert_eq!(actions.len(), 2);
    // if let SchedulerAction::RemoveReplica(pod_id, instance_id, node_id) = &actions[0] {
    //     assert_eq!(instance_id, "pod1@node1");
    //     assert_eq!(pod_id, "pod1");
    //     assert_eq!(node_id, "node1");
    // } else {
    //     panic!("Expected RemoveReplica action");
    // }
    // if let SchedulerAction::ChangePodStatus(pod_id, new_state) = &actions[1] {
    //     assert_eq!(pod_id, "pod1");
    //     assert_eq!(new_state, &ServiceSpecState::Deleted);
    // }
}

// test create service_spec instance with no suitable node
#[test]
fn test_create_pod_instance_no_suitable_node() {
    let now = buckyos_get_unix_timestamp();
    let mut scheduler = NodeScheduler::new_empty(1, now);

    let pod = ServiceSpec {
        id: "pod1".to_string(),
        app_id: "pod1".to_string(),
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Service,
        state: ServiceSpecState::New,
        best_instance_count: 1,
        need_container: false,
        required_cpu_mhz: 1000, // 超出节点资源
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        default_service_port: 80,
    };

    scheduler.add_service_spec(pod);

    // create nodes
    let node1 = create_test_node(
        "node1",
        500, // CPU不足
        1024 * 1024 * 256,
        vec![],
        0.0,
        NodeState::Ready,
        "zone1",
    );
    scheduler.add_node(node1);

    let actions = scheduler.schedule_spec_change();
    assert!(actions.is_err());
}

// test node_affinity and network_affinity
#[test]
fn test_node_and_network_affinity() {
    let now = buckyos_get_unix_timestamp();
    let mut scheduler = NodeScheduler::new_empty(1, now);

    let pod = ServiceSpec {
        id: "pod1".to_string(),
        app_id: "pod1".to_string(),
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Service,
        state: ServiceSpecState::New,
        best_instance_count: 1,
        need_container: false,
        required_cpu_mhz: 100,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: Some("gpu".to_string()),
        network_affinity: Some("zone3".to_string()),
        default_service_port: 80,
    };

    scheduler.add_service_spec(pod);

    // create nodes
    let node1 = create_test_node(
        "node1",
        1000,
        1024 * 1024 * 256,
        vec!["gpu".to_string()],
        0.0,
        NodeState::Ready,
        "zone1",
    );
    let node2 = create_test_node(
        "node2",
        2000,
        1024 * 1025 * 256,
        vec!["cpu".to_string()],
        0.0,
        NodeState::Ready,
        "zone2",
    );
    let node3 = create_test_node(
        "node3",
        3000,
        1024 * 1026 * 256,
        vec!["gpu".to_string()],
        0.0,
        NodeState::Ready,
        "zone3",
    );

    scheduler.add_node(node1);
    scheduler.add_node(node2);
    scheduler.add_node(node3);

    let actions = scheduler.schedule_spec_change().unwrap();
    assert_eq!(actions.len(), 2);
    if let SchedulerAction::InstanceReplica(instance) = &actions[0] {
        assert_eq!(instance.spec_id, "pod1");
        assert_eq!(instance.node_id, "node3");
    }
    if let SchedulerAction::ChangeServiceStatus(spec_id, new_state) = &actions[1] {
        assert_eq!(spec_id, "pod1");
        assert_eq!(new_state, &ServiceSpecState::Deployed);
    } else {
        panic!("Unexpected action");
    }
} 