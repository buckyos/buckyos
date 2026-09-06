# App Service / App Installer UI DataModel — beta 2.2

更新：2026-09-05；源码复核基线：`4f22b93a`。本模型描述已实现的 Mock 原型，并为真实集成明确边界。旧数字任务 ID、旧 Mock 缓存及 Draft v0.5 模型不迁移。

依据：[PRD](../../../../../../product/app_service/BuckyOS_App_Service_PRD.md)、[Sys Dlg](../../../../../../product/BuckyOS%20Sys%20Dlg.md)、[权威安装协议](../../../../../../doc/App%20安装协议.md)。从本目录到仓库根是六层；代码单一事实来源为 [types.ts](./types.ts)、[schemas.ts](./schemas.ts)，Mock 只作为数据提供方。

## 1. 结构与入口

| 入口 | 职责 |
|---|---|
| `AppServiceAppPanel.tsx` / `components/layout/AppServiceShell.tsx` | 首页、来源、详情导航，原有窗口及移动端容器 |
| `pages/InstallWizard.tsx` / `components/installer/FilePickerDialog.tsx` | 来源表单、本地字节检查、Personal Server 示例选择、可取消导入 |
| `src/sysdlg/AppInstaller.tsx` | 严格公开参数解析、检查草稿、计划编辑、sudo、提交与恢复 |
| `components/RuntimeSummary.tsx` | 安装结果以外的独立、按类型显示的运行证据 |
| `mock/fixtures.ts` | 应用声明、节点清单、初始多 owner 安装与三层列表 |
| `mock/store.ts` | inspect/submit/retry/resume/cancel、引用寿命、任务及运行模拟、缓存隔离 |
| `hooks/use-app-service-store.ts` | App Service、公共 Installer、Task Center 共用 Store |
| `src/components/sudo.tsx` | 统一 sudo 交互；可选 requestPassword 适配，默认真实请求路径保留 |
| `src/api/task_mgr.ts` | 在 Mock Task Center 映射已有安装任务，保留原 task_id，不产生第二份任务 |
| `src/i18n/app-service.ts` | beta 2.2 词条及统一 sudo 中英文 |

这些 `src/...` 路径以 `src/frame/desktop` 为根。界面仍是 Mock，不隐式切换到真实安装；无新依赖或通用框架。

## 2. 身份与模型分层

```text
SourceReference → InspectionDraft → InstallPlan + PlanReadiness
                                 → submit → InstallTask
                                            ├─ InstallRecord
                                            └─ RuntimeView / DeploymentIdentity
AppServiceItem = 应用实例或 SystemServiceId 的页面投影
```

检查页面依赖 InspectionDraft，不能要求先有 InstallTask。

