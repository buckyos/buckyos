# App Service / App Installer beta 2.2 原型更新 TODO

> 创建日期：2026-09-05
> 状态：本轮 68 项原型待办已完成并验收；真实集成项独立保留为未完成（见第 9.4 节）。
> Review 基线：`4f22b93a`。执行前重新核对当前协议、共享类型与接口；本文记录的后端现状不是永久约束。
> 本轮任务：更新 Mock 原型、交互状态、UI DataModel 和产品文档。真实后端集成另行安排。

## 1. 目标与范围

当前 App Service 及公共 App Installer 直接使用 `AppServiceMockStore`，UI DataModel 仍引用旧 PRD / 安装协议 Draft v0.5。beta 2.2 已改变身份模型、AppDoc 校验、安装计划、提交与恢复语义。此次先让原型准确表达新版产品行为，为真实接口集成提供可评审的基线。

- [x] 保留首页、应用详情、来源页和公共 App Installer 的基本布局、三层信息架构、桌面窗口行为及移动端支持。
- [x] 重做检查、计划编辑、授权提交、执行、结果与恢复之间的状态关系。
- [x] 同步修改 Mock、输入 schema、UI DataModel、产品文档和相关 Playwright 用例。
- [x] beta 2.2 不做旧数字 task_id、旧 DataModel 或旧 Mock 缓存的兼容迁移。

本轮不实现新的安装后端、URL 抓取服务、Docker 采集、应用 Settings 保存接口或真实 DV 安装；不修改 Rust 协议及业务逻辑。现有能力可复用，未实现的能力必须记录为后续集成项。升级只覆盖从安装入口进入的升级确认，不扩展完整版本管理、批量操作、卸载、商店或日志分析平台。

## 2. 必读资料与修改入口

协议与实现依据：

- [App 安装协议 beta 2.2](<../doc/App 安装协议.md>)：身份、AppDoc、签名、Registry、InstallPlan、取消边界、运行投影。
- [App 基础设施职责边界](<../doc/app_service/BuckyOS App基础设施职责边界.md>)：PIKG 为普通 App 公开交付单元的产品意图；其中签名封装等细节仍须对照权威协议与代码。
- [共享安装类型](../src/kernel/buckyos-api/src/app_install.rs)、[AppDoc](../src/kernel/buckyos-api/src/app_doc.rs)、[应用运行类型](../src/kernel/buckyos-api/src/app_mgr.rs)。
- [control-panel RPC 路由](../src/frame/control_panel/src/main.rs)、[安装入口](../src/frame/control_panel/src/app_installer.rs)、[安装引擎](../src/frame/control_panel/src/app_install_engine.rs)、[来源解析驱动](../src/frame/control_panel/src/app_install_driver.rs)、[计划校验](../src/frame/control_panel/src/app_install_planner.rs)。
- [应用查询与状态](../src/frame/control_panel/src/app_servcie_mgr.rs)、[Scheduler 安装执行](../src/kernel/scheduler/src/install_plan_executor.rs)、[TaskManager ID](../src/kernel/task_manager/src/task_store.rs)。

允许修改的主要入口：

| 范围 | 文件或目录 |
|---|---|
| App Service 页面、Store、模型、schema | `src/frame/desktop/src/app/app-service/` |
| 公共 Installer 及拉起协议 | `src/frame/desktop/src/sysdlg/AppInstaller.tsx`、必要的 `src/sysdlg/index.tsx` 改动 |
| 原型回归 | `src/frame/desktop/tests/e2e/pages/app-service.spec.ts`、`app-installer.spec.ts` |
| 文案 | `src/frame/desktop/src/i18n/` 中与本任务相关的词条 |
| 产品文档 | `product/app_service/BuckyOS_App_Service_PRD.md`、`product/BuckyOS Sys Dlg.md` |
| 模型文档 | `src/frame/desktop/src/app/app-service/UI_DATAMODEL.md` |

优先复用 `src/frame/desktop/src/components/sudo.tsx`、`src/api/app_mgr.ts`、`src/api/task_mgr.ts`、现有文件浏览和 NDM 导入能力的模型与模式。不要为本任务引入新的依赖或通用框架。接手时读取仓库 AGENTS.md 及适用的 `harness/SKILLS/webui-prototype/SKILL.md`。

