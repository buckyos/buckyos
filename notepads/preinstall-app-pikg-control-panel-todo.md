# 预装 App：由 Control Panel 消费 PIKG + install plan TODO

> 基线：`6d758af9`（2026-08-24）。
>
> beta 2.2 是 breaking change；只考虑全新 SystemConfig，不做旧 `pre_install_apps`、旧 AppSpec 或旧 bootstrap execution record 的兼容读取。

## 1. 任务目标

把 `src/rootfs/etc/scheduler/boot.template.toml` 中的 `pre_install_apps` 从“内嵌完整 AppDoc/ServiceSpecConfig，并由 scheduler bootstrap 拼装、执行 InstallPlan”改为：

```text
pre-install app config = local PIKG path + install plan
```

预装 App 的执行主体改为 **Control Panel**：

- `scheduler --boot` 只建立可启动的 SystemConfig 和完成 kernel/frame service boot schedule；
- Control Panel 成功登录系统并启动后，读取预装配置；
- Control Panel 把 rootfs 内的 PIKG 转换为受控 staging source，复用现有 App Installer 的 Resolve → Inspect → Acquire → Verify → Prepare → Deploy → Activate 全流程；
- Scheduler 只继续承担现有的最终 InstallPlan 提交、AppRegistry/AppSpec CAS 和普通目标状态调度，不增加 PIKG reader、预装扫描、预装任务或专用恢复逻辑。

端到端样例使用 `buckyos-systest.buckyos.bns.did`：从 `src/apps/sys_test` 的代码和构建产物现场构造标准 PIKG，最终输出到 `src/rootfs/data/cache`，再由 Control Panel 的预装任务安装并启动。

## 2. 为什么由 Control Panel 执行

- Control Panel 是系统服务；它能完成 runtime login 并启动服务，说明 system-config、TaskManager、NamedStore、Scheduler 等安装依赖通常已经可用。
- Control Panel 已经拥有 App 安装的共享类型、PIKG staging、PIKG reader、planner、materialize、InstallEngine、TaskManager 持久任务和 InstallRunner 恢复循环。
- 预装和用户主动安装可以共用同一条安装流水线，只在 source/policy/approval 上有内部差异。
- Scheduler 是内核状态推导与提交组件，不应该理解 rootfs 文件、PIKG 格式、安装来源、信任策略或 TaskManager 安装事务。
- 默认原则：任何能在 Control Panel 完成的预装逻辑，都不要加入 Scheduler。

## 3. 当前实现与需要撤销的历史惯性

- `boot.template.toml` 仍内嵌 filebrowser 和 systest 的完整 AppDoc、service config，并使用 `111...` / `666...` 等伪造 `pkg_objid`。
- `SystemConfigBuilder::stage_bootstrap_install_plan()` 从这些字段临时构造不完整的 `InstallPlan`：
  - source 被写成 `Catalog`，不是 PIKG；
  - `required_contents` 固定为空；
  - target 使用 scheduler 编译机的 OS/arch；
  - builder 重新解释了 AppDoc 和 package selector。
- `do_boot_scheduler()` 在第一次 boot schedule 之前调用 `recover_install_plan_executions()`，形成潜在的 boot 阶段安装 App 路径。
- Scheduler 当前还使用 `recover_install_plan_executions()` 处理 builder 预置的 pending record；普通预装不应再通过这条特殊 bootstrap 路径产生 record。
- Control Panel 已有可恢复安装基础设施：
  - `ControlPanelServer::new()` 创建共享 `PikgStagingStore`、`InstallEngine` 和 `InstallRunner`；
  - `InstallRunner::start()` 已有 TaskManager startup scan 和 sweep；
  - `InstallPolicy::SystemInternal + auto_confirm=true` 已支持系统内部自动确认；
  - `ProductionInstallDriver::materialize_candidate_pikg()` 已负责把 AppDoc、PackageMeta 和 payload 写入 NamedStore；
  - Prepare 最终调用 scheduler `submit_install_plan()`，Activate 轮询 execution record。
- `src/apps/sys_test/build.mjs` 当前只生成 `dist/`，`bucky_project.yaml` 再把它复制到 `rootfs/bin/buckyos_systest/`，没有构造 PIKG。
- systest 身份存在漂移：boot template / WebUI 使用 `did:bns:buckyos-systest.buckyos`，而 `src/rootfs/local/did_docs/systest.buckyos.bns.did.doc.json` 仍是 `did:bns:systest.buckyos`。

