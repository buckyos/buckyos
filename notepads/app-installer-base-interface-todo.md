# App Installer 底层基础接口 TODO

> 背景：正在整理 App Installer 底层组件的基础接口。
>
> beta 2.2 是 breaking change，不保留旧接口兼容层。
>
> 本文先整理类型职责与调用关系，不直接修改当前实现。

## 1. 需要冻结的核心结论

一次被用户确认的安装，应由以下数据唯一确定：

```text
Validated AppDoc body
+ InstallPlan
+ Task Context（user_id、事务 id 等非应用配置）
```

其中 `InstallTarget` 和 `InstallParams` 是生成 Plan 前的输入，Plan 产生后必须作为冻结快照包含在 `InstallPlan` 中，不能在 Prepare/Deploy 阶段成为第二套输入。

建议统一表达为：

```text
Inspect(
    AppDoc,
    DidResolutionSnapshot,
    InstallTargetSnapshot,
    InstallParams,
    InstallPolicy,
) -> InstallPlan

Confirm(InstallPlan.plan_fingerprint) -> InstallApproval

Prepare/Deploy(
    Validated AppDoc,
    Approved InstallPlan,
    Task Context,
) -> AppServiceSpec
```

## 2. 类型职责 TODO

### P0：区分 Target 选择与 Target 快照

- [ ] 对外接口只接受 `target_node_id` 或 `InstallTargetSelector`，不接受完整 `InstallTarget`。
- [ ] 后端根据 `devices/{node}/info` 构造可信 Target 快照；OS、arch、runtime version、kernel version、capabilities 不允许由客户端提交。
- [ ] 评估将当前 `InstallTarget` 改名为 `InstallTargetSnapshot`，明确它是 Plan 内不可变的环境证据。
- [ ] Confirm 前如果目标 Node 信息发生变化，旧 Plan 必须失效并重新 Inspect。

建议形状：

```rust
pub struct InstallTargetSelector {
    pub node_id: String,
}

pub struct InstallTargetSnapshot {
    pub node_did: Option<DID>,
    pub node_id: String,
    pub os: String,
    pub arch: String,
    pub kernel_version: Option<String>,
    pub runtime_version: Option<String>,
    pub capabilities: BTreeMap<String, i64>,
}
```

### P0：明确 InstallParams 只表示用户选择

- [ ] `InstallParams` 只承载用户可调整的安装选项，不混入设备探测结果、下载状态或 Installer 内部状态。
- [ ] `permissions: Vec<PermissionItem>` 表示实际批准权限；必须是 AppDoc 权限声明的合法子集。
- [ ] mount、service settings、环境变量、resource pool、auto-start 等字段都必须有对应的 AppDoc 约束或系统约束。
- [ ] `InstallParams` 变化只能通过重新 Inspect 生成新 Plan，禁止在旧 Plan 上直接 patch。
- [ ] 决定是否需要区分“用户未填写”与“明确提交空值”；不要用 `Default` 混淆这两种语义。

### P0：InstallPlan 成为唯一安装蓝图

- [ ] `InstallPlan` 内冻结 AppDoc reference、DID resolution、Target snapshot、InstallParams、selected packages、required content identity 和最终运行配置。
- [ ] 明确 fingerprint 的完整绑定字段，并把它作为 Plan identity；Target、Params、AppDoc、selected package 或最终配置变化都必须产生新 fingerprint。
- [ ] Prepare/Deploy 只消费 Plan 中的 target、params 和 final config，不再接收独立副本。
- [ ] 最终 `AppServiceSpec.permission`、`enable`、`spec_config` 必须全部从已批准 Plan 确定。
- [ ] `user_id`、事务 id、安装时间、动态分配的 `app_index` 属于 Task/Prepare 上下文，不应伪装成用户安装配置。

建议核心形状：

```rust
pub struct InstallPlan {
    pub schema_version: u32,
    pub app: AppDocumentRef,
    pub resolution: DidResolutionSnapshot,
    pub target: InstallTargetSnapshot,
    pub params: InstallParams,
    pub selected_packages: Vec<SelectedPackage>,
    pub required_contents: Vec<RequiredContent>,
    pub final_config: ServiceSpecConfig,
    pub plan_fingerprint: String,
    pub created_at: u64,
}
```

### P0：拆分不可变 Plan 与动态检查状态

当前 `InstallPlan` 同时包含安装蓝图和 `readiness/location/issues` 等动态观测结果，容易让 Plan 看起来会在 Acquire 前后发生变化。

