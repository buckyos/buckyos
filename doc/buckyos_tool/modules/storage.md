# Storage 模块需求

> 状态：Draft  
> 对应 module：`storage`

## 1. 目标与边界

管理运行目录发现、存储根、容量和外部目录 mount。它是节点级资源管理，不承担文件内容 CRUD
或 NamedData 存储协议。

## 2. 资源模型

- semantic path：system/user/app/data/cache/log 等逻辑目录；
- storage root/device 和 capacity；
- mount spec：host source、logical target、read-only/read-write、owner、persistence、revision；
- mount observed state 和使用它的 App/Service。

不能把 raw Host path 当作跨平台稳定 API。路径查询应同时返回 semantic name、当前节点和实际
resolved path；普通用户默认只能看到其有权限的逻辑路径。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `storage path-list` | read | sync | 获取 system/user/app 逻辑目录 |
| `storage root-list` | read | sync | 列出可用存储根和容量 |
| `storage status` | read | sync | 容量、健康和告警 |
| `storage mount-list` | read | sync | desired/observed mount 状态 |
| `storage dry-run --operation <op>` | privileged | sync/task | 对 mount/unmount 校验 source、target、权限和冲突 |
| `storage apply <operation-id>` | operation-defined | task | 按 mount/unmount 风险级别通过 HostControl 执行 |

## 4. 安全与平台

- mount 默认 read-only，提升为 read-write 必须显式声明。
- source 必须在 Host allowlist 内，禁止 `..`、未解析 symlink 和任意 root mount。
- 容器内 TS 不直接 mount Host；统一调用 HostControlClient。
- Linux/macOS/Windows Desktop 使用同一 spec，平台差异在 Host helper 内消化。
- apply 必须带 operation revision，并在输出中列出受影响 App/Service。

## 5. 待决策项

- mount spec 的 system-config 所有者和 scheduler/node-daemon 收敛协议。
- Desktop 用户选择本机目录后，如何安全转换为 Jarvis 容器可引用的 Host resource id。
