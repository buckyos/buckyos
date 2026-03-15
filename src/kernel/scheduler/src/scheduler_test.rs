// Test scenarios in this file:
// 1. Large cluster / multi-service placement:
//    verify label, zone and container constraints can be combined and that
//    service_info is built from the scheduled replicas.
// 2. Incremental scale-out from 1 node to 7 nodes:
//    under the current minimal-change policy, pure topology growth should not
//    trigger eager replica migration, which avoids unnecessary churn.
// 3. Distributed-system liveness cases:
//    service discovery should only publish fresh running replicas and update
//    cluster membership after heartbeat state changes.
use std::collections::{HashMap, HashSet};

use buckyos_kit::buckyos_get_unix_timestamp;

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

fn create_test_service_spec(id: &str) -> ServiceSpec {
    ServiceSpec {
        id: id.to_string(),
        app_id: id.to_string(),
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Service,
        state: ServiceSpecState::New,
        best_instance_count: 1,
        need_container: false,
        required_cpu_mhz: 100,
        required_memory: 1024 * 1024 * 128,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        app_index: 0,
        service_ports_config: HashMap::new(),
    }
}

fn create_test_replica_instance(
    spec_id: &str,
    node_id: &str,
    state: InstanceState,
    last_update_time: u64,
) -> ReplicaInstance {
    ReplicaInstance {
        node_id: node_id.to_string(),
        spec_id: spec_id.to_string(),
        res_limits: HashMap::new(),
        instance_id: format!("{}@{}", spec_id, node_id),
        last_update_time,
        state,
        service_ports: HashMap::new(),
    }
}

fn node_set(node_ids: &[&str]) -> HashSet<String> {
    node_ids.iter().map(|node_id| node_id.to_string()).collect()
}

fn action_instance_nodes(actions: &[SchedulerAction], spec_id: &str) -> HashSet<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            SchedulerAction::InstanceReplica(instance) if instance.spec_id == spec_id => {
                Some(instance.node_id.clone())
            }
            _ => None,
        })
        .collect()
}

fn service_info_nodes(service_info: &ServiceInfo) -> HashSet<String> {
    match service_info {
        ServiceInfo::SingleInstance(instance) => node_set(&[instance.node_id.as_str()]),
        ServiceInfo::RandomCluster(cluster) => cluster
            .values()
            .map(|(_, instance)| instance.node_id.clone())
            .collect(),
    }
}

// test node state change
// New -> Prepare
// Removing -> Deleted
#[test]
fn test_node_state_change() {
    let mut scheduler = NodeScheduler::new_empty(1);

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
    let mut scheduler = NodeScheduler::new_empty(1);

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
        app_index: 0,
        service_ports_config: HashMap::new(),
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
    let mut scheduler = NodeScheduler::new_empty(1);

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
        app_index: 10,
        service_ports_config: HashMap::new(),
    };
    let pod2 = ServiceSpec {
        id: "pod2".to_string(),
        app_id: "pod2".to_string(),
        owner_id: "user1".to_string(),
        spec_type: ServiceSpecType::App,
        state: ServiceSpecState::Deleted,
        best_instance_count: 1,
        need_container: false,
        required_cpu_mhz: 200,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        app_index: 12,
        service_ports_config: HashMap::new(),
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
    scheduler.update_service_spec_state("pod1", ServiceSpecState::Deleted);
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

#[test]
fn test_app_disable_updates_instance_instead_of_removing_it() {
    let mut scheduler = NodeScheduler::new_empty(1);

    scheduler.add_service_spec(ServiceSpec {
        id: "jarvis@alice".to_string(),
        app_id: "jarvis".to_string(),
        owner_id: "alice".to_string(),
        spec_type: ServiceSpecType::App,
        state: ServiceSpecState::Disable,
        best_instance_count: 1,
        need_container: true,
        required_cpu_mhz: 100,
        required_memory: 1024 * 1024 * 128,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        app_index: 11,
        service_ports_config: HashMap::new(),
    });

    scheduler.add_replica_instance(ReplicaInstance {
        node_id: "node1".to_string(),
        spec_id: "jarvis@alice".to_string(),
        res_limits: HashMap::new(),
        instance_id: "jarvis@alice@node1".to_string(),
        last_update_time: buckyos_get_unix_timestamp(),
        state: InstanceState::Running,
        service_ports: HashMap::new(),
    });

    let actions = scheduler.schedule_spec_change().unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        SchedulerAction::UpdateInstance(instance_id, instance) => {
            assert_eq!(instance_id, "jarvis@alice@node1");
            assert_eq!(instance.instance_id, "jarvis@alice@node1");
            assert_eq!(instance.state, InstanceState::Suspended);
        }
        other => panic!("unexpected action: {:?}", other),
    }
}

