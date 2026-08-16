# User 模块需求

> 状态：Draft  
> 对应 module：`user`

## 1. 目标与边界

管理 Zone 内用户账户、用户类型、状态、Profile、密码和用户自己的 Message Tunnel binding。

本模块不提供任意 RBAC 编辑器。权限必须拆分为：

- `UserType` 驱动的系统角色；
- [App 模块](app.md) 管理的实例可用范围；
- [Content 模块](content.md) 和 [Files 模块](files.md) 管理的资源 ACL。

Profile/contact 中用户可编辑的 `groups` 不能成为系统 RBAC 或 App 授权来源。

## 2. 资源模型

- 稳定 selector：`user_id`。
- 账户状态至少区分 Active、Suspended、Deleted、Banned。
- 删除是软删除；数据清理、App 归属迁移和彻底擦除必须是独立计划。
- 用户公开 Profile、私有 Profile、系统角色、密码和外部账号 binding 是不同资源。

## 3. 初始命令

| 命令 | 访问级别 | 说明 |
| --- | --- | --- |
| `user list` | read | 分页列出用户 |
| `user get <user-id>` | read | 获取账户、类型和状态摘要 |
| `user create` | privileged | 创建本地用户或发起邀请 |
| `user update <user-id>` | write | 修改允许修改的账户字段 |
| `user profile-get <user-id>` | read | 获取权限范围内的 Profile |
| `user profile-set <user-id>` | write | 修改自己的 Profile，Admin 代操作需审计 |
| `user invite-create` | privileged | 创建邀请 |
| `user invite-accept` | write | 接受邀请并完成账户创建 |
| `user change-type <user-id>` | privileged | 修改 Admin/User/Limited/Guest |
| `user change-state <user-id>` | privileged | suspend/resume/ban |
| `user delete <user-id>` | destructive | 软删除，不能删除 root 或当前操作者 |
| `user change-password` | write | 当前用户重新认证后修改密码 |
| `user reset-password <user-id>` | privileged | Admin + scoped sudo 重置密码 |
| `user revoke-sessions <user-id>` | privileged | 使现有 session/refresh token 失效 |
| `user tunnel-list` | read | 列出当前用户 Message Tunnel binding |
| `user tunnel-bind` | write | 绑定并验证外部账号 |
| `user tunnel-unbind` | write | 解除外部账号绑定 |

密码不得作为 argv 参数；重置密码后默认撤销旧 session。

## 4. 服务映射与实现基础

优先使用 Control Panel UserMgr 和 verify-hub。当前已有 list/get/create/update、Profile、邀请、
软删除、change-state、change-type、change-password 和 Message Tunnel set/remove，可作为协议
收敛基础，但 CLI 不直接操作 `users/<id>/*` 配置路径。

## 5. 验收重点

- self、Admin、sudo 三种调用路径有独立测试。
- `user list` 支持分页和结构化过滤。
- 软删除的输出必须明确 `state=deleted`，不能声称数据已擦除。
- 输出永不包含 password hash、private key 和完整外部账号 secret。

## 6. 待决策项

- 创建用户是直接创建、只允许邀请，还是按 UserType 区分。
- session revoke 的正式 verify-hub 协议。
- Message Tunnel 绑定验证与 secret store 的服务边界。
