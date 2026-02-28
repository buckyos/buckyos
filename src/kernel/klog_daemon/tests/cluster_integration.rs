use serde::Deserialize;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::{Child, Command};
use tokio::time::{Instant, sleep};

#[derive(Debug, Clone, Deserialize)]
struct ClusterState {
    node_id: u64,
    server_state: String,
    current_leader: Option<u64>,
    voters: Vec<u64>,
}

struct TestNode {
    node_id: u64,
    port: u16,
    data_dir: PathBuf,
    config_path: PathBuf,
    child: Child,
}

impl TestNode {
    async fn stop(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

impl Drop for TestNode {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

fn unique_tmp_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "buckyos_klog_daemon_it_{}_{}_{}",
        std::process::id(),
        nanos,
        tag
    ))
}

fn choose_free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn choose_unique_ports(count: usize) -> Result<Vec<u16>, String> {
    let mut ports = Vec::with_capacity(count);
    let mut guard = HashSet::new();
    let mut attempts = 0usize;
    while ports.len() < count && attempts < 200 {
        attempts += 1;
        let p = choose_free_port().map_err(|e| format!("choose free port failed: {}", e))?;
        if guard.insert(p) {
            ports.push(p);
        }
    }
    if ports.len() != count {
        return Err(format!(
            "failed to choose {} unique ports after {} attempts; chosen={:?}",
            count, attempts, ports
        ));
    }
    Ok(ports)
}

fn can_bind_localhost() -> bool {
    std::net::TcpListener::bind("127.0.0.1:0").is_ok()
}

fn make_targets_toml(targets: &[String]) -> String {
    let quoted = targets
        .iter()
        .map(|x| format!("\"{}\"", x))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{}]", quoted)
}

fn write_config_file(
    path: &PathBuf,
    node_id: u64,
    port: u16,
    data_dir: &PathBuf,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    target_role: &str,
) -> Result<(), String> {
    let content = format!(
        r#"
node_id = {node_id}

[network]
listen_addr = "127.0.0.1:{port}"
advertise_addr = "127.0.0.1"
advertise_port = {port}

[storage]
data_dir = "{data_dir}"
state_store_sync_write = true

[cluster]
name = "{cluster_name}"
auto_bootstrap = {auto_bootstrap}

[join]
targets = {join_targets}
retry_interval_ms = 500
max_attempts = 0
blocking = true
target_role = "{target_role}"
"#,
        node_id = node_id,
        port = port,
        data_dir = data_dir.display(),
        cluster_name = cluster_name,
        auto_bootstrap = auto_bootstrap,
        join_targets = make_targets_toml(join_targets),
        target_role = target_role,
    );

    std::fs::write(path, content)
        .map_err(|e| format!("failed to write config {}: {}", path.display(), e))
}

fn daemon_bin_filename() -> &'static str {
    if cfg!(windows) {
        "klog_daemon.exe"
    } else {
        "klog_daemon"
    }
}

fn daemon_bin_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(v) = std::env::var("KLOG_DAEMON_BIN") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    if let Ok(v) = std::env::var("CARGO_BIN_EXE_klog_daemon") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(debug_dir) = current_exe.parent().and_then(|p| p.parent())
    {
        candidates.push(debug_dir.join(daemon_bin_filename()));
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir);
        candidates.push(base.join("../../target/debug").join(daemon_bin_filename()));
        candidates.push(
            base.join("../../target/release")
                .join(daemon_bin_filename()),
        );
    }

    // PATH fallback.
    candidates.push(PathBuf::from(daemon_bin_filename()));

    let mut dedup = HashSet::new();
    let mut unique = Vec::new();
    for c in candidates {
        let key = c.to_string_lossy().to_string();
        if dedup.insert(key) {
            unique.push(c);
        }
    }
    unique
}

fn resolve_daemon_bin() -> Result<(PathBuf, Vec<PathBuf>), String> {
    let candidates = daemon_bin_candidates();

    for c in &candidates {
        if c.components().count() > 1 || c.is_absolute() {
            if c.exists() {
                return Ok((c.clone(), candidates));
            }
            continue;
        }
    }

    if let Some(last) = candidates.last() {
        return Ok((last.clone(), candidates));
    }

    Err("no daemon binary candidates available".to_string())
}

