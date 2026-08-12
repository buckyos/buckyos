use klog::KLogLevel;
use klog::network::{
    KLogAppendRequest, KLogClusterStateResponse, KLogDataRequestType, KLogMetaPutRequest,
    KLogMetaQueryRequest, KLogQueryRequest,
};
use klog::rpc::KLogClient;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::process::{Child, Command};
use tokio::time::sleep;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteTarget {
    Leader,
    RoundRobin,
    Random,
}

impl WriteTarget {
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "leader" => Ok(Self::Leader),
            "round-robin" | "round_robin" | "roundrobin" => Ok(Self::RoundRobin),
            "random" => Ok(Self::Random),
            other => Err(format!(
                "invalid --write-target value: {} (expected leader|round-robin|random)",
                other
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Leader => "leader",
            Self::RoundRobin => "round-robin",
            Self::Random => "random",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum BenchOperation {
    Append,
    Query,
    MetaPut,
    MetaQuery,
}

impl BenchOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Query => "query",
            Self::MetaPut => "meta-put",
            Self::MetaQuery => "meta-query",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkloadMix {
    append_weight: u32,
    query_weight: u32,
    meta_put_weight: u32,
    meta_query_weight: u32,
    query_limit: usize,
    query_strong_read: bool,
    meta_query_strong_read: bool,
    meta_key_space: u64,
}

impl Default for WorkloadMix {
    fn default() -> Self {
        Self {
            append_weight: 100,
            query_weight: 0,
            meta_put_weight: 0,
            meta_query_weight: 0,
            query_limit: 20,
            query_strong_read: false,
            meta_query_strong_read: false,
            meta_key_space: 1024,
        }
    }
}

impl WorkloadMix {
    fn total_weight(&self) -> u32 {
        self.append_weight
            .saturating_add(self.query_weight)
            .saturating_add(self.meta_put_weight)
            .saturating_add(self.meta_query_weight)
    }

    fn choose_operation(&self) -> BenchOperation {
        let total = self.total_weight();
        let pick = rand::random::<u32>() % total;

        let mut cursor = self.append_weight;
        if pick < cursor {
            return BenchOperation::Append;
        }

        cursor = cursor.saturating_add(self.query_weight);
        if pick < cursor {
            return BenchOperation::Query;
        }

        cursor = cursor.saturating_add(self.meta_put_weight);
        if pick < cursor {
            return BenchOperation::MetaPut;
        }

        BenchOperation::MetaQuery
    }

    fn validate(&self) -> Result<(), String> {
        if self.total_weight() == 0 {
            return Err(
                "invalid workload: append/query/meta-put/meta-query weights sum must be > 0"
                    .to_string(),
            );
        }
        if self.query_limit == 0 {
            return Err("invalid workload: query_limit must be > 0".to_string());
        }
        if self.meta_key_space == 0 {
            return Err("invalid workload: meta_key_space must be > 0".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkloadMixPatch {
    append_weight: Option<u32>,
    query_weight: Option<u32>,
    meta_put_weight: Option<u32>,
    meta_query_weight: Option<u32>,
    query_limit: Option<usize>,
    query_strong_read: Option<bool>,
    meta_query_strong_read: Option<bool>,
    meta_key_space: Option<u64>,
}

impl WorkloadMix {
    fn apply_patch(&mut self, patch: WorkloadMixPatch) {
        if let Some(v) = patch.append_weight {
            self.append_weight = v;
        }
        if let Some(v) = patch.query_weight {
            self.query_weight = v;
        }
        if let Some(v) = patch.meta_put_weight {
            self.meta_put_weight = v;
        }
        if let Some(v) = patch.meta_query_weight {
            self.meta_query_weight = v;
        }
        if let Some(v) = patch.query_limit {
            self.query_limit = v;
        }
        if let Some(v) = patch.query_strong_read {
            self.query_strong_read = v;
        }
        if let Some(v) = patch.meta_query_strong_read {
            self.meta_query_strong_read = v;
        }
        if let Some(v) = patch.meta_key_space {
            self.meta_key_space = v;
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchConfigFile {
    nodes: Option<usize>,
    concurrency: Option<usize>,
    duration_sec: Option<u64>,
    warmup_sec: Option<u64>,
    payload_bytes: Option<usize>,
    write_target: Option<String>,
    daemon_bin: Option<PathBuf>,
    cluster_name: Option<String>,
    request_node_id: Option<u64>,
    sync_write: Option<bool>,
    report_json: Option<PathBuf>,
    keep_data: Option<bool>,
    workload: Option<WorkloadMixPatch>,
    fault: Option<FaultInjectConfigPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FaultInjectConfig {
    enabled: bool,
    kill_leader_at_sec: Option<u64>,
    wait_new_leader_timeout_sec: u64,
}

impl Default for FaultInjectConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kill_leader_at_sec: None,
            wait_new_leader_timeout_sec: 20,
        }
    }
}

impl FaultInjectConfig {
    fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.kill_leader_at_sec.is_none() {
            return Err(
                "invalid fault config: enabled=true but kill_leader_at_sec is not set".to_string(),
            );
        }
        if self.wait_new_leader_timeout_sec == 0 {
            return Err(
                "invalid fault config: wait_new_leader_timeout_sec must be > 0".to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultInjectConfigPatch {
    enabled: Option<bool>,
    kill_leader_at_sec: Option<u64>,
    wait_new_leader_timeout_sec: Option<u64>,
}

impl FaultInjectConfig {
    fn apply_patch(&mut self, patch: FaultInjectConfigPatch) {
        if let Some(v) = patch.enabled {
            self.enabled = v;
        }
        if let Some(v) = patch.kill_leader_at_sec {
            self.kill_leader_at_sec = Some(v);
            self.enabled = true;
        }
        if let Some(v) = patch.wait_new_leader_timeout_sec {
            self.wait_new_leader_timeout_sec = v;
        }
    }
}

#[derive(Debug, Clone)]
struct BenchConfig {
    nodes: usize,
    concurrency: usize,
    duration_sec: u64,
    warmup_sec: u64,
    payload_bytes: usize,
    write_target: WriteTarget,
    daemon_bin: Option<PathBuf>,
    cluster_name: Option<String>,
    request_node_id: u64,
    sync_write: bool,
    report_json: Option<PathBuf>,
    keep_data: bool,
    workload: WorkloadMix,
    fault: FaultInjectConfig,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            nodes: 3,
            concurrency: 32,
            duration_sec: 30,
            warmup_sec: 3,
            payload_bytes: 256,
            write_target: WriteTarget::RoundRobin,
            daemon_bin: None,
            cluster_name: None,
            request_node_id: 9_001,
            sync_write: true,
            report_json: None,
            keep_data: false,
            workload: WorkloadMix::default(),
            fault: FaultInjectConfig::default(),
        }
    }
}

impl BenchConfig {
    fn validate(&self) -> Result<(), String> {
        if self.nodes == 0 {
            return Err("--nodes must be > 0".to_string());
        }
        if self.concurrency == 0 {
            return Err("--concurrency must be > 0".to_string());
        }
        if self.duration_sec == 0 {
            return Err("--duration-sec must be > 0".to_string());
        }
        if self.payload_bytes == 0 {
            return Err("--payload-bytes must be > 0".to_string());
        }
        if self.request_node_id == 0 {
            return Err("--request-node-id must be > 0".to_string());
        }
        self.workload.validate()?;
        self.fault.validate()?;
        Ok(())
    }

    fn apply_file_patch(&mut self, path: &Path) -> Result<(), String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("failed to read config file {}: {}", path.display(), e))?;
        let patch: BenchConfigFile = toml::from_str(&content)
            .map_err(|e| format!("failed to parse config file {}: {}", path.display(), e))?;

        if let Some(v) = patch.nodes {
            self.nodes = v;
        }
        if let Some(v) = patch.concurrency {
            self.concurrency = v;
        }
        if let Some(v) = patch.duration_sec {
            self.duration_sec = v;
        }
        if let Some(v) = patch.warmup_sec {
            self.warmup_sec = v;
        }
        if let Some(v) = patch.payload_bytes {
            self.payload_bytes = v;
        }
        if let Some(v) = patch.write_target {
            self.write_target = WriteTarget::parse(&v)?;
        }
        if let Some(v) = patch.daemon_bin {
            self.daemon_bin = Some(v);
        }
        if let Some(v) = patch.cluster_name {
            self.cluster_name = Some(v);
        }
        if let Some(v) = patch.request_node_id {
            self.request_node_id = v;
        }
        if let Some(v) = patch.sync_write {
            self.sync_write = v;
        }
        if let Some(v) = patch.report_json {
            self.report_json = Some(v);
        }
        if let Some(v) = patch.keep_data {
            self.keep_data = v;
        }
        if let Some(v) = patch.workload {
            self.workload.apply_patch(v);
        }
        if let Some(v) = patch.fault {
            self.fault.apply_patch(v);
        }

        Ok(())
    }
}

#[derive(Debug)]
struct ManagedNode {
    node_id: u64,
    admin_port: u16,
    rpc_port: u16,
    data_dir: PathBuf,
    config_path: PathBuf,
    child: Child,
}

#[derive(Debug, Clone)]
struct NodeSnapshot {
    node_id: u64,
    admin_port: u16,
    pid: u32,
}

impl ManagedNode {
    async fn stop(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    fn force_kill(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[derive(Debug, Default)]
struct WorkerStats {
    success: u64,
    fail: u64,
    latency_us: Vec<u64>,
    error_code_counts: HashMap<String, u64>,
    operation_stats: HashMap<String, RawOperationStats>,
    append_ids: Vec<u64>,
}

#[derive(Debug, Default)]
struct RawOperationStats {
    success: u64,
    fail: u64,
    latency_us: Vec<u64>,
}

impl WorkerStats {
    fn merge(&mut self, other: WorkerStats) {
        self.success += other.success;
        self.fail += other.fail;
        self.latency_us.extend(other.latency_us);
        for (k, v) in other.error_code_counts {
            *self.error_code_counts.entry(k).or_insert(0) += v;
        }
        for (k, v) in other.operation_stats {
            let op = self.operation_stats.entry(k).or_default();
            op.success += v.success;
            op.fail += v.fail;
            op.latency_us.extend(v.latency_us);
        }
        self.append_ids.extend(other.append_ids);
    }

    fn record_success(&mut self, operation: BenchOperation, latency_us: u64) {
        self.success += 1;
        self.latency_us.push(latency_us);
        let op = self
            .operation_stats
            .entry(operation.as_str().to_string())
            .or_default();
        op.success += 1;
        op.latency_us.push(latency_us);
    }

    fn record_append_id(&mut self, id: u64) {
        self.append_ids.push(id);
    }

    fn record_failure(&mut self, operation: BenchOperation, error_code: String) {
        self.fail += 1;
        *self.error_code_counts.entry(error_code).or_insert(0) += 1;
        let op = self
            .operation_stats
            .entry(operation.as_str().to_string())
            .or_default();
        op.fail += 1;
    }
}

#[derive(Debug, Serialize)]
struct OperationStats {
    total_requests: u64,
    success_requests: u64,
    failed_requests: u64,
    success_rate: f64,
    throughput_tps: f64,
    latency_avg_ms: f64,
    latency_p50_ms: f64,
    latency_p95_ms: f64,
    latency_p99_ms: f64,
    latency_max_ms: f64,
}

#[derive(Debug, Serialize)]
struct BenchReport {
    cluster_name: String,
    nodes: usize,
    write_target: String,
    duration_sec: u64,
    warmup_sec: u64,
    payload_bytes: usize,
    concurrency: usize,
    workload: WorkloadMix,
    fault: FaultInjectReport,
    started_at_unix_ms: u64,
    finished_at_unix_ms: u64,
    total_requests: u64,
    success_requests: u64,
    failed_requests: u64,
    success_rate: f64,
    throughput_tps: f64,
    latency_avg_ms: f64,
    latency_p50_ms: f64,
    latency_p95_ms: f64,
    latency_p99_ms: f64,
    latency_max_ms: f64,
    error_code_counts: BTreeMap<String, u64>,
    operation_stats: BTreeMap<String, OperationStats>,
    append_unique_id_count: usize,
    append_duplicate_id_count: usize,
    node_max_log_ids: BTreeMap<u64, u64>,
    node_max_log_id_consistent: bool,
    node_rpc_ports: BTreeMap<u64, u16>,
    leader_node_id: u64,
}

#[derive(Debug, Clone, Serialize)]
struct FaultInjectReport {
    enabled: bool,
    kill_leader_at_sec: Option<u64>,
    injected: bool,
    old_leader_node_id: Option<u64>,
    new_leader_node_id: Option<u64>,
    injected_at_unix_ms: Option<u64>,
    new_leader_observed_at_unix_ms: Option<u64>,
    first_success_after_fault_unix_ms: Option<u64>,
    leader_failover_ms: Option<u64>,
    first_success_recovery_ms: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct FaultInjectTaskResult {
    injected: bool,
    old_leader_node_id: Option<u64>,
    new_leader_node_id: Option<u64>,
    injected_at_unix_ms: Option<u64>,
    new_leader_observed_at_unix_ms: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Default)]
struct FaultRuntimeShared {
    injected_at_unix_ms: AtomicU64,
    first_success_after_fault_unix_ms: AtomicU64,
}

const HELP: &str = r#"klog_bench: local stress benchmark for klog_daemon

Usage:
  cargo run -p klog_daemon --bin klog_bench -- [options]

Options:
  --config <PATH>            Load benchmark config from TOML file
  --nodes <N>                Number of managed nodes (default: 3)
  --concurrency <N>          Concurrent workers (default: 32)
  --duration-sec <N>         Measure duration seconds (default: 30)
  --warmup-sec <N>           Warmup duration seconds (default: 3)
  --payload-bytes <N>        append/meta value payload bytes (default: 256)
  --write-target <MODE>      leader|round-robin|random (default: round-robin)
  --append-weight <N>        Workload weight for append (default: 100)
  --query-weight <N>         Workload weight for query (default: 0)
  --meta-put-weight <N>      Workload weight for meta put (default: 0)
  --meta-query-weight <N>    Workload weight for meta query (default: 0)
  --query-limit <N>          Query request limit parameter (default: 20)
  --query-strong-read <BOOL> Query strong_read mode (default: false)
  --meta-query-strong-read <BOOL>  Meta query strong_read mode (default: false)
  --meta-key-space <N>       Number of meta keys for random access (default: 1024)
  --fault-kill-leader-at-sec <N>  Inject fault by killing current leader at N seconds
  --fault-wait-new-leader-timeout-sec <N>  Timeout waiting new leader after fault (default: 20)
  --request-node-id <ID>     request node id for generated request_id (default: 9001)
  --sync-write <true|false>  state store sync write mode (default: true)
  --cluster-name <NAME>      optional explicit cluster name
  --daemon-bin <PATH>        optional klog_daemon executable path
  --report-json <PATH>       optional json report output path
  --keep-data                keep temporary node data dirs after run
  --help                     show this help
"#;

#[tokio::main]
async fn main() {
    if let Err(e) = run_main().await {
        eprintln!("klog_bench failed: {}", e);
        std::process::exit(1);
    }
}

async fn run_main() -> Result<(), String> {
    let cfg = parse_args(std::env::args().skip(1).collect())?;
    cfg.validate()?;

    let daemon_bin = resolve_daemon_bin(cfg.daemon_bin.clone())?;
    let cluster_name = cfg
        .cluster_name
        .clone()
        .unwrap_or_else(|| format!("klog_bench_{}_{}", std::process::id(), now_unix_ms()));

    println!(
        "klog_bench starting: cluster_name={}, nodes={}, concurrency={}, duration_sec={}, warmup_sec={}, payload_bytes={}, write_target={}, daemon_bin={}, workload(append={}, query={}, meta_put={}, meta_query={}, query_limit={}, query_strong_read={}, meta_query_strong_read={}, meta_key_space={}), fault(enabled={}, kill_leader_at_sec={:?}, wait_new_leader_timeout_sec={})",
        cluster_name,
        cfg.nodes,
        cfg.concurrency,
        cfg.duration_sec,
        cfg.warmup_sec,
        cfg.payload_bytes,
        cfg.write_target.as_str(),
        daemon_bin.display(),
        cfg.workload.append_weight,
        cfg.workload.query_weight,
        cfg.workload.meta_put_weight,
        cfg.workload.meta_query_weight,
        cfg.workload.query_limit,
        cfg.workload.query_strong_read,
        cfg.workload.meta_query_strong_read,
        cfg.workload.meta_key_space,
        cfg.fault.enabled,
        cfg.fault.kill_leader_at_sec,
        cfg.fault.wait_new_leader_timeout_sec
    );

    let mut nodes = spawn_managed_cluster(&cfg, &cluster_name, &daemon_bin).await?;
    let node_snapshots = nodes
        .iter()
        .map(|n| {
            let pid = n.child.id().ok_or_else(|| {
                format!(
                    "failed to get process id for node_id={}, admin_port={}",
                    n.node_id, n.admin_port
                )
            })?;
            Ok(NodeSnapshot {
                node_id: n.node_id,
                admin_port: n.admin_port,
                pid,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let run_res = async {
        let leader_id =
            wait_consistent_leader(&admin_ports(&nodes), Duration::from_secs(40)).await?;
        println!("cluster ready: leader_node_id={}", leader_id);

        let rpc_ports = nodes
            .iter()
            .map(|n| (n.node_id, n.rpc_port))
            .collect::<BTreeMap<_, _>>();

        let leader_rpc_port = rpc_ports
            .get(&leader_id)
            .copied()
            .ok_or_else(|| format!("leader rpc port not found: leader_id={}", leader_id))?;

        if cfg.warmup_sec > 0 {
            println!("warmup start: {}s", cfg.warmup_sec);
            let warmup_deadline = Instant::now() + Duration::from_secs(cfg.warmup_sec);
            run_workload_phase(
                &cfg,
                &rpc_ports,
                leader_rpc_port,
                warmup_deadline,
                false,
                None,
            )
            .await?;
            println!("warmup done");
        }

        println!("measure start: {}s", cfg.duration_sec);
        let started_at = now_unix_ms();
        let started = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(cfg.duration_sec);
        let fault_shared = Arc::new(FaultRuntimeShared::default());
        let mut fault_task_result = FaultInjectTaskResult::default();
        let fault_task = if cfg.fault.enabled {
            if let Some(kill_at_sec) = cfg.fault.kill_leader_at_sec {
                if kill_at_sec < cfg.duration_sec {
                    let snapshots = node_snapshots.clone();
                    let shared = Arc::clone(&fault_shared);
                    let wait_timeout = cfg.fault.wait_new_leader_timeout_sec;
                    Some(tokio::spawn(async move {
                        run_fault_injector_task(snapshots, kill_at_sec, wait_timeout, shared).await
                    }))
                } else {
                    fault_task_result.error = Some(format!(
                        "fault kill_leader_at_sec={} >= duration_sec={}, skip injection",
                        kill_at_sec, cfg.duration_sec
                    ));
                    None
                }
            } else {
                fault_task_result.error =
                    Some("fault.enabled=true but kill_leader_at_sec is not set".to_string());
                None
            }
        } else {
            None
        };

        let stats = run_workload_phase(
            &cfg,
            &rpc_ports,
            leader_rpc_port,
            deadline,
            true,
            Some(Arc::clone(&fault_shared)),
        )
        .await?;

        if let Some(handle) = fault_task {
            fault_task_result = handle
                .await
                .map_err(|e| format!("fault injector task join failed: {}", e))?;
        }
        let elapsed = started.elapsed();
        let finished_at = now_unix_ms();
        let node_max_log_ids = collect_node_max_log_ids(&rpc_ports, cfg.request_node_id).await;
        let fault_report = build_fault_report(&cfg, &fault_shared, fault_task_result);

        let report = build_report(
            &cfg,
            &cluster_name,
            &rpc_ports,
            leader_id,
            fault_report,
            node_max_log_ids,
            started_at,
            finished_at,
            elapsed,
            stats,
        );

        print_report(&report);
        if let Some(path) = &cfg.report_json {
            write_report_json(path, &report)?;
            println!("report written: {}", path.display());
        }

        Ok::<(), String>(())
    }
    .await;

    for node in &mut nodes {
        node.stop().await;
    }
    if !cfg.keep_data {
        for node in &nodes {
            let _ = fs::remove_file(&node.config_path);
            let _ = fs::remove_dir_all(&node.data_dir);
        }
    }

    run_res
}

async fn spawn_managed_cluster(
    cfg: &BenchConfig,
    cluster_name: &str,
    daemon_bin: &Path,
) -> Result<Vec<ManagedNode>, String> {
    let mut nodes: Vec<ManagedNode> = Vec::with_capacity(cfg.nodes);
    let mut used_ports = HashSet::new();

    for idx in 0..cfg.nodes {
        let node_id = (idx + 1) as u64;
        let raft_port = pick_unused_port(&mut used_ports)?;
        let inter_node_port = pick_unused_port(&mut used_ports)?;
        let admin_port = pick_unused_port(&mut used_ports)?;
        let rpc_port = pick_unused_port(&mut used_ports)?;

        let join_targets = if idx == 0 {
            Vec::new()
        } else {
            vec![format!("127.0.0.1:{}", nodes[0].admin_port)]
        };

        let data_dir = unique_tmp_path(&format!("bench_node{}_data", node_id));
        let config_path = unique_tmp_path(&format!("bench_node{}_config.toml", node_id));
        fs::create_dir_all(&data_dir)
            .map_err(|e| format!("failed to create data dir {}: {}", data_dir.display(), e))?;

        write_config_file(
            &config_path,
            node_id,
            raft_port,
            inter_node_port,
            admin_port,
            rpc_port,
            &data_dir,
            cluster_name,
            idx == 0,
            &join_targets,
            cfg.sync_write,
        )?;

        let mut child = Command::new(daemon_bin)
            .env("KLOG_CONFIG_FILE", &config_path)
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                format!(
                    "failed to spawn klog_daemon: bin={}, config={}, err={}",
                    daemon_bin.display(),
                    config_path.display(),
                    e
                )
            })?;

        if let Err(e) = wait_node_http_ready(
            &mut child,
            node_id,
            admin_port,
            inter_node_port,
            rpc_port,
            Duration::from_secs(15),
        )
        .await
        {
            let _ = child.kill().await;
            return Err(e);
        }

        nodes.push(ManagedNode {
            node_id,
            admin_port,
            rpc_port,
            data_dir,
            config_path,
            child,
        });
    }

    let ports = admin_ports(&nodes);
    let expected = (1..=(cfg.nodes as u64)).collect::<Vec<_>>();
    wait_cluster_voters(&ports, &expected, Duration::from_secs(60)).await?;
    Ok(nodes)
}

async fn run_workload_phase(
    cfg: &BenchConfig,
    rpc_ports: &BTreeMap<u64, u16>,
    leader_rpc_port: u16,
    deadline: Instant,
    show_progress: bool,
    fault_shared: Option<Arc<FaultRuntimeShared>>,
) -> Result<WorkerStats, String> {
    let ports = rpc_ports.values().copied().collect::<Vec<_>>();
    if ports.is_empty() {
        return Err("no rpc ports available".to_string());
    }
    let leader_endpoint = format!("127.0.0.1:{}", leader_rpc_port);
    let endpoint_addrs = ports
        .iter()
        .map(|p| format!("127.0.0.1:{}", p))
        .collect::<Vec<_>>();
    let payload = "x".repeat(cfg.payload_bytes);

    let req_total = Arc::new(AtomicU64::new(0));
    let req_success = Arc::new(AtomicU64::new(0));
    let req_fail = Arc::new(AtomicU64::new(0));

    let progress_task = if show_progress {
        let req_total = Arc::clone(&req_total);
        let req_success = Arc::clone(&req_success);
        let req_fail = Arc::clone(&req_fail);
        let duration = Duration::from_millis(1_000);
        Some(tokio::spawn(async move {
            let mut last_total = 0_u64;
            loop {
                sleep(duration).await;
                let now_total = req_total.load(Ordering::Relaxed);
                let now_success = req_success.load(Ordering::Relaxed);
                let now_fail = req_fail.load(Ordering::Relaxed);
                let delta = now_total.saturating_sub(last_total);
                last_total = now_total;
                println!(
                    "progress: total={}, success={}, fail={}, instant_rps={}",
                    now_total, now_success, now_fail, delta
                );
                if Instant::now() >= deadline {
                    break;
                }
            }
        }))
    } else {
        None
    };

    let mut tasks = Vec::with_capacity(cfg.concurrency);
    for worker_id in 0..cfg.concurrency {
        let endpoint_addrs = endpoint_addrs.clone();
        let leader_endpoint = leader_endpoint.clone();
        let payload = payload.clone();
        let req_total = Arc::clone(&req_total);
        let req_success = Arc::clone(&req_success);
        let req_fail = Arc::clone(&req_fail);
        let write_target = cfg.write_target;
        let request_node_id = cfg.request_node_id;
        let fault_shared = fault_shared.clone();

        let workload = cfg.workload.clone();

        tasks.push(tokio::spawn(async move {
            let mut stats = WorkerStats::default();
            let mut rr = worker_id;
            let mut append_seq = 0_u64;
            let timeout = Duration::from_secs(4);
            let leader_client =
                KLogClient::from_daemon_addr(leader_endpoint.as_str(), request_node_id)
                    .with_timeout(timeout);
            let clients = endpoint_addrs
                .iter()
                .map(|endpoint| {
                    KLogClient::from_daemon_addr(endpoint.as_str(), request_node_id)
                        .with_timeout(timeout)
                })
                .collect::<Vec<_>>();
            let base_append_req = KLogAppendRequest {
                message: payload.clone(),
                timestamp: None,
                node_id: None,
                level: Some(KLogLevel::Info),
                source: Some("klog_bench".to_string()),
                attrs: None,
                request_id: None,
            };
            let base_query_req = KLogQueryRequest {
                start_id: None,
                end_id: None,
                limit: Some(workload.query_limit),
                desc: Some(true),
                level: None,
                source: None,
                attr_key: None,
                attr_value: None,
                strong_read: Some(workload.query_strong_read),
            };

            while Instant::now() < deadline {
                let client = match write_target {
                    WriteTarget::Leader => &leader_client,
                    WriteTarget::RoundRobin => {
                        let idx = rr % endpoint_addrs.len();
                        rr = rr.wrapping_add(1);
                        &clients[idx]
                    }
                    WriteTarget::Random => {
                        let idx = (rand::random::<u64>() as usize) % endpoint_addrs.len();
                        &clients[idx]
                    }
                };

                let operation = workload.choose_operation();
                req_total.fetch_add(1, Ordering::Relaxed);
                let begin = Instant::now();
                let result: Result<Option<u64>, _> = match operation {
                    BenchOperation::Append => {
                        append_seq = append_seq.wrapping_add(1);
                        let mut req = base_append_req.clone();
                        req.request_id = Some(format!(
                            "bench-{}-{}-{}",
                            request_node_id, worker_id, append_seq
                        ));
                        client.append_log(req).await.map(|resp| Some(resp.id))
                    }
                    BenchOperation::Query => {
                        client.query_log(base_query_req.clone()).await.map(|_| None)
                    }
                    BenchOperation::MetaPut => {
                        let key_idx = rand::random::<u64>() % workload.meta_key_space;
                        let req = KLogMetaPutRequest {
                            key: format!("bench/meta/{}", key_idx),
                            value: payload.clone(),
                            expected_revision: None,
                        };
                        client.put_meta(req).await.map(|_| None)
                    }
                    BenchOperation::MetaQuery => {
                        let key_idx = rand::random::<u64>() % workload.meta_key_space;
                        let req = KLogMetaQueryRequest {
                            key: Some(format!("bench/meta/{}", key_idx)),
                            prefix: None,
                            limit: Some(1),
                            strong_read: Some(workload.meta_query_strong_read),
                        };
                        client.query_meta(req).await.map(|_| None)
                    }
                };

                match result {
                    Ok(maybe_append_id) => {
                        req_success.fetch_add(1, Ordering::Relaxed);
                        stats.record_success(operation, begin.elapsed().as_micros() as u64);
                        if let Some(id) = maybe_append_id {
                            stats.record_append_id(id);
                        }
                        if let Some(shared) = fault_shared.as_ref() {
                            let injected_at = shared.injected_at_unix_ms.load(Ordering::Relaxed);
                            if injected_at != 0 {
                                let _ = shared.first_success_after_fault_unix_ms.compare_exchange(
                                    0,
                                    now_unix_ms(),
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        req_fail.fetch_add(1, Ordering::Relaxed);
                        stats.record_failure(operation, format!("{:?}", e.error_code));
                    }
                }
            }

            stats
        }));
    }

    let mut merged = WorkerStats::default();
    for t in tasks {
        let stats = t
            .await
            .map_err(|e| format!("workload worker join failed: {}", e))?;
        merged.merge(stats);
    }

    if let Some(task) = progress_task {
        let _ = task.await;
    }

    Ok(merged)
}

fn build_report(
    cfg: &BenchConfig,
    cluster_name: &str,
    node_rpc_ports: &BTreeMap<u64, u16>,
    leader_node_id: u64,
    fault: FaultInjectReport,
    node_max_log_ids: BTreeMap<u64, u64>,
    started_at_unix_ms: u64,
    finished_at_unix_ms: u64,
    elapsed: Duration,
    stats: WorkerStats,
) -> BenchReport {
    let total = stats.success + stats.fail;
    let success_rate = if total == 0 {
        0.0
    } else {
        stats.success as f64 / total as f64
    };
    let throughput = if elapsed.as_secs_f64() == 0.0 {
        0.0
    } else {
        stats.success as f64 / elapsed.as_secs_f64()
    };

    let mut lat = stats.latency_us;
    lat.sort_unstable();

    let avg_ms = if lat.is_empty() {
        0.0
    } else {
        (lat.iter().sum::<u64>() as f64 / lat.len() as f64) / 1000.0
    };

    let p50_ms = percentile_ms(&lat, 50.0);
    let p95_ms = percentile_ms(&lat, 95.0);
    let p99_ms = percentile_ms(&lat, 99.0);
    let max_ms = lat.last().copied().unwrap_or(0) as f64 / 1000.0;
    let operation_stats = stats
        .operation_stats
        .into_iter()
        .map(|(op, raw)| (op, build_operation_stats(raw, elapsed)))
        .collect::<BTreeMap<_, _>>();
    let append_unique_id_count = stats
        .append_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .len();
    let append_duplicate_id_count = stats
        .append_ids
        .len()
        .saturating_sub(append_unique_id_count);
    let node_max_log_id_consistent = {
        let uniq = node_max_log_ids.values().copied().collect::<HashSet<_>>();
        if uniq.is_empty() {
            false
        } else {
            uniq.len() == 1
        }
    };

    BenchReport {
        cluster_name: cluster_name.to_string(),
        nodes: cfg.nodes,
        write_target: cfg.write_target.as_str().to_string(),
        duration_sec: cfg.duration_sec,
        warmup_sec: cfg.warmup_sec,
        payload_bytes: cfg.payload_bytes,
        concurrency: cfg.concurrency,
        workload: cfg.workload.clone(),
        fault,
        started_at_unix_ms,
        finished_at_unix_ms,
        total_requests: total,
        success_requests: stats.success,
        failed_requests: stats.fail,
        success_rate,
        throughput_tps: throughput,
        latency_avg_ms: avg_ms,
        latency_p50_ms: p50_ms,
        latency_p95_ms: p95_ms,
        latency_p99_ms: p99_ms,
        latency_max_ms: max_ms,
        error_code_counts: stats.error_code_counts.into_iter().collect(),
        operation_stats,
        append_unique_id_count,
        append_duplicate_id_count,
        node_max_log_ids,
        node_max_log_id_consistent,
        node_rpc_ports: node_rpc_ports.clone(),
        leader_node_id,
    }
}

fn build_operation_stats(raw: RawOperationStats, elapsed: Duration) -> OperationStats {
    let total = raw.success + raw.fail;
    let success_rate = if total == 0 {
        0.0
    } else {
        raw.success as f64 / total as f64
    };
    let throughput = if elapsed.as_secs_f64() == 0.0 {
        0.0
    } else {
        raw.success as f64 / elapsed.as_secs_f64()
    };

    let mut lat = raw.latency_us;
    lat.sort_unstable();
    let latency_avg_ms = if lat.is_empty() {
        0.0
    } else {
        (lat.iter().sum::<u64>() as f64 / lat.len() as f64) / 1000.0
    };

    OperationStats {
        total_requests: total,
        success_requests: raw.success,
        failed_requests: raw.fail,
        success_rate,
        throughput_tps: throughput,
        latency_avg_ms,
        latency_p50_ms: percentile_ms(&lat, 50.0),
        latency_p95_ms: percentile_ms(&lat, 95.0),
        latency_p99_ms: percentile_ms(&lat, 99.0),
        latency_max_ms: lat.last().copied().unwrap_or(0) as f64 / 1000.0,
    }
}

fn build_fault_report(
    cfg: &BenchConfig,
    shared: &FaultRuntimeShared,
    mut task: FaultInjectTaskResult,
) -> FaultInjectReport {
    let injected_at_raw = shared.injected_at_unix_ms.load(Ordering::Relaxed);
    let first_success_raw = shared
        .first_success_after_fault_unix_ms
        .load(Ordering::Relaxed);

    let injected_at = if injected_at_raw == 0 {
        task.injected_at_unix_ms
    } else {
        Some(injected_at_raw)
    };
    if injected_at.is_none() && task.injected {
        task.error
            .get_or_insert_with(|| "fault injected but injected_at timestamp missing".to_string());
    }

    let first_success_after_fault = if first_success_raw == 0 {
        None
    } else {
        Some(first_success_raw)
    };
    let leader_failover_ms = match (injected_at, task.new_leader_observed_at_unix_ms) {
        (Some(start), Some(end)) if end >= start => Some(end - start),
        _ => None,
    };
    let first_success_recovery_ms = match (injected_at, first_success_after_fault) {
        (Some(start), Some(end)) if end >= start => Some(end - start),
        _ => None,
    };

    FaultInjectReport {
        enabled: cfg.fault.enabled,
        kill_leader_at_sec: cfg.fault.kill_leader_at_sec,
        injected: task.injected,
        old_leader_node_id: task.old_leader_node_id,
        new_leader_node_id: task.new_leader_node_id,
        injected_at_unix_ms: injected_at,
        new_leader_observed_at_unix_ms: task.new_leader_observed_at_unix_ms,
        first_success_after_fault_unix_ms: first_success_after_fault,
        leader_failover_ms,
        first_success_recovery_ms,
        error: task.error,
    }
}

async fn collect_node_max_log_ids(
    rpc_ports: &BTreeMap<u64, u16>,
    request_node_id: u64,
) -> BTreeMap<u64, u64> {
    let mut out = BTreeMap::new();
    for (node_id, rpc_port) in rpc_ports {
        let client =
            KLogClient::from_daemon_addr(&format!("127.0.0.1:{}", rpc_port), request_node_id)
                .with_timeout(Duration::from_secs(3));
        let req = KLogQueryRequest {
            start_id: None,
            end_id: None,
            limit: Some(1),
            desc: Some(true),
            level: None,
            source: None,
            attr_key: None,
            attr_value: None,
            strong_read: Some(true),
        };
        if let Ok(resp) = client.query_log(req).await {
            let max_id = resp.items.first().map(|e| e.id).unwrap_or(0);
            out.insert(*node_id, max_id);
        }
    }
    out
}

async fn run_fault_injector_task(
    snapshots: Vec<NodeSnapshot>,
    kill_leader_at_sec: u64,
    wait_new_leader_timeout_sec: u64,
    shared: Arc<FaultRuntimeShared>,
) -> FaultInjectTaskResult {
    let mut out = FaultInjectTaskResult::default();
    let admin_ports = snapshots.iter().map(|n| n.admin_port).collect::<Vec<_>>();
    let by_node = snapshots
        .iter()
        .map(|n| (n.node_id, n.clone()))
        .collect::<HashMap<_, _>>();

    sleep(Duration::from_secs(kill_leader_at_sec)).await;

    let old_leader = match wait_consistent_leader(&admin_ports, Duration::from_secs(15)).await {
        Ok(v) => v,
        Err(e) => {
            out.error = Some(format!(
                "fault inject failed before kill: unable to determine current leader: {}",
                e
            ));
            return out;
        }
    };
    out.old_leader_node_id = Some(old_leader);

    let leader_node = match by_node.get(&old_leader) {
        Some(v) => v,
        None => {
            out.error = Some(format!(
                "fault inject failed: leader node {} not found in snapshots",
                old_leader
            ));
            return out;
        }
    };

    if let Err(e) = kill_process_pid(leader_node.pid) {
        out.error = Some(format!(
            "fault inject failed: kill leader node_id={}, pid={} error={}",
            old_leader, leader_node.pid, e
        ));
        return out;
    }

    out.injected = true;
    out.injected_at_unix_ms = Some(now_unix_ms());
    if let Some(ts) = out.injected_at_unix_ms {
        shared.injected_at_unix_ms.store(ts, Ordering::Relaxed);
    }

    match wait_new_leader_with_tolerance(
        &admin_ports,
        old_leader,
        Duration::from_secs(wait_new_leader_timeout_sec),
    )
    .await
    {
        Ok(new_leader) => {
            out.new_leader_node_id = Some(new_leader);
            out.new_leader_observed_at_unix_ms = Some(now_unix_ms());
        }
        Err(e) => {
            out.error = Some(format!(
                "fault inject observe new leader failed after old_leader={}: {}",
                old_leader, e
            ));
        }
    }

    out
}

fn kill_process_pid(pid: u32) -> Result<(), String> {
    let output = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("spawn kill command failed: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "kill command failed: status={}, stdout={}, stderr={}",
        output.status, stdout, stderr
    ))
}

async fn wait_new_leader_with_tolerance(
    admin_ports: &[u16],
    old_leader: u64,
    timeout: Duration,
) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build new-leader client: {}", e))?;

    let quorum = admin_ports.len() / 2 + 1;
    let deadline = Instant::now() + timeout;
    let mut last_observation = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting new leader with tolerance: old_leader={}, quorum={}, last_observation={}",
                old_leader, quorum, last_observation
            ));
        }

        let mut leader_count = HashMap::<u64, usize>::new();
        let mut observations = Vec::new();
        for port in admin_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    observations.push(format!(
                        "port={}, node_id={}, leader={:?}, voters={:?}",
                        port, state.node_id, state.current_leader, state.voters
                    ));
                    if let Some(leader) = state.current_leader
                        && leader != old_leader
                    {
                        *leader_count.entry(leader).or_insert(0) += 1;
                    }
                }
                Err(e) => {
                    observations.push(format!("port={}, err={}", port, e));
                }
            }
        }

        last_observation = observations.join(" | ");
        for (leader, cnt) in leader_count {
            if cnt >= quorum {
                return Ok(leader);
            }
        }

        sleep(Duration::from_millis(200)).await;
    }
}

fn percentile_ms(sorted_lat_us: &[u64], p: f64) -> f64 {
    if sorted_lat_us.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * (sorted_lat_us.len() as f64 - 1.0)).round() as usize;
    sorted_lat_us[rank] as f64 / 1000.0
}

fn print_report(report: &BenchReport) {
    println!("===== klog_bench report =====");
    println!(
        "cluster={}, nodes={}, leader_node_id={}, write_target={}",
        report.cluster_name, report.nodes, report.leader_node_id, report.write_target
    );
    println!(
        "duration={}s (warmup={}s), concurrency={}, payload={}B",
        report.duration_sec, report.warmup_sec, report.concurrency, report.payload_bytes
    );
    println!(
        "workload: append={}, query={}, meta_put={}, meta_query={}, query_limit={}, query_strong_read={}, meta_query_strong_read={}, meta_key_space={}",
        report.workload.append_weight,
        report.workload.query_weight,
        report.workload.meta_put_weight,
        report.workload.meta_query_weight,
        report.workload.query_limit,
        report.workload.query_strong_read,
        report.workload.meta_query_strong_read,
        report.workload.meta_key_space
    );
    println!(
        "requests: total={}, success={}, fail={}, success_rate={:.2}%",
        report.total_requests,
        report.success_requests,
        report.failed_requests,
        report.success_rate * 100.0
    );
    println!("throughput: {:.2} req/s", report.throughput_tps);
    println!(
        "latency(ms): avg={:.3}, p50={:.3}, p95={:.3}, p99={:.3}, max={:.3}",
        report.latency_avg_ms,
        report.latency_p50_ms,
        report.latency_p95_ms,
        report.latency_p99_ms,
        report.latency_max_ms
    );
    if !report.error_code_counts.is_empty() {
        println!("error_code_counts:");
        for (k, v) in &report.error_code_counts {
            println!("  {} => {}", k, v);
        }
    }
    if !report.operation_stats.is_empty() {
        println!("operation_stats:");
        for (op, stat) in &report.operation_stats {
            println!(
                "  {}: total={}, success={}, fail={}, success_rate={:.2}%, tps={:.2}, p95={:.3}ms, p99={:.3}ms",
                op,
                stat.total_requests,
                stat.success_requests,
                stat.failed_requests,
                stat.success_rate * 100.0,
                stat.throughput_tps,
                stat.latency_p95_ms,
                stat.latency_p99_ms
            );
        }
    }
    println!(
        "correctness: append_unique_id_count={}, append_duplicate_id_count={}, node_max_log_id_consistent={}, node_max_log_ids={:?}",
        report.append_unique_id_count,
        report.append_duplicate_id_count,
        report.node_max_log_id_consistent,
        report.node_max_log_ids
    );
    println!(
        "fault: enabled={}, injected={}, old_leader={:?}, new_leader={:?}, leader_failover_ms={:?}, first_success_recovery_ms={:?}, error={:?}",
        report.fault.enabled,
        report.fault.injected,
        report.fault.old_leader_node_id,
        report.fault.new_leader_node_id,
        report.fault.leader_failover_ms,
        report.fault.first_success_recovery_ms,
        report.fault.error
    );
    println!("node_rpc_ports: {:?}", report.node_rpc_ports);
}

fn write_report_json(path: &Path, report: &BenchReport) -> Result<(), String> {
    let parent = path.parent().map(Path::to_path_buf);
    if let Some(parent) = parent
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(&parent)
            .map_err(|e| format!("failed to create report dir {}: {}", parent.display(), e))?;
    }

    let json = serde_json::to_string_pretty(report)
        .map_err(|e| format!("failed to serialize report json: {}", e))?;
    fs::write(path, json)
        .map_err(|e| format!("failed to write report json {}: {}", path.display(), e))
}

fn parse_args(args: Vec<String>) -> Result<BenchConfig, String> {
    let mut cfg = BenchConfig::default();
    if let Some(config_path) = extract_config_path(&args)? {
        cfg.apply_file_patch(&config_path)?;
    }

    let mut i = 0usize;

    while i < args.len() {
        let key = args[i].as_str();
        match key {
            "--help" | "-h" => {
                println!("{}", HELP);
                std::process::exit(0);
            }
            "--keep-data" => {
                cfg.keep_data = true;
                i += 1;
            }
            "--config" => {
                let _ = next_value(&args, i, "--config")?;
                i += 2;
            }
            "--nodes" => {
                cfg.nodes = parse_next::<usize>(&args, i, "--nodes")?;
                i += 2;
            }
            "--concurrency" => {
                cfg.concurrency = parse_next::<usize>(&args, i, "--concurrency")?;
                i += 2;
            }
            "--duration-sec" => {
                cfg.duration_sec = parse_next::<u64>(&args, i, "--duration-sec")?;
                i += 2;
            }
            "--warmup-sec" => {
                cfg.warmup_sec = parse_next::<u64>(&args, i, "--warmup-sec")?;
                i += 2;
            }
            "--payload-bytes" => {
                cfg.payload_bytes = parse_next::<usize>(&args, i, "--payload-bytes")?;
                i += 2;
            }
            "--write-target" => {
                let v = next_value(&args, i, "--write-target")?;
                cfg.write_target = WriteTarget::parse(v)?;
                i += 2;
            }
            "--append-weight" => {
                cfg.workload.append_weight = parse_next::<u32>(&args, i, "--append-weight")?;
                i += 2;
            }
            "--query-weight" => {
                cfg.workload.query_weight = parse_next::<u32>(&args, i, "--query-weight")?;
                i += 2;
            }
            "--meta-put-weight" => {
                cfg.workload.meta_put_weight = parse_next::<u32>(&args, i, "--meta-put-weight")?;
                i += 2;
            }
            "--meta-query-weight" => {
                cfg.workload.meta_query_weight =
                    parse_next::<u32>(&args, i, "--meta-query-weight")?;
                i += 2;
            }
            "--query-limit" => {
                cfg.workload.query_limit = parse_next::<usize>(&args, i, "--query-limit")?;
                i += 2;
            }
            "--query-strong-read" => {
                let v = next_value(&args, i, "--query-strong-read")?;
                cfg.workload.query_strong_read = parse_bool(v, "--query-strong-read")?;
                i += 2;
            }
            "--meta-query-strong-read" => {
                let v = next_value(&args, i, "--meta-query-strong-read")?;
                cfg.workload.meta_query_strong_read = parse_bool(v, "--meta-query-strong-read")?;
                i += 2;
            }
            "--meta-key-space" => {
                cfg.workload.meta_key_space = parse_next::<u64>(&args, i, "--meta-key-space")?;
                i += 2;
            }
            "--fault-kill-leader-at-sec" => {
                cfg.fault.kill_leader_at_sec =
                    Some(parse_next::<u64>(&args, i, "--fault-kill-leader-at-sec")?);
                cfg.fault.enabled = true;
                i += 2;
            }
            "--fault-wait-new-leader-timeout-sec" => {
                cfg.fault.wait_new_leader_timeout_sec =
                    parse_next::<u64>(&args, i, "--fault-wait-new-leader-timeout-sec")?;
                i += 2;
            }
            "--request-node-id" => {
                cfg.request_node_id = parse_next::<u64>(&args, i, "--request-node-id")?;
                i += 2;
            }
            "--sync-write" => {
                let v = next_value(&args, i, "--sync-write")?;
                cfg.sync_write = parse_bool(v, "--sync-write")?;
                i += 2;
            }
            "--cluster-name" => {
                cfg.cluster_name = Some(next_value(&args, i, "--cluster-name")?.to_string());
                i += 2;
            }
            "--daemon-bin" => {
                cfg.daemon_bin = Some(PathBuf::from(next_value(&args, i, "--daemon-bin")?));
                i += 2;
            }
            "--report-json" => {
                cfg.report_json = Some(PathBuf::from(next_value(&args, i, "--report-json")?));
                i += 2;
            }
            _ => {
                return Err(format!("unknown arg: {}\n\n{}", key, HELP));
            }
        }
    }

    Ok(cfg)
}

fn extract_config_path(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut path = None;
    let mut i = 0usize;
    while i < args.len() {
        let key = args[i].as_str();
        if key == "--config" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| "missing value for --config".to_string())?;
            path = Some(PathBuf::from(value));
            i += 2;
            continue;
        }
        i += 1;
    }
    Ok(path)
}

fn parse_next<T: std::str::FromStr>(args: &[String], i: usize, flag: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    let v = next_value(args, i, flag)?;
    v.parse::<T>()
        .map_err(|e| format!("invalid {} value '{}': {}", flag, v, e))
}

fn next_value<'a>(args: &'a [String], i: usize, flag: &str) -> Result<&'a str, String> {
    args.get(i + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("missing value for {}", flag))
}

