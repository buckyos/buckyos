# 05. 交付与升级（pkg-system / repo-server / pkg-env / node-daemon）

BuckyOS 把“交付与升级”做成系统主链路的一部分，而不是外部运维行为。
它强调：把失败面拆成“分发面 ready / 控制面意图 / 调度写回 / 节点收敛”，从而让升级更像“系统自愈”。

## 组件分工（概念边界）
- repo-server：Zone 内的包索引与内容缓存中心，持有 pkg-index-db（实现中多称 meta-index-db / `meta_index.db`）与 chunk 数据，负责把目标版本准备到 ready。
- pkg-env：本机包运行环境（strict/dev 模式、index-db 继承等），提供 load/install/gc 等基础能力。
- node-daemon：节点收敛器，读取 node_config，将本机拉到目标状态（安装/部署/启动/停止/升级触发）。

设计细节（强参考）：`new_doc/ref/notepads/app-pkg-system.md`

## “ready” 的语义（BuckyOS 很关键的差异点）
在 BuckyOS 的升级链路里，repo-server 会把“目标版本可用性”前置：
- repo-server 同步 pkg-index-db。
- repo-server 根据已安装 app/service 列表，下载 sub_pkg 与依赖 chunk。
- 当一个 app/service 的依赖都准备好，称该 app/service 在本 Zone “ready”。

这意味着：
- 只要 Zone 内 repo-server ready 完成，后续节点部署尽量做到 zero-depend（即便断网、新加节点也能从 Zone 内完成安装）。

进一步的工程含义：
- ready 不是“某节点拉到镜像就算”，而是“Zone 内分发面准备完成”的语义。
- ready 成功后，node-daemon 的重试收敛应尽量不再依赖公网（只依赖 Zone 内 repo-server）。

## pkg-env 与 pkg-index-db（把两个概念连起来）
理解 BuckyOS 的交付链路，关键是把 pkg-env 看成“本机的包视图”，把 pkg-index-db 看成“可验证的版本索引整体”。

- pkg-env 负责“本地到底有哪些版本可 load / install”。
- pkg-index-db 负责“给定一个 pkg_id（可能是非精确版本），应该解析到哪个版本/objid”。

notepads 里明确指出一个关键点：`env.try_load` 会更偏向“只根据 pkg-index-db 决定目标版本”，而不是“只看本地已有目录”（`new_doc/ref/notepads/app-pkg-system.md`）。
这也是 BuckyOS 能支持“非精确版本 + 自动追随 latest”的基础。

## 关键数据结构（从代码抽取的最小视图）

### 1) ServicePkg：node-daemon 对“一个可执行 pkg”的最小封装
对应代码：`src/kernel/node_daemon/src/service_pkg.rs`

它把“pkg_id + pkg-env 路径 + 脚本执行上下文”封装成一个可被收敛循环调用的对象：

```rust
pub struct ServicePkg {
    pub pkg_id: String,
    pub pkg_env_path: PathBuf,
    pub current_dir: Option<PathBuf>,
    pub env_vars: HashMap<String, String>,
    pub media_info: Mutex<Option<MediaInfo>>,
}

impl ServicePkg {
    pub async fn try_load(&self) -> bool {
        let mut media_info = self.media_info.lock().await;
        if media_info.is_none() {
            let pkg_env = PackageEnv::new(self.pkg_env_path.clone());
            let new_media_info = pkg_env.load(&self.pkg_id).await;
            if new_media_info.is_ok() {
                *media_info = Some(new_media_info.unwrap());
                return true;
            }
        }
        false
    }
}
```

几个读代码时很容易忽略、但对系统语义很重要的点：
- `pkg_env_path` 明确了“这个 pkg 是从哪个 pkg-env 被 load 出来的”；同一个 node 上通常存在多个 env（例如 root env / bin env / app env），混用会直接导致“版本/索引不一致”的问题。
- `env_vars` 是脚本执行的上下文注入点：node-daemon 会通过环境变量/上下文把 session token、实例配置等传入 pkg 的 `start/stop/status/deploy` 脚本（见 `src/kernel/node_daemon/src/kernel_mgr.rs`）。

### 2) pkg-index-db（meta-index-db）：升级稳定性的“版本真相源”
相关代码：
- repo-server 的 meta-index-db 分层与发布流程注释：`src/frame/repo_service/src/repo_server.rs`
- node-daemon 同步 root env 的 `meta_index.db`：`src/kernel/node_daemon/src/node_daemon.rs`
- root env 目录：`src/kernel/buckyos-api/src/runtime.rs`

node-daemon 对 index-db 的一个关键动作是：先把“将要切换到的新 index-db”下载好，并确保关键系统包对应的 chunk 已 ready，再原子更新本机 env 的 index-db：
- `check_and_update_root_pkg_index_db()`：下载 `/ndn/repo/meta_index.db` 到 `meta_index.downloading`，并 `try_update_index_db`（`src/kernel/node_daemon/src/node_daemon.rs`）。
- `make_sure_system_pkgs_ready()`：在更新 index-db 之前先 `check_pkg_ready` 并 pull 缺失 chunk（`src/kernel/node_daemon/src/node_daemon.rs`）。

这体现了一个很核心的策略：
- “先 ready（分发面）再切换索引（控制面视图）”。