## 4. 必须冻结的边界

### P0.1 Boot 与 Scheduler 边界

- [ ] `scheduler --boot` 不打开 PIKG、不写 NamedStore、不创建 App 安装 Task、不 claim/execute/retry 普通 App InstallPlan、不创建普通 AppSpec/InstallRecord、不分配 AppRegistry app/instance。
- [ ] 删除 `do_boot_scheduler()` 中的 `recover_install_plan_executions()` 调用。
- [ ] `SystemConfigBuilder::add_default_apps()` 只解析并保留预装配置，不再生成 `InstallPlanExecutionRecord`。
- [ ] boot 可以拒绝损坏的 `system/install_settings` JSON/schema；PIKG 缺失、损坏或安装失败发生在 Control Panel 启动后，不得反向让系统 boot 失败。
- [ ] Scheduler 不读取 `system/install_settings.pre_install_apps`，不解析 `pikg_path`，不依赖 Control Panel 的 PIKG module。
- [ ] Scheduler 的公开 plan RPC 和 Registry/AppSpec commit 逻辑原则上保持不变。除删除 bootstrap 调用/旧 builder 入口外，本任务不顺带重写 scheduler execution protocol。

### P0.2 Control Panel 边界

- [ ] Control Panel runtime login 成功、`ControlPanelServer` 建立且安装 runner 启动后，再启动 `PreInstallReconciler`。
- [ ] PreInstallReconciler 是 Control Panel 内部后台任务，不是新的公共 RPC。
- [ ] reconciler 只负责把 rootfs seed 转换为一项标准、持久、可恢复的 App install/update task；Stage 推进继续由 `InstallEngine + InstallRunner` 完成。
- [ ] 预装失败只影响对应 Task/状态，不得退出 Control Panel 进程或让 HTTP service 不可用。
- [ ] Control Panel 重启后，TaskManager startup scan/sweep 恢复同一任务；reconciler 不另建第二套安装状态机。

### P0.3 `pre_install_apps` 数据契约

目标形状建议为：

```json
{
  "pre_install_apps": {
    "buckyos-systest.buckyos.bns.did": {
      "schema_version": 1,
      "pikg_path": "data/cache/buckyos-systest.buckyos.bns.did-0.5.1.pikg",
      "install_plan": {
        "...": "最终 InstallPlan 或明确命名的 plan seed"
      }
    }
  }
}
```

- [ ] `PreInstallAppConfig` 使用 `#[serde(deny_unknown_fields)]`，只保留 `schema_version`、`pikg_path`、`install_plan`；删除旧 `app_doc + flatten ServiceSpecConfig` 形状。
- [ ] map key 必须等于 plan/PIKG AppDID 派生出的 canonical AppId。
- [ ] `pikg_path` 是 `$BUCKYOS_ROOT` 相对路径；Control Panel canonicalize 后必须位于 `$BUCKYOS_ROOT/data/cache`，并拒绝绝对路径、`..`、symlink escape 和非普通文件。
- [ ] raw path 只存在于可信 rootfs 配置和 Control Panel 内部 reconciler；不得加入通用 `InstallSource`、公开 RPC、scheduler plan 或 TaskManager 对外 schema。
- [ ] reconciler 必须先把文件复制到现有 `PikgStagingStore` 的 immutable staging 区，再以 `InstallSource::LocalPikg { staging_handle }` 创建标准安装任务。后续 Stage 不再读取可替换的 rootfs 原路径。

`install_plan` 的类型必须明确，不能混用两个语义：

- 如果它就是 `buckyos_api::InstallPlan`，Control Panel 必须像 `apps.submit` 一样重新 Inspect PIKG，并要求 submitted plan 与 authoritative inspection 的 fingerprint 完全一致。
- 如果模板只保存 owner、target selector、mount、service setting 等预装意图，而 PIKG digest、AppDoc ObjectId、PackageMeta、required contents、target snapshot 和 fingerprint 需要运行时产生，则新增明确命名的 `PreInstallPlanSeed` / `PreInstallPlanTemplate`，由 Control Panel planner 生成最终 `InstallPlan`。
- 推荐最终 Task transaction 和 scheduler execution record 中只出现标准 immutable `InstallPlan`；seed 只存在于 `system/install_settings`。
- 禁止为了预装方便接受空 fingerprint、空 `required_contents`、伪造 ObjectId 或绕过重新 Inspect 的“特殊 InstallPlan”。

