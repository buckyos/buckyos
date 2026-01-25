# 09. 常见踩坑与工程建议（基于当前实现）

本章聚焦“容易踩坑的真实问题”，并给出定位方向。

## 1) boot/config 与 owner 公钥强相关
- ZoneBootConfig 校验强依赖 owner 公钥；`boot/config` 里也隐含了后续 trust_keys / session_token 验证所需的 key 素材。
- owner 变更/密钥丢失会导致旧设备启动失败，需要重新激活或通过更权威的 owner key 获取渠道恢复。
- Symptom：
  - node-daemon 启动阶段反复失败/退出；常见日志包括：
    - `resolve zone did failed! ...` / `parse zone config failed! ...`
    - `zone boot config's owner is not match node_identity's owner_did!`
  - system-config 启动后无法正确刷新 trust_keys；常见日志包括：
    - `Failed to parse zone config from boot/config: ...`
    - `Missing verify_hub_info from zone_config`
- Likely cause：
  - `node_identity.json` 里的 `owner_public_key` / `owner_did` 已经与当前 zone 的 boot 文档不一致（例如 owner 换人 / key rotation）。
  - 设备上残留了用于“离线调试”的本地 zone boot 文件，覆盖了线上查询（见下面代码锚点）。
  - KV 里的 `boot/config` 已经被写入/恢复为旧版本，且与当前 owner key 不匹配。
- Where to look in code/KV：
  - code: `src/kernel/node_daemon/src/node_daemon.rs`（`looking_zone_boot_config()`；本地 `*.zone.json` 覆盖逻辑 + owner 校验）
  - code: `src/kernel/scheduler/src/main.rs`（boot 期写入 `boot/config`；以及 boot 后调用 `system_config_client.refresh_trust_keys()`）
  - code: `src/kernel/sys_config_service/src/main.rs`（`handle_refresh_trust_keys()`；日志：`TRUST_KEYS cleared,refresh_trust_keys`、`update owner_public_key ... to trust keys`）
  - KV: `boot/config`
  - (辅助) did-doc 本地缓存线索：`src/kernel/scheduler/src/system_config_builder.rs`（`local/did_docs/*.doc.json` 缓存命中/缺失提示）
- Quick confirm steps：
  - 读 KV：确认 `boot/config` 里的 `owner` / `verify_hub_info` / 默认 key 与当前设备期望一致（KV key：`boot/config`）。
  - 对比 node 身份：检查 `buckyos/etc/node_identity.json`（owner_did、owner_public_key）是否与 `boot/config` 对得上。
  - 排查本地覆盖：检查 `buckyos/etc/<zone_host>.zone.json` 是否存在（`looking_zone_boot_config()` 会优先加载它）。
  - grep 日志：在 `system-config` / `node-daemon` 日志里搜索上述关键字（尤其是 `owner is not match`、`Failed to parse zone config from boot/config`）。

参考：
- `new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md`

## 2) 启动顺序导致循环失败
典型危险组合：
- node-daemon 依赖 system-config 与 gateway_config。
- cyfs-gateway / system-config 未就绪时，node-daemon 可能进入反复尝试。
- Symptom：
  - node-daemon 每 5s 打一次循环日志但“系统永远起不来”；常见日志锚点：
    - `node daemon main loop step:`（node-daemon 主循环）
    - `get system_config_client failed! ...`（拿不到 system-config client）
    - `load node gateway_cconfig from system_config failed!` 或 `get node gateway_config failed from system_config_service!`
  - system-config 服务已起，但 RBAC/鉴权失败导致所有 KV 调用都返回 NoPermission。
- Likely cause：
  - system-config 未启动/未监听（OOD 场景下应由 node-daemon 负责 keep；见 `keep_system_config_service()`）。
  - `boot/config` 尚未生成（首次启动时，node-daemon 会进入 BOOT_INIT 并尝试启动 scheduler `--boot`）。
  - cyfs-gateway 尚未加载新的 `nodes/<node>/gateway_config`（node-daemon 只有在 gateway_config 变化时才写 `buckyos/etc/node_gateway.json` 并触发 reload）。
  - scheduler 没有跑起来或卡在 `dump_configs_for_scheduler` / `exec_tx` 失败，导致 node_config / gateway_config 没被写入。
