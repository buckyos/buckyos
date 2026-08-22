# App 模块需求

> 状态：Draft  
> 对应 module：`app`

## 1. 目标与边界

管理 App Catalog 信息、安装事务、已安装 AppSpec、运行实例和 App–User 可用范围。CLI 只调用
Installer、Control Panel、TaskManager 和调度查询接口，不实现安装、部署或调度算法。

普通 App 的本地安装和升级以 PIKG 为唯一文件型输入；AppDoc、PackageMeta 和 payload 不作为
多个需要用户分别提交的入口。App 自身业务设置由 App API 管理，不包装成通用系统配置。
Secret 只以 SecretRef 形式出现。

本模块不构建、重新封装、签名或发布 PIKG。本地构造和分析见 [PIKG 模块](pikg.md)；
发布职责边界见 § 7。

## 2. 资源模型

- Catalog：App DID/AppDoc，未安装也可以查询。
- Install Task/Operation：解析、dry-run、批准和提交过程。
- PIKG Source：本地 `.pikg` 文件经 CLI 上传/stage 后形成的不可变 Installer 输入，由
  `pikg_digest` 固定字节。
- Installed App：稳定 ID 为 `app_instance_id=<app_id>@<owner_user_id>`。
- Runtime Instance：scheduler/node-daemon 实际运行的实例。
- Status 必须分别展示 desired、task、scheduled、runtime 和 readiness。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `app catalog-get <identifier>` | read | sync | 解析未安装 AppDoc |
| `app list` | read | sync | 列出当前用户可见的安装实例 |
| `app get <app-instance-id>` | read | sync | 获取安装与运行摘要，包含 AppDoc Object ID 和 PIKG 来源摘要 |
| `app dry-run --operation <op>` | read | task/either | install/upgrade/config/uninstall 无副作用预检和 diff；install/upgrade 支持 `--pikg <path>` |
| `app apply <operation-id>` | operation-defined | task | 按 dry-run 结果中的访问级别批准并执行 operation |
| `app install <identifier>` | write | task | 从 Catalog/App DID 创建安装事务，不跳过 inspect/confirm；与 `--pikg` 互斥 |
| `app install --pikg <path>` | write | task | 上传并 stage 本地 PIKG，再创建同一标准安装事务 |
| `app upgrade <app-instance-id>` | write | task | 从权威 Catalog 当前版本创建升级事务；可用 `--pikg <path>` 显式指定新 PIKG |
| `app uninstall <app-instance-id>` | destructive | task | 必须指定 `--data <retain-or-delete>` |
| `app start <app-instance-id>` | write | task | 进入期望运行态 |
| `app stop <app-instance-id>` | write | task/either | 进入期望停止态 |
| `app restart <app-instance-id>` | write | task | 明确 rolling 或 recreate 语义 |
| `app status <app-instance-id>` | read | sync | 聚合 desired/observed/task 状态及 PIKG 验证/readiness 状态 |
| `app availability-get <app-instance-id>` | read | sync | 获取可用范围策略 |
| `app availability-set <app-instance-id>` | write | sync | 使用 revision/CAS 修改策略 |
| `app availability-check <app-instance-id>` | read | sync | 解释某用户是否可使用 |
| `app db-list <app-instance-id>` | read | sync | 列出声明的数据库 instance |
| `app db-resolve <app-instance-id>` | privileged | sync | 返回脱敏 binding 或 SecretRef |

## 4. PIKG 输入语义

### 4.1 本地文件上传

`--pikg <path>` 是 CLI 所在机器的文件路径，不是 Control Panel/Installer 服务器的本地路径。CLI 必须：

1. 将相对路径按 CLI 当前工作目录解析，只申请该显式文件的读权限，打开普通文件并执行最小
   格式/大小预检；
2. 以字节流上传到 Installer 的 staging 边界，不得把客户端 path 原样传给服务端打开；
3. 计算本地 digest，并与 staging 返回的 `pikg_digest` 比对；
4. 用 staging handle + digest 创建 dry-run/install/upgrade 事务；
5. 转交给 Installer 使用共享 PIKG verifier 重新完整验证。

本地 digest 和上传必须基于同一次打开的文件快照/句柄，防止 path 在校验与上传之间被替换。
首版 `--pikg` 只接受本地文件，不将 HTTP(S) URL 或服务端路径解释为隐式输入；远程 PIKG URL
在发布 URL 协议确定后另行设计。