| 类型 | 核心字段与含义 |
|---|---|
| `TaskId` | string，不透明；Mock 生成 t- 加 32 位 UUID hex，与当前 TaskManager 同类格式；不解析数值 |
| `ScopeContext` | zone_id / user_id / role；原型默认 prototype-zone / alice / admin，独立于真实登录 |
| `InstallTarget` | node_id、node_did、label、os、arch、online；来自节点清单，页面不可自造 OS/arch |
| `SourceReference` | reference_id、kind、display_name、size_bytes、expires_at、state；referrer 仅来源说明；fixture/scenario/content_available 是 Mock 提供方字段 |
| `PickedPikgFile` | location/name/sizeBytes，device 使用暂存 File 对象，Personal Server 使用受控 fixture 名；File 不持久化、不向外部返回 |
| `SourceParseResult` | `{ok:true,source}` 或 `{ok:false,code}`，错误码经 i18n 映射；URL/对象/JSON 不伪造可导入结果 |
| `AppDocumentView` | AppDoc v1 的检查/配置所需投影，不是完整 AppDoc wire DTO；保留 schema_version/doc_type/did/object_id/app_id/show_name/version/app_type、运行类型及发布 owner/controller/author |
| `PermissionItem` | scope_path/actions/required/exp；批准值必须是原声明的精确子集，包含全部必需项 |
| `Endpoint` | name/protocol/inner_port/required/route；普通表单只配置已声明服务 |
| `MountDeclaration` | kind/path/target_path/access/required；三类逻辑映射分开，不伪装主机实际路径 |
| `EnvironmentDeclaration` | name/default/required/sensitive/system；sensitive/system 是 UI 提供方分类，后端需补敏感字段标注或 secret reference 契约 |
| `InspectionDraft` | draft_id/source/app/input/status/plan/readiness/inspection_revision/error/suggestions；可以没有 Plan，永远不是任务 |
| `InstallPlan` | readonly schema_version=4、plan_fingerprint、app_instance_id、owner_user_id、app、target、input、plan_use、previous_version/generation、download_bytes、created_at/expires_at |
| `PlanReadiness` | 独立的 document/signature/owner/authority/content/target/config、document_status、local_developer_authority、install、issues |
| `FieldIssue` | field/code/action；action 为 edit/recheck/change-source，控制具体修复提示 |
| `InstallTask` | task_id/schema_id/app_instance_id/owner_user_id/app/plan/readiness/source/phase/outcome/stage/progress/desired_state_committed/available_actions/retry_of/error/result/tick/updated_at |
| `InstallRecord` | 实例及归属、AppDID/AppDoc Object ID、task_id、version、state、deployment，以及只读 app_name/app_host_name/app_index/address |
| `DeploymentIdentity` | app_instance_id/task_id/app_doc_object_id/spec_generation；必须全匹配，不能只按 AppDID 判断 |
| `RuntimeView` | type/status/expected_deployment/evidence、docker、process_health/web_accessible/environment_ready、agent_bindings/reason |
| `AppServiceItem` | id 与 app_instance_id 或 system_service_id 对应；owner、名称、版本、layer/status/runtime/record；只读 spec/settings/serviceInfo、诊断、日志与 available_actions/operation/operation_error |
| `SubmitResult` | submitted 返回 task_id/retry_of；satisfied 返回 task_id:null 和 app_instance_id |

AppDID 样本为 `did:bns:nextcloud.buckyos`；AppId 为 `nextcloud.buckyos.bns.did`；alice 的 AppInstanceId 为 `nextcloud.buckyos.bns.did@alice`。系统服务行使用 `scheduler` 等 SystemServiceId，app_instance_id=null。不同 owner 的同 AppDID 不能去重到一张卡片。

`AppDocumentView.runtime_type` 是部署方式（docker/script/web/agent/unknown），与 AppDoc.app_type 的 dapp/web/agent 不混为同一枚举。`docker:null` 不表示“原生服务”。

## 3. 输入 schema

所有配置输入从 `InstallInput = z.infer<typeof installerApprovalSchema>` 推导。`createInstallInputSchema(app, targets, localPikg)` 增加声明驱动的跨字段校验；React 表单通过 zodResolver 接入。

```ts
type InstallInput = {
  target_node_id: string
  policy: 'NORMAL' | 'LOCAL_DEVELOPER'
  offline: boolean
  install_params: {
    selected_components: string[]
    permissions: PermissionItem[]
    data_mount_points: Record<string, MountPointConfig>
    local_cache_mount_points: Record<string, MountPointConfig>
    external_mount_points: Record<string, MountPointConfig>
    service_settings: { services: Record<string, ServiceSetting> }
    bash_envs: Record<string, string>
    auto_start: boolean
    expected_instance_count: 1
  }
}
type MountPointConfig = {
  target_path: string
  access: 'read_only' | 'read_write' | 'read_write_append'
}
type ServiceSetting = {
  enabled: boolean
  expose: {
    route: { type: 'web'; sub_hostname: [] } | { type: 'port'; expose_port: number }
    scope: '' | 'zone'
    allow_guest: boolean
  }
}
```

本轮 UI 将 expected_instance_count 固定 1，不提供 res_pool_id、任意资源、RDB 或无契约配置编辑。快捷域名不进入 InstallParams。空 web sub_hostname 表示不在安装计划请求快捷域名；真实 ServiceSettings 支持更宽的 scope/route，可由后续集成扩展，不能误将 UI 子集写回权威类型。