// test create service_spec instance with no suitable node
#[test]
fn test_create_pod_instance_no_suitable_node() {
    let mut scheduler = NodeScheduler::new_empty(1);

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
        app_index: 10,
        service_ports_config: HashMap::new(),
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
    let mut scheduler = NodeScheduler::new_empty(1);

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
        app_index: 12,
        service_ports_config: HashMap::new(),
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

#[test]
fn test_large_cluster_multi_service_schedule() {
    let mut scheduler = NodeScheduler::new_empty(1);

    let mut node1 = create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["gpu".to_string()],
        0.0,
        NodeState::Ready,
        "zone-a",
    );
    node1.available_cpu_mhz = 3000;

    let mut node2 = create_test_node(
        "node2",
        4000,
        1024 * 1024 * 2048,
        vec!["gpu".to_string()],
        0.0,
        NodeState::Ready,
        "zone-b",
    );
    node2.available_cpu_mhz = 2800;

    let node3 = create_test_node(
        "node3",
        4000,
        1024 * 1024 * 2048,
        vec!["db".to_string()],
        0.0,
        NodeState::Ready,
        "zone-b",
    );
    let node4 = create_test_node(
        "node4",
        4000,
        1024 * 1024 * 2048,
        vec!["db".to_string()],
        0.0,
        NodeState::Ready,
        "zone-c",
    );
    let mut node5 = create_test_node(
        "node5",
        4000,
        1024 * 1024 * 2048,
        vec!["edge".to_string()],
        0.0,
        NodeState::Ready,
        "zone-a",
    );
    node5.support_container = false;
    let node6 = create_test_node(
        "node6",
        4000,
        1024 * 1024 * 2048,
        vec!["edge".to_string()],
        0.0,
        NodeState::Ready,
        "zone-b",
    );
    let node7 = create_test_node(
        "node7",
        4000,
        1024 * 1024 * 2048,
        vec!["frontend".to_string()],
        0.0,
        NodeState::Ready,
        "zone-a",
    );
    let node8 = create_test_node(
        "node8",
        4000,
        1024 * 1024 * 2048,
        vec!["frontend".to_string()],
        0.0,
        NodeState::Ready,
        "zone-c",
    );

    for node in [node1, node2, node3, node4, node5, node6, node7, node8] {
        scheduler.add_node(node);
    }

    let mut gpu_service = create_test_service_spec("gpu-service");
    gpu_service.best_instance_count = 2;
    gpu_service.node_affinity = Some("gpu".to_string());

    let mut db_service = create_test_service_spec("db-service");
    db_service.node_affinity = Some("db".to_string());
    db_service.network_affinity = Some("zone-c".to_string());

    let mut edge_service = create_test_service_spec("edge-service");
    edge_service.node_affinity = Some("edge".to_string());
    edge_service.need_container = true;

    let mut frontend_service = create_test_service_spec("frontend-service");
    frontend_service.best_instance_count = 2;
    frontend_service.node_affinity = Some("frontend".to_string());

    for spec in [gpu_service, db_service, edge_service, frontend_service] {
        scheduler.add_service_spec(spec);
    }

    let actions = scheduler.schedule(None).unwrap();
    assert_eq!(actions.len(), 14);

    assert_eq!(
        action_instance_nodes(&actions, "gpu-service"),
        node_set(&["node1", "node2"])
    );
    assert_eq!(
        action_instance_nodes(&actions, "db-service"),
        node_set(&["node4"])
    );
    assert_eq!(
        action_instance_nodes(&actions, "edge-service"),
        node_set(&["node6"])
    );
    assert_eq!(
        action_instance_nodes(&actions, "frontend-service"),
        node_set(&["node7", "node8"])
    );

    let deployed_specs: HashSet<String> = actions
        .iter()
        .filter_map(|action| match action {
            SchedulerAction::ChangeServiceStatus(spec_id, ServiceSpecState::Deployed) => {
                Some(spec_id.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        deployed_specs,
        node_set(&[
            "gpu-service",
            "db-service",
            "edge-service",
            "frontend-service",
        ])
    );

    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("gpu-service").unwrap()),
        node_set(&["node1", "node2"])
    );
    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("db-service").unwrap()),
        node_set(&["node4"])
    );
    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("edge-service").unwrap()),
        node_set(&["node6"])
    );
    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("frontend-service").unwrap()),
        node_set(&["node7", "node8"])
    );
}