上传失败、digest 不一致或命令取消时不得创建可 apply 的 operation。staging 产物必须有过期和
回收策略，不能由 CLI 输出的临时服务端路径充当稳定协议。

### 4.2 验证与信任

- `app install/upgrade --pikg` 不得因为开发者已执行 `pikg info` 而跳过 Installer 验证。
- 公开安装同时验证 Owner 签名、BNS 当前 AppDoc、包内 Object ID、namespace 与 payload digest。
- 包内存在 `APPDOC.jwt` 不等于已发布；无法证明 BNS 当前权威状态时，公开安装必须拒绝。
- 未签名 `APPDOC.json` 只能在显式、受限、可撤销的 local developer authority 下安装；CLI 不得自行
  创建、扩大或伪造该 authority。
- InstallPlan、install record 和 `app get/status` 必须保留 App DID、AppDoc Object ID、`pikg_digest`、
  签名/发布验证结果与安装来源。

### 4.3 升级

`app upgrade <app-instance-id> --pikg <path>` 把新 PIKG 视为新版本的完整安装输入。CLI 不比较、抽取或单独上传
subpackage；对象级去重、增量获取和内容复用属于 Installer/Repo 内部优化。新 PIKG 必须可独立验证，
且升级事务必须校验其 App DID 与目标 `app_instance_id` 一致。

## 5. 配置与安全

部署配置包括资源、replica、placement、endpoint、mount 和数据库声明，修改后可能触发 scheduler
重新调度。`app apply` 必须携带 operation/revision，不能直接写 AppSpec。

数据库连接串可能包含凭证。默认输出只显示 backend、instance、schema version 和脱敏 endpoint；
完整连接信息只能以受控文件、env export 或 SecretRef 交付，不能进入日志。

## 6. 实现基础

当前已有 apps.list/details/availability、install/install_package/confirm/retry/cancel、update、
uninstall、start、stop 和 TaskManager 安装事务。新 CLI 应映射这些正式能力并补齐 restart、
统一 status、dry-run/apply、客户端 PIKG staging 和数据保留语义。

## 7. 发布职责边界

本版不设计发布阶段命令，并明确不新增 `app publish`。App 模块管理安装与运行期望状态，
不应同时承担发行物托管和 BNS 权威发布。

后续方向是把三种权限分开设计：

- PIKG 字节的上传与可下载性校验属于 Repo/PIKG Provider 领域，`repo publish <pikg>` 只是候选命名，
  当前不注册该命令；
- 对 PIKG 内嵌 AppDoc 的 Owner 签名属于 Publisher/signing 领域，其授权不能从 Repo 上传权或
  BNS 发布权推导；
- BNS revision 更新和发布后回读属于 BNS 领域，必须使用独立设计的 BNS 命令和授权，
  不能从“能签名 AppDoc”或“能上传 PIKG”推导“能更新 BNS”。

在非 BuckyOS 系统上使用 `cyfs-dir-server` 托管 PIKG 的上传手册，等正式 URL 结构、不可变版本
路径和“AppDoc Object ID → PIKG URL”映射协议确定后再补充。本版不冻结临时 URL 约定。

## 8. 验收标准

- `app install --pikg` 和 `app upgrade <app-instance-id> --pikg` 均只上传一个 PIKG，不要求用户单独提供
  AppDoc、PackageMeta 或 payload。
- 客户端 path 不会被当作服务端 path，上传前后 digest 一致。
- 无签名 PIKG 在没有 local developer authority 时被拒绝，带签名但非 BNS 当前版本的 PIKG
  也不会被当作公开可安装版本。
- dry-run/apply 始终引用同一 staging handle、`pikg_digest` 和 AppDoc Object ID，避免 TOCTOU。
- App 模块没有构建、重打包、单独 subpackage 发布或 BNS 写入逻辑。

## 9. 待决策项

- `app install/upgrade/uninstall` 是仅供交互使用的 dry-run + apply 组合入口，还是所有调用方都
  必须显式执行两条命令。
- restart 的默认策略及多实例可用性门槛。
- upgrade rollback 是重新部署旧 AppDoc，还是 Installer 的一等事务。
- Installer PIKG staging/upload 的 RPC 流式传输、断点续传、限额和回收协议。