## 3. 已确认的差异：不要把旧假设带入新版原型

| 主题 | Review 时的协议 / 实现 | 原型应如何处理 |
|---|---|---|
| 任务身份 | TaskManager 使用 `t-<uuid>`；原型与 Sys Dlg 文档仍限定正 i64 | task_id 作为不透明字符串，不按数值解析 |
| 创建时机 | `apps.inspect` 不创建持久任务；`apps.submit` 要求批准的计划 fingerprint 和幂等键 | 区分检查草稿与已提交任务；计划中的预留 task_id 不等于任务已经存在 |
| 重复安装 | submit 可能返回 `satisfied` 且 `task_id:null`，或进入 upgrade | 增加无需安装、升级确认、禁止降级分支 |
| 重试 | 失败重试可创建新任务并返回 `retry_of`；暂停恢复可能沿用原任务 | 使用返回的任务身份，保留尝试关联，不强制“同一个编号重试” |
| 来源导入 | control-panel 拒绝抓取客户端 URL；Object ID 当前要求内容已在本地；staging 已有 finalize/status/release | 原型展示导入与检查阶段，接口未齐的来源单列后续项 |
| 签名 | 权威协议定义 detached envelope；PIKG reader 仍读取 `APPDOC.jwt` | 页面展示签名语义，记录格式差异；不要在此任务中改包格式或宣称 JWT 已被全面移除 |
| 取消 | Scheduler 在 `desired_state_committed` 后拒绝取消；公开 snapshot 目前仍给所有非终态加入 Cancel | 原型按提交边界建模；记录后端动作能力需修正，不能直接照搬粗略动作列表 |
| 完成 | Scheduler 发布 NodeConfig 后即可 Completed；并未由此证明节点部署和健康启动完成 | 安装提交结果与运行就绪状态独立展示 |
| 管理员授权 | Installer 密码框只验证非空；真实 sudo 组件已存在，安装入口尚缺相应强制授权闭环 | 原型统一授权交互并模拟成功/失败/过期；后端强制校验另列任务 |
| 恢复快照 | `apps.install.status` 有阶段/readiness/错误摘要，缺完整确认页 Plan/AppDoc 和独立运行结果 | 原型定义所需恢复数据，清楚标注待补接口，不能伪装现有接口已提供 |

执行复核（2026-09-05）：当前 HEAD 仍为 `4f22b93a`，上表差异继续成立。共享安装 schema_version 为 4；任务 schema_id 为 `app.install/v1` / `app.update/v1`；TaskManager 的 ID 为 `t-` 加 UUID simple 字符串。`apps.submit` / `apps.install` 当前共用提交路由，但 UI 采用 inspect → fingerprint/幂等提交语义。取消能力的粗略 snapshot 与 Scheduler 的 commit boundary 仍有差异，本轮只在 Mock 修正动作投影。

规则：用当前代码判断“已实现”，用权威协议判断目标约束。两者冲突时记录差异与原型采用的语义。旧 PRD、旧 Sys Dlg 及旧安装流程文档不能覆盖 beta 2.2；也不通过改写权威协议来掩盖实现缺口。

## 4. 新版主流程

```text
选择应用包 / 输入链接或标识
  → 导入、Resolve / Inspect（尚无持久安装任务）
  → 检查结果：
      可安装 → 配置计划 → 重新检查 → 管理员授权 → 提交
      已有相同内容 → 已安装 / 查看应用（不新建任务）
      可升级 → 展示版本及影响 → 确认升级计划 → 授权提交
      不可继续 → 说明原因及可执行动作
  → 提交成功后创建任务，跟随 task_id 展示执行
  → 安装提交结果 + 节点部署 / 运行状态
```