## 5. Control Panel 预装执行 TODO

### P1.1 启动与扫描

- [ ] 新增职责单一的 `pre_install_reconciler.rs`，由 `start_control_panel_service()` 在 runtime login 和 server/runner 初始化完成后启动。
- [ ] 通过 `SystemConfigClient` 读取 `system/install_settings`，严格反序列化后按 canonical AppId 排序处理，保证日志和行为稳定。
- [ ] 等待/重试 TaskManager、NamedStore 和 Scheduler client 就绪；依赖暂不可用属于后台 retryable 状态，不影响 Control Panel 对外服务。
- [ ] 启动时立即 reconcile 一次，之后使用低频 sweep 修复遗漏；不要每几秒重复读取和重建大 PIKG。
- [ ] 每轮只负责确保“期望的预装任务已经存在并已交给 InstallRunner”，不自己执行 Install Stage。

### P1.2 内部 staging

- [ ] 在 `PikgStagingStore` 增加仅供系统内部调用的 `stage_preinstall_file()`，复用 `PikgReader::stage_pikg_file()` 的 digest 绑定和 immutable copy。
- [ ] staging metadata 使用明确的 system principal，例如 creator user 为 Zone Owner/admin、creator app 为 `control-panel`，purpose 为 `Install`，并绑定最终 task lease。
- [ ] staged PIKG 必须重新通过 `PikgReader::open(expected_digest)`；扩展名和 rootfs 可信性都不能代替结构/对象图验证。
- [ ] 同一个 rootfs path 内容发生变化时，由新 PIKG digest / final plan fingerprint 产生新的幂等安装意图；不能复用旧 task 假装完成。

### P1.3 复用标准安装事务

- [ ] 使用 `InstallEngine::create_install_task()` / update 对应入口，不直接调用 `ProductionInstallDriver` 的单个 Stage。
- [ ] request 固定使用：
  - `source = LocalPikg(staging_handle)`；
  - `policy = SystemInternal`；
  - `options.auto_confirm = true`；
  - `submitted_plan = final plan`（若配置提供完整 plan）或由标准 Inspect 生成；
  - `approved_plan_fingerprint` 必须绑定最终 inspection；
  - creator app 为 Control Panel，owner user 使用 boot 配置确定的 Zone Owner。
- [ ] 复用 `apps.submit` 已有的 plan re-inspect、FreshInstall/Upgrade/Satisfied action matrix、mutation ownership 和 idempotency 规则；把这部分抽成内部方法，而不是在 reconciler 复制 RPC handler 代码。
- [ ] 第一次 fresh install 后，如果 rootfs seed 未变化，后续扫描应得到 `Satisfied` 或同一 Task replay；不得重复分配 AppIndex/hostname。
- [ ] rootfs PIKG 升级时按标准 update task 执行，不允许预装路径直接覆盖现有 AppSpec。

### P1.4 幂等与可观察性

- [ ] 幂等键至少绑定 `preinstall + owner_user_id + app_id + pikg_digest + plan_fingerprint`；同一 immutable 输入稳定重放，不同输入不得碰撞。
- [ ] TaskManager 继续作为 Stage/进度/错误的唯一持久真相源。
- [ ] 如需要表达 seed 尚未成功创建 Task 的 preflight 状态，只增加最小 Control Panel 状态 key，例如 `system/control_panel/pre_install_apps/<app_id>`，记录 path、digest、task_id 和 structured error；不得在 Scheduler 新增 preinstall record。
- [ ] preflight 状态与 Task 状态不能成为两套互相竞争的安装状态；一旦 task 创建成功，后续真相只看 TaskManager/InstallRecord。
- [ ] 日志至少包含 app_id、pikg path、digest、task_id、plan fingerprint 和 terminal error code，不打印 PIKG 内容或敏感配置。

## 6. PIKG/plan 绑定 Gate

预装虽使用 `SystemInternal`，仍必须通过标准完整性校验：

