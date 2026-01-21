# 需求清单（NAS）
以下为控制面板的 NAS 视角功能需求，按模块划分，优先覆盖管理/可观测/安全/可扩展能力。

## 系统与运维
- 系统概览：硬件信息、系统版本、运行时间、节点健康度。
- 资源监控：CPU/内存/磁盘/网络吞吐、历史趋势、告警阈值。
- 更新管理：系统/服务版本检查、灰度更新、回滚记录。
- 日志中心：系统日志/访问日志/服务日志聚合与检索。
- 任务与调度：后台任务队列、执行状态、失败重试与通知。

## 存储与数据
- 存储池/卷管理：创建/扩容/缩容、性能与健康状态。
- 磁盘管理：SMART 状态、温度、坏道预警、替换流程。
- 文件服务：SMB/NFS/FTP/WebDAV/rsync 配置与状态。
- 共享与权限：共享目录、ACL、链接分享、有效期与审计。
- 快照/备份/恢复：定期策略、多版本、跨设备/云端备份。

## 应用与服务
- 应用列表：已安装/可用/更新，版本与运行状态。
- 服务编排：依赖关系、端口映射、资源配额、环境变量。
- 应用配置：基于 sys_config 的配置视图与快速修改。
- 应用日志：单应用日志与健康检查。

## 用户与安全
- 用户/角色管理：用户生命周期、角色权限、授权策略。
- 认证与会话：登录、二次验证、会话管理、API Key。
- 设备与客户端：接入设备列表、阻断与限速策略。
- 审计：关键操作审计、异常行为记录。

## 网络与访问
- 网络接口：IP/网关/DNS/MTU、DHCP/静态配置。
- 防火墙与端口：规则管理、端口转发、流量统计。
- 外网与 DDNS：域名/证书管理、反向代理、访问策略。

## 生态与集成
- 通知系统：邮件/短信/IM/Webhook。
- 监控集成：Prometheus/Elastic 等外部可观测对接。
- 插件机制：自定义模块、前端卡片/后端 RPC 扩展。

## 事件通知模块方案（不新增 KernelService）
目标: 在不增加新内核服务的前提下, 由现有 control_panel 统一收敛事件并对外提供通知能力。

### 服务划分
- control_panel: 事件聚合 + 存储 + RPC, 作为唯一入口.
- 其他服务: 通过 control_panel RPC 写入事件, 无需改为独立通知服务.

### 事件来源
- node_daemon / scheduler / gateway / repo-service / sys_config_service 通过 RPC 推送事件.
- control_panel 也可从 sys_config 读取部分配置变更记录, 作为补充来源.

### 数据与存储
- 事件落地到 control_panel 本地 DB (sled/rocksdb), 支持时间/级别/来源索引与保留期.
- sys_config 仅存规则/通道配置, 避免大数据膨胀.

### 权限与审计
- 使用 verify-hub session_token + RBAC 过滤可见范围.
- 关键操作写入审计事件, 由 control_panel 统一持久化.

### 通知通道
- 先实现 webhook / email, 其他通道后续扩展.
- 支持: 静默时间、频率限制、聚合通知.

### RPC 设计（control_panel 内部实现）
- notification.emit
- notification.list
- notification.get
- notification.ack
- notification.unread.count
- notification.rule.list / create / update / delete
- notification.channel.list / create / update / delete

### UI 对接
- Dashboard Recent Events: notification.list (top N, last 24h).
- Notifications 页: 全量列表 + 过滤/标记已读.

# Control Panel RPC 文档

本文件集中记录 control_panel 的 RPC 规划与约定，供前后端协作实现使用。

## 接入方式
- HTTP: POST /kapi/control-panel
- Body: kRPC 请求结构 (method/params/id)

示例请求:
```json
{ "id": 1, "method": "ui.dashboard", "params": { "session_token": "..." } }
```

示例响应:
```json
{ "id": 1, "result": { "status": "success", "data": { } } }
```

## 命名约定
- 统一采用 <module>.<action>，如 `storage.volume.create`
- UI 相关采用 `ui.*`，旧的 `main/layout/dashboard` 仍保留别名

## 通用字段约定
- 鉴权: 除 auth.* 外，建议通过 params 携带 `session_token` 或 `api_key`
- 分页: list 类接口支持 `page`/`page_size` 或 `cursor`/`limit`
- 排序: `sort` + `order` (asc/desc)
- 过滤: `query` 用于搜索关键词

## 版本策略
- 新增字段向后兼容
- 破坏性变更使用新 method 名称

