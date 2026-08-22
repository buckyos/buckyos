# App Installer v0.5 实现 TODO

> 面向下一步 CodeAgent。
>
> 设计真相源：`doc/App 安装协议.md`（Draft v0.5，2026-07-16）。
> 当前实现入口：`src/frame/control_panel/src/app_installer.rs`。
> 本文只规划安装核心；应用商店、好友分享、支付、Web-to-Native 页面和完整桌面 UI 在核心接口稳定后另排。
>
> beta 2.2 是 breaking change：不要保留 `apps.install(app_id, version)` 的双轨兼容逻辑，不要给新字段加“兼容旧数据”的隐式默认值。

---

## 0. 目标完成态

完成本 TODO 后，Installer 至少具备以下闭环：

```text
identifier / local pikg handle
        ↓
Resolve (App DID, "app")
        ↓
Inspect -> InstallPlan -> WaitingForApproval
        ↓ confirm
Acquire missing content -> Verify
        ↓ Content Ready + Trust Ready
Prepare -> Deploy -> Activate -> Health Check
        ↓
InstallRecord + installed proof
```

必须同时满足：

- 支持逻辑入口 `install_app(identifier, referrer?, options?)` 和 `install_package(local_pikg_handle, options?)`。
- App Document body 无论来自 Repo、URL、NamedStore 还是 `pikg`，都只是 candidate；Deploy 前必须完成 `(App DID, doc_type = "app")` 解析和绑定。
- 将内容就绪、DID 信任就绪、配置就绪分开表达；“包内内容齐全”不能等价为“可安全离线安装”。
- Resolve / Inspect / Acquire / Verify / Prepare / Deploy / Activate 都有持久状态、独立错误码、可重试边界和幂等处理。
- 下载和验证结束前不得写 app spec、停止旧版本或触发 scheduler。
- installed proof 只能在 Activate 健康检查成功后生成。
- Control Panel 重启后可以从 TaskManager 中恢复非终态安装任务。

### 本轮明确不做

- 支付、HTTP 402、Receipt 和 DAO 激励结算。
- Curator UI、好友分享 UI、二维码、短码和 Inbox ActionObject。
- 浏览器 URL Scheme 唤起检测。
- WebUI PRD 和完整桌面安装 UI；WebUI PRD 必须等待本协议的共享状态与 RPC 语义定稿后再开始。
- 包级 Packager Signature（协议仍列为 Roadmap）；但数据结构应给未来签名留独立位置，不能把它混入 App Document owner 签名。
- 全生态所有平台内容完备性；只判断当前 InstallPlan 的目标完备性。

### WebUI PRD 的依赖边界

本 TODO 需要在 `buckyos-api/src/app_install.rs` 中定义可被未来 WebUI 稳定消费的**后端语义状态**，但不在这里决定页面结构、交互稿、展示文案或最终 WebUI RPC 形态。

WebUI PRD 开工前必须先冻结：

- App Document identity/schema、App DID 与 App Document Object ID 的关系；
- Resolve/Inspect/Acquire/Verify/Prepare/Deploy/Activate 的阶段和可重试边界；
- Content/Trust/Package/Config/Install readiness 的枚举与组合规则；
- Verification、warning、结构化 error 和 available action 的语义；
- `WaitingForApproval`、确认参数变化与 plan 失效规则；
- 新版本发现规则：以权威 Resolve 得到的 App Document Object ID 和 `document_version` 为准，而不是以 Repo 中更高 semver 为准；
- `Revoked/Tombstoned/Missing/Unknown/Migrated/LocalAuthorityOverride` 在安装和升级界面中的硬约束。

在上述协议定稿前：

- 不新增依赖临时字段或 mock 状态的 WebUI PRD；
- 不让 WebUI 解析 TaskManager 的自由文本 `message` 或直接依赖内部 `Task.progress` 布局；
- 不把 Repo 中发现的候选版本直接展示成“可信新版本”；
- 可以使用后端测试页面/CLI 查看原始状态，但它不构成产品交互承诺。

协议定稿后另起 WebUI PRD，由其引用 `app_install.rs` 的 typed snapshot，并定义安装检查、权限确认、进度、失败恢复、离线就绪和版本升级页面。

---

## 1. 当前实现证据与差距

协议文档比 Installer 主实现新：协议在 2026-07-16 更新，`app_installer.rs` 最近一次提交在 2026-05-28。冲突时以 v0.5 协议为准。

