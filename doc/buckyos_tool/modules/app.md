# App 模块需求

> 状态：Draft  
> 对应 module：`app`

## 1. 目标与边界

管理 App Catalog 信息、安装事务、已安装 App 和运行期望状态。命令语义对齐 `apt-get`：
用户给出 App 名称或安装来源，CLI 负责解析、确认和跟踪到完成；不实现安装、部署或调度
算法，只调用 Installer、Control Panel、TaskManager 和调度查询接口。

同一 App DID 在系统中只有一个 Instance。CLI 稳定选择器是 App 名称，不是
`app_id@owner_user_id`。下列形式都合法，且必须解析到同一个 App DID：

- `did:bns:app1.alice`
- `app1.alice`
- `app1.mysite.com`

普通 App 的本地/远程安装和升级以 PIKG 为唯一文件型输入；AppDoc、PackageMeta 和
payload 不作为多个需要用户分别提交的入口。`install` 对位置参数做来源自动识别，也可用
`--from` 精确指定类型。

本模块只做 App 自身的生命周期管理。App 业务设置由 App API 管理；可用范围、数据库
binding、Secret 解析和其它资源/权限管理不在本模块。Secret 只以 SecretRef 形式出现。

本模块不构建、重新封装、签名或发布 PIKG。本地构造和分析见 [PIKG 模块](pikg.md)；
发布职责边界见 § 8。

## 2. 资源模型

- Catalog：App DID/AppDoc，未安装也可以 `fetch`。
- Install Task/Operation：解析、预检、批准和提交过程。
- PIKG Source：本地文件或网络 URL 经 CLI 取字节并 stage 后形成的不可变 Installer
  输入，由 `pikg_digest` 固定。
- Installed App：稳定 ID 为 App DID。Zone 内同一 App DID 只有一个 Instance。
- Runtime Instance：scheduler/node-daemon 实际运行的实例。
- Status 必须分别展示 desired、task、scheduled、runtime 和 readiness。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `app fetch <app-name>` | read | sync | 从 Catalog 解析 AppDoc，不安装 |
| `app list` | read | sync | 列出已安装 App |
| `app get <app-name>` | read | sync | 获取安装与运行摘要，包含 AppDoc Object ID 和 PIKG 来源摘要 |
| `app dry-run --operation <op>` | read | task/either | install/upgrade/config/uninstall 无副作用预检和 diff |
| `app apply <operation-id>` | operation-defined | task | 按 dry-run 结果中的访问级别批准并执行 operation |
| `app install <source>` | write | task | 自动识别 Catalog 名、本地 PIKG 或网络 PIKG，创建安装事务；不跳过 inspect/confirm |
| `app upgrade [app-name]` | write | task | 无参时检查全部已安装 App 的新版本并 yes/no 升级；带 `<app-name>` 只检查该 App |
| `app uninstall <app-name>` | destructive | task | 必须指定 `--data <retain-or-delete>` |
| `app start <app-name>` | write | task | 进入期望运行态 |
| `app stop <app-name>` | write | task/either | 进入期望停止态 |
| `app restart <app-name>` | write | task | 明确 rolling 或 recreate 语义 |
| `app status [app-name]` | read | sync | 无参时汇总全部已安装 App；带名称时聚合该 App 的 desired/observed/task 与 PIKG 验证/readiness |

`<app-name>` 接受 § 1 列出的任一合法形式。CLI 在调用服务前归一成 App DID。

`install` / `upgrade` / `uninstall` / `start` / `stop` / `restart` / `apply` 的底层实现会返回
`task_id`。默认由本工具跟踪到完成；`--no-wait` 立即返回 `task_id`，之后用
[Task 模块](task.md) 的 `task get` / `task wait` 继续跟踪。见 § 6。

## 4. App 名称

Zone 内 App DID 与 Instance 一一对应，因此不需要 `app_instance_id`。用户输入按下列顺序
解析，命中即停：