- [x] 提交前不显示虚构的任务编号或“任务已在后台运行”。刷新恢复检查草稿时重新 Inspect，不恢复旧授权。
- [x] 提交前关闭是退出准备，模拟释放不再使用的导入引用；提交后关闭是后台执行，不等于取消。
- [x] 最后确认按钮明确为“确认并安装”或“确认并升级”；提交中禁用重复操作。
- [x] 同一次提交的重复请求保持幂等；修改来源/计划后按新请求处理，不重用不匹配的确认。
- [x] 计划过期或条件变化时显示“应用信息或安装条件已变化，请重新检查”，重新展示影响并重新确认。

## 5. 页面修改 TODO

### 5.1 来源页与公共入口

- [x] 主入口为“选择应用包”，支持本地与 Personal Server；次入口为“输入链接或应用标识”。
- [x] AppDoc 文本、Object ID 等移到折叠的高级导入，不让普通用户选择内部对象类型。
- [x] 增加文件准备、上传/导入进度、导入失败、文件引用过期、用户取消等状态。
- [x] 页面只显示文件名、大小、来源和可理解的错误；不向公开 URL/调用方暴露 staging handle、主机路径或文件内容。
- [x] 本地文件不能仅凭后缀判定有效；HTTP URL 不能默认判为 App Meta；JSON 样例使用 AppDoc v1 的 `did`、严格 `doc_type` 等真实字段。
- [x] 本地 PIKG 不默认等于离线就绪；离线模式表示禁止网络获取，仍须验证信任、内容、平台与配置。
- [x] 公共 `identifier` / `task_id` 入口继续互斥，拒绝重复/未知参数；外部初始选项只是建议，不是用户批准。
- [x] 仅在实际创建任务后将直接入口 URL 归一化为 `?task_id=...`；已有任务入口只恢复，不创建替代任务。
- [x] 同步修订 Sys Dlg 中数字任务 ID、旧 `apps.install_package`、立即创建任务等过期说明；明确内部导入上下文与公开入口的职责。

### 5.2 应用检查页

- [x] 默认展示名称、版本、简介、类型、发布来源、安装归属账号、检查结论和待处理项。
- [x] 发布对象 owner、controller、author 与 `owner_user_id` 分开；“安装归属账号”不得使用发布者 DID 代替。
- [x] AppDID、canonical `appdoc:<64 hex>`、签名与权威证据放入可展开详情；样例身份符合 hostname-form DID 规则。
- [x] 分别表达文档有效性、签名校验、controller/owner 约束、权威发布、内容、目标及配置情况；未知不等于通过。
- [x] 去掉无依据的“Highly trusted”；无价格信息不显示“免费”，无权威版本比较不显示 Latest，无尺寸依据不编造安装大小。
- [x] 新增“本地开发安装”：仅允许符合策略的 LocalPikg，用户明确选择，不因缺签名自动降级进入。
- [x] 本地开发模式显示“未公开发布 / 使用本地开发授权”，保留真实权威状态；撤销、迁移、终止等不允许状态仍拒绝。
- [x] 签名封装细节保持在高级详情中；UI 不依赖“JWT 三段文本可解析即可信”的旧假设。

### 5.3 安装配置与最终确认

- [x] 保留单列布局；增加只读安装上下文摘要：归属账号、自动选择的目标设备、OS/arch。高级入口允许选择真实节点，不再固定两台 OOD。
- [x] 必需组件自动选择且不可取消；只有存在可选组件时才展开选择，不向普通用户暴露 package matrix。
- [x] 地址在尚未分配时显示“由系统分配”；现有实例显示实际地址。AppName/AppHostName/AppIndex 是系统只读投影。
- [x] 快捷域名从 InstallParams 移出；可作为安装后独立设置的后续入口，不模拟它随安装计划原子成功。
- [x] 服务开关、暴露范围、端口和访客访问依照 AppDoc endpoint 与真实 ServiceSettings 建模；必需 endpoint 不可关闭。
- [x] 权限展示 `scope_path/actions/required`；必需项不可省略，可选项可选择。移除后端不能兑现的任意 grant 改写。
- [x] 分开持久数据、节点缓存、外部目录，说明用途、权限与数据归属；数据归属按用户与 App 隔离，跨升级稳定。
- [x] 环境变量遵循 AppDoc 声明；BUCKYOS 系统注入的身份与路径变量不可随意覆盖。
- [x] `start_param/container_param` 保留高风险只读说明；无编辑契约的配置不增加输入框。
- [x] 修改选项进入“重新检查计划”状态；处理中及阻塞时不可提交，展示最新 readiness 和字段级问题。
- [x] 最终摘要反映此次批准的配置、下载需求和影响；fingerprint 留在模型/诊断详情，不要求用户手工理解或输入。
- [x] 复用统一 sudo 交互，通过 Mock 适配模拟错误密码、取消、过期和重试；原型测试不依赖真实 Verify Hub。
- [x] 密码、sudo token 不进入任务、URL、localStorage、错误复制或日志；敏感环境变量不在摘要与诊断中明文展示。

