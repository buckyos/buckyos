use serde::{Deserialize, Serialize};
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
    learners: Vec<u64>,
}

#[derive(Debug, Serialize)]
struct AppendLogBody {
    message: String,
    timestamp: Option<u64>,
    node_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AppendLogResponse {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct QueryLogEntry {
    id: u64,
    timestamp: u64,
    node_id: u64,
    message: String,
}

#[derive(Debug, Deserialize)]
struct QueryLogResponse {
    items: Vec<QueryLogEntry>,
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
id = "{cluster_name}"
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

async fn append_log(
    client: &reqwest::Client,
    port: u16,
    message: &str,
    timestamp: Option<u64>,
    node_id: Option<u64>,
) -> Result<AppendLogResponse, String> {
    let url = format!("http://127.0.0.1:{}/klog/data/append", port);
    let body = AppendLogBody {
        message: message.to_string(),
        timestamp,
        node_id,
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, text));
    }

    resp.json::<AppendLogResponse>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

async fn query_logs(
    client: &reqwest::Client,
    port: u16,
    start_id: Option<u64>,
    end_id: Option<u64>,
    limit: Option<usize>,
    desc: Option<bool>,
) -> Result<QueryLogResponse, String> {
    let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:{}/klog/data/query", port))
        .map_err(|e| format!("invalid query url: {}", e))?;
    {
        let mut q = url.query_pairs_mut();
        if let Some(v) = start_id {
            q.append_pair("start_id", &v.to_string());
        }
        if let Some(v) = end_id {
            q.append_pair("end_id", &v.to_string());
        }
        if let Some(v) = limit {
            q.append_pair("limit", &v.to_string());
        }
        if let Some(v) = desc {
            q.append_pair("desc", if v { "true" } else { "false" });
        }
    }

    let resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, text));
    }

    resp.json::<QueryLogResponse>()
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
    wait_cluster_membership(ports, expect_voters, &[], timeout).await
}

async fn wait_cluster_membership(
    ports: &[u16],
    expect_voters: &[u64],
    expect_learners: &[u64],
    timeout: Duration,
) -> Result<Vec<ClusterState>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let expected_voters = expect_voters.iter().copied().collect::<BTreeSet<_>>();
    let expected_learners = expect_learners.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting cluster membership voters={:?}, learners={:?} on ports={:?}",
                expect_voters, expect_learners, ports
            ));
        }

        let mut states = Vec::with_capacity(ports.len());
        let mut all_ok = true;
        for port in ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    let got_voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
                    let got_learners = state.learners.iter().copied().collect::<BTreeSet<_>>();
                    if got_voters != expected_voters || got_learners != expected_learners {
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

async fn send_remove_learner(port: u16, node_id: u64) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let url = format!(
        "http://127.0.0.1:{}/klog/admin/remove-learner?node_id={}",
        port, node_id
    );
    let resp = client
        .post(&url)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|_| String::new());
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("request {} returned {}: {}", url, status, body))
    }
}

async fn send_add_learner(
    port: u16,
    node_id: u64,
    addr: &str,
    node_port: u16,
    blocking: bool,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let url = format!(
        "http://127.0.0.1:{}/klog/admin/add-learner?node_id={}&addr={}&port={}&blocking={}",
        port,
        node_id,
        addr,
        node_port,
        if blocking { "true" } else { "false" }
    );
    let resp = client
        .post(&url)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|_| String::new());
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("request {} returned {}: {}", url, status, body))
    }
}

async fn add_learner_with_retry(
    voter_ports: &[u16],
    node_id: u64,
    node_port: u16,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout adding learner node_id={}, node_port={}, voter_ports={:?}, last_err={}",
                node_id, node_port, voter_ports, last_err
            ));
        }

        let mut states = Vec::with_capacity(voter_ports.len());
        let mut all_ok = true;
        for port in voter_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => states.push(state),
                Err(e) => {
                    all_ok = false;
                    last_err = format!("fetch cluster state failed on port {}: {}", port, e);
                    break;
                }
            }
        }

        if all_ok && states.len() == voter_ports.len() {
            let all_have_learner = states
                .iter()
                .all(|s| s.learners.iter().any(|id| *id == node_id));
            if all_have_learner {
                return Ok(());
            }

            let mut leader_port = None;
            if let Some(leader_id) = states.iter().find_map(|s| s.current_leader)
                && let Some(idx) = states.iter().position(|s| s.node_id == leader_id)
            {
                leader_port = voter_ports.get(idx).copied();
            }

            let mut candidate_ports = Vec::new();
            if let Some(p) = leader_port {
                candidate_ports.push(p);
            }
            for p in voter_ports {
                if !candidate_ports.contains(p) {
                    candidate_ports.push(*p);
                }
            }

            let mut errs = Vec::new();
            for p in candidate_ports {
                match send_add_learner(p, node_id, "127.0.0.1", node_port, true).await {
                    Ok(_) => {
                        errs.clear();
                        break;
                    }
                    Err(e) => errs.push(format!("port={}, err={}", p, e)),
                }
            }
            if errs.is_empty() {
                last_err = format!(
                    "add-learner accepted for node_id={}, waiting membership propagation",
                    node_id
                );
            } else {
                last_err = format!(
                    "add-learner failed for node_id={}: {}",
                    node_id,
                    errs.join(" | ")
                );
            }
        }

        sleep(Duration::from_millis(250)).await;
    }
}