- [ ] PIKG digest 等于最终 `InstallSourceIdentity::Pikg.pikg_digest`。
- [ ] PIKG AppDoc canonical ObjectId 等于 plan app reference、source identity 和 resolution snapshot 中的 ObjectId。
- [ ] PIKG AppDID、boot map key、AppId、AppInstanceId 和 owner_user_id 一致。
- [ ] `selected_packages` 中每个 exact PackageId/PackageMeta ObjectId 都来自该 PIKG，selector 与可信 target snapshot 匹配。
- [ ] `required_contents` 完整覆盖所选 package payload，digest、size、format、subpackage binding 与 PIKG `content_index`/PackageMeta 一致。
- [ ] final `install_params` 和 `service_spec_config` 通过 AppDoc permission、mount、endpoint 和 config tips 约束。
- [ ] `Prepare` 继续调用现有 materialize 逻辑，在 scheduler submit 前把 AppDoc、PackageMeta 和 required payload 写入 NamedStore。
- [ ] 任一校验或 materialize 失败都不能提交 InstallPlan，因而不能产生 Registry/AppSpec 部分状态。

## 7. Scheduler 最小改动 TODO

- [ ] `system_config_builder.rs` 删除普通 `pre_install_apps` 到 bootstrap `InstallPlanExecutionRecord` 的转换。
- [ ] `main.rs::do_boot_scheduler()` 删除 boot 前 execution recovery。
- [ ] 删除只为普通预装 App 存在的 builder helper/import/test；保留通用 scheduler InstallPlan RPC 和 execution protocol。
- [ ] 不在 `system_config_agent.rs` 的 schedule loop 增加 preinstall hook。
- [ ] 不给 scheduler 增加 `pikg_path`、PIKG parser、NamedStore materialize、TaskManager client、Control Panel client或预装重试逻辑。
- [ ] 普通 scheduler 启动对已经由 Control Panel 提交的 execution record 继续使用现有 recovery；是否另行修复当前递归 `schedule_loop(false, true)` 不属于本任务，除非实际 E2E 证明它阻断新流程。

### Jarvis / bootstrap Agent Gate

`add_default_agents()` 当前复用 `stage_bootstrap_install_plan()` 创建 Jarvis runtime plan，并由 scheduler recovery 后再写 `BootstrapAgentProvision`。删除 helper 前必须显式处理：

- [ ] 先确认 Jarvis runtime 是否也属于“预装 App”。若是，优先把它表示为 Control Panel 的另一项 preinstall seed，再在 runtime 安装完成后执行 Agent binding。
- [ ] 如果本任务暂不迁移 Jarvis，拆出明确的 internal Agent bootstrap helper，避免继续借用普通 `pre_install_apps` 类型；不得因此恢复 boot 阶段执行普通 App。
- [ ] 不为解决 Jarvis 依赖在 scheduler 新增更多特殊状态机。需要新的 Agent 安装编排时优先放到 Control Panel，并单独记录范围。

## 8. `buckyos-systest` 原地构造 PIKG

### P3.1 标准 PIKG 工程

- [ ] 在 `src/apps/sys_test/` 增加标准 `dapp_meta/app.json` 和 `dapp_meta/pikg.json`，格式与 `test/app_installer_test/pikg_samples/script-host` 一致。
- [ ] canonical identity 固定为：
  - AppDID：`did:bns:buckyos-systest.buckyos`
  - AppId：`buckyos-systest.buckyos.bns.did`
  - subpackage：`script`
  - service `www` inner port：`3000`
  - `allow_guest = true`
- [ ] script source 指向现场生成的 `src/apps/sys_test/dist`；PIKG 包含 backend、web assets 和运行所需 WebSDK 文件。
- [ ] 使用现有 `src/tools/buckyos-tool/buckyos pikg build/pack/info`，不为 systest 写第二套 tar/zip/AppDoc/PackageMeta 生成器。

### P3.2 构建输出