### 5.4 执行、失败、重试与恢复

- [x] 默认展示用户可理解的阶段与当前操作，详情才展开底层阶段；百分比未知时使用阶段进度，不伪造百分比。
- [x] 正确处理 waiting/readiness、phase/outcome、失败、取消与完成，不再用单一 running/failed 粗略覆盖。
- [x] 提交边界之前提供显式取消，之后提示“配置已提交，无法取消，可后台查看进度”；不承诺任意阶段回滚。
- [x] 失败动作按 retryable、修复动作和提交边界展示；不要始终提供“重试 / 返回修改 / 更换来源”三个按钮。
- [x] 失败重试跟随返回的新 task_id，保留 `retry_of` 和上一尝试；暂停恢复允许返回原 task_id。
- [x] Mock 支持多个任务、按 task_id 查询、关闭重开、刷新和任务不存在/无权限；缓存至少按 Zone/用户/模型版本隔离。
- [x] 与 Task Center 共用任务身份并可跳转；不得为同一安装另造一个展示用任务。
- [x] 模拟安装状态恢复需要的 Plan/AppDoc/结果视图；文档明确它们中哪些已有读取接口、哪些需要后续扩充。

### 5.5 结果、首页与应用详情

- [x] 使用 `app_instance_id` 关联卡片、任务与详情；同 AppDID 的不同 owner 安装分别显示。
- [x] 首页保留应用 / 系统服务 / 内核三层；系统服务使用 SystemServiceId，不伪造 AppInstanceId。
- [x] 定义管理员管理范围与普通用户可见范围；不要把 `apps.list` 的授权可用集合直接声称为 Zone 全量安装清单。
- [x] 安装结果与运行状态分别建模，至少覆盖：配置已提交/部署中、已安装/启动中、运行正常、未启动、启动失败、运行状态未知。
- [x] 任务 Completed 不自动把应用设为 running；就绪依赖匹配 deployment 的有效运行证据。
- [x] Docker 类型展示 Engine/Image/Container；Script 展示进程健康；静态 Web 展示部署与可访问状态；Agent runtime 展示环境与 binding。
- [x] Agent runtime 安装不等于创建 Agent；补“已安装但尚无 Agent binding”的场景。
- [x] 未取得诊断数据使用未知/不可用，不把 `docker:null` 一律解释为节点原生服务。
- [x] Start/Stop 根据权限及可用操作启用，操作后等待状态收敛，覆盖失败与超时。
- [x] Settings 区分只读部署配置和可调设置；通用保存契约未明确的字段不得模拟成已经持久保存并生效。记录后续保存与生效语义需求。

## 6. UI DataModel 与文档交付

- [x] 从 `mock/types.ts` 中抽离稳定 UI 类型，Mock 作为数据提供方，不作为业务类型唯一归属。
- [x] 分离来源引用、检查草稿、不可变 Plan、动态 readiness、执行任务、安装记录和运行视图；不能让“检查页面必须先有 InstallTask”。
- [x] 明确 task_id 为 string；任务模型容纳 schema_id、phase/outcome、retry_of、可用动作与错误，映射未知枚举有 fallback。
- [x] 定义 runtime type、owner、AppInstanceId、deployment 与证据有效性；详情不再只依赖字符串字典。
- [x] 对齐 InstallParams 的组件、权限、三类挂载、服务设置、环境变量与自动启动；schema 覆盖跨字段约束。
- [x] 更新 UI_DATAMODEL 的字段含义、状态、输入 schema、接口映射、Mock 场景和变更记录，清除“冻结”但已过期的数字 ID / 原任务重试假设。
- [x] 更新 PRD 和 Sys Dlg；区分本轮确定的原型行为、当前实现能力与后续接口需求。保持权威安装协议内容不变。

