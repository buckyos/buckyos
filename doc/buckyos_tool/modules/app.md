# App 模块需求

> 状态：Frozen（beta 2.2 服务契约已落地）
> 对应 module：`app`

## 1. 目标与边界

管理 App Catalog 信息、首次安装计划、安装事务、已安装 App 和运行期望状态。CLI 负责解析
用户输入、读取本地或网络来源、交互构造首次安装计划、确认操作并跟踪到完成；不实现安装、
部署或调度算法，只调用 Installer、Control Panel、TaskManager 和调度查询接口。

App 的稳定用户选择器是 App 名称或 App DID，不要求用户手工输入
`AppInstanceId = app_id@owner_user_id`。同一 Owner 对一个 App DID 最多有一个 AppSpec；可见
多个同名安装而无法唯一确定目标时必须失败，不做模糊选择。

普通 App 的本地/远程安装和升级以 PIKG 为唯一文件型输入；AppDoc、PackageMeta 和 payload
不作为多个需要用户分别提交的入口。首次安装必须先通过 `fetch` 构造并保存 `InstallPlan`，
再将计划文件交给 `install --plan`。对已经安装的 App，`install` 不接受计划文件，而是基于
现有安装状态生成默认升级计划；相同版本不重装，低版本拒绝降级。

本模块只做 App 自身的生命周期管理。App 业务设置由 App API 管理；可用范围、数据库
binding、Secret 解析和其它资源/权限管理不在本模块。Secret 只以 SecretRef 形式出现。

本模块不构建、重新封装、签名或发布 PIKG。本地构造和分析见 [PIKG 模块](pikg.md)；
发布职责边界见 § 9。

## 2. 资源模型

- Catalog：App DID/AppDoc，未安装也可以 `fetch`。
- InstallPlan：首次安装的声明式 JSON 计划。它绑定 App 身份、来源快照、目标环境和用户选择，
  由 `fetch` 帮助用户构造，由 `install --plan` 消费。
- Install Task：`install` / `upgrade` / `uninstall` 在同一次命令里完成预检、确认和提交。
- PIKG Source：本地文件或网络 URL 经 CLI 读取后形成的不可变输入快照，由 `pikg_digest`
  固定；Catalog 来源由 AppDoc Object ID 固定。
- Installed App：产品身份为 AppDID，持久 key 为可逆 AppId，Owner 范围的运行目标为
  `AppInstanceId { app_id, owner_user_id }`。
- 默认 Web route label 是 Scheduler 从 `system/app_registry` 持久分配的 AppHostName；
  `status` 展示实际 `web_hosts`，CLI 不自行推导。
- Runtime Instance：scheduler/node-daemon 实际运行的实例。
- Status：分别展示 desired、task、scheduled、runtime 和 readiness，并区分目标版本与实际
  运行版本。

`InstallPlan` 是首次安装的必要输入，不是可反复执行的 operation-id，也不是升级参数。计划
文件可以交给用户审阅、版本控制或传递给另一个调用方，但只在完全相同的 Zone DID、owner
和 target snapshot 下可携带；每次安装仍必须重新验证身份、来源摘要、目标环境、权限
和有效性，换 Zone/用户/scope 必然使计划失效。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `app fetch <source> [--plan <path>]` | read/local-write | sync | 解析 AppDoc；给出 `--plan` 时通过交互或显式输入构造首次安装计划并写入本地文件 |
| `app list` | read | sync | 列出当前调用方可见的已安装 App |
| `app get <app-name>` | read | sync | 获取安装与运行摘要，包含 AppDoc Object ID 和 PIKG 来源摘要 |
| `app install <source> [--plan <path>]` | write | task/either | 未安装 App 必须给出计划；已安装 App 禁止给出计划并按默认升级计划处理 |
| `app upgrade [app-name]` | write | task/either | 无参时检查全部已安装 App 的 Catalog 更新；带名称时只检查一个 App |
| `app uninstall <app-name>` | destructive | task | 必须指定 `--data <retain-or-delete>` |
| `app start <app-name>` | write | task | 进入期望运行态 |
| `app stop <app-name>` | write | task/either | 进入期望停止态 |
| `app restart <app-name>` | write | task | 默认 recreate；当前 `--strategy rolling` 返回稳定 unsupported 错误 |
| `app status [app-name]` | read | sync | 聚合 desired、task、scheduled、runtime、目标版本、实际版本和 readiness |