| 当前代码 | 现状 | 与 v0.5 的差距 |
|---|---|---|
| `buckyos-api/src/app_doc.rs::AppDoc` | `PackageMeta` flatten + `pkg_list`；没有必填 `id` / `doc_type` | 无法执行 `document.id == app_did`，也无法把 App 语义版本和 DID 发布版本分开记录 |
| `ControlPanelServer::handle_apps_install` | 参数为 `app_id + version`，从 `repo.list()` 选择 semver 最新记录 | Repo/应用商店取得的 body 被直接信任；没有 `(did, "app")` 解析 |
| `AppInstaller::run_install_task` | 下载/pin、写 spec、记 proof、等实例串在一个函数内 | 无 InstallPlan、无确认点、无 Stage 输出、无恢复入口 |
| `ensure_content_pinned` | 要求根 content 已被 RepoService collect；只围绕一个 root content_id | 本地 `pikg` 不能脱离 RepoService 安装；内容来源和内容身份混在一起 |
| `task_manager/download_executor.rs` | 识别 AppDoc 后递归下载 `pkg_list` 中全部 subpackage | 未按目标 Node/平台/参数选择内容，不支持部分包的精确 missing list |
| `run_install_task` | 写 spec 后立即 `repo.add_proof(installed)`，之后才等实例 ready | proof 时机错误；部署/启动失败也可能已经留下 installed proof |
| `run_upgrade_task` | 先 `stop_app()`，之后才检查/下载新内容 | 直接违反 Download Before Install；下载失败会造成无谓停机 |
| `should_wait_for_instance` | Static Web 不等待实例 | Web 安装没有统一 Activate/健康检查语义 |
| `wait_for_instance_ready` / `wait_for_instances_removed` | 每秒轮询 system-config | 没有使用 TaskManager/KEvent 的长任务模式；重启后循环消失 |
| `publish_app_to_repo` | 生成 tar.gz、PackageMeta 和最终 AppDoc NamedObject | 没有生成 `APPDOC.* + PACKAGE_META.json + payload` 的 `.pikg` |
| `test/app_installer_test` | `app.publish -> apps.install(app_id, version)`；Agent 跳过，ready 超时可接受 | 没有 DID 状态、离线包、坏包、恢复、回滚和 proof 时机测试 |

可直接复用的设施：

- `name-client::resolve_did_ex` 和 `DidDocType::Custom("app")`；`zone_did_resolver.rs` 已能输出 `documentStatus / documentVersion / effectiveOwner / authoritySeq / docHash`。
- TaskManager 2.0 持久化不可变 `Task.input`、可变 `Task.progress`、Result、组合状态和事件；Control Panel 直接执行并按自身 schema 恢复安装任务，不使用 runner inbox。
- NamedStore 的 Object ID 读取/写入和 TaskManager 对指定 ObjId 的下载能力。
- RepoService 的 collect/pin/proof 原语；它应作为内容来源和传播记录复用，但不能再成为所有安装的强制入口。
- Control Panel 已依赖 `zip`、`sha2`、`tar`、`flate2`，实现首版 `pikg` 不应新增 crate。

工作区注意：`src/frame/control_panel/src/pikg.rs` 当前是用户创建的 0 字节未跟踪文件，且尚未在 `main.rs` 注册 module。开始实现前先确认它仍为空且没有用户新内容，不得覆盖并行工作。

---

## 2. 不可妥协的实现规则

1. **App Document First**：RepoRecord、URL、Object ID、`pikg` 和分享对象都只提供 candidate/body/location。
2. **DID Resolver 与 Object Provider 分离**：`pikg` 只能提供 `object_id -> bytes`，不得注册 name-client provider，不得影响 App DID 发布状态。
3. **签字权与发布权分离**：owner 签名不能把 `Missing/unknown` 变成 `Active`；`Revoked/Tombstoned` 不能 fallback。
4. **expected_owner 不来自 candidate**：只能使用 resolver 权威绑定或 DID 名字结构的确定性结果。
5. **Missing 不等于 unknown**：错误码、重试性、UI action 和 cache 行为必须不同。
6. **Download Before Install**：Trust/Content/Config 未 ready 时禁止写 spec、停旧实例或执行部署。
7. **目标相关的部分包**：不得因为 `pikg` 没有其它平台内容就判坏；也不得下载未被当前计划选中的所有 subpackage。
8. **Load 与 Import 分离**：`PikgReader` 必须能事务内读取而不自动写 RepoService；是否写 NamedStore/缓存由 Prepare 策略决定。
9. **不接受任意服务端路径**：外部 RPC 的本地包输入必须是 upload/staging handle，不能让普通用户传 `/tmp/x.pikg` 或任意绝对路径。
10. **安装证明最后写**：只有 Activate + health check 成功后才写 proof 和最终 installed record。
11. **错误必须结构化**：禁止继续用 `ReasonError("...")` 文本承载协议状态。
12. **大文件操作不阻塞 async runtime**：ZIP 索引、tar 检查、hash 和同步文件 I/O 使用 `spawn_blocking` 或流式 async I/O。

---

## 3. 开工前必须冻结的协议决策

以下内容在 v0.5 §14 仍是 Roadmap。CodeAgent 不得静默自行发明永久格式。建议采用下列最小决策，并同步修改 `doc/App 安装协议.md` 后再进入实现。

### D1. `pikg` 外层编码

- **建议**：首版固定为 ZIP/ZIP64，MIME 为 `application/vnd.buckyos.pikg+zip`，复用 Control Panel 已有 `zip` 依赖。
- 必须固定：magic/version、ZIP64 要求、重复 entry 规则、最大 entry 数/metadata 大小、是否允许压缩 payload、整个 `pikg` digest 的计算对象。
- 如果 owner 不接受 ZIP/ZIP64，本轮只能先实现 `PikgSource` trait 和目录型测试 provider，不能声称 `.pikg` 文件协议已完成。

### D2. App Document schema 与 Object ID

- **建议**：复用并升级现有 `AppDoc`，增加必填 `id: DID`、`doc_type: "app"`，保留现有运行时需要的 `name/version/pkg_list/install_config_tips/permissions`，不要再造第二套同名 AppDoc。
- 必须固定 App Document 和 Package Meta 的 canonical JSON 算法，以及 App Document Object ID 的 obj type。不要直接对 `serde_json::to_string()` 结果做 hash。
- `version` 只表示 App 语义版本；`document_version/versionId` 只放 resolver snapshot，禁止复用一个字段。
- 这是共享类型 breaking change，必须同步所有 AppDoc builder、rootfs/preinstall fixture、scheduler/node-daemon/task-manager 反序列化和文档。