## 7. Mock 与 Playwright 验收场景

| 场景 | 必须验证的结果 |
|---|---|
| 普通 PIKG 安装 | 导入→检查→配置→授权→提交→任务→部署/运行结果 |
| LocalDeveloper | 显式选择后才可安装未发布包，不能伪造公开信任；终止身份仍拒绝 |
| 重复安装 / 升级 / 降级 | satisfied 不建任务；升级展示影响并重新确认；不支持的降级有明确提示 |
| 参数修改与 PlanStale | 重新检查时禁用提交，旧 fingerprint/批准不能继续使用 |
| 错误密码 / 取消 / 过期 | 不创建执行任务或绕过确认，敏感值不进入持久化和日志 |
| 信任 / 内容 / 配置 / 平台阻塞 | 错误归类正确，提示对应修复动作；离线模式不偷偷进入网络获取 |
| 重复点击提交 | 同一请求只有一个任务，无重复卡片 |
| 取消边界 | 提交前可取消，提交后不能承诺撤销；关闭只转后台 |
| 失败重试与暂停恢复 | 关联新旧任务或沿用原任务，以返回值为准 |
| 刷新 / 直接链接 / 关闭重开 | 已有任务恢复正确；检查草稿重新检查；缺失/无权限不创建替代任务 |
| 多任务 / 多 owner / 切换用户 | 卡片和任务不串用，缓存不会展示其他用户的安装内容 |
| 安装已提交但运行未就绪 | 不显示虚假的运行成功；覆盖超时、离线、启动失败和 auto_start=false |
| Web / Script / Docker / Agent runtime | 依赖状态与类型一致，Agent binding 单独解释 |
| 中英文与移动端 | 文案完整，表单/错误/操作可用，长 DID/task_id 不造成横向溢出 |

## 8. 实施顺序与完成标准

1. 重核协议与实现，更新第 3 节差异记录；梳理模型和状态转换。
2. 修改 Mock、schema 与页面，先完成 PIKG 主路径，再补边界场景与公共恢复入口。
3. 同步 PRD、Sys Dlg、UI_DATAMODEL，并更新相关 Playwright 用例。
4. 执行验证，记录结果、截图位置和后续真实集成清单，再提交评审。

在 `src/frame/desktop` 下，按执行时的现有脚本运行：

```bash
pnpm run check
pnpm run build
VITE_CP_USE_MOCK=true pnpm exec playwright test tests/e2e/pages/app-service.spec.ts tests/e2e/pages/app-installer.spec.ts --project=chromium
```

Playwright 当前使用 4173 端口并允许复用服务；执行前确认复用的是本轮 Mock 实例，不连接生产服务。复用共享组件有改动时，再运行受影响用例。此次原型验收不以真实安装 DV 代替，也不需要启动/重装 BuckyOS。

- [x] 上述主路径与边界场景通过，附桌面/移动端关键页面截图，无新增控制台错误。
- [x] 源码、schema、Mock、测试、PRD、Sys Dlg、UI_DATAMODEL 的身份与状态语义一致。
- [x] 未用假成功掩盖尚未接入或尚未实现的能力；每个后续缺口都有接口/字段/行为说明。
- [x] 完成报告说明改了什么、验证结果、主要入口、已知协议/实现差异及未验证项。

后续真实集成清单至少保留：来源导入桥接与签名封装对齐、sudo 后端强制校验、恢复读取快照、取消动作能力修正、运行就绪聚合、全量管理列表、Settings 保存、Docker/跨节点日志诊断，以及真实浏览器端到端安装验证。这些项目不因本轮 Mock 原型通过而标为完成。


## 9. 完成报告（2026-09-05）

### 9.1 已交付行为与主要入口