fn parse_bool(v: &str, flag: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        _ => Err(format!(
            "invalid {} value '{}': expected true|false|1|0|yes|no",
            flag, v
        )),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn unique_tmp_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "buckyos_klog_bench_{}_{}_{}",
        std::process::id(),
        nanos,
        tag
    ))
}

fn pick_unused_port(used: &mut HashSet<u16>) -> Result<u16, String> {
    for _ in 0..200 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("failed to bind temporary socket: {}", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("failed to read temporary socket addr: {}", e))?
            .port();
        drop(listener);

        if used.contains(&port) {
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            used.insert(port);
            return Ok(port);
        }
    }
    Err("failed to find free unique port".to_string())
}

fn resolve_daemon_bin(cli_path: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = cli_path {
        if p.exists() {
            return Ok(p);
        }
        return Err(format!("--daemon-bin path does not exist: {}", p.display()));
    }

    let mut candidates = Vec::new();

    if let Ok(v) = std::env::var("KLOG_DAEMON_BIN") {
        let t = v.trim();
        if !t.is_empty() {
            candidates.push(PathBuf::from(t));
        }
    }

    if let Ok(v) = std::env::var("CARGO_BIN_EXE_klog_daemon") {
        let t = v.trim();
        if !t.is_empty() {
            candidates.push(PathBuf::from(t));
        }
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        candidates.push(dir.join("klog_daemon"));
        candidates.push(dir.join("klog_daemon.exe"));
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir);
        candidates.push(base.join("../../target/debug/klog_daemon"));
        candidates.push(base.join("../../target/release/klog_daemon"));
        candidates.push(base.join("../../target/debug/klog_daemon.exe"));
        candidates.push(base.join("../../target/release/klog_daemon.exe"));
    }

    for p in &candidates {
        if p.exists() {
            return Ok(p.clone());
        }
    }

    if let Some(path_like) = find_in_path("klog_daemon") {
        return Ok(path_like);
    }

    let list = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");

    Err(format!(
        "unable to find klog_daemon binary. checked=[{}]. build first with `cargo build -p klog_daemon --bin klog_daemon` or pass --daemon-bin",
        list
    ))
}

fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join(bin))
        .find(|p| p.exists())
}

fn write_config_file(
    path: &Path,
    node_id: u64,
    raft_port: u16,
    inter_node_port: u16,
    admin_port: u16,
    rpc_port: u16,
    data_dir: &Path,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    sync_write: bool,
) -> Result<(), String> {
    let join_targets_toml = join_targets
        .iter()
        .map(|x| format!("\"{}\"", x))
        .collect::<Vec<_>>()
        .join(", ");

    let content = format!(
        r#"node_id = {node_id}

[network]
listen_addr = "127.0.0.1:{raft_port}"
inter_node_listen_addr = "127.0.0.1:{inter_node_port}"
admin_listen_addr = "127.0.0.1:{admin_port}"
rpc_listen_addr = "127.0.0.1:{rpc_port}"
advertise_addr = "127.0.0.1"
advertise_port = {raft_port}
advertise_inter_port = {inter_node_port}
advertise_admin_port = {admin_port}
rpc_advertise_port = {rpc_port}

[storage]
data_dir = "{data_dir}"
state_store_sync_write = {sync_write}

[cluster]
name = "{cluster_name}"
id = "{cluster_name}"
auto_bootstrap = {auto_bootstrap}

[join]
targets = [{join_targets_toml}]
blocking = true
target_role = "voter"

[join.retry]
strategy = "fixed"
initial_interval_ms = 500
max_interval_ms = 500
multiplier = 1.0
jitter_ratio = 0.0
max_attempts = 0
request_timeout_ms = 1500
shuffle_targets_each_round = false
config_change_conflict_extra_backoff_ms = 0

[raft]
election_timeout_min_ms = 150
election_timeout_max_ms = 300
heartbeat_interval_ms = 50
install_snapshot_timeout_ms = 200
max_payload_entries = 300
replication_lag_threshold = 5000
snapshot_policy = "since_last:5000"
snapshot_max_chunk_size_bytes = 3145728
max_in_snapshot_log_to_keep = 1000
purge_batch_size = 1

[admin]
local_only = true

[rpc.append]
timeout_ms = 3000
body_limit_bytes = 1048576
concurrency = 1024

[rpc.query]
timeout_ms = 3000
body_limit_bytes = 1048576
concurrency = 256

[rpc.jsonrpc]
timeout_ms = 3000
body_limit_bytes = 1048576
concurrency = 1024
"#,
        node_id = node_id,
        raft_port = raft_port,
        inter_node_port = inter_node_port,
        admin_port = admin_port,
        rpc_port = rpc_port,
        data_dir = data_dir.display(),
        cluster_name = cluster_name,
        auto_bootstrap = auto_bootstrap,
        join_targets_toml = join_targets_toml,
        sync_write = sync_write,
    );

    fs::write(path, content)
        .map_err(|e| format!("failed to write config {}: {}", path.display(), e))
}