### D3. 安装记录真相源

- **建议**：in-flight transaction 的不可变请求存 TaskManager `Task.input`，可恢复快照存 `Task.progress`；长期记录单独存：

```text
users/{uid}/apps/{app_name}/install_record
users/{uid}/agents/{app_name}/install_record
```

- `AppServiceSpec` 继续只承载 scheduler/node-daemon 所需的部署 spec，避免把完整解析证据和任务历史复制到每个 instance config。
- 写入顺序：先写 `install_record(state=prepared)`，成功后才写 spec 触发 scheduler；Activate 成功后更新为 `installed`。失败/回滚也更新同一记录。

### D4. LOCAL_DEVELOPER authority override

- Zone Resolver 当前已有 `resolver/cache/{did}/{doc_type}/{state|doc}` 数据面，但没有面向 Installer 的、带 scope/warning/不可导出的完整管理 API。
- **建议**：先增加受 admin/local-auth 保护的 Zone Resolver cache-injection API，持久化 `scope`、warning、过期时间和 evidence；Installer 只消费 resolver 结果，不直接写 cache key。
- 在该 API 完成前，`LOCAL_DEVELOPER` 必须返回 `TRUST_RESOLUTION_REQUIRED`，不能用“本地文件”作为隐式信任。

### D5. 本地包交付边界

- **建议**：复用 Control Panel NDM upload，完成上传后返回不可猜测的 staging handle；Installer 根据 handle 在 runtime cache/staging root 下定位 immutable 文件。
- 只有进程内测试/系统内部调用可以接收 `Path`；对外 `apps.install_package` 不暴露服务端 path。

---

## 4. 建议代码结构与修改范围

### 必改入口

- `src/kernel/buckyos-api/src/app_doc.rs`
  - 升级 App Document identity/package selector schema。
- `src/kernel/buckyos-api/src/app_install.rs`（新增）
  - 放共享的 Stage、readiness、plan、resolver snapshot、error、record 类型。
- `src/kernel/buckyos-api/src/taskdata.rs`
  - 把 `AppInstallTaskData/AppUpdateTaskData` 升级为可恢复 transaction 数据。
- `src/kernel/buckyos-api/src/lib.rs`
  - 导出新共享类型。
- `src/frame/control_panel/src/pikg.rs`
  - `pikg` 读取、索引、校验、事务内 Object Provider；不要掺部署逻辑。
- `src/frame/control_panel/src/app_installer.rs`
  - identifier normalization、resolver adapter、planner、stage orchestrator、install record、proof、upgrade/rollback。
- `src/frame/control_panel/src/main.rs`
  - 注册 `pikg` module、新 RPC、安装 runner/recovery loop。
- `src/kernel/task_manager/src/download_executor.rs`
  - 复用指定 ObjId 下载；移除 Installer 对“下载完整 AppDoc 全部 subpackage”的依赖，必要时补批量 missing object 子任务。
- `src/kernel/sys_config_service/src/zone_did_resolver.rs`
  - 仅在 D4 采用 Zone 级开发 override 时补受控写 API；不要为 App 动态注册 resolver-provider。

### 共享类型变更后必须联动检查

- `src/kernel/scheduler/src/{app.rs,system_config_agent.rs,system_config_builder.rs}`
- `src/kernel/node_daemon/src/{app_loader.rs,node_daemon.rs,test_app_loader.rs}`
- `src/kernel/buckyos-api/src/runtime.rs`（当前会按 AppDoc 递归对象）
- `src/tools/buckycli/src/app.rs`
- rootfs/preinstall 中所有 AppDoc JSON。

### 文档和测试

- `doc/App 安装协议.md`（只补已冻结的 D1-D5，不改设计方向）
- `doc/app_service/BuckyOS App安装流程.md`
- `doc/control_panel/Control_Panel_Service.md`
- `doc/arch/system_config_reference.md`（新增 install_record/cache key 时）
- `test/app_installer_test/**`

不要在 `app_installer.rs` 继续堆所有格式和安全校验。至少保持 `pikg` 独立；若 resolver/planner 继续膨胀，再拆内部 module，但不要先做无业务价值的大规模搬文件。

---

## 5. 共享数据模型 TODO

### P0.1 App Document identity

- [ ] `AppDoc` 增加必填 `id: DID` 和固定 `doc_type`；deserialize 时拒绝缺失/非 `app`。
- [ ] builder 必须显式接收或按已冻结规则构造 App DID；禁止从 candidate 的 `owner` 临时拼一个 DID 后当权威身份。
- [ ] package 描述能表达 `name / package_meta ObjId / selector(os, arch, kernel...) / required`；不要继续只靠字段名和编译期 `cfg!()` 选择。
- [ ] 保留运行时需要的 `pkg_id/docker_image_name/source_url` 信息，但 Source 与 Object ID 明确分字段。
- [ ] 单测覆盖 `id/doc_type/version/document_version` 不混淆。

### P0.2 Installer 公共类型

在 `app_install.rs` 至少定义：