本轮 68 项原型待办完成。首页、来源、详情与公共 Installer 保留原有布局和窗口入口，检查阶段改为无持久任务的草稿；重新检查产生不可变计划，sudo 批准后才进行幂等提交。安装任务支持不透明字符串 ID、多任务恢复、取消边界、新任务重试与原任务恢复；结果独立展示配置发布、部署和运行证据。

| 主要入口 | 改动 |
|---|---|
| [types.ts](../src/frame/desktop/src/app/app-service/types.ts)、[schemas.ts](../src/frame/desktop/src/app/app-service/schemas.ts) | 稳定 UI 类型与 Mock 分离；来源、草稿、计划、任务、安装记录、运行证据分层；声明驱动及跨字段输入校验 |
| [mock/store.ts](../src/frame/desktop/src/app/app-service/mock/store.ts)、[mock/fixtures.ts](../src/frame/desktop/src/app/app-service/mock/fixtures.ts) | 多 owner / Zone 隔离、过期与异步检查失效、授权适配、幂等提交、竞争计划 generation 检查、执行及运行场景 |
| [AppInstaller.tsx](../src/frame/desktop/src/sysdlg/AppInstaller.tsx) | 公开参数严格解析、检查配置授权流程、提交后归一化 task_id URL、任务恢复和分离的运行结果 |
| [App Service 页面](../src/frame/desktop/src/app/app-service/pages/)、[RuntimeSummary.tsx](../src/frame/desktop/src/app/app-service/components/RuntimeSummary.tsx) | 本地/Personal Server 来源、分层列表、权限与部署身份、按运行类型显示证据、只读 Settings |
| [sudo.tsx](../src/frame/desktop/src/components/sudo.tsx)、[task_mgr.ts](../src/frame/desktop/src/api/task_mgr.ts) | 复用 sudo 的可选 Mock 密码适配并清空输入；Task Center 从同一 Store 映射原任务身份 |
| [中英文词条](../src/frame/desktop/src/i18n/app-service.ts) | 新流程、错误与授权文案，清理旧信任、任务及安装语义 |
| [UI DataModel](../src/frame/desktop/src/app/app-service/UI_DATAMODEL.md)、[PRD](../product/app_service/BuckyOS_App_Service_PRD.md)、[Sys Dlg](<../product/BuckyOS Sys Dlg.md>) | 同步身份、状态转换、输入约束、Mock 契约、现有接口与待补能力 |

权威安装协议、Rust 业务逻辑及共享类型未修改；没有新增依赖、框架或旧缓存迁移。共享组件改动限于统一授权适配与已有任务身份的投影。

### 9.2 验证结果

以下命令在 `src/frame/desktop` 执行，4173 服务为本轮启动的 `VITE_CP_USE_MOCK=true` 开发实例。

| 验证 | 结果 |
|---|---|
| `pnpm run check` | 通过 |
| `pnpm run build` | 通过；最终代码完成 TypeScript 检查及 Vite 生产构建，仍有 chunk 大于 500 kB 的构建提示 |
| `pnpm exec eslint src/app/app-service src/sysdlg/AppInstaller.tsx src/components/sudo.tsx src/i18n/app-service.ts` | 通过 |
| `VITE_CP_USE_MOCK=true pnpm exec playwright test tests/e2e/pages/app-service.spec.ts tests/e2e/pages/app-installer.spec.ts --project=chromium --workers=3` | 最终完整运行 **49/49 通过**（1.4 分钟）；覆盖中英文及 375px 移动端，无 pageerror、console.error 或意外 `/kapi/` 请求 |
| `VITE_CP_USE_MOCK=true pnpm exec playwright test tests/e2e/pages/users-agents.spec.ts tests/e2e/pages/desktop.spec.ts --project=chromium --workers=3 --output=test-results/app-service-beta22-shared` | 额外共享回归 **15 通过、1 项基线已有失败**，见下文 |
| `git diff --check`（仓库根） | 通过 |