#[test]
fn test_scale_out_to_seven_nodes_keeps_existing_replica_sticky() {
    let mut scheduler = NodeScheduler::new_empty(1);
    scheduler.add_node(create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));

    let api_service = create_test_service_spec("api-service");
    scheduler.add_service_spec(api_service);

    let first_round_actions = scheduler.schedule(None).unwrap();
    assert_eq!(
        action_instance_nodes(&first_round_actions, "api-service"),
        node_set(&["node1"])
    );
    assert_eq!(scheduler.replica_instances.len(), 1);

    for node_index in 2..=7 {
        let new_node_id = format!("node{}", node_index);
        let last_snapshot = scheduler.clone();

        scheduler.add_node(create_test_node(
            new_node_id.as_str(),
            4000,
            1024 * 1024 * 2048,
            vec!["core".to_string()],
            0.0,
            NodeState::New,
            format!("zone-{}", node_index).as_str(),
        ));

        let join_actions = scheduler.schedule(Some(&last_snapshot)).unwrap();
        assert_eq!(join_actions.len(), 1);
        assert!(matches!(
            &join_actions[0],
            SchedulerAction::ChangeNodeStatus(node_id, NodeState::Prepare)
                if node_id == &new_node_id
        ));
        assert_eq!(scheduler.replica_instances.len(), 1);
        assert_eq!(
            scheduler
                .replica_instances
                .values()
                .next()
                .unwrap()
                .node_id
                .as_str(),
            "node1"
        );

        scheduler.nodes.get_mut(&new_node_id).unwrap().state = NodeState::Ready;
        let ready_snapshot = scheduler.clone();
        let steady_actions = scheduler.schedule(Some(&ready_snapshot)).unwrap();
        assert!(
            steady_actions.is_empty(),
            "unexpected actions after {} became ready: {:?}",
            new_node_id,
            steady_actions
        );
        assert_eq!(scheduler.replica_instances.len(), 1);
        assert_eq!(
            scheduler
                .replica_instances
                .values()
                .next()
                .unwrap()
                .node_id
                .as_str(),
            "node1"
        );
    }
}