`<source>` 接受 Catalog/App 名称、本地 PIKG 路径或 HTTP(S) PIKG URL。`<app-name>` 接受
§ 4 列出的任一合法形式。CLI 在调用服务前将名称归一为 App DID 和安装作用域，但不会在
存在歧义时自行选择实例。

`install` / `upgrade` / `uninstall` / `start` / `stop` / `restart` 实际发生变更时返回
`task_id`。默认由本工具跟踪到完成；`--no-wait` 立即返回 `task_id`，之后用
[Task 模块](task.md) 的 `task get` / `task wait` 继续跟踪。没有实际变更的 install/upgrade
返回同步的 `satisfied` 结果，不创建空任务。

本模块不提供独立的 `dry-run` / `apply` 动词。预检、确认和执行都发生在 `install` /
`upgrade` / `uninstall` 自己里面，见 § 5.7。

## 4. App 名称与安装作用域

CLI 不要求用户输入 `app_instance_id`。用户输入按下列顺序解析，命中即停：

1. 完整 DID，例如 `did:bns:app1.alice`；
2. 无点 BNS 短名，例如 `app1`；
3. 含点的权威域名别名，例如 `app1.mysite.com`。

含点裸名绝不直接拼成 `did:bns:*`。域名别名必须由 name-client 的 TXT 权威结果给出唯一
`buckyos-app-did=did:...`（也接受 `app-did=`）后才进入 App DID 解析；0 个结果失败，多个结果
返回 `AMBIGUOUS_APP_TARGET`。需要表达含点 BNS ID 时必须提交完整 `did:bns:...`：

- `did:bns:app1.alice`
- `app1.mysite.com`（仅当 TXT 唯一映射到上面的 DID）

名称解析与已安装目标选择是两个步骤：名称先解析成 App DID，再在当前调用方有权访问的安装
作用域中查找已安装 App。没有匹配、匹配超过一个或调用方无权确认目标时直接失败。

`get` / `status` / `upgrade` / `uninstall` / `start` / `stop` / `restart` 的目标必须是已安装
App。`fetch` 可以指向未安装 App。`install` 是否是首次安装，由当前作用域中是否已有相同
App DID 以及是否提供 `--plan` 共同决定，见 § 5.3。

## 5. 安装来源与 InstallPlan

### 5.1 来源自动识别

`app fetch <source>` 和 `app install <source>` 都只有一个来源位置参数。未指定 `--from` 时按
下列规则识别，命中即停：

1. `http://` 或 `https://` URL → 远程 PIKG；
2. 已存在的普通本地文件，或明显的本地路径（含 `/`、以 `.` 开头，或后缀为 `.pikg`）→
   本地 PIKG；路径存在但不是普通可读文件时直接失败，不得回退成 Catalog 名；
3. 其余 → Catalog / App 名称，按 § 4 解析。

`--from` 用于消除歧义，取值：

| `--from` | 含义 |
| --- | --- |
| `catalog` | `<source>` 是 App 名称 / DID |
| `pikg` | `<source>` 是 CLI 本机 `.pikg` 文件 |
| `url` | `<source>` 是 HTTP(S) PIKG URL |

`--pikg <path>` 是 `--from pikg` 的显式写法，与位置参数 `<source>` 互斥。`--from` 与自动
识别结果冲突时以 `--from` 为准；按 `--from` 无法打开或解析时直接失败，不得改走其它类型。

### 5.2 使用 `fetch` 构造首次安装计划

`app fetch <source>` 只解析来源并输出 Catalog/AppDoc 摘要，不创建安装任务。给出
`--plan <plan-json-path>` 时，它还负责帮助用户构造首次安装所需的 `InstallPlan`：