| 输入 | 限制、默认值与拒绝条件 |
|---|---|
| manualInstallSourceSchema | trim，1–32768 字符，字段 sourceText；只负责输入边界，来源读取另行检查 |
| target_node_id | 1–256 字符，默认当前首选节点；必须在节点清单中 |
| policy/offline | 默认 NORMAL/false；LOCAL_DEVELOPER 仅 LocalPikg；offline=true 缺内容必须阻塞 |
| selected_components | 1–32 项，唯一、全部已声明，自动保留必需组件 |
| permissions | 最多 64 项，scope 最长 512、actions 最多 16；精确包含必需条目，拒绝重复、任意改写 actions 或 grant |
| endpoint | inner_port 只读；expose_port 为 1–65535 整数，已启用端口不能冲突；必需 endpoint 不能关闭，路由类型遵循声明 |
| guest/scope | zone 成员暴露不允许匿名访客；需显式改为公共暴露后才能启用 |
| 三类挂载 | 绝对逻辑路径、最长 256，拒绝反斜杠、NUL 和 .. 穿越；精确匹配声明的目标及权限，保留必需映射，同挂载路径不可跨类型重复 |
| bash_envs | 标识符格式约束，单值最长 4096；只能是声明变量，必需值不可空；禁止 BUCKYOS_* 覆盖；敏感项是 password 输入，摘要用掩码 |
| auto_start | 默认 true，可关闭；不影响安装提交是否成功 |
| taskIdSchema | 非空、最长 256、不含空白或控制字符；没有整数解析或数值上限 |
| AppDoc candidate | schema_version=1、doc_type 严格 app、did 及 owner/controller/author 为 hostname DID，完整必需根字段；pkg/endpoint/mount/env 等声明的已知结构拒绝未知键；service_config_tips 允许协议中的自定义配置 |

AppDoc candidate schema 是输入预检，不做签名、Object ID、包图或权威验证，语法通过仍返回待接入的文本导入提示。它不能成为任意文档安装成功的依据。

默认输入来源是 `mock/fixtures.ts::defaultInput`：首选节点、main 必需组件、必需权限、data/cache 必需挂载、关闭的可选 external/metrics、LANG 默认值、空敏感 API_KEY、auto_start=true。编辑回填使用当前 draft.input；刷新时敏感值清空，重新检查。

合法样本：选择 worker 并保留 main；授权可选 wan 原条目；启用已声明 external 只读目录；关闭自动启动。非法样本：只选 worker、把必需权限 write 改成 read、删除 data 映射、设置 BUCKYOS_APP_INSTANCE_ID、重复暴露端口、向未知节点提交。Playwright 使用独立样本验证这些跨字段拒绝。

## 4. 状态与转换

### 来源

```text
idle → preparing → importing(progress) → ready
                            ├─ failed
                            └─ canceled
ready → expired / released
```

只有已知字节导入进度才有百分比；未知执行进度为 null。选择新来源或离开页面会取消旧分析请求；过时请求不覆盖新选择。大于 64 KiB 的真实文件提示真实 PIKG bridge 待接入，原型不解析实际包格式。

### 检查草稿与批准

```text
dirty → inspecting → ready | blocked
ready → edit → dirty → inspecting
ready → sudo cancel/error/expired → ready（无任务）
ready → changed conditions → stale → inspect → new approval
ready → authorized submit → submitting → submitted(task) | satisfied(no task)
```

每次检查记录 inspection_revision，过时异步结果不覆盖新输入；配置变化立即清除旧 Plan/批准。Plan 递归冻结。动态 readiness 独立于 Plan，不用单一 ready bool 覆盖未知或阻塞。

Mock fingerprint 使用规范化排序后的输入、声明、目标、来源/证据及检查 revision 的 SHA-256；这是用于演示失效边界的模拟 fingerprint，不是后端 JCS 算法或 wire DTO 的替代实现。真实集成直接读取并回送后端 fingerprint，不在浏览器重算。

Mock plan TTL 为 3 分钟；导入引用为 30 分钟。提交使用 draft_id + fingerprint 派生稳定幂等键，并缓存同请求结果/在途 Promise。指纹不匹配、期限变化或已有部署 generation 变化拒绝旧批准。

### 任务与取消

phase = Running / Waiting / Terminal / Unknown；outcome = Succeeded / Failed / Canceled / null。stage = acquire / verify / prepare / deploy / activate / unknown。持久输入中未知阶段归一为 unknown，未知 phase/outcome 映射安全 Unknown，无动作。