- [ ] `InstallSource = Identifier | LocalPikgHandle`。
- [ ] `InstallPolicy = StrictPublic | Normal | TrustedShare | LocalDeveloper | SystemInternal`。
- [ ] `InstallStage = Resolve | Inspect | Acquire | Verify | Prepare | Deploy | Activate`。
- [ ] `DocumentStatus = Active | Missing | Expired | Revoked | Tombstoned | Migrated | Unknown`。
- [ ] `DidResolutionSnapshot`：包含 app_did、doc_type、app_doc_object_id、resolver_id、document_status/version、effective/expected owner、evidence、verification_status、cache_status、authority_seq、warnings、migration_target。
- [ ] `InstallTarget`：node DID/name、OS、arch、kernel/runtime 版本。
- [ ] `InstallPlan`：selected package meta、required content、local/pikg/missing 集合、readiness、permissions、params、plan fingerprint。
- [ ] `VerificationReport`：逐项结果，不能只有 bool。
- [ ] `InstallError { stage, code, retryable, message, action, details }`。
- [ ] `InstallRecord`：DID snapshot、实际 app doc id、package meta ids、pikg digest、target、状态、时间、task id、proof id。
- [ ] `AppInstallStatusSnapshot`：面向 SDK/WebUI 的只读 typed snapshot，聚合 stage、readiness、verification summary、progress、approval、warnings、error、available actions 和 updated_at；不能要求消费方解析 Task.progress。
- [ ] `AppUpdateAvailability`：记录 installed/resolved App Document Object ID、发布版本、语义版本、trust、permission diff、target compatibility 和 update state。

`AppUpdateAvailability.state` 至少区分：

```text
UP_TO_DATE
UPDATE_AVAILABLE
INCOMPATIBLE_TARGET
PERMISSION_RECONFIRM_REQUIRED
TRUST_RESOLUTION_REQUIRED
IDENTITY_REVOKED
UNKNOWN
```

这些类型先固定后端语义；字段定稿后再由 WebUI PRD 决定展示和交互，不在本 TODO 中反向根据临时页面设计修改协议状态。

错误码至少覆盖协议状态：

```text
CONTENT_DOWNLOAD_REQUIRED
TRUST_RESOLUTION_REQUIRED
IDENTITY_REVOKED
UNSUPPORTED_TARGET
INVALID_PACKAGE
CONFIG_BLOCKED
ACQUISITION_FAILED
VERIFICATION_FAILED
DEPLOY_FAILED
ACTIVATION_FAILED
CANCELED
```

`Missing/Expired/Unknown` 的具体 code/action 由 policy 决定，但三者不能序列化成同一个状态。

### P0.3 可恢复 TaskData

- [ ] `AppInstallTaskRequest` 保存原始 source/options/referrer/user，而不是只保存 app_id/version/content_id。
- [ ] `AppInstallTaskData` 保存 current stage、completed stages、candidate handle、resolver snapshot、plan、verification report、prepared deployment、last error/result。
- [ ] 每个 Stage 成功后先完整写 Task.progress，再开始下一 Stage；重启恢复只相信持久快照。
- [ ] plan fingerprint 至少绑定 app_doc_object_id、resolver document_version/status、target、影响 selector 的参数和 selected meta ids；任一变化必须重新 Inspect。
- [ ] 不做旧 `AppInstallTaskRequest` schema 的 legacy parser。

---

## 6. `pikg` TODO

### P1.1 Reader 与安全边界

- [ ] 只通过 staging handle 打开 immutable 文件；验证 handle 解析后的 canonical path 位于 staging root。
- [ ] 先验证 magic/version，再读取中央索引；扩展名只用于 UX，不能作为格式判断。
- [ ] 拒绝绝对路径、`..`、反斜杠绕过、重复 entry、NUL、非法 UTF-8、目录/文件类型冲突和 symlink entry。
- [ ] 不 `extract_all`；按 entry 流式读取到事务目录或直接 hash。
- [ ] 限制 entry 数、APPDOC/PACKAGE_META metadata 大小、单 entry 解压大小和总解压大小，防止 zip bomb。
- [ ] 校验后复制/rename 到以 `pikg_digest` 命名的 immutable staging 文件，后续 Stage 不重新打开用户可替换的原路径，避免 TOCTOU。

### P1.2 APPDOC 与 PACKAGE_META

- [ ] 至少存在 `APPDOC.jwt` 或 `APPDOC.json`。
- [ ] 两者同时存在时，用冻结后的 canonical 表达比较一致性；不允许“字段看起来差不多”即通过。
- [ ] 包内 AppDoc 永远标为 candidate，不在 Reader 中赋予 `Active`。
- [ ] `PACKAGE_META.json.@schema == buckyos.pikg.package-meta.v1`。
- [ ] `app_doc_id` 与实际 AppDoc Object ID 一致。
- [ ] `package_objects` key 与 value 重新计算的 Package Meta ObjId 一致。
- [ ] `content_index` 只能指向真实 entry；path/format/size/digest/sub_pkg_name 必须与 Package Meta 同时一致。
- [ ] 首选归档名必须是 `$sub_pkg_name.tar.gz`；sub_pkg_name 仅允许 `[A-Za-z0-9._-]+`，显式拒绝 `..`。
- [ ] SHA-256 对最终 `.tar.gz` 压缩字节计算，不对解压目录或 tar 中间流计算。
- [ ] 包含未被当前 target 选择的内容合法；引用了但当前 target 缺失的内容进入 missing list，不把整个包判坏。

### P1.3 Object Provider