## 模块与接口

### UI (ui.*)

#### ui.main (legacy: main)
用途: 页面入口/健康检查占位。

请求参数:
- session_token: string, optional

响应字段:
- test: string (当前占位，后续可替换为版本/能力信息)

#### ui.layout (legacy: layout)
用途: 左侧布局与头部用户信息。

请求参数:
- session_token: string

响应字段:
- profile: object
  - name: string
  - email: string
  - avatar: string (URL)
- systemStatus: object
  - label: string
  - state: string (online/offline/warning)
  - networkPeers: number
  - activeSessions: number

#### ui.dashboard (legacy: dashboard)
用途: 仪表盘数据聚合。

请求参数:
- session_token: string

响应字段:
- recentEvents: array
  - title: string
  - subtitle: string
  - tone: string (success/warning/info)
- dapps: array
  - name: string
  - icon: string (emoji or URL)
  - status: string (running/stopped)
- resourceTimeline: array
  - time: string (HH:MM)
  - cpu: number (0-100)
  - memory: number (0-100)
- storageSlices: array
  - label: string
  - value: number (0-100)
  - color: string (hex)
- storageCapacityGb: number
- storageUsedGb: number
- devices: array
  - name: string
  - role: string
  - status: string
  - uptimeHours: number
  - cpu: number
  - memory: number
- memory: object
  - totalGb: number
  - usedGb: number
  - usagePercent: number
- cpu: object
  - usagePercent: number
  - model: string
  - cores: number
- disks: array
  - label: string
  - totalGb: number
  - usedGb: number
  - fs: string
  - mount: string

备注:
- 当前实现使用 sysinfo 实时采样，CPU 有 200ms 采样窗口。

### 认证 (auth.*)

#### auth.login
用途: 登录并获取会话令牌。

请求参数:
- username: string
- password: string
- otp: string, optional

响应字段:
- session_token: string
- refresh_token: string
- expires_at: number (unix seconds)
- user: object { id, name, roles }

#### auth.logout
用途: 注销会话。

请求参数:
- session_token: string

响应字段:
- ok: boolean

#### auth.refresh
用途: 刷新令牌。

请求参数:
- refresh_token: string

响应字段:
- session_token: string
- refresh_token: string
- expires_at: number

### 用户 (user.*)

#### user.list
用途: 列出用户。

请求参数:
- page: number, optional
- page_size: number, optional
- query: string, optional

响应字段:
- items: array of user
- total: number

#### user.get
请求参数:
- user_id: string

响应字段:
- user: object

#### user.create
请求参数:
- username: string
- password: string
- email: string, optional
- roles: array, optional

响应字段:
- user_id: string

#### user.update
请求参数:
- user_id: string
- patch: object

响应字段:
- user: object

#### user.delete
请求参数:
- user_id: string

响应字段:
- ok: boolean

#### user.role.list
请求参数:
- page: number, optional
- page_size: number, optional

响应字段:
- items: array
- total: number

#### user.role.update
请求参数:
- role_id: string
- patch: object

响应字段:
- role: object

### 系统 (system.*)

#### system.overview
用途: 系统概览。

响应字段:
- name: string
- model: string
- os: string
- version: string
- uptime_seconds: number

#### system.status
用途: 健康状态。

响应字段:
- state: string (online/warning/critical)
- warnings: array
- services: array

#### system.metrics
用途: 指标汇总。

响应字段:
- cpu: object
- memory: object
- disk: object
- network: object

#### system.update.check
用途: 检查更新。

响应字段:
- has_update: boolean
- latest_version: string
- notes: string

#### system.update.apply
用途: 应用更新。

请求参数:
- version: string

响应字段:
- task_id: string

#### system.config.test
用途: 测试 system_config 连接与读取逻辑。

