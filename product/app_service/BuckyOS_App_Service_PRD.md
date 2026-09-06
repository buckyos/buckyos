# BuckyOS App Service PRD

- 版本：beta 2.2 原型基线，2026-09-05
- 状态：Mock 原型；真实接口集成单独验收
- 实现复核基线：`4f22b93a`
- 权威依据：[App 安装协议 beta 2.2](../../doc/App%20安装协议.md)
- 配套：[Sys Dlg](../BuckyOS%20Sys%20Dlg.md)、[UI DataModel](../../src/frame/desktop/src/app/app-service/UI_DATAMODEL.md)

## 1. 定位与边界

App Service 提供应用安装、部署与运行观测、基础控制及故障解释。它是生态冷启动阶段的系统管理入口，不承担商店发现、推荐、购买或运营。

| 维度 | Library | App Service |
|---|---|---|
| 主要任务 | 内容分发、购买、消费 | 安装、运行观测、启动与停止 |
| 使用者 | 内容生态参与者 | 普通用户、开发者、维护者 |
| 公开交付物 | 内容资产 | 应用包 PIKG；标识符用于发现应用 |
| 产品形态 | 生态入口 | 系统应用管理 |

本轮保留首页、详情、来源页、公共 App Installer、桌面窗口与移动端行为，更新状态、输入约束与产品文档。不修改权威安装协议、Rust 共享类型、安装引擎或 Scheduler。

不扩展完整版本管理、批量操作、卸载、商店、日志分析平台。升级仅从安装入口进入版本与影响确认。不实现新安装后端、URL 抓取服务、Docker 采集、Settings 保存或真实 DV 安装。

## 2. 身份与可见性

- `AppDID` 是产品身份，采用小写 hostname-form DID，如 `did:bns:nextcloud.buckyos`。
- `AppId` 是严格可逆的 raw hostname，如 `nextcloud.buckyos.bns.did`。
- `AppInstanceId = AppId@owner_user_id` 是应用卡片、详情、任务、安装记录的关联标识。同一 AppDID 的 alice 与 bob 安装分别显示。
- 发布对象的 `owner`、`controller`、`author` 与安装归属账号 `owner_user_id` 分开。发布者 DID 不能替代安装账号。
- `task_id` 是不透明字符串。TaskManager 当前生成 `t-<uuid>`；UI 不数值化、不以正 i64 范围校验。
- 系统服务使用真实 `SystemServiceId`（如 `scheduler`、`verify-hub`），不创建虚假的 AppInstanceId。
- AppName、AppHostName、AppIndex 是系统只读投影。分配前地址显示“由系统分配”，已有实例显示已分配地址；升级保留稳定的实例身份和投影。

管理员原型展示当前管理范围中的应用，包含多 owner 示例。普通用户只看自己的安装及获授权的系统信息，管理员操作不可用。当前 `apps.list` 是授权可用集合，不能声称为 Zone 全量安装清单。

本原型使用独立的 Mock 身份上下文，默认 `prototype-zone / alice / admin`，不读取真实凭据。切换模拟 Zone、用户或角色会切换缓存命名空间。真实集成必须从已认证 principal 获取归属与权限，不能信任浏览器缓存作为授权依据。

## 3. 信息架构与页面

首页保留三层结构：应用卡片优先、系统服务列表其次、内核只读信息最后。卡片显示名称、版本、安装归属、设备与运行状态；任务入口显示同一个真实 Mock task_id，不另造展示用任务。支持多个任务、关闭重开、草稿重新检查，以及 loading、empty、error、ready 状态。

应用详情保留状态、部署配置、Settings、运行信息及诊断区域。Spec 只读；Settings 当前只读展示，不提供假保存成功。无诊断数据显示未知/不可用，无日志时显示暂无日志。Start/Stop 仅在权限、操作能力及当前状态允许时启用；提交后等待状态收敛，失败或超时显示明确原因。