1. 解析来源并固定 App DID、AppDoc Object ID、版本以及 PIKG digest；
2. 取得 Installer 给出的默认计划；
3. 在交互模式中展示目标平台、组件、权限、mount、Service Settings、资源池和自动启动等
   可选项，用户直接接受默认值或修改允许调整的部分；
4. 重新计算计划摘要与 fingerprint；
5. 将最终计划以 JSON 写入 `--plan` 指定的本地路径。

`--plan` 在 `fetch` 中是输出文件，在 `install` 中是输入文件。相对路径都按 CLI 当前工作目录
解析。CLI 不得静默覆盖已经存在的计划文件；覆盖必须由通用的显式确认行为保护。

`--non-interactive` 下不得发起问答。调用方可通过通用 `--input` 提供计划选项；未提供的字段
使用 Installer 默认值。默认值仍不能形成完整、可安装计划时返回稳定的
`PLAN_INPUT_REQUIRED`，不得猜测权限、mount、SecretRef 或其它必要配置。

计划文件只保留不可变安装语义，至少包括：

- schema version 和 plan fingerprint；
- App DID、AppDoc Object ID、版本和权威解析摘要；
- 来源类型；PIKG 来源还必须包含 `pikg_digest`；
- 目标 OS、架构以及计划绑定的其它目标约束；
- 选择的组件、内容 identity、权限和安装参数；
- 最终运行配置、计划用途（`FreshInstall/Upgrade/Satisfied`）、安装作用域和创建时间。

`readiness`、content location/source、estimated bytes、target/config issue、权限候选、resolver
warning 与本次检查时间属于 `InstallInspection.status/resolution_status`，不写入长期 Plan。Acquire
只更新动态 status，不能修改已经批准的 immutable Plan 或 fingerprint。

计划文件不得包含 session token、私钥、明文 Secret、服务端临时路径或 staging handle。计划中
只能出现 SecretRef。`fetch --plan` 没有安装、调度或修改 Zone 期望状态的副作用。

### 5.3 首次安装与升级的判定

`--plan` 是否存在是首次安装与升级的明确分界，不提供 `--no-upgrade`：

| 当前作用域状态 | `--plan` | 目标版本 | `install` 结果 |
| --- | --- | --- | --- |
| 未安装 | 有 | 任意可安装版本 | 校验计划后首次安装 |
| 未安装 | 无 | 任意 | `PLAN_REQUIRED`，提示先执行 `fetch <source> --plan <path>` |
| 已安装 | 有 | 任意 | `PLAN_NOT_APPLICABLE`，首次安装计划不得用于覆盖现有安装 |
| 已安装 | 无 | 高于当前版本 | 使用默认升级计划升级 |
| 已安装 | 无 | 等于当前版本 | `satisfied`，不重装且不创建任务 |
| 已安装 | 无 | 低于当前版本 | `DOWNGRADE_NOT_ALLOWED` |

升级不接受首次安装参数或计划文件。默认升级计划必须以当前已安装状态为基础，保留用户已经
选择的组件、权限、mount、Service Settings、环境、资源池、实例数量和启停期望；只应用新
版本要求的必要变化。新增权限、不兼容配置或目标环境变化必须在确认摘要中明确展示，不能
静默重置成首次安装默认值。

版本判定必须同时固定权威 AppDoc identity 和 Installer 认定的版本。相同版本视为已满足，
不因为换了一个同版本 PIKG 而重装。不同 App DID 的计划或 PIKG 不得用于覆盖已安装目标。

示例：

```bash
# Catalog 首次安装：先生成计划，再用同一来源执行
buckyos app fetch did:bns:app1.alice --plan ./app1.install-plan.json
buckyos app install did:bns:app1.alice --plan ./app1.install-plan.json

# 本地 PIKG 首次安装
buckyos app fetch ./demo-0.1.0.pikg --plan ./demo.install-plan.json
buckyos app install ./demo-0.1.0.pikg --plan ./demo.install-plan.json

# 已安装 App：不带 --plan，按默认升级计划处理
buckyos app install did:bns:app1.alice
buckyos app install https://example.com/apps/app1-1.2.0.pikg
```