- Where to look in code/KV：
  - code: `src/kernel/node_daemon/src/node_daemon.rs`（`node_daemon_main_loop()`：每 5s 循环；`keep_system_config_service()`；`load_node_gateway_config()`；首次启动 `BOOT_INIT` 逻辑）
  - code: `src/kernel/sys_config_service/src/main.rs`（请求入口日志：`GET: full_res_path:...`；以及 token 校验 `verify_session_token()`）
  - code: `src/kernel/scheduler/src/system_config_agent.rs`（调度 loop；关键日志：`schedule loop step:`、`dump_configs_for_scheduler failed`、`exec_tx failed`、`will update system/rbac/policy => ...`）
  - KV: `boot/config`（不存在时表示还在 boot/init 流程）
  - KV: `nodes/<node_id>/config`、`nodes/<node_id>/gateway_config`（scheduler 产物；node-daemon 读它们）
  - file: `buckyos/etc/node_gateway.json`（node-daemon 落地，用于触发 cyfs-gateway reload）
  - notepads: `new_doc/ref/notepads/整理node_daemon启动服务.md`
- Quick confirm steps：
  - 确认 OOD 上 system-config 固定端口可达：默认 `http://127.0.0.1:3200/kapi/system_config`（见 `src/kernel/sys_config_service/src/main.rs` 常量）。
  - 如果是首次启动：观察 node-daemon 是否打印 `enter the BOOT_INIT process...`；并确认 scheduler `--boot` 有没有成功（日志 `do boot scheduler success!`）。
  - 读 KV：检查 `nodes/<node>/config` / `nodes/<node>/gateway_config` 是否存在（不存在通常是 scheduler 没写入/没跑起来）。
  - grep 日志：
    - node-daemon：`node daemon main loop step:`、`BOOT_INIT`、`get system_config_client failed`。
    - scheduler：`schedule loop step:`、`exec_tx failed`。
    - system-config：`GET:` / `No read permission` / `TokenExpired`。

参考：
- `new_doc/ref/notepads/整理node_daemon启动服务.md`

## 3) RBAC 策略变更不是立刻生效（缓存传播）
- `system/rbac/policy` 与 `system/rbac/model` 读取带 cache。
- notepads 记录为最长约 10s 才能全系统看见新策略。
- Symptom：
  - 你刚写入/更新了 `system/rbac/policy`，但某些服务仍然返回 `NoPermission`；过几秒（<=10s）才“突然好了”。
  - 你以为 scheduler 已经更新 policy，但客户端/服务侧读到的还是旧值。
- Likely cause：
  - `SystemConfigClient` 对 `system/rbac/*`（以及 `services/*`）启用了 10s 的进程内 cache（并非 KV 没更新，而是读路径命中缓存）。
  - 目前没有 watch/主动失效机制（代码里也有 TODO）。
- Where to look in code/KV：
  - code: `src/kernel/buckyos-api/src/system_config.rs`（`CONFIG_CACHE_TIME: 10`；`cache_key_control` 包含 `system/rbac/`；日志：`get system_config from CONFIG_CACHE ...`）
  - code: `src/kernel/scheduler/src/system_config_agent.rs`（RBAC 更新写入点；日志：`will update system/rbac/policy => ...`）
  - KV: `system/rbac/model`、`system/rbac/policy`、(参考) `system/rbac/base_policy`
  - notepads: `new_doc/ref/notepads/rbac.md`
- Quick confirm steps：
  - 读 KV：直接对比 `system/rbac/policy` 的最新值是否已经写入（如果 KV 已更新但行为未变，基本就是 cache）。
  - grep cache 命中：在目标服务日志里搜 `get system_config from CONFIG_CACHE`，确认是否读到了缓存。
  - 等待窗口：等待 10s 再重试同一操作，观察是否自然恢复。
  - 强制触发调度：如果你是通过“修改用户/应用/节点”来期望 policy 更新，确认 scheduler 是否真的执行了 `update_rbac()`（看 `will update system/rbac/policy` 日志）。