- [ ] `src/apps/sys_test/build.mjs` 在生成 `dist` 后触发 PIKG build、pack 和 info self-check。
- [ ] 最终 PIKG 通过临时文件 + rename/copy 原子落到 `src/rootfs/data/cache/<stable-name>.pikg`；失败不得留下半包或覆盖上一次成功包。
- [ ] `dapp_dist/` 和 rootfs cache 中生成的 `.pikg` 加入 ignore 规则，同时保留 `src/rootfs/data/cache/readme.md`。
- [ ] 构建输出提供生成/验证 install plan 所需的 AppDoc ObjectId、PackageMeta ObjectId、payload digest/size 和 PIKG digest；配置中不得维护伪造 identity。
- [ ] 当前 `buckyos-tool pikg build` 会把当前时间写入 AppDoc/PackageMeta，ObjectId 和 PIKG digest 会随重建变化。plan 必须由同一次构建结果生成/刷新，或先给工具增加 reproducible timestamp 输入；禁止长期硬编码一次构建的 digest/ObjectId。
- [ ] 核对 `bucky_project.yaml` build 顺序和 data path 复制语义，保证 fresh `start.py --all` 前 PIKG 已存在于 rootfs，安装后位于 `$BUCKYOS_ROOT/data/cache`。

### P3.3 清理旧旁路与身份漂移

- [ ] 预装安装不再依赖 `rootfs/bin/buckyos_systest` 作为 package source；确认它是否仍需作为调试产物，无其它消费者时再从 `bucky_project.yaml` modules 删除。
- [ ] 审计并更新/删除 `src/rootfs/local/did_docs/systest.buckyos.bns.did.doc.json`，不能保留与 canonical systest DID 冲突的文档。
- [ ] 联动检查 `test/test_control_panel/test_app_mgr.ts`、`test/test_control_panel/test_local_user.ts`、Desktop launch mapping 和所有硬编码旧 `buckyos_systest` / `systest.buckyos` 的断言。

### P3.4 其它预装项 Gate

- [ ] boot template 当前还包含 `buckyos-filebrowser.buckyos.bns.did`，但 rootfs cache 中没有与其 plan 精确绑定的真实 PIKG。切换 schema 时必须二选一：接入真实、可验证的 filebrowser PIKG，或者暂时从 `pre_install_apps` 删除该项。
- [ ] 不允许为保持 filebrowser “看起来已预装”而保留假 ObjectId、空 `required_contents`、Catalog source 或绕过 PIKG materialize 的例外。

## 9. 测试与验收

### P4.1 单元/集成测试

- [ ] `PreInstallAppConfig` 接受新 schema，拒绝旧 AppDoc/ServiceSpecConfig、unknown fields 和不安全路径。
- [ ] `create_init_list_by_template()` 完成后 AppRegistry 为空，没有普通 AppSpec、InstallRecord 或 scheduler preinstall execution record。
- [ ] boot 流程面对故意缺失的 PIKG 仍可完成 kernel schedule；Scheduler 没有读取该文件。
- [ ] Control Panel 登录并启动后才创建 systest Task；request 使用 LocalPikg、SystemInternal、auto-confirm 和稳定 idempotency key。
- [ ] submitted plan 必须经过重新 Inspect；fingerprint 或任何 PIKG identity 不匹配时拒绝创建/推进部署。
- [ ] reconciler 重放不会创建重复 Task；Control Panel 在 Resolve/Verify/Prepare/Deploy/Activate 任一阶段重启后由现有 InstallRunner 恢复。
- [ ] PIKG 缺失/损坏或 NamedStore 暂不可用不会终止 Control Panel，且留下可诊断的 structured error。
- [ ] first install、Satisfied replay 和 rootfs 新 PIKG upgrade 三条路径都覆盖。
- [ ] Scheduler 测试只验证 boot 不再执行预装、Control Panel 提交的标准 plan 仍能正常 commit；不新增 scheduler PIKG/preinstall 测试夹具。

### P4.2 构建检查

在 `src/` 下至少执行：

```bash
cargo test -p buckyos-api -p scheduler -p control_panel
uv run buckyos-build.py -s buckyos_systest
uv run buckyos-build.py
```

并在工具目录执行 PIKG 自检：

```bash
cd tools/buckyos-tool
deno task test
./buckyos pikg info ../../rootfs/data/cache/<systest-pikg-name>.pikg
```

验收构建结果：

- [ ] rootfs cache 中只有完整且 `pikg info` 通过的 systest PIKG。
- [ ] PIKG 内 AppDID/AppId、install plan、boot template map key 完全一致。
- [ ] plan 中没有伪造 ObjectId，`required_contents` 非空且对应 PIKG payload。
- [ ] `git diff --check` 通过，生成物不会污染待提交文件列表。