### 5.4 计划匹配、客户端取字节与 staging

本地路径和网络 URL 都是 CLI 所在机器上的来源，不是 Control Panel/Installer 服务器的本地
路径。执行 `fetch` 或 `install` 时，CLI 必须：

1. 将相对路径按 CLI 当前工作目录解析；只申请该显式文件或本次下载临时文件的读权限；
   对本地文件打开普通文件并做最小格式/大小预检；对 URL 由 CLI 发起 HTTP(S) GET，不得
   把客户端 path 或未校验 URL 交给服务端打开；
2. 对 PIKG 计算本次字节快照的 digest；首次安装时必须与计划中的 `pikg_digest` 一致；
3. 安装命令将字节流上传到 Installer 的 staging 边界，并比对 staging 返回的 digest；
4. 用 staging handle 重新绑定同一 digest/AppDoc Object ID，并与已确认计划创建同一个安装或
   升级事务；Plan 自身从不保存 handle；
5. Installer 使用共享 PIKG verifier 重新完整验证。

本地 digest 和读取必须基于同一次打开的文件快照/句柄或同一次下载缓冲，防止 path/URL 内容
在校验与上传之间被替换。Catalog 首次安装时，当前权威 AppDoc Object ID 必须与计划一致；
权威 revision、目标环境或计划依赖已经变化时返回 `PLAN_STALE`，要求重新 `fetch --plan`，
不得静默改写旧计划或降级执行。

上传失败、digest 不一致、计划不匹配或命令取消时不得提交安装/升级。staging 产物必须有
过期和回收策略，不能写入计划文件，也不能由 CLI 输出的临时服务端路径充当稳定协议。

### 5.5 验证与信任

- `fetch` 对计划来源做足以生成可信计划的验证；`install/upgrade` 仍必须重新执行完整验证，
  不得因为计划已经存在或开发者已执行 `pikg info` 而跳过验证。
- 公开安装同时验证 Owner 签名、BNS 当前 AppDoc、包内 Object ID、namespace 与 payload
  digest。
- 包内存在 `APPDOC.jwt` 不等于已发布；无法证明 BNS 当前权威状态时，公开安装必须拒绝。
- 未签名 `APPDOC.json` 只能在显式、受限、可撤销的 local developer authority 下安装；CLI
  不得自行创建、扩大或伪造该 authority。
- InstallPlan、install record 和 `app get/status` 必须保留 App DID、AppDoc Object ID、
  `pikg_digest`、签名/发布验证结果与安装来源。

### 5.6 `upgrade` 与来源驱动升级

`app install <source>` 不带 `--plan` 时负责单个、来源驱动的升级。`app upgrade [app-name]`
只检查已安装 App 的 Catalog 当前权威版本，不接收 PIKG 路径、URL 或 InstallPlan：

1. 无 `<app-name>` 时检查全部已安装 App；
2. 带 `<app-name>` 时只检查该 App；
3. 为每个目标生成基于当前安装状态的默认升级计划；
4. 列出可升级集合、版本 diff、权限/配置变化和风险摘要；
5. 交互模式确认后执行；`--yes` 跳过本地确认；`--non-interactive` 且未给 `--yes` 时，
   存在实际升级则返回 `CONFIRMATION_REQUIRED`，没有升级项则成功返回空计划。

需要用指定 PIKG 或 URL 升级单个 App 时使用 `app install <source>`，且不得给出 `--plan`。
来源中的 App DID 必须与已安装目标一致。无参 `upgrade` 的范围是“当前调用方可管理的全部
已安装 App”，不是隐式“当前默认 App”。

### 5.7 预检、确认和执行

`install` / `upgrade` / `uninstall` 在一条命令里完成预检、确认和执行，不拆成独立的
`dry-run` 和 `apply` 动词。

首次安装：

1. 读取来源与 `--plan` 文件；
2. 验证计划、来源 identity/digest、目标环境和 readiness；
3. 展示计划将产生的变化；
4. 用户确认后提交同一份已验证计划和来源快照，并跟踪到完成。

升级：