移动端使用单列布局、可滚动窗口、至少 44px 的操作区域；长 DID、task_id、权限路径与诊断字符串可换行。普通信息先展示，身份与底层阶段放入展开详情。中文与英文均覆盖新增流程。

## 4. 来源页

主入口是“选择应用包”：本地文件与 Personal Server。次入口为“输入链接或应用标识”。AppDoc 文本与 canonical Object ID 放在折叠高级导入中，普通用户不选择内部对象类型。

来源过程区分文件准备、上传/导入进度、成功、失败、引用过期与用户取消。只向用户显示文件名、真实已知大小、来源及可理解错误；公开 URL、Dialog 返回值及诊断不包含 staging handle、主机路径或文件内容。

本地文件不能仅根据 `.pikg` 后缀视为有效；HTTP URL 也不能默认归类为 App Meta。原型的本地文件检查使用有明确格式标记和内容字段的 Mock PIKG 样本，Personal Server 列表使用同一批固定样本。真实 PIKG 文件与任意 URL、Object ID、AppDoc 文本不会被虚构为导入成功：页面说明所缺桥接能力，允许改选来源。

离线模式禁止网络获取，但仍验证信任、内容、平台和配置。本地包只证明有文件引用；内容是否完整是独立条件，不能自动标记离线就绪。

## 5. 主流程

```text
选择应用包 / 输入标识
  → 准备并导入来源
  → Resolve / Inspect：检查草稿，无持久执行任务
  → 可安装：查看并配置计划
    已有相同内容：已安装 / 查看应用，不创建任务
    可升级：旧版本 → 新版本、替换影响与中断提示
    不支持降级 / 信任 / 内容 / 目标 / 配置阻塞：解释与修复动作
  → 配置变化：计划失效 → 重新检查
  → 最新计划与影响摘要 → 管理员授权 → 确认提交
  → submit 返回 task_id：进入任务执行
  → 安装提交结果 + 独立部署与运行视图
```

提交前不显示任务编号，也不显示“任务已在后台运行”。检查计划中的预留 task_id 不表示 TaskManager 已有任务，UI 草稿无需该字段。

提交前关闭表示退出准备并释放不再使用的导入引用；刷新后重新 Inspect，不恢复管理员授权。提交后关闭只转为后台查看，不等于取消。已有任务入口只恢复，缺失或无权限不创建替代任务。

同一次提交使用固定 fingerprint 与幂等键，并合并并发重复请求。来源或配置改变后必须形成新计划和新请求；旧批准不可复用。条件或计划过期显示“应用信息或安装条件已变化，请重新检查”，重新展示版本、权限、下载与影响，再次授权确认。

## 6. 应用检查

默认展示名称、版本、简介、运行类型、发布来源、安装账号、结论和待处理项。AppDID、`appdoc:<64 lowercase hex>`、发布 owner/controller/author、签名及权威证据放在高级详情。

文档有效性、签名、controller/owner 约束、权威发布、内容、目标、配置分别表达 READY / NOT_READY / UNKNOWN。未知不是通过。无价格信息不显示免费，无权威版本比较不显示 Latest，无大小依据不推测安装占用。不使用“Highly trusted”。

“本地开发安装”只向 LocalPikg 来源提供，默认仍为普通安装；缺少签名不会自动进入开发模式。用户明确选择并重新检查后，限定本地开发授权可接受符合策略的未发布包；页面保留 Missing/Unknown 等实际权威状态及未知签名状态。Revoked、Tombstoned、Migrated 不允许被开发模式绕过。开发安装不等于公开发布。

权威协议定义 detached signature envelope；当前 PIKG reader 仍支持 `APPDOC.jwt`。UI 依赖验证语义，不依赖三段 JWT 文本可解析这一旧假设，本轮不修改包格式。

## 7. 安装计划与最终确认

保持单列配置。顶部只读摘要显示安装账号、系统选定目标设备、OS/arch；高级选择来自当前节点清单，ID 与 DID 必须一致，不能固定成两台 OOD。

