# App 模块需求

> 状态：Draft  
> 对应 module：`app`

## 1. 目标与边界

管理 App Catalog 信息、安装事务、已安装 AppSpec、运行实例和 App–User 可用范围。CLI 只调用
Installer、Control Panel、TaskManager 和调度查询接口，不实现安装、部署或调度算法。

App 自身业务设置由 App API 管理，不包装成通用系统配置。Secret 只以 SecretRef 形式出现。

## 2. 资源模型

- Catalog：App DID/AppDoc，未安装也可以查询。
- Install Task/Operation：解析、dry-run、批准和提交过程。
- Installed App：稳定 ID 为 `app_instance_id=<app_id>@<owner_user_id>`。
- Runtime Instance：scheduler/node-daemon 实际运行的实例。
- Status 必须分别展示 desired、task、scheduled、runtime 和 readiness。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `app catalog-get <identifier>` | read | sync | 解析未安装 AppDoc |
| `app list` | read | sync | 列出当前用户可见的安装实例 |
| `app get <app-instance-id>` | read | sync | 获取安装与运行摘要 |
| `app dry-run --operation <op>` | read | task/either | install/upgrade/config/uninstall 无副作用预检和 diff |
| `app apply <operation-id>` | operation-defined | task | 按 dry-run 结果中的访问级别批准并执行 operation |
| `app install <identifier>` | write | task | 创建安装事务，不跳过 inspect/confirm |
| `app upgrade <app-instance-id>` | write | task | 创建升级事务 |
| `app uninstall <app-instance-id>` | destructive | task | 必须指定 `--data <retain-or-delete>` |
| `app start <app-instance-id>` | write | task | 进入期望运行态 |
| `app stop <app-instance-id>` | write | task/either | 进入期望停止态 |
| `app restart <app-instance-id>` | write | task | 明确 rolling 或 recreate 语义 |
| `app status <app-instance-id>` | read | sync | 聚合 desired/observed/task 状态 |
| `app availability-get <app-instance-id>` | read | sync | 获取可用范围策略 |
| `app availability-set <app-instance-id>` | write | sync | 使用 revision/CAS 修改策略 |
| `app availability-check <app-instance-id>` | read | sync | 解释某用户是否可使用 |
| `app db-list <app-instance-id>` | read | sync | 列出声明的数据库 instance |
| `app db-resolve <app-instance-id>` | privileged | sync | 返回脱敏 binding 或 SecretRef |

## 4. 配置与安全

部署配置包括资源、replica、placement、endpoint、mount 和数据库声明，修改后可能触发 scheduler
重新调度。`app apply` 必须携带 operation/revision，不能直接写 AppSpec。

数据库连接串可能包含凭证。默认输出只显示 backend、instance、schema version 和脱敏 endpoint；
完整连接信息只能以受控文件、env export 或 SecretRef 交付，不能进入日志。

## 5. 实现基础

当前已有 apps.list/details/availability、install/install_package/confirm/retry/cancel、update、
uninstall、start、stop 和 TaskManager 安装事务。新 CLI 应映射这些正式能力并补齐 restart、
统一 status、dry-run/apply 和数据保留语义。

## 6. 待决策项

- `app install/upgrade/uninstall` 是仅供交互使用的 dry-run + apply 组合入口，还是所有调用方都
  必须显式执行两条命令。
- restart 的默认策略及多实例可用性门槛。
- upgrade rollback 是重新部署旧 AppDoc，还是 Installer 的一等事务。
