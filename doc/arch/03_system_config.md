# 03. system-config（系统状态存储 / Source of Truth）

system-config 是 Zone 的“真相源”（KV）：调度器、控制面工具、node-daemon、权限系统都围绕它运作。
它的设计目标不是“做一个通用配置中心”，而是把控制面状态收敛成可审计、可事务化、可回放的数据模型（见 `new_doc/arch/01_overview.md` 对系统心智模型的描述）。

## 核心职责
- 提供统一 KV 状态存储，承载：`boot/config`、service spec、node_config、RBAC 模型与策略、调度快照等。
- 提供事务化修改能力：控制面/工具可以一次性写入一组 KV 变更（避免“写到一半被读到”的中间态）。
- 承载部分可信初始化逻辑：从 `boot/config` 加载并刷新 trust keys，以及加载 RBAC model/policy（服务端实现见 `src/kernel/sys_config_service/src/main.rs`）。

实现锚点：
- 服务端口固定为 3200：`src/kernel/sys_config_service/src/main.rs`（`SYSTEM_CONFIG_SERVICE_MAIN_PORT: u16 = 3200`）
- 客户端缓存 TTL 为 10s：`src/kernel/buckyos-api/src/system_config.rs`（`CONFIG_CACHE_TIME:u64 = 10`）

## 服务端（sys_config_service）实现要点
system-config 服务以 kRPC 方式暴露在 `/kapi/system_config`，并在入口统一做 token 校验，再分发到各 handler：
- 入口/分发模式：`src/kernel/sys_config_service/src/main.rs`（`SystemConfigServer::process_request` 中的 method match）
- session_token 校验：`src/kernel/sys_config_service/src/main.rs`（`verify_session_token`）
- 统一 RBAC enforce：`src/kernel/sys_config_service/src/main.rs`（`handle_get`/`handle_set`/`handle_exec_tx` 等都会对 key 做 `kv://...` 归一化后调用 `rbac::enforce`）

当前暴露的 RPC 方法（以服务端 method 字符串为准）：
- `sys_config_get` / `sys_config_set` / `sys_config_create` / `sys_config_delete`
- `sys_config_append` / `sys_config_list` / `sys_config_set_by_json_path`
- `sys_config_exec_tx`（事务）
- `dump_configs_for_scheduler`（为 scheduler/node-daemon 打包导出一批配置）
- `sys_refresh_trust_keys`（重新加载 trust keys + RBAC enforcer）

### handler 典型模式（可当作代码阅读地图）
- **Key 归一化**：`src/kernel/sys_config_service/src/main.rs` 的 `get_full_res_path` 把 `kv://` 前缀、路径分隔符、`/./..` 等归一化，得到 `(full_res_path, real_key_path)`。
- **权限校验**：每个读写接口都会用 `rbac::enforce(userid, appid, full_res_path, "read"|"write")` 做校验（例如 `handle_get` / `handle_set`）。
- **数据存取**：统一通过 `SYS_STORE: Arc<Mutex<dyn KVStoreProvider>>` 访问底层存储；当前默认使用 `SledStore`（见 `src/kernel/sys_config_service/src/main.rs` 顶部 `SledStore::new()` 初始化）。
- **事务 exec_tx**：`handle_exec_tx` 先对 actions 中每个 key 做单独的 RBAC 校验，再把 action 转成 `KVAction`，最终调用 `store.exec_tx(tx_actions, main_key)`。
- **scheduler dump**：`dump_configs_for_scheduler` 限制 appid 为 `scheduler` 或 `node-daemon`，并批量 `list_data(prefix)` 导出（包含 `boot/`、`devices/`、`users/`、`services/`、`system/`、`nodes/`）。

## Key data structures and API
本节只描述“系统层面必须知道的接口与语义”，忽略 UI/工具封装。