| 配置 | 原型行为与约束 |
|---|---|
| 组件 | 必需组件自动选择且不可取消；只有存在可选组件时显示选择，不展示 package matrix |
| 服务 | 根据 AppDoc endpoint 提供开关、暴露范围、声明的端口与访客访问；必需 endpoint 不能关闭，不任意改写协议或路由类型 |
| 权限 | 展示原始 `scope_path/actions/required`；必需项完整保留，可选项选择；不提供任意 grant 改写 |
| 持久数据 | 按 owner 与 App 隔离，逻辑目录跨升级稳定；说明数据归属 |
| 节点缓存 | 可清理，属于选定节点，不冒充持久数据 |
| 外部目录 | 按声明选择已有用户目录，保留原归属与访问模式，不任意输入主机路径 |
| 环境变量 | 仅编辑 AppDoc 声明；必需值不能为空；BUCKYOS 注入身份和路径不可覆盖 |
| 高风险参数 | `start_param/container_param` 只读说明，无编辑契约不增加输入框 |
| 自动启动 | 独立选择；关闭时安装成功也不能声称运行正常 |
| 系统投影 | AppName/AppHostName/AppIndex 和地址只读；未分配显示待系统分配 |
| 快捷域名 | 安装后的独立设置需求，不进入 InstallParams，也不模拟原子成功 |

表单使用 react-hook-form 与 Zod。未知节点、缺少必需组件/权限/挂载、越界端口、重复端口、路径穿越、系统环境覆盖等输入被拒绝；字段级错误可定位，修改后重新检查。处理中、失效或阻塞时不能提交。

最终摘要反映此次批准的目标、版本、组件、权限、三类挂载、服务暴露、环境变量、自动启动、离线策略、下载与影响。敏感环境变量仅显示掩码；fingerprint 放在诊断详情，用户不用输入或理解它。

最终按钮明确为“确认并安装”或“确认并升级”。统一 sudo 对话框通过 Mock 适配模拟密码错误、取消、授权过期与重试，测试不连接 Verify Hub。密码仅留在授权表单内存，授权 grant 仅留在此次操作内存，不进入任务、URL、缓存、日志或错误复制。敏感环境变量不持久化；草稿刷新后需重填。

## 8. 执行、失败和恢复

任务保留 schema_id、phase/outcome、stage、readiness、错误、可用动作、retry_of。未知枚举有未知视图，不推断成功，不提供危险操作。进度未知时展示阶段，不编造百分比。

| 时机 | 取消与恢复 |
|---|---|
| 检查/授权前 | 退出准备，释放来源引用；不创建取消任务 |
| 已提交任务，desired state 未提交 | 用户可显式取消；关闭继续后台执行 |
| desired_state_committed 后 | 不再允许取消；提示配置已提交、可后台查看 |
| 下载等可重试失败 | 跟随 retry 返回的新 task_id，保留 retry_of 和上一尝试 |
| 暂停 | resume 可以返回原 task_id |
| 已提交配置后调度失败 | 保留安装记录，重试继续后续调度，不能返回任意修改或更换来源 |
| 任务不存在/无权限 | 单独错误，无替代任务 |

失败动作依赖 retryable、修复动作与提交边界，不能无条件提供三个固定按钮。取消不承诺任意阶段回滚。错误复制使用固定安全字段白名单。

任务支持多个实例、刷新、关闭重开及直接链接。与 Task Center 共享相同的 task_id、phase/outcome 和实例身份；Mock Task Center 读取同一 Store 的投影，不创建另一份安装任务。

## 9. 安装结果与运行证据

Scheduler 发布 NodeConfig 后可以 Completed，这证明安装配置已发布，不能证明目标节点部署完成或健康启动。

| 用户状态 | 所需依据 |
|---|---|
| 配置已提交 / 部署中 | desired state 或 NodeConfig 已提交，尚未收到匹配部署的运行报告 |
| 已安装 / 启动中 | 收到匹配 deployment 的部署证据，健康启动仍待完成 |
| 运行正常 | 匹配当前 deployment、未过期的有效健康/可访问证据 |
| 未启动 | 部署完成且 auto_start=false，或停止操作已收敛 |
| 启动失败 | 部署结果保留，独立启动报告失败 |
| 运行状态未知 | 无报告、节点离线、超时、报告过期或不匹配 deployment |