async fn wait_node_http_ready(
    child: &mut Child,
    node_id: u64,
    admin_port: u16,
    inter_node_port: u16,
    rpc_port: u16,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(600))
        .build()
        .map_err(|e| format!("failed to build readiness client: {}", e))?;

    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("failed to check process status: {}", e))?
        {
            return Err(format!(
                "node exited before ready: node_id={}, status={}, last_err={}",
                node_id, status, last_err
            ));
        }

        match fetch_cluster_state(&client, admin_port).await {
            Ok(_) => {
                let inter_url = format!(
                    "http://127.0.0.1:{}{}?limit=1",
                    inter_node_port,
                    KLogDataRequestType::Query.klog_path()
                );
                let rpc_url = format!(
                    "http://127.0.0.1:{}{}?limit=1",
                    rpc_port,
                    KLogDataRequestType::Query.klog_path()
                );

                match client.get(&inter_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match client.get(&rpc_url).send().await {
                            Ok(resp) if resp.status().is_success() => return Ok(()),
                            Ok(resp) => {
                                last_err = format!("rpc query not ready status={}", resp.status());
                            }
                            Err(e) => {
                                last_err = format!("rpc query request failed: {}", e);
                            }
                        }
                    }
                    Ok(resp) => {
                        last_err = format!("inter-node query not ready status={}", resp.status());
                    }
                    Err(e) => {
                        last_err = format!("inter-node query request failed: {}", e);
                    }
                }
            }
            Err(e) => {
                last_err = e;
            }
        }

        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting node ready: node_id={}, admin_port={}, last_err={}",
                node_id, admin_port, last_err
            ));
        }

        sleep(Duration::from_millis(120)).await;
    }
}

