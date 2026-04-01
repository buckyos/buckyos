# buckyos-intergate-service skill

# Role

You are an expert BuckyOS platform engineer. Your task is to integrate an already-implemented system service into the BuckyOS build/scheduler/RBAC 体系，使其能被构建、调度、启动和访问。

# Context

这个 skill 对应 Service Dev Loop 的 **Stage 6–7**（接入 build/scheduler、服务启动/日志/身份/心跳）。

**前提**：服务代码已实现并通过 `cargo test`，服务常量（UNIQUE_ID、PORT）、AppDoc 生成函数、Client/Handler 代码均已在 `buckyos-api` 中定义并导出，crate 已加入 workspace。即 `implement-system-service` 阶段已全部完成。

本阶段的目标是让服务从"能编译"变成"能被 scheduler 拉起、login 成功、heartbeat 正常、gateway 可路由"。

**集成不涉及业务逻辑修改**，只需在 4 个文件中添加注册信息。参照已有服务（`repo_service`、`aicc`、`msg_center`）即可完成，但文件分散容易遗漏，因此本 skill 的核心价值是**把路径列清楚、把常见错误标出来**。

# Applicable Scenarios

Use this skill when:

- 一个新系统服务需要首次接入 BuckyOS 构建和调度体系。
- 一个已有服务重构后需要重新注册（改名、改端口等）。

Do NOT use this skill for:

- 服务业务逻辑实现（use `implement-system-service`）。
- 协议设计（use `design-krpc-protocol`）。
- DV Test（use `service-dv-test`）。

# Input

1. **Service Name** — kebab-case，如 `my-service`。
2. **Service Port** — 需确认不与现有端口冲突（见下方端口表）。
3. **是否需要 Settings** — 若有，提供 Settings 结构定义。

---

# 已占用端口表

集成前 **MUST** 确认端口不冲突：

| 端口 | 服务 | 定义位置 |
|------|------|----------|
| 3200 | system-config | (内部) |
| 3210 | verify-hub | `verify_hub_client.rs:14` |
| 3220 | scheduler | `scheduler_client.rs:7` |
| 3380 | task-manager | `task_mgr.rs:18` |
| 4000 | repo-service | `repo_client.rs:13` |
| 4020 | control-panel | `control_panel.rs:15` |
| 4030 | kmsg | (msg_queue.rs) |
| 4040 | aicc | `aicc_client.rs:13` |
| 4050 | msg-center | `msg_center_client.rs:14` |
| 4060 | opendan | `lib.rs:58` |
| 4100 | smb-service | `system_config_builder.rs:417` |

所有端口常量定义在 `src/kernel/buckyos-api/src/` 下对应的 `*_client.rs` 文件中。

---

# 集成步骤：4 个文件，4 步完成

## Step 1: bucky_project.yaml — 注册构建模块和 rootfs 映射

**文件**: `src/bucky_project.yaml`

参照: 文件中 `repo_service` 的两处注册。

**两处修改**：

### 1a. `modules` 段 — 添加模块定义

```yaml
modules:
  # ... existing ...
  my_service:
    type: rust
    name: my_service
```

### 1b. `apps.buckyos.modules` 段 — 添加 rootfs 目标路径

```yaml
apps:
  buckyos:
    modules:
      # ... existing ...
      my_service: bin/my-service/
```

**关键**: 两处的 key **必须一致**（都用下划线形式 `my_service`），rootfs 目录名用 kebab-case（`bin/my-service/`）。

## Step 2: system_config_builder.rs — 添加服务注册方法

**文件**: `src/kernel/scheduler/src/system_config_builder.rs`

参照: `add_repo_service()` (line 360) 或 `add_aicc()` (line 305)

```rust
pub async fn add_my_service(&mut self) -> Result<&mut Self> {
    let service_doc = generate_my_service_doc();
    let config = build_kernel_service_spec(
        MY_SERVICE_UNIQUE_ID,
        MY_SERVICE_SERVICE_PORT,
        1,  // expected_instance_count
        service_doc,
    ).await?;
    self.insert_json("services/my-service/spec", &config)?;

    // 若有 settings：
    // let settings = MyServiceSettings { ... };
    // self.insert_json_if_absent("services/my-service/settings", &settings)?;

    Ok(self)
}
```

**关键**: `insert_json` 的 key **MUST** 是 `services/<kebab-name>/spec`，与 UNIQUE_ID 完全一致。

## Step 3: scheduler/main.rs — 在 builder 链中调用注册方法

**文件**: `src/kernel/scheduler/src/main.rs`

参照: line 116–123 的 builder 链

```rust
builder
    // ... existing .add_*() calls ...
    .add_my_service()
    .await?
    // ...
```

**注意**: 该文件中有**多处**使用 `SystemConfigBuilder` 或调用 `create_init_list_by_template` 的地方（约 line 34 和 line 369）。搜索所有调用点，确保都添加了新服务。

## Step 4: boot.template.toml — 添加 RBAC 角色

**文件**: `src/rootfs/etc/scheduler/boot.template.toml`

参照: line 183–194 的 `g, repo-service, kernel` 等行

```
g, my-service, kernel
```

这行将服务加入 `kernel` 角色组，使其拥有访问 system_config 等内核资源的权限。

---

# 集成后的运行链路

理解完整链路有助于排查问题：