| 条件 | 可用动作 |
|---|---|
| Running，未提交 desired state | cancel、后台查看 |
| Waiting（暂停内容准备） | resume、cancel |
| 失败、可重试、未提交配置 | retry；来源问题可 change-source |
| 失败、已提交配置 | retry 后续调度；不能 cancel 或更换来源 |
| Succeeded/Canceled | 只读任务、结果和可用应用详情 |
| 未知任务状态 | 未知视图，无状态修改动作 |

同实例的竞争计划在提交配置时再次检查 generation；冲突失败提供重新检查入口，不能覆盖已提交部署。取消由模型再次检查 desired_state_committed；UI 不直接信任粗略的 available_actions。retry 前保留原 task；未提交失败返回新 task_id/retry_of，暂停返回原 ID，已提交调度失败也恢复原执行身份。公共 URL 按返回身份更新。

### 安装结果与运行

InstallRecord.state = configuration_submitted / installed。后者对应当前 Scheduler 安装记录“NodeConfig 已发布”的含义，不能单独证明节点健康。

RuntimeView.status = installing / deploying / starting / running / stopping / stopped / activation_failed / error / unknown。

任务完成后先保持 deploying；Mock 独立提供匹配 deployment 的部署报告，再进入 starting 或 auto_start=false 的 stopped，之后独立健康报告才进入 running。offline、timeout、无报告或旧 deployment 证据使状态保持未知。启动失败保留成功安装记录。

`runtimeEvidenceValid` 要求 expected_deployment 与报告中的四个身份字段全部相同，且 valid_until 晚于当前时间。查看旧任务不会显示新部署的运行成功。Docker/Script/Web/Agent 各自消费相应诊断字段；Agent binding=0 有独立解释。

### 页面数据状态

| 页面 | loading / progress | ready / empty | error |
|---|---|---|---|
| 首页 | skeleton；多个执行任务阶段 | 三层列表；无应用空态，保留系统和内核 | 可重试的读取错误 |
| 来源 | 准备与导入进度 | 尚未选来源；有效引用 | 输入、导入、取消、过期分开 |
| 检查 | Resolve/Inspect 状态 | 可安装、已满足、升级 | 信任/身份/平台/配置问题及修复动作 |
| 配置 | dirty/inspecting/submitting | 当前批准摘要 | 字段校验、PlanStale、sudo 错误/过期 |
| 任务 | Running/Waiting、已提交边界 | 历史任务和结果 | Missing/Forbidden、分类失败、未知阶段 |
| 详情 | 控制操作待收敛 | 分类型运行视图；日志为空 | 无权限/应用缺失、操作失败/超时、诊断不可用 |

## 5. 缓存、恢复与敏感数据

缓存键：`buckyos.app-service.v4:<zone_id>:<user_id>:<role>`；内容含 model_version=4 与完整 scope 校验。无旧缓存读取或迁移。模拟身份切换清空在途 timers/grants 并加载新命名空间；reload/公共 task URL 仅访问当前命名空间。

缓存保存多个任务、安装卡片、非敏感草稿与幂等结果。检查草稿不持久化 Plan/readiness/批准；恢复后 re-inspect。敏感环境值清空，密码和 sudo token 不写缓存。File 对象、字节、staging handle、真实主机路径不属于公开模型。错误复制只含 task_id/app_instance_id/stage/phase/outcome/error.code/fingerprint/retry_of。

运行中的 Mock 页面有 timer 推进；刷新/重开后从持久 tick 恢复。完全关闭所有浏览器页面时没有真正后台进程；重开后继续模拟执行，不能将此描述为后端任务执行能力。

任务快照中的 Plan 是脱敏展示快照，真实后端执行数据不得取自浏览器 localStorage。后续恢复接口需要安全的 secret reference/脱敏字段协议；原型不把脱敏后缺失的环境值伪装成可直接重放的真实安装输入。

## 6. 列表与聚合

当前原型没有后端分页：应用清单为有限 Mock 集合，按 fixture 顺序，新增实例置顶；任务按 updated_at 降序。不提供版本列表、批量操作或用户可配置分页。普通用户过滤 owner，管理员显示 Mock 管理范围。同 AppDID 不同 owner 独立；同实例重试/升级不新增重复卡片。