### P4.3 Fresh-install E2E

在根目录使用全新 DV 环境验证：

```bash
uv run src/start.py --all
uv run src/check.py
```

必须观察并断言：

- [ ] `scheduler --boot` 先成功结束，日志中没有预装 PIKG/InstallPlan 执行。
- [ ] Control Panel 成功登录并开始服务后，才出现 preinstall reconcile、PIKG staging/verify 和 App install Task 日志。
- [ ] TaskManager 中 systest 安装任务经过标准 Stage 并完成；scheduler 只看到 Control Panel 提交的普通 execution record。
- [ ] `users/<admin>/apps/buckyos-systest.buckyos.bns.did/spec`、InstallRecord、AppRegistry allocation 和目标 NodeConfig projection identity 一致。
- [ ] NamedStore 可按 plan 读取 AppDoc、PackageMeta 和 payload；不依赖旧 `rootfs/bin/buckyos_systest` 旁路。
- [ ] node-daemon 启动 systest，service info/instance evidence 收敛，浏览器入口及 `/sdk/appservice/selftest` 可用。
- [ ] 重启 Control Panel/BuckyOS 不重复安装、不重新分配 AppIndex/hostname、不重复物化内容。
- [ ] 删除/损坏 rootfs cache PIKG 的负例只让对应 preinstall task 失败；Control Panel、Scheduler 和其它系统服务仍正常工作。

现有控制面和 App Installer DV 测试也要回归：

```bash
uv run test/run.py --list
cd test/app_installer_test
pnpm test
```

根据 `--list` 的实际名称执行 app manager/local user/systest 相关 DV case，不在本文猜测 case 名。

## 10. 文档联动

- [ ] 更新 `doc/App 安装协议.md`：bootstrap 只保存预装 seed，Control Panel 通过标准 Installer 执行，Scheduler 只消费最终 plan。
- [ ] 更新 `doc/arch/system_config_reference.md` 的 `system/install_settings`、首次初始化和 Control Panel 消费方说明。
- [ ] 更新 `doc/arch/10_user_lifecycle_and_permissions.md` 中旧 AppId 和默认 App 行为。
- [ ] 更新 `doc/app_service/BuckyOS App基础设施职责边界.md`：rootfs preinstall 是 Control Panel 的 system-internal Installer source，不是 Scheduler 特例。
- [ ] 协议、共享类型、rootfs template、Control Panel、Scheduler 边界和测试 fixture 同步更新。

## 11. 建议提交顺序

1. 冻结 `PreInstallAppConfig` 以及 plan seed/final `InstallPlan` 边界，补 schema/path 测试。
2. 移除 scheduler boot/builder 中普通预装 plan 的生成和执行，证明 boot 不碰 PIKG。
3. 在 Control Panel 增加内部 rootfs staging 和 PreInstallReconciler，复用 InstallEngine/InstallRunner/RPC submit 公共逻辑。
4. 为 systest 增加 PIKG 工程与 rootfs cache 构建输出，收敛 canonical identity。
5. 切换 boot template，处理 filebrowser 和 Jarvis Gate。
6. 补 restart/negative/fresh-install E2E 并更新文档。

每一步保持 `cargo test` 和 `buckyos-build` 可过；不要先删除旧 sys_test 产物，再等待后续提交补 PIKG。

## 12. 完成定义

- boot 调用图中没有普通 App 安装；坏预装包不会破坏系统 boot。
- `pre_install_apps` 只表达受控 PIKG location 和明确的 install plan/seed，不再内嵌 AppServiceSpec 或伪造 package identity。
- Control Panel 是预装任务的唯一执行/恢复主体，并完整复用普通 App Installer 与 TaskManager。
- Scheduler 没有新增 PIKG、rootfs path、preinstall scan、TaskManager 或专用重试逻辑，只接收最终 InstallPlan 并完成既有调度职责。
- systest PIKG 由当前源码现场构造、位于 rootfs data cache、可离线验证和物化，并通过 fresh-install E2E。
- 文档、协议、共享类型、测试和 canonical App identity 已联动更新；没有新增第三方依赖或兼容层。