#[test]
fn test_service_info_only_publishes_alive_running_replicas() {
    let mut scheduler = NodeScheduler::new_empty(1);
    for node_id in ["node1", "node2", "node3"] {
        scheduler.add_node(create_test_node(
            node_id,
            4000,
            1024 * 1024 * 2048,
            vec!["search".to_string()],
            0.0,
            NodeState::Ready,
            "zone-search",
        ));
    }

    let mut search_service = create_test_service_spec("search-service");
    search_service.state = ServiceSpecState::Deployed;
    scheduler.add_service_spec(search_service);

    let now = buckyos_get_unix_timestamp();
    scheduler.add_replica_instance(create_test_replica_instance(
        "search-service",
        "node1",
        InstanceState::Running,
        now,
    ));
    scheduler.add_replica_instance(create_test_replica_instance(
        "search-service",
        "node2",
        InstanceState::Running,
        0,
    ));
    scheduler.add_replica_instance(create_test_replica_instance(
        "search-service",
        "node3",
        InstanceState::Suspended,
        now,
    ));

    let first_actions = scheduler.schedule(None).unwrap();
    assert_eq!(first_actions.len(), 1);
    match &first_actions[0] {
        SchedulerAction::UpdateServiceInfo(spec_id, ServiceInfo::SingleInstance(instance)) => {
            assert_eq!(spec_id, "search-service");
            assert_eq!(instance.node_id, "node1");
        }
        other => panic!("unexpected action: {:?}", other),
    }
    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("search-service").unwrap()),
        node_set(&["node1"])
    );
    assert!(
        scheduler
            .service_info_refresh_times
            .get("search-service")
            .copied()
            .unwrap_or(0)
            > 0
    );

    let last_snapshot = scheduler.clone();
    let mut last_snapshot = last_snapshot;
    let refresh_time = last_snapshot
        .service_info_refresh_times
        .get("search-service")
        .copied()
        .unwrap();
    last_snapshot.service_info_refresh_times.insert(
        "search-service".to_string(),
        refresh_time.saturating_sub(31),
    );
    scheduler
        .replica_instances
        .get_mut("search-service@node2")
        .unwrap()
        .last_update_time = buckyos_get_unix_timestamp();

    let second_actions = scheduler.schedule(Some(&last_snapshot)).unwrap();
    assert_eq!(second_actions.len(), 1);
    match &second_actions[0] {
        SchedulerAction::UpdateServiceInfo(spec_id, ServiceInfo::RandomCluster(cluster)) => {
            assert_eq!(spec_id, "search-service");
            assert_eq!(cluster.len(), 2);
            assert_eq!(
                cluster
                    .values()
                    .map(|(_, instance)| instance.node_id.clone())
                    .collect::<HashSet<_>>(),
                node_set(&["node1", "node2"])
            );
        }
        other => panic!("unexpected action: {:?}", other),
    }
    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("search-service").unwrap()),
        node_set(&["node1", "node2"])
    );
}

#[test]
fn test_service_info_refresh_is_throttled_within_30_seconds() {
    let mut last_snapshot = NodeScheduler::new_empty(1);
    last_snapshot.add_node(create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));

    let mut api_service = create_test_service_spec("api-service");
    api_service.state = ServiceSpecState::Deployed;
    last_snapshot.add_service_spec(api_service.clone());

    let now = buckyos_get_unix_timestamp();
    last_snapshot.add_replica_instance(create_test_replica_instance(
        "api-service",
        "node1",
        InstanceState::Running,
        now,
    ));

    let first_actions = last_snapshot.schedule(None).unwrap();
    assert_eq!(first_actions.len(), 1);

    let first_refresh_time = last_snapshot
        .service_info_refresh_times
        .get("api-service")
        .copied()
        .unwrap();

    let mut scheduler = NodeScheduler::new_empty(2);
    scheduler.add_node(create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));
    scheduler.add_service_spec(api_service);

    let actions = scheduler.schedule(Some(&last_snapshot)).unwrap();
    assert!(
        actions.is_empty(),
        "service_info refresh should be throttled within 30s: {:?}",
        actions
    );
    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("api-service").unwrap()),
        node_set(&["node1"])
    );
    assert_eq!(
        scheduler
            .service_info_refresh_times
            .get("api-service")
            .copied()
            .unwrap(),
        first_refresh_time
    );
}