async fn remove_learners_with_retry(
    voter_ports: &[u16],
    final_voters: &[u64],
    learner_ids: &[u64],
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let final_voters_set = final_voters.iter().copied().collect::<BTreeSet<_>>();
    let expected_learners_set = learner_ids.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout removing learners with voters={:?}, ports={:?}, last_err={}",
                final_voters, voter_ports, last_err
            ));
        }

        let mut states = Vec::with_capacity(voter_ports.len());
        let mut all_state_ok = true;
        for port in voter_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => states.push(state),
                Err(e) => {
                    all_state_ok = false;
                    last_err = format!("fetch cluster state failed on port {}: {}", port, e);
                    break;
                }
            }
        }

        if all_state_ok && states.len() == voter_ports.len() {
            let mut all_voters_match = true;
            let mut all_learners_empty = true;
            for state in &states {
                let voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
                if voters != final_voters_set {
                    all_voters_match = false;
                }
                if !state.learners.is_empty() {
                    all_learners_empty = false;
                }
            }
            if all_voters_match && all_learners_empty {
                return Ok(());
            }

            let mut pending_learners = BTreeSet::new();
            for state in &states {
                for learner in &state.learners {
                    pending_learners.insert(*learner);
                }
            }
            if !expected_learners_set.is_empty() {
                pending_learners = pending_learners
                    .intersection(&expected_learners_set)
                    .copied()
                    .collect();
            }

            let mut leader_port = None;
            for state in &states {
                if state.current_leader == Some(state.node_id)
                    && let Some(idx) = states.iter().position(|s| s.node_id == state.node_id)
                {
                    leader_port = voter_ports.get(idx).copied();
                    break;
                }
            }
            if leader_port.is_none()
                && let Some(leader_id) = states.iter().find_map(|s| s.current_leader)
            {
                if let Some(idx) = states.iter().position(|s| s.node_id == leader_id) {
                    leader_port = voter_ports.get(idx).copied();
                }
            }

            if pending_learners.is_empty() {
                last_err = format!(
                    "learners are not empty yet but no pending learner id extracted from voter states; states={:?}",
                    states
                        .iter()
                        .map(|s| format!("node_id={}, learners={:?}", s.node_id, s.learners))
                        .collect::<Vec<_>>()
                );
            } else if let Some(port) = leader_port {
                let mut remove_errors = Vec::new();
                for learner_id in pending_learners {
                    if let Err(e) = send_remove_learner(port, learner_id).await {
                        remove_errors.push(format!("node_id={}, err={}", learner_id, e));
                    }
                }
                if remove_errors.is_empty() {
                    last_err = format!(
                        "remove-learner accepted on leader port {}; waiting learners to drain",
                        port
                    );
                } else {
                    last_err = format!(
                        "remove-learner failed on leader port {}: {}",
                        port,
                        remove_errors.join(" | ")
                    );
                }
            } else {
                last_err = format!(
                    "leader not discovered from states: {:?}",
                    states
                        .iter()
                        .map(|s| format!("node_id={}, leader={:?}", s.node_id, s.current_leader))
                        .collect::<Vec<_>>()
                );
            }
        }

        sleep(Duration::from_millis(250)).await;
    }
}