```
1. buckyos build
   → bucky_project.yaml 决定编译哪些 crate
   → 产物复制到 rootfs/bin/<service-name>/

2. 系统启动 / 激活
   → scheduler 调用 create_init_list_by_template()
   → builder 链中 add_my_service() 写入 system_config:
     services/my-service/spec → KernelServiceSpec

3. scheduler 调度循环
   → 读取 services/my-service/spec
   → 根据 selector_type + 节点资源选择目标节点
   → 写入 nodes/<node>/config 中的 instance 配置

4. node daemon 收到 instance 配置
   → 从 rootfs/bin/<service-name>/ 找到二进制
   → 注入环境变量（含 MY_SERVICE_SESSION_TOKEN）
   → 启动进程

5. 服务进程启动
   → init_buckyos_api_runtime() 自动读取 SESSION_TOKEN
   → login() 与 verify-hub 建立身份
   → heartbeat 上报到 services/my-service/instances/<node>

6. gateway 路由
   → scheduler 根据 instance 信息生成 gateway_config
   → /kapi/my-service 请求被路由到 127.0.0.1:<port>
```

**环境变量名生成规则**（`buckyos-api/src/lib.rs:88`）：`"my-service"` → `MY_SERVICE_SESSION_TOKEN`（大写 + 连字符变下划线）。服务代码无需手动处理，`init_buckyos_api_runtime()` 自动读取。

---

# 验证检查清单

## 构建验证

- [ ] `buckyos build` 成功
- [ ] 二进制出现在 `rootfs/bin/<service-name>/` 下

## 运行验证

- [ ] Scheduler 日志中可看到 service spec 被加载
- [ ] Node daemon 日志中可看到服务进程被启动
- [ ] `login()` 成功（日志中无 token 错误）
- [ ] Heartbeat 正常（scheduler 不将 instance 标记为 unavailable）
- [ ] Gateway 可将 `/kapi/<service-name>` 请求路由到服务

---

# Common Failure Modes

## 1. UNIQUE_ID 不一致（最常见）

**症状**: Scheduler 创建了 instance 但 node daemon 找不到二进制，或 gateway 路由失败。
**原因**: 以下三处名称不一致：
- `_client.rs` 中的 `UNIQUE_ID`
- `system_config_builder.rs` 中 `insert_json` 的 key 路径
- `bucky_project.yaml` 中的 rootfs 目录名

**修复**: 统一 kebab-case。例如 `UNIQUE_ID = "my-service"` → config key = `services/my-service/spec` → rootfs = `bin/my-service/`。

## 2. bucky_project.yaml 两处 key 不匹配

**症状**: `buckyos build` 成功但二进制未出现在预期的 `rootfs/bin/<name>/` 下。
**原因**: `modules` 段和 `apps.buckyos.modules` 段的 key 不一致。
**修复**: 两处 key 必须相同（都用下划线形式 `my_service`）。

## 3. 忘记在 builder 链中调用 add 方法

**症状**: 系统启动后 `services/my-service/spec` 不存在，scheduler 不调度该服务。
**原因**: 写了 `add_my_service()` 方法但忘记在 `main.rs` builder 链中调用。
**修复**: 在 `create_init_list_by_template()` 中添加 `.add_my_service().await?`。

## 4. 忘记加入 RBAC policy

**症状**: Login 成功，但调用 system_config 或其他内核服务时返回 permission denied。
**原因**: boot.template.toml 中未添加 `g, my-service, kernel`。
**修复**: 添加 RBAC 角色映射行。

## 5. 端口冲突

**症状**: 服务启动失败，`address already in use`。
**原因**: 端口与已有服务冲突。
**修复**: 查端口表，选未占用端口。

## 6. 声明端口与实际监听端口不一致

**症状**: Gateway 路由到错误端口，或健康检查失败。
**原因**: `generate_*_service_doc()` 声明的端口与 `Runner::new(port)` 使用的不同。
**修复**: 确保 `_client.rs` 中的 `SERVICE_PORT` 常量同时被 AppDoc 生成和服务代码引用。

## 7. Node daemon 找不到可执行文件

**症状**: 日志中 `resolve executable failed`。
**原因**: rootfs 目录名与编译产物不匹配。Node daemon 按以下优先级查找：
1. `kernel_pkg.toml` 中的 `kernel_pkg_service_name`
2. Package unique name
3. 自动发现目录下的可执行文件

**修复**: 确保 `rootfs/bin/<service-name>/` 下有且仅有一个可执行文件，或添加 `kernel_pkg.toml`。

## 8. main.rs 中遗漏了第二个调用点

**症状**: 首次 boot 正常，但 `schedule_boot` 等场景下服务未注册。
**原因**: `scheduler/src/main.rs` 中有多处使用 `SystemConfigBuilder` 的地方。
**修复**: 搜索文件中所有 `SystemConfigBuilder` 使用点，确保都添加了新服务。

## 9. 全新安装后 RBAC 不生效

**症状**: 已有环境正常，全新安装后权限报错。
**原因**: 旧环境可能手动改过 policy，新安装完全依赖 boot.template.toml。
**修复**: 确保 boot.template.toml 是 RBAC policy 的唯一真相来源。

---

# 快速参照

| # | 文件 | 动作 | 参照 |
|---|------|------|------|
| 1 | `src/bucky_project.yaml` | modules 定义 + rootfs 映射 | `repo_service` 行 |
| 2 | `src/kernel/scheduler/src/system_config_builder.rs` | `add_*()` 方法 | `add_repo_service()` L360 |
| 3 | `src/kernel/scheduler/src/main.rs` | builder 链中调用 | L116–123 |
| 4 | `src/rootfs/etc/scheduler/boot.template.toml` | RBAC `g, <name>, kernel` | L183–194 |

4 个文件，0 个新建，全部追加。遗漏任何一个都会导致集成失败。
