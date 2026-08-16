# Group 模块需求

> 状态：Draft  
> 对应 module：`group`

## 1. 目标与边界

管理 Self-Host-Group 的 GroupDoc、成员、角色、成员证明、策略和 subgroup。Group 是 DID 与
可验证状态资源，不是一个进程，因此不提供含义错误的 `group start/stop`。

如果需要暂停群消息，使用 freeze/close 或消息策略；如果 GroupMgr/MessageHub 服务本身需要
重启，由 [System 模块](system.md)管理服务生命周期。

## 2. 资源模型

- Group DID / GroupDoc 和业务 revision；
- Owner/Admin/Member/Guest 等角色；
- invite、join request、GroupMemberProof；
- 独立 Group 作为 nested member；
- parent 内部 subgroup。二者不得混为同一种对象。

## 3. 初始命令

| 命令 | 访问级别 | 说明 |
| --- | --- | --- |
| `group create` | write | 创建 GroupDoc 和 owner member |
| `group get <group-did>` | read | 获取文档、策略和状态摘要 |
| `group update <group-did>` | write | 修改 profile 或允许的策略 |
| `group freeze <group-did>` | privileged | 暂停接收新的领域写入 |
| `group unfreeze <group-did>` | privileged | 恢复写入 |
| `group close <group-did>` | destructive | 关闭但保留可验证历史 |
| `group invite <group-did>` | write | 邀请成员 |
| `group join-request <group-did>` | write | 申请加入 |
| `group approve <group-did>` | write | 批准申请或 proof |
| `group reject <group-did>` | write | 拒绝申请或 proof |
| `group member-list <group-did>` | read | 分页列出成员和 proof 状态 |
| `group member-remove <group-did>` | write | 移除成员 |
| `group member-role-set <group-did>` | privileged | 更新角色 |
| `group subgroup-create <group-did>` | write | 创建 parent 内 subgroup |
| `group subgroup-update <group-did>` | write | 修改 subgroup |
| `group subgroup-list <group-did>` | read | 列出 subgroup |
| `group expand <group-did>` | read | 有界展开 nested groups |
| `group parent-list <group-did>` | read | 查询 parent group |
| `group access-check <group-did>` | read | 解释某 DID 的有效权限 |

## 4. 实现基础与待决策

当前 GroupMgr 已有 create/get/update、collection/attribution policy、invite/proof/join/approve/
reject、member role、subgroup、expand、parent 和 access check。freeze/close 是新增领域语义，
不能只在 CLI 本地模拟。

- GroupDoc 删除、关闭和归档的最终关系需要协议化。
- nested independent group 与 subgroup 的 CLI selector 需要稳定 schema。
- 跨 Zone proof 发布和本地投影的 task 边界需要明确。