1. 解析来源和当前已安装状态；
2. 生成保留当前设置的默认升级计划；
3. 展示版本、权限、配置和运行风险 diff；
4. 用户确认后提交该默认计划，并跟踪到目标版本实际就绪。

交互模式使用 yes/no 确认；`--yes` 跳过本地确认。`--non-interactive` 且未给 `--yes` 时，
存在实际变更则返回 `CONFIRMATION_REQUIRED`。确认必须绑定所展示计划的 fingerprint；来源、
目标或计划发生变化后必须重新预检和确认。

`--dry-run` 只完成预检并打印最终计划，不确认、不提交、不创建远程任务，也不产生需要再次
`apply` 的 operation-id。首次安装的 `--dry-run` 仍要求提供 `--plan`。

```bash
buckyos app install did:bns:app1.alice --plan ./app1.install-plan.json
buckyos app install did:bns:app1.alice --yes
buckyos app install did:bns:app1.alice --dry-run
```

## 6. 任务跟踪与完成条件

会返回 Task 的 App 命令默认跟踪到完成，进度写 stderr / `jsonl`，stdout 输出终态
envelope。调用方发出一条命令，默认等到安装、升级或生命周期变更真正完成。

`--no-wait` 立即返回 `task_id` 和 task summary，不跟踪到完成。调用方随后使用：

```bash
buckyos task get <task-id>
buckyos task wait <task-id>
```

本地等待超时只停止等待，不隐式 `task cancel`。超时或 `--no-wait` 的结果里必须带上可继续
跟踪的 `task_id`。发起操作的用户必须能够读取并继续其任务；确认、重试、取消和等待使用
同一任务身份与权限边界。

安装或升级任务只有在目标 `DeploymentIdentity {app_instance_id, task_id,
app_doc_object_id, spec_generation, pikg_digest}` 已进入 scheduled，且约定实例数全部提供新鲜、
健康、epoch/session 匹配的 runtime evidence 时才能成功。Static Web 还要求目标内容已物化且
cyfs-gateway ack 对应 config generation。旧版本 Started、目录或路由不能满足条件。当前升级
切换是 in-place/recreate，可能有停机窗口；Activate 失败恢复 previous spec 后还必须等待
previous deployment 重新就绪，且结果不能把 rolled back 写成目标升级成功。

## 7. 配置与安全

本模块不直接写 AppSpec，安装和升级都走 Installer。不提供 availability 或数据库 binding
的专用动词；这类能力属于对应的资源/权限模块。

默认输出和 InstallPlan 不得包含 token、私钥、明文 Secret、完整数据库连接串或服务端临时
路径。Secret 只以 SecretRef 出现。计划文件是普通本地文件，CLI 必须使用仅当前用户可读写
的默认权限创建，并在输出中明确其路径和 fingerprint。

## 8. 服务能力要求

支撑本模块的系统服务必须提供以下稳定能力，CLI 不得通过解析内部存储或拼装调度状态代替：

- 解析 Catalog、PIKG 和 URL 来源并返回不可变 App 身份摘要；
- 返回可交互调整的首次安装默认计划，并验证最终 InstallPlan/fingerprint；
- 根据已安装状态生成保留现有设置的默认升级计划；
- 区分首次安装、可升级、已满足、拒绝降级和计划失效；
- 对客户端 PIKG 字节提供 digest 绑定的 staging 边界；
- 返回用户可读、机器可解析的计划、任务、验证、调度和运行状态；
- 让发起者能够确认、跟踪、重试和取消自己的任务；
- 以目标版本运行证据作为安装/升级成功条件，并在失败时提供明确回滚结果；
- 为 install/update/uninstall/start/stop/restart 和 Catalog batch upgrade 提供可恢复、可继续
  跟踪的 TaskMgr 2.0 语义；retry 新建带 `retry_of` 的 task，不复活 Terminal task。

CLI 可以组合这些能力完成同一条用户命令，但不能自行比较并决定权威版本、构造运行 spec、
绕过验证、直接修改系统真相源或把内部 Task 数据格式当作稳定接口。

## 9. 发布职责边界