async fn spawn_node(
    node_id: u64,
    port: u16,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    target_role: &str,
) -> Result<TestNode, String> {
    let data_dir = unique_tmp_path(&format!("node{}_data", node_id));
    let config_path = unique_tmp_path(&format!("node{}_config.toml", node_id));
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("failed to create data dir {}: {}", data_dir.display(), e))?;
    write_config_file(
        &config_path,
        node_id,
        port,
        &data_dir,
        cluster_name,
        auto_bootstrap,
        join_targets,
        target_role,
    )?;

    let (daemon_bin, candidates) = resolve_daemon_bin()?;
    let mut cmd = Command::new(&daemon_bin);
    cmd.env("KLOG_CONFIG_FILE", &config_path)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let mut child = cmd.spawn().map_err(|e| {
        let candidate_strings = candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        format!(
            "failed to spawn klog_daemon: bin={}, cwd={}, config={}, candidates=[{}], err={}. If running outside cargo, set KLOG_DAEMON_BIN or run `cargo build -p klog_daemon` first",
            daemon_bin.display(),
            cwd,
            config_path.display(),
            candidate_strings,
            e
        )
    })?;

    wait_node_http_ready_after_spawn(&mut child, node_id, port, Duration::from_secs(12)).await?;

    Ok(TestNode {
        node_id,
        port,
        data_dir,
        config_path,
        child,
    })
}

async fn wait_node_http_ready_after_spawn(
    child: &mut Child,
    node_id: u64,
    port: u16,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .map_err(|e| format!("failed to build readiness http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("failed to poll node process status: {}", e))?
        {
            return Err(format!(
                "node process exited before HTTP ready: node_id={}, port={}, status={}, last_err={}",
                node_id, port, status, last_err
            ));
        }

        match fetch_cluster_state(&client, port).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = e;
            }
        }

        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting node HTTP ready: node_id={}, port={}, last_err={}",
                node_id, port, last_err
            ));
        }

        sleep(Duration::from_millis(120)).await;
    }
}

async fn fetch_cluster_state(client: &reqwest::Client, port: u16) -> Result<ClusterState, String> {
    let url = format!("http://127.0.0.1:{}/klog/admin/cluster-state", port);
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
    resp.json::<ClusterState>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

async fn wait_single_node_leader(port: u16, node_id: u64, timeout: Duration) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting for single-node leader: node_id={}, port={}",
                node_id, port
            ));
        }

        if let Ok(state) = fetch_cluster_state(&client, port).await {
            let voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
            if state.current_leader == Some(node_id) && voters == BTreeSet::from([node_id]) {
                return Ok(());
            }
        }

        sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_cluster_voters(
    ports: &[u16],
    expect_voters: &[u64],
    timeout: Duration,
) -> Result<Vec<ClusterState>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let expected = expect_voters.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting cluster voters={:?} on ports={:?}",
                expect_voters, ports
            ));
        }

        let mut states = Vec::with_capacity(ports.len());
        let mut all_ok = true;
        for port in ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    let got = state.voters.iter().copied().collect::<BTreeSet<_>>();
                    if got != expected {
                        all_ok = false;
                    }
                    states.push(state);
                }
                Err(_) => {
                    all_ok = false;
                    break;
                }
            }
        }

        if all_ok && states.len() == ports.len() {
            return Ok(states);
        }

        sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_new_leader_on_ports(
    ports: &[u16],
    old_leader: u64,
    timeout: Duration,
) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_observation = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting new leader after old_leader={} on ports={:?}; last_observation={}",
                old_leader, ports, last_observation
            ));
        }

        let mut leader_by_role = None;
        let mut leaders = BTreeSet::new();
        let mut all_ok = true;
        let mut observations = Vec::new();
        for port in ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    observations.push(format!(
                        "port={}, node_id={}, state={}, current_leader={:?}, voters={:?}",
                        port, state.node_id, state.server_state, state.current_leader, state.voters
                    ));

                    if state.server_state == "Leader" && state.node_id != old_leader {
                        leader_by_role = Some(state.node_id);
                    }

                    if let Some(leader) = state.current_leader {
                        if leader != old_leader {
                            leaders.insert(leader);
                        } else {
                            all_ok = false;
                        }
                    } else {
                        all_ok = false;
                    }
                }
                Err(_) => {
                    observations.push(format!("port={}, state_fetch=err", port));
                    all_ok = false;
                }
            }
        }
        last_observation = observations.join(" | ");

        if let Some(leader) = leader_by_role {
            return Ok(leader);
        }

        if all_ok && leaders.len() == 1 {
            return leaders
                .iter()
                .next()
                .copied()
                .ok_or_else(|| "leader set unexpectedly empty".to_string());
        }

        sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_consistent_leader_on_ports(ports: &[u16], timeout: Duration) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_observation = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting consistent leader on ports={:?}; last_observation={}",
                ports, last_observation
            ));
        }

        let mut leaders = BTreeSet::new();
        let mut all_ok = true;
        let mut observations = Vec::new();
        for port in ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    observations.push(format!(
                        "port={}, node_id={}, state={}, current_leader={:?}, voters={:?}",
                        port, state.node_id, state.server_state, state.current_leader, state.voters
                    ));
                    if let Some(leader) = state.current_leader {
                        leaders.insert(leader);
                    } else {
                        all_ok = false;
                    }
                }
                Err(e) => {
                    observations.push(format!("port={}, state_fetch_err={}", port, e));
                    all_ok = false;
                }
            }
        }
        last_observation = observations.join(" | ");

        if all_ok && leaders.len() == 1 {
            return leaders
                .iter()
                .next()
                .copied()
                .ok_or_else(|| "leader set unexpectedly empty".to_string());
        }

        sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