- [ ] 定义最小 `ObjectProvider`/`ContentProvider` trait，返回 verified bytes/reader + provenance。
- [ ] provider 顺序：已安装/NamedStore -> 当前 pikg -> 远程 Source；offline mode 到 pikg 后必须停止，不能偷偷联网。
- [ ] `load_from_pikg()` 不自动调用 RepoService `store/pin`。
- [ ] Prepare 如果为了现有 PackageEnv/node-daemon 必须把已验证对象写 NamedStore，应把这个动作封装为显式 `materialize_for_deploy()`，不能反向改变 Resolve/Verify 结论。

### P1.4 单测 fixture

不要手工为每个用例拼二进制。新增测试 builder，能生成正常包后定点破坏：path、size、digest、ObjId、双 APPDOC、缺 entry、重复 entry。

---

## 7. Resolve 与 Inspect TODO

### P2.1 identifier normalization

- [ ] DID/name：直接得到 App DID，再调用 `resolve_did_ex(app_did, Custom("app"), policy)`。
- [ ] Object ID/URL/`pikg`：只做无安装副作用的最小 Acquisition，读取 candidate 并提取 App DID；可信 Resolve 完成前不得构造 spec。
- [ ] RepoRecord 只作为 candidate/source；删除 `resolve_repo_app_release()` 中“semver 最新即可信版本”的安装决策。历史版本必须由固定 ObjId/发布集合能力明确选择。
- [ ] `did:key` / `did:dev` 等 key DID 不能作为 App resolve 输入。

### P2.2 name-client adapter

- [ ] 为生产实现包一层内部 `AppDidResolver` trait，单测使用 fake；不要在 Installer 测试里初始化全局真实 resolver。
- [ ] 生产 adapter 复用 pinned `name-client::resolve_did_ex` 的验证结果，不在 Control Panel 手写第二套 owner JWT 验签。
- [ ] 映射并保留 `resolution_metadata / document / document_metadata`，不得只取 body。
- [ ] 所有 body 验证 `document.id == input app_did`。
- [ ] Anchored body 校验权威 `doc_hash`；NeedProof candidate 校验 expected_owner 和 name-client 返回的验证状态。
- [ ] `Revoked/Tombstoned` 立即产生不可重试 `IDENTITY_REVOKED`，清除/屏蔽正候选，不允许包内/Repo/旧 cache fallback。
- [ ] `Migrated` 只跟随 resolver 的 migration_target，记录用户可见 warning。
- [ ] `Unknown` 在有未作废 verified cache 时按 policy 使用；否则进入 `WAITING_FOR_TRUST_RESOLUTION`，不能伪装 `Missing`。
- [ ] LOCAL_DEVELOPER 只接受 D4 的 scoped `LocalAuthorityOverride` 结果，并把 scope/warning 写入 plan/record/proof details。

### P2.3 InstallPlan

- [ ] target OS/arch 来自用户选中的目标 Node 信息，禁止用 Control Panel 编译期 `cfg!(target_*)` 代替。
- [ ] 根据 AppDoc selector、required flag、目标版本和影响 selector 的安装参数选 package。
- [ ] 对每个 Package Meta/内容计算 location：installed、named_store、pikg、missing。
- [ ] 独立计算：Document Syntax Validity、DID Trust Readiness、Package Integrity、Content Readiness、Config Readiness、Install Readiness。
- [ ] 输出 permissions summary、缺失对象、预计下载量和 Source，不在 Inspect 时写系统目录。
- [ ] plan 进入 Task.progress 后把任务置为 `WaitingForApproval`；用户修改 target/params 后重新计算 plan，禁止在旧 plan 上 patch selected package。

---

## 8. Acquire 与 Verify TODO

### P3.1 Acquire

- [ ] 只下载 plan.missing 中的对象；已安装、NamedStore 和 pikg 命中必须复用。
- [ ] 每个远程获取使用 TaskManager child download task，继承 parent/root id。
- [ ] 等待 child task 使用 `wait_for_task_end_kevent()`；不要继续调用纯轮询 `wait_for_task_end()`。
- [ ] Source 失败可换 Source，但所有 Source 最后按同一 Object ID/Digest 验证。
- [ ] 明确 offline 时不得创建 download task。
- [ ] Repo collect/pin 只在确实使用 Repo 内容传播记录时调用；完整本地 `pikg` 安装不能要求 RepoService 已有 record。
- [ ] Acquisition 完成后原子更新 plan 的 content locations 和 task stage；下载失败只标 Acquire 失败。

### P3.2 Verify

- [ ] Resolve 的发布状态和 expected_owner 验证结果不可被包签名覆盖。
- [ ] 重新读取/验证所有 selected Package Meta ObjId 和内容 digest，不能相信 Inspect 缓存的 bool。
- [ ] 验证 target/runtime/kernel 约束、权限声明、路径和宿主 mount 安全。
- [ ] 验证时固定已校验内容句柄；Prepare 不能从原 URL/用户路径重新取一份不同内容。
- [ ] 输出逐对象 `VerificationReport`，失败清楚指出 object/path/check。
- [ ] 只有 Trust + Content + Config 全部 ready 才进入 Prepare。

---

## 9. Prepare / Deploy / Activate TODO

### P4.1 Prepare