async fn ensure_learners_absent_for_duration(
    voter_ports: &[u16],
    absent_learner_ids: &[u64],
    duration: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let absent = absent_learner_ids.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        for port in voter_ports {
            let state = fetch_cluster_state(&client, *port).await?;
            let got = state.learners.iter().copied().collect::<BTreeSet<_>>();
            let overlap = got.intersection(&absent).copied().collect::<Vec<_>>();
            if !overlap.is_empty() {
                return Err(format!(
                    "unexpected learner rejoin observed: port={}, overlap={:?}, learners={:?}",
                    port, overlap, state.learners
                ));
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

async fn spawn_three_voter_cluster(
    cluster_name: &str,
    port1: u16,
    port2: u16,
    port3: u16,
) -> Result<Vec<TestNode>, String> {
    let join_seed = vec![format!("127.0.0.1:{}", port1)];
    let mut nodes = Vec::new();
    nodes.push(spawn_node(1, port1, cluster_name, true, &[], "voter").await?);
    wait_single_node_leader(port1, 1, Duration::from_secs(20)).await?;

    nodes.push(spawn_node(2, port2, cluster_name, false, &join_seed, "voter").await?);
    let _ = wait_cluster_voters(&[port1, port2], &[1, 2], Duration::from_secs(40)).await?;

    nodes.push(spawn_node(3, port3, cluster_name, false, &join_seed, "voter").await?);
    let _ =
        wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(50)).await?;
    let leader =
        wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40)).await?;
    if ![1_u64, 2_u64, 3_u64].contains(&leader) {
        return Err(format!(
            "unexpected leader id after voter cluster bootstrap: {}",
            leader
        ));
    }

    Ok(nodes)
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
async fn test_single_node_business_log_append_and_query() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip single-node business-log test: localhost bind is not available");
        return Ok(());
    }

    let port = choose_free_port().map_err(|e| format!("choose free port failed: {}", e))?;
    let cluster_name = format!("klog_business_log_{}", port);
    let mut node = spawn_node(1, port, &cluster_name, true, &[], "voter").await?;

    let result = async {
        wait_single_node_leader(port, 1, Duration::from_secs(20)).await?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let append1 = append_log(&client, port, "kernel-init", Some(100), Some(1)).await?;
        let append2 = append_log(&client, port, "driver-up", Some(101), Some(1)).await?;
        let append3 = append_log(&client, port, "service-ready", Some(102), Some(1)).await?;

        if !(append1.id < append2.id && append2.id < append3.id) {
            return Err(format!(
                "append ids are not strictly increasing: [{}, {}, {}]",
                append1.id, append2.id, append3.id
            ));
        }

        let asc = query_logs(
            &client,
            port,
            Some(append1.id),
            Some(append3.id),
            Some(10),
            Some(false),
        )
        .await?;
        let asc_ids = asc.items.iter().map(|e| e.id).collect::<Vec<_>>();
        if asc_ids != vec![append1.id, append2.id, append3.id] {
            return Err(format!("unexpected asc ids: {:?}", asc_ids));
        }

        let desc = query_logs(
            &client,
            port,
            Some(append1.id),
            Some(append3.id),
            Some(2),
            Some(true),
        )
        .await?;
        let desc_ids = desc.items.iter().map(|e| e.id).collect::<Vec<_>>();
        if desc_ids != vec![append3.id, append2.id] {
            return Err(format!("unexpected desc ids: {:?}", desc_ids));
        }

        if asc.items[0].message != "kernel-init"
            || asc.items[1].message != "driver-up"
            || asc.items[2].message != "service-ready"
        {
            return Err(format!(
                "unexpected query messages: [{}, {}, {}]",
                asc.items[0].message, asc.items[1].message, asc.items[2].message
            ));
        }

        if asc.items[0].timestamp != 100
            || asc.items[1].timestamp != 101
            || asc.items[2].timestamp != 102
            || asc.items.iter().any(|e| e.node_id != 1)
        {
            return Err("unexpected query timestamps or node_id".to_string());
        }

        Ok(())
    }
    .await;

    node.stop().await;
    result
}