请求参数:
- key: string, optional (默认: boot/config)
- service_url: string, optional (默认: http://127.0.0.1:3200/kapi/system_config)
- session_token: string, optional (也可走 RPC token)

响应字段:
- key: string
- value: string
- version: number
- isChanged: boolean

### 存储 (storage.*)

#### storage.volumes
用途: 列出卷/阵列。

响应字段:
- items: array

#### storage.volume.get
请求参数:
- volume_id: string

响应字段:
- volume: object

#### storage.volume.create
请求参数:
- name: string
- raid_level: string
- disks: array

响应字段:
- volume_id: string

#### storage.volume.expand
请求参数:
- volume_id: string
- disks: array

响应字段:
- task_id: string

#### storage.volume.delete
请求参数:
- volume_id: string

响应字段:
- ok: boolean

#### storage.disks
响应字段:
- items: array

#### storage.smart
请求参数:
- disk_id: string

响应字段:
- smart: object

#### storage.raid.rebuild
请求参数:
- volume_id: string

响应字段:
- task_id: string

### 共享 (share.*)

#### share.list
响应字段:
- items: array

#### share.get
请求参数:
- share_id: string

响应字段:
- share: object

#### share.create
请求参数:
- name: string
- path: string
- permissions: object

响应字段:
- share_id: string

#### share.update
请求参数:
- share_id: string
- patch: object

响应字段:
- share: object

#### share.delete
请求参数:
- share_id: string

响应字段:
- ok: boolean

### 文件 (files.*)

#### files.browse
用途: 列目录。

请求参数:
- path: string
- include_hidden: boolean, optional

响应字段:
- entries: array

#### files.stat
请求参数:
- path: string

响应字段:
- stat: object

#### files.mkdir
请求参数:
- path: string
- recursive: boolean, optional

响应字段:
- ok: boolean

#### files.delete
请求参数:
- path: string
- recursive: boolean, optional

响应字段:
- ok: boolean

#### files.move
请求参数:
- src: string
- dst: string
- overwrite: boolean, optional

响应字段:
- ok: boolean

#### files.copy
请求参数:
- src: string
- dst: string
- overwrite: boolean, optional

响应字段:
- ok: boolean

#### files.upload.init
请求参数:
- path: string
- size: number
- sha256: string, optional

响应字段:
- upload_id: string
- part_size: number

#### files.upload.part
请求参数:
- upload_id: string
- part_number: number
- data: string (base64 或二进制承载)

响应字段:
- etag: string

#### files.upload.complete
请求参数:
- upload_id: string
- parts: array

响应字段:
- ok: boolean
- path: string

#### files.download
请求参数:
- path: string

响应字段:
- url: string 或 download_token: string

### 备份 (backup.*)

#### backup.jobs
响应字段:
- items: array

#### backup.job.create
请求参数:
- name: string
- source: string
- target_id: string
- schedule: string

响应字段:
- job_id: string

#### backup.job.run
请求参数:
- job_id: string

响应字段:
- task_id: string

#### backup.job.stop
请求参数:
- job_id: string

响应字段:
- ok: boolean

#### backup.targets
响应字段:
- items: array

#### backup.restore
请求参数:
- target_id: string
- snapshot_id: string
- dest: string

响应字段:
- task_id: string

### 应用 (apps.*)

#### apps.list
用途: 列出当前已部署的应用/服务（基于 sys_config 的 services）。

请求参数:
- key: string, optional (默认: services)

响应字段:
- key: string
- items: array
  - name: string
  - icon: string
  - category: string
  - status: string (installed/available)
  - version: string
  - settings: object|string|null (sys_config value)

#### apps.install
请求参数:
- app_id: string
- version: string, optional

响应字段:
- task_id: string

#### apps.update
请求参数:
- app_id: string
- version: string

响应字段:
- task_id: string

#### apps.uninstall
请求参数:
- app_id: string

响应字段:
- task_id: string

#### apps.start
请求参数:
- app_id: string

响应字段:
- ok: boolean

#### apps.stop
请求参数:
- app_id: string

响应字段:
- ok: boolean

### 网络 (network.*)

#### network.interfaces
响应字段:
- items: array

#### network.interface.update
请求参数:
- name: string
- dhcp: boolean
- ip: string, optional
- gateway: string, optional
- mtu: number, optional

响应字段:
- ok: boolean

#### network.dns
请求参数:
- servers: array, optional

响应字段:
- servers: array

#### network.ddns
请求参数:
- provider: string
- hostname: string
- enabled: boolean

响应字段:
- ok: boolean

#### network.firewall.rules
响应字段:
- items: array

#### network.firewall.update
请求参数:
- rules: array

响应字段:
- ok: boolean

### 设备 (device.*)

#### device.list
响应字段:
- items: array

#### device.block
请求参数:
- device_id: string

响应字段:
- ok: boolean

#### device.unblock
请求参数:
- device_id: string

响应字段:
- ok: boolean

### 通知 (notification.*)

#### notification.list
请求参数:
- page: number, optional
- page_size: number, optional

响应字段:
- items: array
- total: number

### 日志 (log.*)

#### log.system
请求参数:
- level: string, optional
- since: string, optional
- limit: number, optional

响应字段:
- items: array

#### log.access
请求参数:
- since: string, optional
- limit: number, optional

响应字段:
- items: array

### 安全 (security.*)

#### security.2fa.enable
请求参数:
- method: string (totp/sms)

响应字段:
- secret: string, optional
- ok: boolean

#### security.2fa.disable
请求参数:
- session_token: string

响应字段:
- ok: boolean

#### security.keys
用途: API key 列表与撤销。

请求参数:
- action: string (list/revoke/create)
- key_id: string, optional

响应字段:
- items: array, optional
- key: string, optional
- ok: boolean

### 文件服务 (file_service.*)

#### file_service.list
用途: 列出文件服务状态 (SMB/NFS/AFP/FTP/WebDAV/rsync/SFTP/SSH)。

响应字段:
- items: array { name, enabled, status }

#### file_service.<proto>.get / file_service.<proto>.update
用途: 获取/更新协议服务配置。

<proto> 清单:
- smb, nfs, afp, ftp, webdav, rsync, sftp, ssh

请求参数:
- enabled: boolean, optional
- config: object, optional

响应字段:
- enabled: boolean
- config: object
- ok: boolean (update)

### iSCSI (iscsi.*)

#### iscsi.targets / iscsi.luns / iscsi.sessions
用途: 列出 target / LUN / session。

响应字段:
- items: array

#### iscsi.target.create / iscsi.target.update / iscsi.target.delete
用途: 管理 iSCSI target。

请求参数:
- target_id: string, optional
- name: string, optional
- auth: object, optional

响应字段:
- target_id: string
- ok: boolean

#### iscsi.lun.create / iscsi.lun.update / iscsi.lun.delete
用途: 管理 iSCSI LUN。

请求参数:
- lun_id: string, optional
- target_id: string, optional
- size_gb: number, optional

响应字段:
- lun_id: string
- ok: boolean

### 快照 (snapshot.*)

#### snapshot.list
用途: 列出快照。

请求参数:
- volume_id: string, optional
- share_id: string, optional

响应字段:
- items: array

#### snapshot.create / snapshot.delete / snapshot.restore
用途: 创建/删除/恢复快照。

请求参数:
- snapshot_id: string, optional
- volume_id: string, optional
- share_id: string, optional

响应字段:
- snapshot_id: string
- task_id: string
- ok: boolean

#### snapshot.schedule.list / snapshot.schedule.update
用途: 快照计划配置。

请求参数:
- schedule: object, optional

响应字段:
- items: array
- ok: boolean

### 复制 (replication.*)

#### replication.jobs
用途: 列出复制任务。

响应字段:
- items: array

#### replication.job.create / replication.job.run / replication.job.pause / replication.job.delete
用途: 复制任务管理。

请求参数:
- job_id: string, optional
- source: string, optional
- target: string, optional

响应字段:
- job_id: string
- task_id: string
- ok: boolean

#### replication.status
用途: 查看复制状态。

响应字段:
- status: object

### 同步 (sync.*)

#### sync.providers
用途: 列出云同步/厂商。

响应字段:
- items: array

#### sync.tasks
用途: 列出同步任务。

响应字段:
- items: array

#### sync.task.create / sync.task.run / sync.task.pause / sync.task.resume / sync.task.delete
用途: 同步任务管理。

请求参数:
- task_id: string, optional
- source: string, optional
- target: string, optional

响应字段:
- task_id: string
- ok: boolean

### 配额 (quota.*)

#### quota.get / quota.update / quota.defaults
用途: 获取/更新配额或默认配额。

请求参数:
- scope: string (user/share)
- target_id: string, optional
- limit_gb: number, optional

响应字段:
- items: array
- ok: boolean

### ACL/权限 (acl.*)

#### acl.get / acl.update / acl.reset
用途: 获取/更新/重置 ACL。

请求参数:
- path: string
- acl: object, optional

响应字段:
- acl: object
- ok: boolean

### 回收站 (recycle_bin.*)

#### recycle_bin.get / recycle_bin.update
用途: 获取/更新回收站设置。

请求参数:
- enabled: boolean, optional
- retention_days: number, optional

响应字段:
- enabled: boolean
- retention_days: number
- ok: boolean

#### recycle_bin.list / recycle_bin.restore / recycle_bin.delete
用途: 回收站文件操作。

请求参数:
- item_id: string, optional

响应字段:
- items: array
- ok: boolean

### 索引与搜索 (index.* / search.*)

#### index.status / index.rebuild
用途: 索引状态与重建。

响应字段:
- status: object
- task_id: string

#### search.query
用途: 文件/媒体搜索。

请求参数:
- query: string
- scope: string, optional

响应字段:
- items: array

### 媒体 (media.*)

#### media.library.scan / media.library.status
用途: 媒体库扫描与状态。

响应字段:
- status: object
- task_id: string

#### media.dlna.get / media.dlna.update
用途: DLNA 服务配置。

响应字段:
- enabled: boolean
- config: object
- ok: boolean

### 下载 (download.*)

#### download.tasks
用途: 列出下载任务。

响应字段:
- items: array

#### download.task.create / download.task.pause / download.task.resume / download.task.delete
用途: 下载任务管理 (BT/HTTP/FTP)。

请求参数:
- task_id: string, optional
- url: string, optional

响应字段:
- task_id: string
- ok: boolean

### 容器 (container.*)

#### container.list / container.images
用途: 列出容器与镜像。

响应字段:
- items: array

#### container.create / container.start / container.stop / container.update / container.delete
用途: 容器管理。

请求参数:
- container_id: string, optional
- image: string, optional
- config: object, optional

响应字段:
- container_id: string
- ok: boolean

#### container.image.pull / container.image.remove
用途: 镜像管理。

请求参数:
- image: string

响应字段:
- ok: boolean

### 虚拟机 (vm.*)

#### vm.list
用途: 列出虚拟机。

响应字段:
- items: array

#### vm.create / vm.start / vm.stop / vm.delete
用途: 虚拟机管理。

请求参数:
- vm_id: string, optional
- config: object, optional

响应字段:
- vm_id: string
- ok: boolean

#### vm.snapshot.create / vm.snapshot.restore
用途: 虚拟机快照。

请求参数:
- vm_id: string
- snapshot_id: string, optional

响应字段:
- snapshot_id: string
- ok: boolean

### 证书 (cert.*)

#### cert.list / cert.issue / cert.import / cert.delete / cert.renew
用途: 证书管理 (TLS/Let's Encrypt)。

请求参数:
- cert_id: string, optional
- domain: string, optional

响应字段:
- cert_id: string
- ok: boolean

### 反向代理 (proxy.*)

#### proxy.list / proxy.create / proxy.update / proxy.delete
用途: 反向代理规则管理。

请求参数:
- proxy_id: string, optional
- config: object, optional

响应字段:
- proxy_id: string
- ok: boolean

### VPN (vpn.*)

#### vpn.profiles
用途: 列出 VPN 配置。

响应字段:
- items: array

#### vpn.profile.create / vpn.profile.update / vpn.profile.delete
用途: VPN 配置管理。

请求参数:
- profile_id: string, optional
- config: object, optional

响应字段:
- profile_id: string
- ok: boolean

#### vpn.status / vpn.connect / vpn.disconnect
用途: VPN 连接状态/操作。

响应字段:
- status: object
- ok: boolean

### 电源 (power.*)

#### power.shutdown / power.reboot
用途: 关机/重启。

响应字段:
- ok: boolean

#### power.schedule.list / power.schedule.update
用途: 定时开关机。

请求参数:
- schedule: object, optional

响应字段:
- items: array
- ok: boolean

#### power.wol.send
用途: 发送 Wake-on-LAN。

请求参数:
- mac: string

响应字段:
- ok: boolean

### 时间 (time.*)

#### time.get / time.update
用途: 获取/更新系统时间与时区。

请求参数:
- timezone: string, optional
- datetime: string, optional

响应字段:
- time: string
- timezone: string
- ok: boolean

#### time.ntp.get / time.ntp.update
用途: NTP 设置。

请求参数:
- servers: array, optional
- enabled: boolean, optional

响应字段:
- enabled: boolean
- servers: array
- ok: boolean

### 硬件 (hardware.*)

#### hardware.sensors / hardware.fans
用途: 温度/风扇/电压等硬件状态。

响应字段:
- items: array

#### hardware.led.update
用途: 设备 LED/指示灯配置。

请求参数:
- mode: string

响应字段:
- ok: boolean

#### hardware.ups.get / hardware.ups.update
用途: UPS 配置与状态。

请求参数:
- config: object, optional

响应字段:
- status: object
- ok: boolean

### 审计 (audit.*)

#### audit.events / audit.export
用途: 审计事件查询与导出。

请求参数:
- since: string, optional
- limit: number, optional

响应字段:
- items: array
- url: string, optional

### 杀毒 (antivirus.*)

#### antivirus.status
用途: 查看杀毒服务状态。

响应字段:
- status: object

#### antivirus.scan
用途: 触发扫描。

请求参数:
- path: string, optional
- deep: boolean, optional

响应字段:
- task_id: string

#### antivirus.signatures.update
用途: 更新病毒库。

响应字段:
- ok: boolean

#### antivirus.quarantine.list / antivirus.quarantine.delete
用途: 隔离区管理。

请求参数:
- item_id: string, optional

响应字段:
- items: array
- ok: boolean

### 系统配置中心 (sys_config.*)

#### sys_config.get / sys_config.set / sys_config.list / sys_config.tree / sys_config.history
用途: 读取/写入/枚举/查看树结构/查看配置历史。

请求参数:
- key: string, optional
- value: string, optional
- prefix: string, optional
- depth: number, optional (tree)

响应字段:
- key: string
- value: string
- items: array
- tree: object
- ok: boolean

### 调度器 (scheduler.*)

#### scheduler.status / scheduler.queue.list / scheduler.task.list / scheduler.task.cancel
用途: 调度状态与任务队列管理。

请求参数:
- task_id: string, optional

响应字段:
- status: object
- items: array
- ok: boolean

### 节点与激活 (node.*)

#### node.list / node.get / node.services.list
用途: 节点列表与详情。

请求参数:
- node_id: string, optional

响应字段:
- items: array
- node: object

#### node.restart / node.shutdown
用途: 节点重启/关机。

请求参数:
- node_id: string

响应字段:
- ok: boolean

#### node.activate / node.activation.status
用途: 节点激活与状态。

请求参数:
- node_id: string

响应字段:
- status: object
- ok: boolean

### 任务管理 (task.*)

#### task.list / task.get / task.cancel / task.retry / task.logs
用途: 任务查询与控制。

请求参数:
- task_id: string, optional

响应字段:
- items: array
- task: object
- logs: array
- ok: boolean

### 验证中心 (verify.*)

#### verify.status / verify.sessions / verify.session.revoke
用途: 认证会话状态管理。

请求参数:
- session_id: string, optional

响应字段:
- status: object
- items: array
- ok: boolean

### 仓库服务 (repo.*)

#### repo.sources / repo.pkgs / repo.tasks
用途: 包仓与任务状态。

响应字段:
- items: array

#### repo.install / repo.publish / repo.sync
用途: 安装/发布/同步包。

请求参数:
- pkg_id: string, optional
- source: string, optional

响应字段:
- task_id: string
- ok: boolean

### 消息总线 (msgbus.*)

#### msgbus.status / msgbus.topics / msgbus.publish
用途: 总线状态与消息发布。

请求参数:
- topic: string, optional
- payload: object, optional

响应字段:
- status: object
- items: array
- ok: boolean

### Web 服务 (nginx.*)

#### nginx.status / nginx.sites / nginx.site.update / nginx.reload
用途: 站点与反代配置管理。

请求参数:
- site_id: string, optional
- config: object, optional

响应字段:
- status: object
- items: array
- ok: boolean

### K8s 服务 (k8s.*)

#### k8s.status / k8s.nodes / k8s.deployments / k8s.deployment.scale
用途: 集群与工作负载管理。

请求参数:
- deployment: string, optional
- replicas: number, optional

响应字段:
- status: object
- items: array
- ok: boolean

### Slog 日志服务 (slog.*)

#### slog.status / slog.streams / slog.query
用途: 日志采集与查询。

请求参数:
- query: string, optional
- limit: number, optional

响应字段:
- status: object
- items: array

### 网关与 Zone (gateway.* / zone.*)

#### gateway.status / gateway.routes.list / gateway.routes.update
用途: 网关路由管理。

请求参数:
- routes: array, optional

响应字段:
- status: object
- ok: boolean

#### zone.info / zone.config.get / zone.config.update / zone.devices.list
用途: zone 配置与设备管理。

请求参数:
- device_id: string, optional
- config: object, optional

响应字段:
- info: object
- items: array
- ok: boolean

### RBAC 与权限 (rbac.*)

#### rbac.model.get / rbac.model.update / rbac.policy.get / rbac.policy.update
用途: RBAC 模型与策略管理。

请求参数:
- model: string, optional
- policy: string, optional

响应字段:
- model: string
- policy: string
- ok: boolean

### 运行时 (runtime.*)

#### runtime.info / runtime.reload
用途: 运行时状态与配置热加载。

响应字段:
- info: object
- ok: boolean