async fn fetch_cluster_state(
    client: &reqwest::Client,
    admin_port: u16,
) -> Result<KLogClusterStateResponse, String> {
    let url = format!("http://127.0.0.1:{}/klog/admin/cluster-state", admin_port);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, body));
    }
    resp.json::<KLogClusterStateResponse>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

async fn wait_cluster_voters(
    admin_ports: &[u16],
    expected_voters: &[u64],
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build cluster-state client: {}", e))?;

    let expect = expected_voters.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting voters={:?} on admin_ports={:?}",
                expected_voters, admin_ports
            ));
        }

        let mut ok = true;
        for port in admin_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    let got = state.voters.iter().copied().collect::<BTreeSet<_>>();
                    if got != expect {
                        ok = false;
                        break;
                    }
                }
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }

        if ok {
            return Ok(());
        }

        sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_consistent_leader(admin_ports: &[u16], timeout: Duration) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build leader-wait client: {}", e))?;

    let deadline = Instant::now() + timeout;
    let mut last = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting consistent leader on admin_ports={:?}, last={}",
                admin_ports, last
            ));
        }

        let mut leaders = BTreeSet::new();
        let mut all_ok = true;
        let mut obs = Vec::new();

        for port in admin_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    obs.push(format!(
                        "port={}, node_id={}, leader={:?}, voters={:?}",
                        port, state.node_id, state.current_leader, state.voters
                    ));
                    if let Some(leader) = state.current_leader {
                        leaders.insert(leader);
                    } else {
                        all_ok = false;
                    }
                }
                Err(e) => {
                    obs.push(format!("port={}, err={}", port, e));
                    all_ok = false;
                }
            }
        }

        last = obs.join(" | ");
        if all_ok && leaders.len() == 1 {
            if let Some(leader) = leaders.iter().next().copied() {
                return Ok(leader);
            }
        }

        sleep(Duration::from_millis(200)).await;
    }
}

fn admin_ports(nodes: &[ManagedNode]) -> Vec<u16> {
    nodes.iter().map(|n| n.admin_port).collect()
}

impl Drop for ManagedNode {
    fn drop(&mut self) {
        self.force_kill();
    }
}