#[tokio::test]
async fn test_three_node_append_via_follower_auto_forwards_to_leader() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip follower-forward append test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(3)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let cluster_name = format!("klog_append_forward_{}_{}_{}", port1, port2, port3);
    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;

    let result = async {
        let leader_id =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        let leader_port = match leader_id {
            1 => port1,
            2 => port2,
            3 => port3,
            _ => return Err(format!("unexpected leader id: {}", leader_id)),
        };
        let follower_port = [port1, port2, port3]
            .into_iter()
            .find(|p| *p != leader_port)
            .ok_or_else(|| "failed to choose follower port".to_string())?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build http client: {}", e))?;

        let appended = append_log(
            &client,
            follower_port,
            "forwarded-from-follower",
            Some(300),
            Some(2),
        )
        .await?;

        let queried = query_logs(
            &client,
            leader_port,
            Some(appended.id),
            Some(appended.id),
            Some(1),
            Some(false),
        )
        .await?;
        if queried.items.len() != 1 {
            return Err(format!(
                "unexpected query result len: expected=1, got={}",
                queried.items.len()
            ));
        }

        let item = &queried.items[0];
        if item.id != appended.id || item.message != "forwarded-from-follower" {
            return Err(format!(
                "unexpected queried item: id={}, message={}",
                item.id, item.message
            ));
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

    let ports = choose_unique_ports(5)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let port5 = ports[4];
    let cluster_name = format!(
        "klog_cluster_concurrent_{}_{}_{}_{}_{}",
        port1, port2, port3, port4, port5
    );
    let join_seed = vec![format!("127.0.0.1:{}", port1)];

    let (node1, node2, node3, node4, node5) = tokio::try_join!(
        spawn_node(1, port1, &cluster_name, true, &[], "voter"),
        spawn_node(2, port2, &cluster_name, false, &join_seed, "voter"),
        spawn_node(3, port3, &cluster_name, false, &join_seed, "voter"),
        spawn_node(4, port4, &cluster_name, false, &join_seed, "learner"),
        spawn_node(5, port5, &cluster_name, false, &join_seed, "learner"),
    )?;

    let mut nodes = vec![node1, node2, node3, node4, node5];
    let result = async {
        let states = wait_cluster_membership(
            &[port1, port2, port3, port4, port5],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(80),
        )
        .await?;
        for state in states
            .iter()
            .filter(|s| s.node_id == 4 || s.node_id == 5)
        {
            if state.voters.contains(&state.node_id) || !state.learners.contains(&state.node_id) {
                return Err(format!(
                    "learner node state mismatch: node_id={}, voters={:?}, learners={:?}, server_state={}",
                    state.node_id, state.voters, state.learners, state.server_state
                ));
            }
        }

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!("unexpected leader id: {}", leader));
        }

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(45),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }
    .await;

    for n in &mut nodes {
        n.stop().await;
    }
    result
}

#[tokio::test]
async fn test_remove_offline_learner_succeeds() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip remove-offline-learner test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(4)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let cluster_name = format!(
        "klog_remove_offline_learner_{}_{}_{}_{}",
        port1, port2, port3, port4
    );

    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;

        let learner_idx = nodes
            .iter()
            .position(|n| n.node_id == 4)
            .ok_or_else(|| "learner node(4) process not found".to_string())?;
        nodes[learner_idx].stop().await;

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!(
                "unexpected leader after offline learner removal: {}",
                leader
            ));
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
async fn test_remove_both_learners_when_one_offline() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip remove-two-learners test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(5)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let port5 = ports[4];
    let cluster_name = format!(
        "klog_remove_two_learners_{}_{}_{}_{}_{}",
        port1, port2, port3, port4, port5
    );

    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);
    nodes.push(spawn_node(5, port5, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        add_learner_with_retry(&[port1, port2, port3], 5, port5, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(60),
        )
        .await?;

        let learner4_idx = nodes
            .iter()
            .position(|n| n.node_id == 4)
            .ok_or_else(|| "learner node(4) process not found".to_string())?;
        nodes[learner4_idx].stop().await;

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4, 5],
            Duration::from_secs(55),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;

        let leader =
            wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40))
                .await?;
        if ![1_u64, 2_u64, 3_u64].contains(&leader) {
            return Err(format!(
                "unexpected leader after removing both learners: {}",
                leader
            ));
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
async fn test_offline_learner_rejoin_requires_add_learner_again() -> Result<(), String> {
    if !can_bind_localhost() {
        eprintln!("skip learner-rejoin test: localhost bind is not available");
        return Ok(());
    }

    let ports = choose_unique_ports(4)?;
    let port1 = ports[0];
    let port2 = ports[1];
    let port3 = ports[2];
    let port4 = ports[3];
    let cluster_name = format!(
        "klog_learner_rejoin_requires_add_{}_{}_{}_{}",
        port1, port2, port3, port4
    );

    let mut nodes = spawn_three_voter_cluster(&cluster_name, port1, port2, port3).await?;
    nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);

    let result = async {
        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(50),
        )
        .await?;

        let learner_idx = nodes
            .iter()
            .position(|n| n.node_id == 4)
            .ok_or_else(|| "learner node(4) process not found".to_string())?;
        nodes[learner_idx].stop().await;

        remove_learners_with_retry(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(55),
        )
        .await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[],
            Duration::from_secs(60),
        )
        .await?;

        // Re-start the removed learner node without auto-join target.
        nodes.push(spawn_node(4, port4, &cluster_name, false, &[], "learner").await?);
        ensure_learners_absent_for_duration(&[port1, port2, port3], &[4], Duration::from_secs(5))
            .await?;

        add_learner_with_retry(&[port1, port2, port3], 4, port4, Duration::from_secs(45)).await?;
        let _ = wait_cluster_membership(
            &[port1, port2, port3],
            &[1, 2, 3],
            &[4],
            Duration::from_secs(60),
        )
        .await?;
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