- [ ] 从 confirmed InstallPlan 构造 `AppServiceSpec`；不再由 `build_default_install_config()` 静默替用户决定全部端口、mount 和权限。
- [ ] 为现有 node-daemon/PackageEnv materialize selected objects；不要 materialize 未选择平台。
- [ ] 检查端口/域名冲突、目标 Node 可用性、目录权限和资源条件。
- [ ] app_index 分配需要解决并发竞态；当前“扫描 max + 1”在两个并发安装下会重复。优先使用 system-config CAS/revision 或专用序列 key，不加进程内假锁冒充分布式正确性。
- [ ] 先写 `install_record(state=prepared)`，再写 spec；重复执行时按 task_id/plan fingerprint 幂等。

### P4.2 Deploy

- [ ] 写 spec 是 Deploy 的开始点，不得提前。
- [ ] task/record 保存旧 spec、新 spec、spec path 和已物化内容，支持失败清理。
- [ ] 同一 user + App DID 只允许一个 Deploy/Upgrade transaction；重复请求返回现有 task 或结构化冲突。
- [ ] 取消只在安全边界生效：Prepare 前可直接取消；spec 已写后必须走回滚/停止收敛再标 canceled。

### P4.3 Activate 与健康检查

- [ ] AppService/Agent：等待明确 Started + health evidence；只看到 spec 已写不算成功。
- [ ] Static Web：补统一部署完成/可读取的状态证据，不能继续直接跳过检查。
- [ ] 等待 instance/deployment 状态优先增加 KEvent；若当前 system-config 没有对应事件，可保留低频 timeout sweep 作为 backstop，但主状态机不能只存在于内存轮询中。
- [ ] 区分 `deployed_but_activation_failed` 与 `installed`。
- [ ] 健康检查成功后才：更新 InstallRecord=installed -> 写 installed proof -> Task Completed。
- [ ] proof details 增加 app_did、doc_type、app_doc_id、DID resolution snapshot、package meta ids、pikg digest、target；本地 override 不得显示为 Anchored。

### P4.4 Upgrade 与回滚

- [ ] 重写 `run_upgrade_task` 顺序：Resolve -> Inspect -> Acquire -> Verify -> Prepare 完成后，才停止/切换旧版本。
- [ ] 新版本 Activate 成功前保留旧 spec 和回滚材料。
- [ ] 新版本失败时恢复旧 spec/运行状态；这只是本机部署回滚，绝不能把 resolver 已撤销的旧 AppDoc重新标为当前可信。
- [ ] 权限、target 或 selector 参数变化必须重新确认。
- [ ] 升级 proof 仍只在新版本健康检查成功后生成。

---

## 10. RPC、任务 runner 与恢复 TODO

### P5.1 RPC 面

下列是安装协议所需的写操作草案。面向 WebUI 的 `status/list/update-check` 等只读 RPC，应在 `AppInstallStatusSnapshot` 和 `AppUpdateAvailability` 定稿后随 WebUI PRD 一起固定；在此之前可以提供内部调试入口，但不能让前端依赖临时 JSON。

建议新协议：

```text
apps.install
  { identifier, referrer?, options? } -> { task_id }

apps.install_package
  { staging_handle, options? } -> { task_id }

apps.install.confirm
  { task_id, target, install_params, accepted_permissions } -> { task_id }

apps.install.retry
  { task_id } -> { task_id }

apps.install.cancel
  { task_id } -> { task_id }
```

- [ ] `apps.install` 删除旧 `app_id/version` 参数语义，不保留分支兼容。
- [ ] 初始调用跑到 Inspect 后进入 `WaitingForApproval`；confirm 后从持久 plan 继续。
- [ ] `SYSTEM_INTERNAL` 可显式 auto-confirm；NORMAL/STRICT_PUBLIC 不默认跳过权限确认。
- [ ] 所有 RPC 做 principal/user scope 校验；不能确认、重试、取消他人的 task。
- [ ] `apps.install_package` 只收 staging handle。

### P5.2 runner/recovery

- [ ] 安装任务使用 `SelfApp` executor，由 Control Panel 已鉴权业务接口创建并直接执行。
- [ ] Control Panel 启动时列出自身 runner 下 Pending/Running 的 app.install/app.update Task，按 TaskManager 持久快照恢复；WaitingForApproval/Paused 等待显式业务操作。
- [ ] 不为同进程执行另建 MsgQueue 或 runner inbox；TaskManager 是唯一持久真相源，低频 sweep 只修复异常遗漏。
- [ ] `apps.install.confirm/retry` 更新状态后显式启动本地执行体。
- [ ] Stage handler 必须可重复调用：已完成输出存在且 fingerprint 一致则跳过；不一致则从最早失效 Stage 重算。
- [ ] 服务关闭时不把正常未完成任务标 Failed；重启后继续。

---

## 11. 发布侧最小联动

安装链路跑通后再改 `app.publish`，不要把 publish 和 install 同时塞进第一个提交。

- [ ] `app.publish` 生成带 `id/doc_type` 的 AppDoc 和目标 Package Meta。
- [ ] 生成 `$sub_pkg_name.tar.gz`；当前 docker 逻辑把 `amd64_docker_image.tar` 再包进另一个 tar.gz 且内部名为 `$app_id.tar`，需按冻结后的 Package Meta/content_index 规则统一。
- [ ] 输出本地 `.pikg` 并用同一个 `PikgReader` 自校验，禁止 packer/verifier 两套规则漂移。
- [ ] 发布到 Repo/NamedStore 和发布 App DID 是两个动作；`repo.store/announce` 不能替代 `(App DID, "app")` 权威发布。
- [ ] `app.publish` 返回 app_did、app_doc_id、pikg handle/path、pikg digest 和发布状态，不只返回一个 `obj_id`。
- [ ] 本地开发未发布时明确返回 candidate/local override 状态。