### 1) Rust 客户端：SystemConfigClient（含 10s cache）
实现：`src/kernel/buckyos-api/src/system_config.rs`
- `SystemConfigClient` 内部使用 kRPC 调用服务端（默认 URL 为 `http://127.0.0.1:3200/kapi/system_config`）。
- 缓存 TTL：`CONFIG_CACHE_TIME:u64 = 10`，并且只对部分 key 前缀启用（`SystemConfigClient::new` 中的 `cache_key_control` 当前包含 `services/`、`system/rbac/`）。
- 缓存结构：进程级 `CONFIG_CACHE: HashMap<String,(String,u64)>`（key -> (value, version)），并用 `current_version` + `CONFIG_CACHE_TIME` 做过期判断（见 `get_config_cache`）。

对外 API（以方法名为准）：
- `get(key) -> SystemConfigValue`：优先从缓存返回；缓存未命中才会 kRPC 调用 `sys_config_get`。
- `set(key,value)` / `create(key,value)` / `delete(key)` / `append(key,value)` / `list(prefix)`
- `set_by_json_path(key, json_path, value)`：服务端 `sys_config_set_by_json_path`。
- `exec_tx(tx_actions, main_key)`：服务端 `sys_config_exec_tx`。
- `dump_configs_for_scheduler()`：服务端 `dump_configs_for_scheduler`。

### 2) 服务端事务结构：exec_tx actions map
服务端期望的事务请求体是一个 map：key -> action，并在 `src/kernel/sys_config_service/src/main.rs` 的 `handle_exec_tx` 里解析。
- action 类型字符串：`create` / `update` / `append` / `set_by_path` / `remove`
- `main_key`（可选）：形如 `"<key>:<revision>"`，服务端会解析为 `(String, u64)` 传给 `store.exec_tx`（见 `handle_exec_tx`）。

## 关键 KV 路径（必须掌握）
下面的路径在系统关键链路里出现频率最高，也是排查问题最常用的入口。
- `boot/config`：ZoneConfig（系统引导后最核心的全局配置；同时影响 trust keys 刷新逻辑，见 `src/kernel/sys_config_service/src/main.rs` 的 `handle_refresh_trust_keys`）
- `system/rbac/*`：RBAC 模型与策略（至少包括：`system/rbac/model`、`system/rbac/base_policy`、`system/rbac/policy`）
- `system/scheduler/snapshot`：调度快照（scheduler 用于判断输入/结果变化，写入逻辑见 `src/kernel/scheduler/src/system_config_agent.rs`）

补充：scheduler dump 还会打包导出整个 prefix（见 `src/kernel/sys_config_service/src/main.rs` 的 `dump_configs_for_scheduler`）：
- `boot/`、`devices/`、`users/`、`services/`、`system/`、`nodes/`

参考：
- `new_doc/ref/doc/key data.md`（列出部分关键 KV；以代码为准）
- `src/kernel/scheduler/src/system_config_agent.rs`（调用 `dump_configs_for_scheduler`、写入 `system/scheduler/snapshot`、更新 `system/rbac/policy`）

## 关键流程伪代码（客户端与 scheduler 视角）
伪代码只表达“必须理解的控制流与约束”，不是实现细节复刻。

### 1) get（带 10s cache 的读取）
对应实现：`src/kernel/buckyos-api/src/system_config.rs`（`SystemConfigClient::get` + `get_config_cache`）

```text
system_config.get(key):
  if key matches cache prefixes (services/, system/rbac/):
    v = CONFIG_CACHE.get(key)
    if v exists and not expired by CONFIG_CACHE_TIME:
      return (value=v.value, is_changed=false)

  // cache miss
  value = kRPC.call("sys_config_get", {key})
  if value is null:
    return KeyNotFound

  revision = now_unix_timestamp()
  CONFIG_CACHE[key] = (value, revision)
  return (value=value, version=revision, is_changed=maybe_changed)
```

### 2) set（写入并更新/失效缓存）
对应实现：`src/kernel/buckyos-api/src/system_config.rs`（`SystemConfigClient::set`）

```text
system_config.set(key, value):
  kRPC.call("sys_config_set", {key, value})
  revision = now_unix_timestamp()
  CONFIG_CACHE[key] = (value, revision)  // only if key is cacheable
```