1. 完整 DID，例如 `did:bns:app1.alice`；
2. BNS 短名，例如 `app1.alice`；
3. 站点/域名形式，例如 `app1.mysite.com`。

`get` / `status` / `upgrade` / `uninstall` / `start` / `stop` / `restart` 的目标必须是已安装
App，或对 `upgrade --from pikg` 能从 PIKG 读出并与目标 DID 对齐。`fetch` / `install` 的
Catalog 名可以指向尚未安装的 App。无法唯一解析时直接失败，不做模糊匹配，也不使用
“当前默认 App”。

## 5. 安装来源

### 5.1 `install` 自动识别

`app install <source>` 只有一个位置参数。未指定 `--from` 时按下列规则识别，命中即停：

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

示例：

```bash
buckyos app install app1.alice
buckyos app install did:bns:app1.alice
buckyos app install ./demo-0.1.0.pikg
buckyos app install --from pikg ./demo-0.1.0.pikg
buckyos app install --pikg ./demo-0.1.0.pikg
buckyos app install https://example.com/apps/demo-0.1.0.pikg
buckyos app install --from url https://example.com/apps/demo-0.1.0.pikg
```

### 5.2 客户端取字节与 staging

本地路径和网络 URL 都是 CLI 所在机器上的来源，不是 Control Panel/Installer 服务器的本地
路径。CLI 必须：

1. 将相对路径按 CLI 当前工作目录解析；只申请该显式文件或本次下载临时文件的读权限；
   对本地文件打开普通文件并做最小格式/大小预检；对 URL 由 CLI 发起 HTTP(S) GET，不得
   把客户端 path 或未校验 URL 交给服务端打开；
2. 以字节流上传到 Installer 的 staging 边界；
3. 计算本次字节快照的 digest，并与 staging 返回的 `pikg_digest` 比对；
4. 用 staging handle + digest 创建 dry-run/install/upgrade 事务；
5. 转交给 Installer 使用共享 PIKG verifier 重新完整验证。

本地 digest 和上传必须基于同一次打开的文件快照/句柄或同一次下载缓冲，防止 path/URL
内容在校验与上传之间被替换。首版网络来源只接受 `http://` 和 `https://`；不把服务端
本地路径解释为隐式输入。权威发布 URL、不可变版本路径和“AppDoc Object ID → PIKG URL”
映射见 § 8，本版不冻结临时 URL 约定。

上传失败、digest 不一致或命令取消时不得创建可 apply 的 operation。staging 产物必须有
过期和回收策略，不能由 CLI 输出的临时服务端路径充当稳定协议。

### 5.3 验证与信任

- `app install/upgrade` 使用 PIKG 时，不得因为开发者已执行 `pikg info` 而跳过 Installer
  验证。
- 公开安装同时验证 Owner 签名、BNS 当前 AppDoc、包内 Object ID、namespace 与 payload
  digest。
- 包内存在 `APPDOC.jwt` 不等于已发布；无法证明 BNS 当前权威状态时，公开安装必须拒绝。
- 未签名 `APPDOC.json` 只能在显式、受限、可撤销的 local developer authority 下安装；CLI
  不得自行创建、扩大或伪造该 authority。
- InstallPlan、install record 和 `app get/status` 必须保留 App DID、AppDoc Object ID、
  `pikg_digest`、签名/发布验证结果与安装来源。

### 5.4 升级

`app upgrade` 对齐 `apt-get upgrade`：

1. 无 `<app-name>` 时检查全部已安装 App 是否有 Catalog 新版本；
2. 带 `<app-name>` 时只检查该 App；
3. 列出可升级集合、版本 diff 和风险摘要；
4. 交互模式 yes/no 确认后执行；`--yes` 跳过本地确认；`--non-interactive` 且未给 `--yes`
   时，存在可升级项则返回 `CONFIRMATION_REQUIRED`，没有可升级项则成功返回空计划。