#[test]
fn test_service_info_refresh_allows_update_after_30_seconds() {
    let mut last_snapshot = NodeScheduler::new_empty(1);
    last_snapshot.add_node(create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));

    let mut api_service = create_test_service_spec("api-service");
    api_service.state = ServiceSpecState::Deployed;
    last_snapshot.add_service_spec(api_service.clone());

    let now = buckyos_get_unix_timestamp();
    last_snapshot.add_replica_instance(create_test_replica_instance(
        "api-service",
        "node1",
        InstanceState::Running,
        now,
    ));
    last_snapshot.schedule(None).unwrap();
    last_snapshot
        .service_info_refresh_times
        .insert("api-service".to_string(), now.saturating_sub(31));

    let mut scheduler = NodeScheduler::new_empty(2);
    scheduler.add_node(create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));
    scheduler.add_service_spec(api_service);

    let actions = scheduler.schedule(Some(&last_snapshot)).unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        SchedulerAction::UpdateServiceInfo(spec_id, ServiceInfo::RandomCluster(cluster)) => {
            assert_eq!(spec_id, "api-service");
            assert!(cluster.is_empty());
        }
        other => panic!("unexpected action: {:?}", other),
    }
    match scheduler.service_infos.get("api-service").unwrap() {
        ServiceInfo::RandomCluster(cluster) => assert!(cluster.is_empty()),
        other => panic!("unexpected service info: {:?}", other),
    }
    assert!(
        scheduler
            .service_info_refresh_times
            .get("api-service")
            .copied()
            .unwrap()
            >= now
    );
}

#[test]
fn test_deployed_service_without_instances_recreates_replica() {
    let mut scheduler = NodeScheduler::new_empty(1);
    scheduler.add_node(create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));
    scheduler.add_node(create_test_node(
        "node2",
        5000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));

    let mut api_service = create_test_service_spec("api-service");
    api_service.state = ServiceSpecState::Deployed;
    scheduler.add_service_spec(api_service);

    let actions = scheduler.schedule(None).unwrap();
    assert_eq!(
        action_instance_nodes(&actions, "api-service"),
        node_set(&["node2"])
    );
    assert_eq!(scheduler.replica_instances.len(), 1);
    assert!(
        actions.iter().all(|action| {
            !matches!(action, SchedulerAction::ChangeServiceStatus(spec_id, _) if spec_id == "api-service")
        }),
        "deployed service should not emit redundant state changes: {:?}",
        actions
    );
}

#[test]
fn test_best_instance_count_change_triggers_scale_out_for_deployed_service() {
    let mut scheduler = NodeScheduler::new_empty(1);
    scheduler.add_node(create_test_node(
        "node1",
        4000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));
    scheduler.add_node(create_test_node(
        "node2",
        5000,
        1024 * 1024 * 2048,
        vec!["core".to_string()],
        0.0,
        NodeState::Ready,
        "zone-1",
    ));

    let mut api_service = create_test_service_spec("api-service");
    api_service.state = ServiceSpecState::Deployed;
    scheduler.add_service_spec(api_service.clone());
    scheduler.add_replica_instance(create_test_replica_instance(
        "api-service",
        "node1",
        InstanceState::Running,
        buckyos_get_unix_timestamp(),
    ));

    let last_snapshot = scheduler.clone();
    scheduler
        .specs
        .get_mut("api-service")
        .unwrap()
        .best_instance_count = 2;

    let actions = scheduler.schedule(Some(&last_snapshot)).unwrap();
    assert_eq!(
        action_instance_nodes(&actions, "api-service"),
        node_set(&["node2"])
    );
    assert_eq!(scheduler.replica_instances.len(), 2);
    assert_eq!(
        service_info_nodes(scheduler.service_infos.get("api-service").unwrap()),
        node_set(&["node1", "node2"])
    );
}