- [ ] 决定是否拆出 `InstallInspection` / `InstallPlanStatus`。
- [ ] `RequiredContent` 只保存稳定的内容身份、期望 digest、package binding；`installed/named_store/pikg/missing` 等位置放入状态对象。
- [ ] readiness、estimated download bytes、target/config issues 放入状态对象，不进入不可变 Plan，或者明确标注它们不属于 fingerprint material。
- [ ] Acquire 只更新状态，不改变已批准的安装语义；如果发现 PackageMeta 展开结果与 Plan 不一致，则 Plan 失效并重新 Inspect。

建议关系：

```rust
pub struct InstallInspection {
    pub plan: InstallPlan,
    pub readiness: PlanReadiness,
    pub content_locations: Vec<ContentLocationSnapshot>,
    pub target_issues: Vec<String>,
    pub config_issues: Vec<String>,
    pub estimated_download_bytes: u64,
}
```

### P0：收紧 Approval 语义

- [ ] `InstallApproval` 以 `plan_fingerprint` 为唯一批准对象；重复保存 target/params 时只能作为审计副本，不能成为第二真相源。
- [ ] `apps.install.confirm` 必须接收 UI 实际展示过的 `plan_fingerprint`，拒绝 stale fingerprint，避免用户批准到未看过的新 Plan。
- [ ] Confirm 不应同时修改 target/params 并立即批准重算后的 Plan。
- [ ] 增加独立的“修改选项并重新 Inspect”接口，返回新 Plan/Inspection；UI 展示新结果后再 Confirm。

建议 RPC 分工：

```text
apps.install / apps.install_package
    -> 创建事务并返回 task_id

apps.install.plan.update { task_id, target_node_id?, install_params? }
    -> 重新 Inspect，返回新的 InstallInspection

apps.install.confirm { task_id, plan_fingerprint }
    -> 只批准已展示的 Plan
```

### P1：明确 AppDoc body 的持久化与消费

- [ ] `InstallPlan` 只保存 `AppDocumentRef`，Task transaction 保存经过解析和 Object ID 校验的 AppDoc body。
- [ ] Verify 必须重新计算 AppDoc Object ID，并与 Plan reference、DID resolution snapshot 一致。
- [ ] Prepare 构造 `AppServiceSpec` 时只能使用上述 validated AppDoc body。
- [ ] 不允许 Prepare 再从 Repo/URL 读取另一份同名 AppDoc。

## 3. 推荐调用流程

```text
1. Resolve identifier / pikg candidate
2. 得到 validated AppDoc + DidResolutionSnapshot
3. 客户端只选择 target_node_id，后端生成 InstallTargetSnapshot
4. 用户提交 InstallParams
5. Inspect 编译 InstallPlan，并计算 fingerprint
6. 返回 InstallInspection 给 UI
7. UI 使用 fingerprint 确认当前展示的 Plan
8. Acquire/Verify 只补齐和验证 Plan 指定的内容
9. Prepare/Deploy 使用 AppDoc + Approved InstallPlan 构造 AppServiceSpec
10. Activate 成功后写 InstallRecord/proof
```

## 4. 验收条件

- [ ] 从类型签名可以直接看出哪些字段来自用户、哪些来自目标节点、哪些是 Installer 推导结果。
- [ ] 客户端无法伪造 Target capabilities/runtime/OS/arch。
- [ ] 任意 Target/Params/AppDoc/final config 变化都会生成不同 fingerprint。
- [ ] Confirm 无法批准 stale 或未展示过的 Plan。
- [ ] Prepare/Deploy 没有独立 Target/Params 输入。
- [ ] AppDoc + Approved InstallPlan 足以唯一决定应用权限、启停状态、运行配置和内容集合。
- [ ] readiness/content location 的变化不会被误解为安装语义发生变化。
- [ ] 协议文档、`buckyos-api` 共享类型、Control Panel RPC、SDK/WebUI 和 DV Test 同步更新。

## 5. 预计影响入口

- `src/kernel/buckyos-api/src/app_install.rs`
- `src/kernel/buckyos-api/src/taskdata.rs`
- `src/frame/control_panel/src/app_install_engine.rs`
- `src/frame/control_panel/src/app_install_driver.rs`
- `src/frame/control_panel/src/app_install_planner.rs`
- `src/frame/control_panel/src/app_installer.rs`
- `doc/App 安装协议.md`
- `doc/control_panel/Control_Panel_Service.md`
- `test/app_installer_test/**`