---

## 12. 测试矩阵

### 单元测试（必须先于 DV）

- [ ] AppDoc：缺 id、错误 doc_type、App DID/body id 不一致。
- [ ] Resolver：Active/Anchored、NeedProof verified、Missing、Expired、Unknown、有 verified cache、Revoked、Tombstoned、Migrated、LocalAuthorityOverride。
- [ ] Resolver：candidate 自声明 owner 与 expected_owner 不同必须拒绝。
- [ ] pikg：正常单平台部分包、APPDOC 双文件一致/不一致、Package Meta ObjId 错、digest 错、size 错、missing entry、重复 entry。
- [ ] pikg 安全：`../`、绝对路径、反斜杠穿越、symlink、zip bomb 限额、TOCTOU 替换。
- [ ] Planner：目标 Node 与 Control Panel 不同架构时选择目标架构，不使用编译期 cfg。
- [ ] Readiness：Content Ready + Trust Missing = TRUST_RESOLUTION_REQUIRED；Trust Ready + Content Missing = CONTENT_DOWNLOAD_REQUIRED。
- [ ] Offline：provider fake 断言零网络调用。
- [ ] State machine：每个 Stage 失败/重试/取消；在任意 Stage 模拟重启后恢复。
- [ ] 幂等：重复 confirm/retry 不重复写 spec、下载、proof。
- [ ] 并发：同一用户同一 App 冲突；两个不同 App 的 app_index 不重复。
- [ ] proof：Activate 失败时没有 installed proof；成功时只写一次。
- [ ] upgrade：Verify 失败时旧实例仍运行；Activate 失败时恢复旧 spec。

为此需要把 AppInstaller 对 global runtime 的直接访问包在小 trait/adapter 后面，使用 in-process fake 测试 SystemConfig、TaskManager、Repo、NamedStore 和 DID Resolver。不要让单元测试依赖真实 Zone。

### 集成测试

更新 `test/app_installer_test`：

- [ ] fixture 先 pack `.pikg`，再走 `apps.install_package`。
- [ ] 增 DID/owner cache fixture，分别验证 OFFLINE_READY 和 TRUST_RESOLUTION_REQUIRED。
- [ ] Static Web、Docker、Agent 三类都不能长期 skip；若环境缺 Docker只跳 Docker。
- [ ] 不再接受“Task 因等待 ready 超时也算通过”。
- [ ] 验证 install_record、Task stage/result、spec、实际运行状态和 proof 顺序。
- [ ] 增 Control Panel 中途重启恢复用例。
- [ ] 增本地 `pikg` 安装时 RepoService 无记录仍能成功的用例。

### 验证命令

```bash
cd src
cargo fmt --check
cargo test -p buckyos-api -p control_panel -p task_manager -p repo_service -p scheduler -p node_daemon
cargo test
uv run buckyos-build.py --skip-web

cd ..
cd test/app_installer_test
pnpm install
pnpm test
```

需要 DV 环境的用例再从仓库根目录执行：

```bash
uv run src/check.py
uv run test/run.py --list
uv run test/run.py -p <app-installer-dv-case>
```

---

## 13. 建议提交顺序

每一步独立可编译、可测，不做一个覆盖几十个文件且无法定位回归的巨型提交。

1. **协议决策 + shared types**：D1-D5、AppDoc/app_install/taskdata，修完所有编译联动。
2. **pikg reader/verifier**：纯本地单测，不接部署。
3. **resolver + planner**：fake resolver/provider 下生成 InstallPlan/readiness。
4. **持久 task state machine + RPC confirm/recovery**：先用 fake deployer 跑全 Stage。
5. **Acquire/Verify 接现有 TaskManager/NamedStore/Repo**。
6. **Prepare/Deploy/Activate 接 scheduler/node-daemon，修 proof 和 upgrade 顺序**。
7. **publish 生成 pikg + 集成测试 + 文档**。
8. **全 workspace test/build + DV**。

每个提交结束必须回答：改了什么、为何这样改、跑了什么验证、剩余风险；共享类型/协议/数据路径变更要显式列出所有消费者。

---

## 14. 已知风险与停止条件

### 已知风险

- v0.5 仍是 Draft，D1/D2 不冻结就无法产生可互操作的 `.pikg` 和稳定 ObjId。
- `name-client` 是 git 依赖；实现前需以当前 lock/branch API 校准 `resolve_did_ex` 字段，Installer 内只做 adapter，不复制上游验证算法。
- 当前 node-daemon/PackageEnv 主要从 NamedStore 加载。首版 materialize 策略必须说明它是部署适配，不得把“写 RepoService”误设为安全前提。
- Static Web 当前缺统一 instance ready 语义，需要 scheduler/node-daemon 补可观察状态，否则 Activate 无法严格完成。
- system-config 目前没有显式跨 key 事务；install_record + spec 的崩溃恢复必须靠写入顺序和幂等 reconciliation 保证。
- 当前 app_index 分配有并发竞态，不能把它留到协议迁移之后再修。

### CodeAgent 必须停止并请求确认的情况