async fn test_single_node_smoke() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip single-node smoke: localhost bind is not available");
        return Ok(());
    }

    let port = choose_free_port().map_err(|e| format!("choose free port failed: {}", e))?;
    let cluster_name = format!("klog_smoke_{}", port);
    let mut node = spawn_node(1, port, &cluster_name, true, &[], "voter").await?;

    let wait_result = wait_single_node_leader(port, 1, Duration::from_secs(20)).await;
    node.stop().await;
    wait_result
}

#[tokio::test]
async fn test_three_node_cluster_and_failover() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip three-node cluster test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_cluster_{}_{}_{}", port1, port2, port3);
    let join_seed = vec![format!("127.0.0.1:{}", port1)];

    let mut nodes = Vec::new();
    nodes.push(spawn_node(1, port1, &cluster_name, true, &[], "voter").await?);
    wait_single_node_leader(port1, 1, Duration::from_secs(20)).await?;

    // Join one by one to avoid concurrent membership-change races that may leave
    // the cluster in a hard-to-elect transitional state.
    nodes.push(spawn_node(2, port2, &cluster_name, false, &join_seed, "voter").await?);
    let _ = wait_cluster_voters(&[port1, port2], &[1, 2], Duration::from_secs(40)).await?;

    nodes.push(spawn_node(3, port3, &cluster_name, false, &join_seed, "voter").await?);

    let states =
        wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(40)).await?;
    let leader = states
        .iter()
        .find_map(|s| s.current_leader)
        .ok_or_else(|| "cluster has no leader after convergence".to_string())?;
    if ![1_u64, 2_u64, 3_u64].contains(&leader) {
        for n in &mut nodes {
            n.stop().await;
        }
        return Err(format!("unexpected leader id: {}", leader));
    }

    let leader_index = nodes
        .iter()
        .position(|n| n.node_id == leader)
        .ok_or_else(|| format!("cannot find leader node process for id={}", leader))?;
    nodes[leader_index].stop().await;

    let remaining_ports = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            if i == leader_index {
                None
            } else {
                Some(n.port)
            }
        })
        .collect::<Vec<_>>();
    let new_leader =
        wait_new_leader_on_ports(&remaining_ports, leader, Duration::from_secs(40)).await?;

    for n in &mut nodes {
        n.stop().await;
    }

    if new_leader == leader {
        return Err(format!(
            "new leader did not change: old={}, new={}",
            leader, new_leader
        ));
    }

    Ok(())
}

#[tokio::test]
async fn test_three_node_concurrent_startup_converges() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip concurrent-startup cluster test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_cluster_concurrent_{}_{}_{}", port1, port2, port3);
    let join_seed = vec![format!("127.0.0.1:{}", port1)];

    let (node1, node2, node3) = tokio::try_join!(
        spawn_node(1, port1, &cluster_name, true, &[], "voter"),
        spawn_node(2, port2, &cluster_name, false, &join_seed, "voter"),
        spawn_node(3, port3, &cluster_name, false, &join_seed, "voter"),
    )?;

    let mut nodes = vec![node1, node2, node3];
    let result = async {
        let _ = wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(60))
            .await?;
        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!("unexpected leader id: {}", leader));
        }
        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}

#[tokio::test]
async fn test_bootstrap_late_start_converges() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip bootstrap-late-start cluster test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_cluster_bootstrap_late_{}_{}_{}", port1, port2, port3);
    let join_seed = vec![format!("127.0.0.1:{}", port1)];

    let (node2, node3) = tokio::try_join!(
        spawn_node(2, port2, &cluster_name, false, &join_seed, "voter"),
        spawn_node(3, port3, &cluster_name, false, &join_seed, "voter"),
    )?;

    let mut nodes = vec![node2, node3];
    // Simulate unpredictable startup order: non-bootstrap nodes start first.
    sleep(Duration::from_millis(1500)).await;
    nodes.push(spawn_node(1, port1, &cluster_name, true, &[], "voter").await?);

    let result = async {
        let _ = wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(70))
            .await?;
        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!("unexpected leader id: {}", leader));
        }
        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}