### 3) exec_tx（事务写入一组 KV）
对应实现：
- 客户端：`src/kernel/buckyos-api/src/system_config.rs`（`SystemConfigClient::exec_tx`，会把 KVAction map 编码成 actions object）
- 服务端：`src/kernel/sys_config_service/src/main.rs`（`handle_exec_tx`）

```text
system_config.exec_tx(actions, main_key?):
  req.actions = {
    key1: {action: "create"|"update"|"append"|"set_by_path"|"remove", ...},
    key2: {...},
  }
  if main_key provided:
    req.main_key = "<key>:<revision>"

  kRPC.call("sys_config_exec_tx", req)

  // client-side: remove caches for touched keys
  for key in actions.keys:
    CONFIG_CACHE.remove(key)
```

### 4) scheduler dump_configs_for_scheduler（读取全量输入状态）
对应实现：
- 服务端：`src/kernel/sys_config_service/src/main.rs`（`dump_configs_for_scheduler`）
- scheduler 调用点：`src/kernel/scheduler/src/system_config_agent.rs`（`SystemConfigClient::dump_configs_for_scheduler`）

```text
dump_configs_for_scheduler():
  require appid in {"scheduler", "node-daemon"}

  result = {}
  result += store.list_data("boot/")
  result += store.list_data("devices/")
  result += store.list_data("users/")
  result += store.list_data("services/")
  result += store.list_data("system/")
  result += store.list_data("nodes/")
  return result
```

## differences vs etcd/consul（显式取舍）
BuckyOS 的 system-config 当前是“系统内置 KV + RBAC + 事务接口”的组合（服务端实现见 `src/kernel/sys_config_service/src/main.rs`，默认存储为 `SledStore`），与 etcd/Consul 这类通用分布式 KV/控制面基础设施在目标与代价上明显不同。

明确差异与取舍：
- **一致性与可用性模型**：etcd 以 Raft 提供线性一致的读写；Consul 的 KV 同样基于 Raft（并用 gossip 做健康检查/成员信息）。BuckyOS 当前实现是单个 system-config 服务进程 + 本地 KV provider（`SledStore`），不提供 etcd/Consul 那种“跨节点强一致 + 自动 failover”的集群语义。
- **事务能力**：etcd 有成熟的 compare/txn 语义；Consul 也有 Check-and-Set 等机制。BuckyOS 提供 `sys_config_exec_tx` 来一次提交多 key 变更（服务端 `handle_exec_tx`），但其语义与隔离/冲突处理方式以 `KVStoreProvider::exec_tx` 的实现为准。
- **watch/订阅能力**：etcd/Consul 都有 watch / blocking query 这类原生机制；BuckyOS 客户端目前以读 + 缓存为主，并在 `src/kernel/buckyos-api/src/system_config.rs` 顶部明确标注了 `//TODO: add WATCH`，因此系统层面主要依赖 scheduler loop/node-daemon loop 的轮询式收敛。
- **API 面与运维复杂度**：etcd/Consul 带来更大的 API 面（watch、lease/TTL、集群运维、监控、升级等）但也提供成熟生态；BuckyOS 选择更窄的系统内接口（get/set/list/exec_tx + scheduler dump），把复杂性留在“数据模型 + 调度器幂等”上，而不是引入一个外部通用控制面基础设施。

## 工程注意事项
- **RBAC 策略缓存传播延迟**：`system/rbac/*` 在客户端默认启用 10s 缓存（`src/kernel/buckyos-api/src/system_config.rs`），会影响“刚改完权限立刻生效”的预期；详见 `new_doc/ref/notepads/rbac.md`。
- **scheduler/node-daemon 与 dump 权限**：`dump_configs_for_scheduler` 在服务端限制 appid 只能是 `scheduler` 或 `node-daemon`（见 `src/kernel/sys_config_service/src/main.rs` 的 `dump_configs_for_scheduler`），因此排查“scheduler 看不到输入”时，要先确认调用身份与 token 生成链路。
- **启动顺序**：system-config 是关键路径组件，启动顺序错误会导致 node-daemon/scheduler 进入循环失败；经验总结见 `new_doc/arch/09_pitfalls.md`。