参考：
- `new_doc/ref/notepads/rbac.md`

## 4) 端口不要假设固定（除少数例外）
- 常见服务端口通过“实例启动→上报 instance_info→scheduler 生成 service_info→客户端选择”确定。
- 少数固定端口：system-config 当前实现固定 3200（`SYSTEM_CONFIG_SERVICE_MAIN_PORT`）。
- Symptom：
  - 你硬编码访问 `:<port>` 直连某个 service instance，结果在另一台机器/另一次启动就连不上。
  - scheduler 产出的 `service_info` / node-gateway 反向代理可以访问，但你绕过它访问“旧端口”，表现为 sporadic connection refused。
- Likely cause：
  - 除了少数 kernel service，服务端口属于“调度结果”，并不稳定；端口冲突时实例会自行换端口并通过 instance_info 上报。
  - 你没有走统一入口（node-gateway / `kapi/<service>`），也没有基于 `service_info` 做 endpoint 选择。
- Where to look in code/KV：
  - notepads: `new_doc/ref/notepads/scheduler.md`（端口分配范式：instance 上报 → scheduler 整理 → client 选择）
  - code: `src/kernel/sys_config_service/src/main.rs`（固定端口 `SYSTEM_CONFIG_SERVICE_MAIN_PORT: 3200`；以及服务启动日志 `Starting system config service on port ...`）
  - code: `src/kernel/buckyos-api/src/runtime.rs`（默认 system-config url：`http://127.0.0.1:3200/kapi/system_config`；以及 node-gateway 入口 `DEFAULT_NODE_GATEWAY_PORT: 3180`）
  - KV: `services/<svc>/info`（调度器写回的可用 endpoint 集合；客户端应从这里选）
- Quick confirm steps：
  - 优先验证“走 service_info”是否可用：先读 `services/<svc>/info`，看里面的 endpoint/端口是不是你硬编码的那个。
  - 如果你必须直连：从日志/`service_info` 找到该 instance 当前端口，而不是猜。
  - system-config 特例确认：检查 system-config 日志里是否打印 `Starting system config service on port 3200`；确认你访问的是 `http://127.0.0.1:3200/kapi/system_config`。

参考：
- `new_doc/ref/notepads/scheduler.md`
- `src/kernel/sys_config_service/src/main.rs`

## 5) pkg-index-db 与 ready 语义是升级稳定性的前提
- repo-server 未 ready 时，node-daemon 可能出现 NotExist→deploy 失败→循环。
- 先确保 repo-server ready，再写入 system-config 意图并触发调度。
- Symptom：
  - node-daemon 日志反复出现：`not exist,deploy and start it!` / `pkg ... not exist`；系统服务（repo-service / control-panel 等）总是处于 NotExist。
  - repo-service 日志出现 meta-index-db 同步/合并失败、找不到可写记录等（见下面 repo_server 锚点日志字符串）。
- Likely cause：
  - node 的 root pkg env 的 `meta_index.db` 没更新到 repo-service 当前版本（node-daemon 会尝试走 NDN 拉取并更新）。
  - “ready” 语义未满足：chunk 未就绪、依赖包未 ready，导致 install/deploy 只是触发了流程，但最终拿不到可运行的 pkg。
  - 在 repo-service 尚未 ready 的时候就开始写入/变更 system-config 意图（例如期望某 service/app 立即被拉起），会让 node-daemon 进入循环尝试。