`app upgrade [app-name] --pikg <path>` / `--from pikg|url` 把新 PIKG 视为该 App 新版本的
完整安装输入。未给 `<app-name>` 时，目标 DID 必须能从 PIKG 内嵌 AppDoc 读出，且该 App
必须已安装。CLI 不比较、抽取或单独上传 subpackage；对象级去重、增量获取和内容复用属于
Installer/Repo 内部优化。新 PIKG 必须可独立验证，升级事务必须校验其 App DID 与目标
已安装 App 一致。

无参 `upgrade` 的范围是“全部已安装 App”，不是隐式“当前默认 App”。

## 6. 任务跟踪

会返回 Task 的 App 命令默认跟踪到完成，进度写 stderr / `jsonl`，stdout 输出终态
envelope。行为对齐 `apt-get`：调用方发出一条命令，默认等到安装或升级结束。

`--no-wait` 立即返回 `task_id` 和 task summary，不跟踪到完成。调用方随后使用：

```bash
buckyos task get <task-id>
buckyos task wait <task-id>
```

本地等待超时只停止等待，不隐式 `task cancel`。超时或 `--no-wait` 的结果里必须带上可继续
跟踪的 `task_id`。

## 7. 配置与安全

部署配置变更若走 `dry-run` / `apply`，必须携带 operation/revision，不能直接写 AppSpec。
本模块不提供 availability 或数据库 binding 的专用动词；这类能力属于对应的资源/权限模块。

默认输出不得包含 token、私钥或完整数据库连接串。Secret 只以 SecretRef 出现。

## 8. 实现基础

当前已有 apps.list/details、install/install_package/confirm/retry/cancel、update、uninstall、
start、stop 和 TaskManager 安装事务。新 CLI 应映射这些正式能力，并把选择器改为 App
名称；补齐 restart、统一 status、dry-run/apply、来源自动识别、客户端 PIKG staging、默认
任务跟踪和卸载数据保留语义。不再把 availability 或 db binding API 暴露为本模块命令。

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

- `app install <source>` 能自动识别 Catalog 名、本地 `.pikg` 和 HTTP(S) URL；`--from` /
  `--pikg` 可锁定类型。用户不必单独提供 AppDoc、PackageMeta 或 payload。
- `did:bns:app1.alice`、`app1.alice` 和 `app1.mysite.com` 解析到同一 App DID；已安装目标
  不再使用 `app_id@user`。
- `app fetch` 只取 Catalog AppDoc，不创建安装事务。
- `app upgrade` 无参检查全部已安装 App，带 `<app-name>` 缩小范围；确认前无升级副作用。
- 客户端 path 和 URL 不会被当作服务端 path，上传前后 digest 一致。
- 无签名 PIKG 在没有 local developer authority 时被拒绝，带签名但非 BNS 当前版本的 PIKG
  也不会被当作公开可安装版本。
- dry-run/apply 始终引用同一 staging handle、`pikg_digest` 和 AppDoc Object ID，避免
  TOCTOU。
- 返回 Task 的命令默认跟踪到完成；`--no-wait` 只返回 `task_id`，可用 `task wait` 续等。
- 本模块没有 availability、db-list/db-resolve、构建、重打包、单独 subpackage 发布或 BNS
  写入逻辑。

## 11. 待决策项

- `app install/upgrade/uninstall` 在 Agent / `--non-interactive` 下是直接执行，还是仍必须
  先 `dry-run` 再 `apply`。交互模式已按 apt-get 做成检查 + yes/no。
- restart 的默认策略及多实例可用性门槛。
- upgrade rollback 是重新部署旧 AppDoc，还是 Installer 的一等事务。
- Installer PIKG staging/upload 的 RPC 流式传输、断点续传、限额和回收协议。
- 远程 PIKG 的权威 URL 结构和“AppDoc Object ID → PIKG URL”映射协议。