Task Center 的 Mock adapter 从同一 Store 取任务，并映射 TaskCenter 的 rootTaskId/taskId 为安装 task_id。retry_of 是尝试关联，不假造 TaskManager parent/root 关系。其查询与事件策略沿用 Task Center，真实列表分页、全量安装总数和权限由后续 API 定义。

## 7. Mock 数据契约与场景

### 可操作样本

Personal Server 文件列表提供 nextcloud、paperless、home-dashboard。已安装列表覆盖 Docker、Script、静态 Web、Agent runtime 与同 AppDID 的 alice/bob 实例，以及 SystemServiceId 行。

本地测试文件内容（文件名可为 `nextcloud.pikg`）：

```json
{
  "format": "buckyos-pikg-mock-v1",
  "app": "nextcloud",
  "scenario": "normal",
  "content_available": true
}
```

这只是 UI 测试文件，不是 PIKG 包格式。后缀正确但内容错误会被拒绝。`content_available` 未明确为 true 时，不能认为有完整本地内容；开发模式也不能跳过它。已知 fixture 控制数据不会暴露为公开参数或用户批准。

标识符示例：`nextcloud`、`did:bns:nextcloud.buckyos`、`paperless`、`nextcloud-upgrade`。固定产品已有安装时返回 satisfied；已预装的 `opendan` 要测试安装执行可用 `opendan-upgrade`。场景可通过 Mock 标识符的受控后缀或本地 fixture.scenario 选择。

| 场景 | 预期 |
|---|---|
| normal | 检查、确认、授权、提交、任务完成，再独立收敛运行 |
| developer | 仅本地包明确选 LOCAL_DEVELOPER 后通过，Missing/未知签名不改成公开已验证 |
| revoked/migrated/tombstoned | 包括本地开发模式都阻塞 |
| trust-pending/signature-fail/owner-fail/document-fail | 不同检查维度与对应错误码 |
| content-missing/unsupported/config-invalid | 内容、目标、声明配置阻塞，离线不获取网络 |
| fail-download | 首次可重试失败，新任务带 retry_of |
| paused | Waiting，可取消或以原 ID resume |
| fail-committed | 配置已提交后失败，仅恢复调度 |
| stale | 第一次批准失效，重新检查、授权后可提交 |
| activation-fail | 安装提交成功，独立启动失败 |
| offline-node/runtime-timeout/runtime-unknown/stale-evidence | 无有效运行成功证据 |
| expired-reference/import-fail | 文件引用过期/导入失败，可更换来源 |
| upgrade/downgrade | 有旧版本时展示影响，禁止降级 |
| 列表 appServiceScenario=loading/empty/error | 数据读取基本状态 |
| appServiceScenario=operation-fail/operation-timeout | Start/Stop 请求失败与收敛超时 |

默认 Mock 管理员密码为 `prototype-admin`，`prototype-expired` 模拟过期 grant，其余密码报错。这些值仅为测试，不是系统凭据。`t-forbidden` 是无权限错误样本；未知 ID 不建任务。

模拟切换身份：通过测试或开发控制台设置 `buckyos.app-service.context.v4` 为 `{zone_id,user_id,role}` 后刷新，或派发该 key 的 storage 事件。此开关不应出现在真实产品授权链中。

AppDoc v1 输入预检样本：

```json
{
  "schema_version": 1,
  "doc_type": "app",
  "did": "did:bns:example.buckyos",
  "owner": "did:bns:buckyos",
  "controller": "did:bns:buckyos",
  "author": "did:bns:buckyos",
  "create_time": 1788566400,
  "last_update_time": 1788566400,
  "exp": 1893456000,
  "version": "1.0.0",
  "app_type": "web",
  "show_name": "Example",
  "selector_type": "static",
  "pkg_list": {
    "web": {
      "pkg_id": "example#1.0.0",
      "pkg_objid": "pkg:a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
      "required": true
    }
  },
  "service_config_tips": {}
}
```

此样本 ID 为占位内容标识，不提供签名或 payload，也不意味着可以安装。`id` 替代 `did`、doc_type=APPDOC、缺 schema_version 等旧输入均不被迁移。

## 8. KRPC 映射及未接入字段