- Where to look in code/KV：
  - code: `src/kernel/node_daemon/src/node_daemon.rs`（升级相关：`check_and_update_root_pkg_index_db()`；以及 `make_sure_system_pkgs_ready()`；关键日志：`remote index db is not same...`、`pkg ... is not ready!`）
  - code: `src/kernel/node_daemon/src/run_item.rs`（循环触发点；日志：`not exist,deploy and start it!`）
  - code: `src/frame/repo_service/src/repo_server.rs`（repo-server/index-db 逻辑与日志锚点：
    - `Repo Service enable auto sync, will sync from remote meta-index-db`
    - `open default meta-index-db failed`
    - `merge meta-index-db failed`
    - `no source-meta-index and no pub-meta-index, there is no record can write to meta-index-db found`
    - `update meta-index-db:source tag root, download url:`）
  - KV: (间接) repo-service settings 常通过 `services/repo-service/settings` 控制；node-daemon/调度器会以 KV 意图驱动实例。
  - notepads: `new_doc/ref/notepads/app-pkg-system.md`、`new_doc/ref/notepads/repo-server重构.md`
- Quick confirm steps：
  - 看 node-daemon 是否在更新 root env：grep `update root env's meta-index.db OK` / `download new meta-index.db success`。
  - 看 repo-service 是否能打开/合并 index-db：grep `merge meta-index-db failed` / `open default meta-index-db failed`。
  - 如果反复 NotExist：先不要纠结“调度写没写”，优先确认 pkg/index-db ready（否则调度再正确也拉不起）。
  - 等 repo-service ready 后，再触发一次调度/写入意图（避免在缺失依赖时进入 deploy loop）。

参考：
- `new_doc/ref/notepads/app-pkg-system.md`
- `new_doc/ref/notepads/repo-server重构.md`

## 6) 单机多 gateway 实例冲突
- 同设备只运行一个 gateway 是最安全的默认；多实例冲突会导致端口/隧道/路由问题。
- Symptom：
  - 本机出现“有时能访问、有时 502/超时”的非确定性现象；重启其中一个 gateway 后症状变化。
  - node-daemon 明明写了新的 `node_gateway.json` 并提示 reload，但实际流量仍然走旧配置/旧路由。
  - 端口冲突（尤其是 `3180` / `3200` 相关路径）导致某个 gateway 起不来或绑定到意外端口。
- Likely cause：
  - desktop 环境/开发环境同时跑了两个 cyfs-gateway（例如一个是系统由 node-daemon 拉起，另一个是你手工启动/另一套安装）。
  - node-daemon 依赖 `nodes/<node>/gateway_config` 写入 `buckyos/etc/node_gateway.json`，但另一个 gateway 进程并不会跟随 reload。
- Where to look in code/KV：
  - code: `src/kernel/node_daemon/src/node_daemon.rs`（gateway 配置链路：`load_node_gateway_config()` → 写 `buckyos/etc/node_gateway.json` → `keep_cyfs_gateway_service(..., need_reload)`；日志：`node gateway_config changed, will write to node_gateway.json and reload`、`*** keep cyfs-gateway service with sn:`）
  - KV: `nodes/<node_id>/gateway_config`（scheduler 产物；node-daemon 读取它）
  - file: `buckyos/etc/node_gateway.json`（当前生效的 gateway 配置落地文件）
  - notepads: `new_doc/ref/notepads/整理node_daemon启动服务.md`（强调同机只跑一个 cyfs-gateway）
  - notepads: `new_doc/ref/notepads/cyfs-gateway与buckyos的集成问题.md`
- Quick confirm steps：
  - 确认是否有“重复进程”：检查本机是否存在两套 cyfs-gateway（系统拉起 vs 手工/另一目录）。
  - 读 KV vs 文件：对比 `nodes/<node>/gateway_config` 与 `buckyos/etc/node_gateway.json` 是否一致；如果 KV 已更新但文件没变，问题多在 node-daemon；如果文件已变但行为没变，问题多在 gateway reload/多实例。
  - grep reload 链路：在 node-daemon 日志里搜 `node gateway_config changed` / `reload`；确认 need_reload 分支是否触发。
  - 如果端口疑似冲突：优先确认固定端口的 system-config 是否能稳定监听 3200（`src/kernel/sys_config_service/src/main.rs`），避免把“KV 不可达”误判为 gateway 路由问题。

参考：
- `new_doc/系统架构设计.md`（索引页中列出）
- `new_doc/ref/notepads/cyfs-gateway与buckyos的集成问题.md`