## 安装/升级主链路（从 ready 到收敛）
典型链路（更接近系统真实控制流，而不是 UI 视角）：
1) repo-server 同步 pkg-index-db，并把目标 app/service 准备成 ready。
2) 控制面/工具写入 system-config（安装/升级意图、服务 spec、用户 app config 等）。
3) scheduler 基于 system-config 推导 instances，并写回 `nodes/<node>/config`。
4) node-daemon 周期性读取 node_config，驱动本机安装/部署/启动，直到收敛。

## 关键流程伪代码（repo-server ready → system-config 意图 → 调度 → 节点收敛）

伪代码强调数据依赖与时序边界：ready 在前、意图在中、调度写回、节点收敛在后。

```text
repo_server_loop():
  sync_pkg_index_db()                      // get new meta-index-db
  for each installed_or_pinned_app:
    prepare_ready(app)                     // download sub_pkgs + chunks into Zone cache
  mark_ready(app)

apply_intent():
  // control plane / UI / tool
  system_config.exec_tx({
    "users/<u>/apps/<app>/config": new_spec,
    "services/<svc>/spec": updated_spec,
  })

scheduler_loop():
  input = system_config.dump()
  actions = schedule(input)
  system_config.exec_tx(actions_to_kv(actions))  // writes nodes/<node>/config

node_daemon_loop():
  // keep local env view consistent with repo-server's index-db
  sync_pkg_index_db_from_repo_server()

  node_config = system_config.get("nodes/<node>/config")
  for inst in node_config.instances:
    state = inst.target_state

    // when index-db updated, env.try_load(pkg_id) may point to a new version
    if pkg.status() == NotExist:
      deploy(pkg)                           // install_pkg + pkg-type deploy script

    ensure_run_item_state(inst, state)      // NotExist => deploy => start
```

其中 node-daemon 的“NotExist → deploy → start”收敛语义在当前实现里是显式状态机：
- `ensure_run_item_state()`（`src/kernel/node_daemon/src/run_item.rs`）

## 升级触发模型（两类触发，本质不同）
BuckyOS 支持两种本质上不同的升级触发：
1) node_config 指定精确版本：升级/降级都视为“目标状态变更”。
2) node_config 指定非精确版本：由 pkg-index-db 最新版本变化触发（典型：`#^1.3`、latest 语义等）。

notepads 中强调：node-daemon 应通过正确管理 env 与 pkg-index-db 同步，来同时支撑这两种模式（`new_doc/ref/notepads/app-pkg-system.md`）。

## 常见风险与定位（交付链路特有）

### 1) NotExist → deploy 失败 → 循环（通常不是 deploy 脚本本身的问题）
典型根因是“repo-server 未 ready / 索引不一致”，导致 node-daemon 进入反复尝试：
- 索引告诉你“应该是新版本”，但本地/Zone cache 没准备好，`status()` 看到 `NotExist`，于是持续触发 deploy。

定位建议：
- 先确认 repo-server ready（分发面是否已把依赖 chunk 准备好）。
- 再确认 node-daemon 是否已同步到同一份 `meta_index.db`（避免节点看到不同的 latest）。

参考：
- `new_doc/ref/notepads/app-pkg-system.md`
- `new_doc/ref/notepads/repo-server重构.md`

### 2) pkg-index-db 不一致（repo-server / root env / bin env 之间视图分裂）
常见场景：
- repo-server 已切换到新 meta-index-db，但 node 仍在旧 index-db 上解析 pkg_id。
- node 已切换 index-db，但 repo-server 的 chunk 尚未 ready，导致“索引领先于内容”。

当前实现中，node-daemon 在切换 index-db 前会先检查系统关键 pkgs 的 ready，并 pull 缺失 chunk（`src/kernel/node_daemon/src/node_daemon.rs`），这正是为了解决“索引领先于内容”的崩溃面。

repo-server 侧也会对 pkg_list 与 chunk_id 做一致性检查，避免“索引记录与实际 chunk 不匹配”进入可发布状态（`src/frame/repo_service/src/repo_server.rs`）。

### 3) GC 与磁盘长期膨胀（ready 越可靠，GC 越关键）
ready 把分发面准备前置了，但副作用是：
- repo-server 的 chunk/cache 会持续增长。
- node 本地 env 里多个版本也会累积（尤其是自动追随 latest 的场景）。

notepads 给出一个明确方向：env 的 GC 很可能需要以“已安装 pkg 列表”为起点，对依赖图做染色，未被染色的版本才可回收（`new_doc/ref/notepads/app-pkg-system.md`）。
repo-server 侧也同样需要“pin/已安装 app 列表”等有效性标记来决定哪些 chunk 不能删（`new_doc/ref/notepads/repo-server重构.md`）。

## 与类似系统的差异点
- 相比“镜像仓库 + 拉取镜像 + 编排启动”的常见模式，BuckyOS 把“索引可验证整体（pkg-index-db）+ ready 语义 + Zone 内缓存”作为升级可靠性的基础。
- 升级链路更像：先确保分发面准备好（repo-server ready），再写入系统意图（system-config），由调度与节点收敛完成落地（scheduler + node-daemon）。

这套分层让失败更容易被定位：准备失败/调度失败/节点收敛失败分别对应不同组件。