额外回归的失败为 `desktop.spec.ts:599` 的 `window modal only blocks its owner window`。它打开 Settings 的 General 页后，在第 620 行直接查找 Language combobox，而该控件在 Appearance 页。已通过 `git archive 4f22b93a src/frame/desktop` 创建独立基线副本，复用现有依赖、仅把 Playwright 服务端口改为 4174，运行同一用例，复现相同的 30 秒定位超时。相关 Settings 页面和测试不在本轮修改中，保留原状；[基线错误上下文](./app-service-beta2.2-prototype-artifacts/baseline-desktop-regression.md)已归档。

未运行 Rust 测试、完整 BuckyOS 构建或真实安装 DV；本轮没有改动这些实现，也不将 Mock 验收解释为真实链路通过。

### 9.3 截图与原型体验

截图已检查并归档到 `notepads/app-service-beta2.2-prototype-artifacts/`；测试再次运行时会在 `src/frame/desktop/test-results/app-service-beta22/` 生成新截图。

| 视口 | 页面截图 |
|---|---|
| 桌面 | [首页](./app-service-beta2.2-prototype-artifacts/desktop-home.png)、[来源](./app-service-beta2.2-prototype-artifacts/desktop-source.png)、[安装计划](./app-service-beta2.2-prototype-artifacts/desktop-plan.png)、[运行详情](./app-service-beta2.2-prototype-artifacts/desktop-detail.png) |
| 移动端 375px | [来源](./app-service-beta2.2-prototype-artifacts/mobile-source.png)、[检查](./app-service-beta2.2-prototype-artifacts/mobile-check.png)、[升级计划](./app-service-beta2.2-prototype-artifacts/mobile-plan.png)、[结果与部署状态](./app-service-beta2.2-prototype-artifacts/mobile-result.png) |

可从桌面的 App Service → 添加应用 → Personal Server 示例开始，或访问 `/sysdlg/app_installer?identifier=nextcloud`。Mock 管理员密码为 `prototype-admin`，`prototype-expired` 模拟过期，其余值模拟错误密码。这些是测试输入，不是系统凭据。

本地导入仅接受 UI DataModel 第 7 节定义的 JSON Mock PIKG fixture，不是实际 PIKG 解析器；URL、AppDoc 文本及 Object ID 的真实导入明确显示待接入。Mock 身份独立于真实登录。完全关闭浏览器后没有真正后台进程，重开时从缓存恢复模拟执行。缓存和诊断不保存密码、sudo grant 或敏感环境变量明文。

### 9.4 后续真实集成（未完成，不计入本轮 68 项）

当前实现依据及目标字段详见 [PRD 第 10 节](../product/app_service/BuckyOS_App_Service_PRD.md#10-当前能力与后续集成)和 [UI DataModel 第 8 节](../src/frame/desktop/src/app/app-service/UI_DATAMODEL.md#8-krpc-映射及未接入字段)。

- [ ] 来源桥接：连接浏览器、文件浏览和 NDM 的真实导入流程；定义 URL 导入方以及 staging 引用所有权、过期、释放。
- [ ] 签名封装：对齐 detached envelope 与现有 `APPDOC.jwt` Builder/Reader/验证链。
- [ ] sudo 后端强制校验：提交和重试校验提升权限、有效期及 owner/scope；补敏感配置的 secret reference 契约。
- [ ] 恢复读取快照：提供安全的 Plan/AppDoc 展示投影、结果、commit point、retry_of 和可见性校验。
- [ ] 取消动作能力：让公开 status 的 available_actions 遵循 desired_state_committed 边界。
- [ ] 运行就绪聚合：按 deployment 聚合节点部署与健康报告，校验时效，覆盖离线、超时和 Start/Stop 收敛。
- [ ] 全量管理列表：明确 Zone 管理权限、owner 范围、分页和总量，区别于 apps.list 的授权可用集合。
- [ ] Settings 保存：定义可编辑字段、校验、保存、权限及即时生效/重启/重部署语义。
- [ ] Docker/跨节点诊断：补采集、deployment 绑定、日志读取权限与数据不可用语义。
- [ ] 真实浏览器端到端安装：验证上传、授权、执行、取消/恢复及真实节点报告；另行安排 DV。