本版不设计发布阶段命令，并明确不新增 `app publish`。App 模块管理安装与运行期望状态，
不应同时承担发行物托管和 BNS 权威发布。

后续方向是把三种权限分开设计：

- PIKG 字节的上传与可下载性校验属于 Repo/PIKG Provider 领域，`repo publish <pikg>` 只是
  候选命名，当前不注册该命令；
- 对 PIKG 内嵌 AppDoc 的 Owner 签名属于 Publisher/signing 领域，其授权不能从 Repo 上传权
  或 BNS 发布权推导；
- BNS revision 更新和发布后回读属于 BNS 领域，必须使用独立设计的 BNS 命令和授权，
  不能从“能签名 AppDoc”或“能上传 PIKG”推导“能更新 BNS”。

在非 BuckyOS 系统上使用 `cyfs-dir-server` 托管 PIKG 的上传手册，等正式 URL 结构、不可变
版本路径和“AppDoc Object ID → PIKG URL”映射协议确定后再补充。本版不冻结临时 URL 约定。

## 10. 验收标准

- `app fetch <source>` 能解析 Catalog 名、本地 `.pikg` 和 HTTP(S) URL；给出 `--plan` 时能
  通过交互或显式输入生成完整 JSON InstallPlan，且不产生安装副作用。
- 首次安装未提供 `--plan` 时稳定返回 `PLAN_REQUIRED`；计划中的 App identity、来源 digest
  或目标环境不匹配时拒绝执行，不静默修改计划。
- 已安装 App 使用 `app install <source>` 且不带 `--plan` 时，只升级、不降级、相同版本不
  重装；带 `--plan` 时返回 `PLAN_NOT_APPLICABLE`。
- 升级默认计划保留当前组件、权限、mount、Service Settings、环境、资源池、实例数量和
  启停期望；新增权限或不兼容变化在确认前明确展示。
- App 名称先解析到 App DID，再在调用方可访问的安装作用域中唯一选择目标；CLI 不要求用户
  输入 `app_id@user`，但遇到多目标时不会猜测。
- 客户端 path 和 URL 不会被当作服务端 path；`fetch`、计划、staging 和最终事务使用一致的
  AppDoc Object ID / `pikg_digest`。
- 无签名 PIKG 在没有 local developer authority 时被拒绝，带签名但非 BNS 当前版本的 PIKG
  也不会被当作公开可安装版本。
- `install` / `upgrade` / `uninstall` 在同一次命令内完成预检、确认和提交；`--dry-run` 只
  打印最终计划，不创建远程任务。
- 安装/升级 Task 只有在 scheduled 和 runtime 都证明目标版本就绪后才成功；旧版本状态不能
  误报为目标版本成功。
- 返回 Task 的命令默认跟踪到完成；`--no-wait` 只返回 `task_id`，发起者可用 `task wait`
  续等，也可按权限确认、重试或取消。
- 本模块没有独立 `dry-run`/`apply`、availability、db-list/db-resolve、构建、重打包、单独
  subpackage 发布或 BNS 写入逻辑。

## 11. 已冻结与待决策项

- restart 默认 `recreate`；当前稳定拒绝 `rolling`，所有约定实例必须按目标 deployment
  readiness 收敛。
- upgrade rollback 是 Installer 一等事务：恢复冻结 previous spec/record 并等待 previous
  deployment；它不承诺逆转业务数据迁移。
- App staging 已冻结不可猜测 handle、principal/Zone/purpose/digest 绑定、TTL、分层限额、
  lease 引用保护、release 与启动 GC；上传字节仍复用 NDN 通道。
- 当前 Plan schema 是严格 v4，unknown field、旧 schema、source/scope/target/config/fingerprint
  变化都使计划失效；fingerprint 是 JCS 等值/完整性标识，不是授权凭据。
- 尚待 Catalog/Repo 另行冻结远程 PIKG 的权威 URL 结构和“AppDoc Object ID → PIKG URL”映射；
  这不改变 URL 必须由 CLI 下载字节、Control Panel 不打开客户端 URL 的边界。