| UI 模型/操作 | 当前读取或写入能力 | 集成要求 |
|---|---|---|
| SourceReference | apps.staging.finalize/status/release | 文件浏览/NDM/浏览器受控上传桥接；URL 的独立导入方；引用寿命与所有权 |
| AppDocumentView | apps.inspect 的 plan.app_doc 与应用引用 | 从经过后端验证的 AppDoc 派生，不使用浏览器文本解析证明信任 |
| InstallPlan | apps.inspect -> InstallInspection.plan | 读取不可变 plan/fingerprint；reserved task_id 不作为已存在任务展示 |
| PlanReadiness | InstallInspection.status、resolution_status | 把分维度 ready 与未知映射到 UI；补文档/签名/owner 展示证据 |
| InstallInput | InstallParams + InstallTarget + policy/offline | 服务、权限、三类 mount、bash_envs/auto_start 精确映射；未支持字段不假装保存 |
| submit | apps.submit 要求批准 fingerprint 与幂等键，可 satisfied/null | 前端确认与后端 sudo 强制校验闭环；返回身份是唯一任务依据 |
| retry/cancel/status | apps.install.retry/cancel/status | retry_of/返回 ID、commit boundary 动作；不要照搬所有非终态 Cancel |
| InstallTask 恢复 | status 已有阶段/readiness/错误摘要；TaskManager 有 input/progress/result | 补完整 Plan/AppDoc 展示投影、结果、commit point、尝试关联及访问权限 |
| InstallRecord/Registry | Scheduler 安装记录与应用投影 | 暴露只读分配结果、当前 deployment；Completed 不证明 node 就绪 |
| RuntimeView | 当前安装 status 不提供完整独立运行结果 | 聚合 node report、deployment 一致性、时效、离线、超时及类型诊断 |
| 全量管理列表 | apps.list 是授权可用集合 | 管理员 Zone 清单、owner、总数、分页和操作能力定义 |
| Start/Stop | 既有 app_mgr 模型与生命周期 API 可复用 | 权限及 operation 返回、状态收敛、失败/超时；本轮仅模拟 |
| Settings | 只读读取模型 | 需要保存端点、字段权限、校验、生效与重启语义 |
| Docker/日志 | 原型有类型投影 | 后端采集与跨节点读取、权限和未知状态 |

权威签名 envelope 与当前 APPDOC.jwt reader 的格式差异独立保留。详细后续清单在 PRD 第 10 节，不能由本轮 Mock 验收代替真实浏览器或 DV 验证。

## 9. 字段稳定性

| 分类 | 字段 |
|---|---|
| 语义稳定 | AppDID/AppInstanceId/SystemServiceId、owner_user_id、task_id:string、retry_of、phase/outcome、声明权限与三类 mount、deployment 证据独立性 |
| 可扩展 | RuntimeType 与未知 fallback、readiness/issues、runtime diagnostics、应用设置读取投影、节点能力 |
| Mock 实现细节 | fixture/scenario/tick、默认账号/节点、模拟字节数、TTL、缓存键版本、fingerprint 计算、时间间隔 |
| 待真实契约 | 全量管理分页、sudo 安装入口强制校验、恢复聚合快照、敏感 env secret reference、运行报告聚合、Settings 保存 |

“稳定”指本轮 UI 语义，不重新冻结后端尚缺的协议字段。

## 10. 变更与验证

2026-09-05：从 `mock/types.ts` 提取稳定类型至 `types.ts`；拆开 source/draft/plan/readiness/task/record/runtime；任务 ID 改为字符串；多任务缓存按 Zone/用户/模型隔离；新任务 retry_of；取消边界；独立运行证据；声明驱动输入 schema；统一 sudo Mock 适配；Task Center 共用身份；移除旧缓存迁移、假信任、假运行成功、任意 grant 和 Settings 假保存。

测试文件：`tests/e2e/pages/app-service.spec.ts` 与 `app-installer.spec.ts`。覆盖 UI 主路径及独立的 schema、并发幂等和异步检查失效验证；所有 UI 用例收集 pageerror、console.error 和意外 `/kapi/` 请求。截图与最终执行结果见[执行清单](../../../../../../notepads/app-service-beta2.2-prototype-todo.md)。