`DeploymentIdentity` 包含 app_instance_id、task_id、AppDoc Object ID、spec_generation。旧部署证据不能证明新部署就绪；查看历史任务时不能借用后续部署的运行成功。

Docker 展示 Engine/Image/Container；Script 展示进程健康；静态 Web 展示部署可访问状态；Agent runtime 展示环境与 binding。`docker:null` 表示没有取得 Docker 数据，不能一律解释为原生服务。安装 Agent runtime 不创建 Agent，必须覆盖“运行时已安装但尚无 Agent binding”。

## 10. 当前能力与后续集成

本节以当前 Rust 源码为实现依据，以权威协议为目标约束；以下待办不因 Mock 测试通过而完成。

| 后续项 | 当前能力 | 必须补齐的接口/字段/行为 |
|---|---|---|
| 来源桥接 | staging finalize/status/release 已有；Control Panel 拒绝抓取客户端 URL；Object ID 要求本地内容 | 浏览器/文件浏览/NDM 上传导入桥接、受控引用的续期/释放、URL 导入方；不可把 URL 直接传给安装引擎抓取 |
| 签名封装 | AppDoc 定义 detached envelope；PIKG reader 仍处理 APPDOC.jwt | Builder/Reader/发布者共享验证和格式对齐；不能宣称 JWT 已全面移除 |
| sudo 强制校验 | 统一 sudo_by_password 组件存在 | apps.submit/confirm/retry 的强制提升权限校验、过期处理与 owner/scope 绑定 |
| 恢复读取快照 | apps.install.status 有阶段、readiness、错误摘要 | 返回完整确认所需 Plan/AppDoc 展示快照、commit point、retry_of、结果与可见性；不能声称已有接口包含所有这些字段 |
| 取消动作 | Scheduler 拒绝 desired_state_committed 后取消 | status 的 available_actions 按提交边界计算，而非为全部非终态添加 Cancel |
| 运行就绪聚合 | Completed 可以表示 NodeConfig 已发布 | 按 DeploymentIdentity 聚合节点部署/健康报告、时效、offline/timeout/unknown 及自动启动结果 |
| 全量管理清单 | apps.list 是授权可用集合 | 管理员 Zone 安装清单的权限、owner 选择、分页与总量语义 |
| Settings 保存 | 无通用保存契约 | 字段可编辑性、校验、保存接口、权限、重启/重部署/即时生效与失败语义 |
| Docker/跨节点诊断 | 页面已有只读信息模型 | 数据采集、节点与 deployment 绑定、日志读取权限和可用性，不能用空值证明健康 |
| 真实端到端验证 | 本轮仅 Mock | 真浏览器上传、授权、安装、取消/恢复与节点运行报告验证；单独安排，不启动或重装 DV |

## 11. 验收与变更记录

以两组 Playwright 用例覆盖普通 PIKG、显式 LocalDeveloper、禁止终止身份、重复/升级/降级、参数修改、PlanStale、错误密码/取消/过期、内容与信任阻塞、幂等并发、取消边界、失败重试、暂停恢复、刷新、任务权限、多 owner/用户/Zone、四种 runtime、运行未就绪、中英文和 375px 移动端。测试必须检查控制台错误与意外后端请求。

执行 `pnpm run check`、`pnpm run build` 和本轮 Mock 的 Chromium 回归；复用 4173 服务前核实其环境。关键截图和实际验证结果记录在[执行清单](../../notepads/app-service-beta2.2-prototype-todo.md)。

2026-09-05：替换旧 v0.3 / Draft v0.5 的原型语义；取消数字任务 ID、检查即创建任务、原编号重试、任意权限 grant、安装时快捷域名、假 Settings 保存及 Completed 即 running 的假设。不提供旧 Mock 缓存或 DataModel 迁移。