- D1/D2/D4 的选择与本文建议不同。
- 需要新增第三方 crate 或跨仓库修改 `name-client/package-lib/named_store` 才能继续。
- 无法在不暴露任意本地 path 的情况下完成 `install_package` 上传闭环。
- 为实现离线安装必须让 `pikg` 动态注册 DID resolver-provider。
- 为了“先跑通”准备把 Unknown 当 Missing、把 owner 自声明当 expected_owner、或在健康检查前写 installed proof。

---

## 完成记录（2026-07-17，CodeAgent）

按 §13 提交顺序落地为 beta2.2 上的 7 个提交（`c07e76a9..76c7c9af`，另有一个前置纯 fmt 提交 `083de86a`）。

### 冻结决策（已同步 `doc/App 安装协议.md` §14.0）

- **D1**：ZIP/ZIP64 + `@schema=buckyos.pikg.package-meta.v1` 承载格式版本；4096 entries / APPDOC 1MiB / metadata 单 8MiB 总 64MiB；digest 对 entry 解压后字节；`pikg_digest=sha256(整文件)`。
- **D2**：canonical = JCS（`build_named_object_by_json`）；**App Document obj type = `appdoc`**；AppDoc 必填 `id`+`doc_type`；SubPkgDesc 增 `selector{os,arch,min_kernel_version}`/`required`（省略=true），已知 key 派生表；App DID 派生规则 `did:bns:{name}.{owner_id}`（builder 默认，`.app_did()` 覆盖）。
- **D3**：`users/{uid}/apps|agents/{app}/install_record`；prepared → spec → installed → proof 顺序纪律。
- **D4**：本轮无 resolver 注入 API；LOCAL_DEVELOPER 无证据 → `TRUST_RESOLUTION_REQUIRED`；测试用 root 权限 KV 种 `resolver/cache/{did}/app/{state|doc}`。
- **D5**：staging handle = `pikg:sha256:<hex>`（staging root 下 immutable 文件）或 NDN chunk id；决不收服务端路径。

### 落地面（均已实现并测试）

- 共享类型：`buckyos-api/src/app_install.rs`（全部 P0.2 类型 + 错误码 + `to_full_patch` 显式 null 镜像）；taskdata 可恢复事务（**已删 legacy parser**）。
- control_panel 新模块：`pikg.rs`（含自有中央目录扫描——zip crate 会静默去重同名 entry）、`app_install_resolver/planner/engine/driver/deployer/runner`。
- RPC：`apps.install{identifier}`（旧 app_id/version 语义已删）、`apps.install_package{staging_handle}`、`apps.install.confirm/retry/cancel`、`apps.update` 走同一流水线；`app.publish` 产 `.pikg` 并同 Reader 自校验，返回 app_did/app_doc_id/pikg_handle/digest/app_doc/publish_status。
- app_index：`system/app_installer/app_index_seq` + `exec_tx` CAS（修扫描竞态）。
- runner：业务 RPC 直接启动执行体；TaskManager 启动扫描 + 60s sweep 恢复异常遗漏，不另建 MsgQueue/runner inbox。
- 升级：Prepare 完成才写新 spec；Activate 失败自动回滚旧 spec 并作废 Deploy 供 retry 重写。

### 验证结果

- `cargo fmt --check` 干净；全 workspace `cargo test` **1872 通过 / 0 失败**（引擎 fake 全流程、恢复、幂等、取消边界、冲突、升级回滚等 46 个新单测）。
- `uv run buckyos-build.py --skip-web` 成功并部署。
- 集成测试（本机 macOS dev zone）：**static web 用例全链路通过**（publish→pikg→种证据→install_package→WaitingForApproval→confirm→…→Activate→install_record=installed+proof 回填）；agent 用例已解除 skip，管线一路通到容器启动，本机被 Docker Desktop 文件共享（`/opt/buckyos` 不在共享列表）挡住——属机器配置，README 已记录；docker 用例本机镜像拉取被网络阻断（`BUCKYOS_TEST_SKIP_DOCKER=1` 跳过）。

### 顺带修复 / 迁移

- **node_daemon 既有 bug**：`ensure_pkg_meta_indexed` 静默改名内容寻址 meta → 完整性校验必败（此前被 agent 用例长期 skip 掩盖）；已改为显式报错，fixture pkg 名改带点（`e2e.{app}-agent`）符合 PackageEnv 前缀契约。
- **存量数据迁移**：旧 zone 的持久 AppDoc（node config/specs/kernel service specs/install_settings）需一次性补 `id`/`doc_type`（纯增量）；本机 dev zone 已迁移（26 处）。scheduler 单测暴露的 `get_buckyos_root_dir()` 进程级缓存问题已顺带修复（`create_init_list_by_template` 显式传 root）。

### 遗留（不阻塞本 TODO）

- 纯 HTTPS Manifest URL 入口只支持内嵌 ObjId 的 URL（§13.1 兼容层与 Web-to-Native 同批后做）。
- NDN chunk 形态的 staging handle 物化已实现；websdk 上传→install_package 的端到端用例待 websdk 侧支持后补。
- Control Panel 中途重启恢复的集成用例需 DV 编排能力（恢复语义已由引擎单测覆盖）。
- Static Web Activate 证据 = services info key 或本机 web 目录（scheduler/node-daemon 统一 instance 语义仍是协议已知风险）。
- WebUI 只读 RPC（status/list/update-check）按计划等 `AppInstallStatusSnapshot`/`AppUpdateAvailability` 冻结后随 WebUI PRD 另做（类型已定义）。
