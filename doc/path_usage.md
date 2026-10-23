# BuckyOS里的Path

## 应用服务视角

应用看到的Path是一个在标准的linux FS，我们在标准的linux FS上做的扩展有

> Agent 是独立身份，不等同于 App。`AgentSpec.binding` 把 AgentDID/AgentId 绑定到普通
> App 的 AppInstanceId + service_name。Jarvis RootFS 模板的 AppId 是
> `jarvis.buckyos.bns.did`；基于模板创建的每个 Agent 由 OpenDAN 加载，构造 App 的 AppId 等于其 AgentId。

/opt/buckyos/bin/$appid ：我们鼓励应用开发把二进制文件放在这个目录，这样脱离容器运行时改动也很小（比如容器变成了VM）

/tmp/ => /tmp/buckyos/$appid : App临时数据,有读写权限
/config/* =>  buckyos的system_config : 只读,扩展的system config配置读取,遵循system_config配置。写入需要通过SDK API
    /config/boot/config => buckyos的system_config的/boot/config
/home/$username/ => cyfs://$zone_id/home/$username : 用户home目录，根据配置通常是只读或不可访问, 
/home/$username/.local/share/$appid => cyfs://$zone_id/home/$username/.local/share/$appid ：app有读写权限的永久数据区，谨慎写入。会在app卸载时用户可选删除
/mnt/alice.bns.did/pub/ => cyfs://alice.bns.did/pub/ ：buckyos支持的internet filesystem,由另一个zone配置权限
/srv/library/ => cyfs://$zone_id/srv/library/ : zone级别的库数据，对zone外不可见，zone内默认所有用户都有读写权限，但是只对自己创建的文件有删除权限
/srv/publish/ => cyfs://$zone_id/srv/publish/ : zone级别的分享数据，通常只读
/home/$username/shared/ =>cyfs://$zone_id/home/$username/shared/ : 用户级别的分享数据，通常只读

> 用户存储配额由res_pool管理,各目录的实际可用空间受用户res_pool配置限制

### Docker Worker Volume 视角

Docker App 和 Agent/Script Worker 默认将宿主 `$BUCKYOS_ROOT/etc/` 只读映射到
`/opt/buckyos/etc/`。AppService runtime 从 `machine.json` 读取 `web3_bridge`、`force_https` 等机器策略，
身份和启动凭据仍由应用环境变量提供。缺少 `machine.json` 时 runtime 使用默认机器策略。
`security/` 不挂载。启动配置 `start_config.json`、开发 CA 私钥 `ca_key.pem` 和包含私钥或 API 凭据的
开发模板 `scheduler/boot.template.toml` 放在 security 根目录，默认 `$BUCKYOS_ROOT/security/`。
Rust 服务和备份脚本支持 `BUCKYOS_SECURITY_ROOT`；显式目标目录的配置生成器使用目标下的 `security/`。
公开基础模板仍在 `etc/scheduler/`，scheduler 优先读取 security 中的私有模板。
已有安装在开放 `etc/` 前须迁走敏感文件，并恢复公开基础模板；node-daemon 检测到 `etc/start_config.json`
或 `etc/ca_key.pem`、基础模板中含 PEM 私钥时会拒绝创建容器。更新 node-daemon 后需重建已有容器，
并更新容器内使用 `buckyos-api` 的程序；OpenDAN 需要重建 `paios/aios` 镜像。

正式 Docker 模式下,App/Agent 还会看到两类不属于用户 home 数据的运行卷:

```
/opt/buckyos/instance => 每个 app instance 私有 Docker volume,读写+可执行
/opt/buckyos/tools    => 节点共享 ExtTool volume,只读+可执行
```

Agent 容器另有两组只读 bind mount(普通 script 容器没有):

```
/opt/buckyos/libexec/buckyos-tool/{cli,dist,package.json,distribution.json}
                          => $buckyos_root/libexec/buckyos-tool/ 同名项,宿主 BuckyOS Tool 脚本与 SDK
/mnt/buckyos/agent_skills => 已安装的 pkg_list.agent_skills 子包(仅声明了该子包时)
```

宿主 Tool 的 `runtime/`(宿主 Deno)不挂入;容器内 `buckyos` 命令和 `buckyos-agent` 工具用镜像自带的 Deno 运行这些挂载的脚本,源码与宿主系统同版本。

/opt/buckyos/instance 用来保存容器运行期需要写入、但不应进入用户数据目录的内容,例如:

- `/opt/buckyos/instance/pkg/`: package 工作副本
- `/opt/buckyos/instance/cache/`: deno/uv/npm/pip 等依赖缓存 upper
- `/opt/buckyos/instance/tools/bin`: App 私有 runtime bin 视图
- `/opt/buckyos/instance/tools/<agent_id>/<session_id>`: OpenDAN Agent Session Exec Bin

/opt/buckyos/tools 是 ExtTool 共享只读卷,常见结构:

- `/opt/buckyos/tools/store`: 系统/平台提供的可执行工具集合
- `/opt/buckyos/tools/deno-cache`: 预热 deno cache lower
- `/opt/buckyos/tools/uv-cache`: 预热 uv cache lower

应用不能把运行时生成物写入 `/opt/buckyos/tools`;需要写入且可执行的内容放到 `/opt/buckyos/instance`。
`/home/$username/.local/share/$appid` 仍是 app 的永久 user/app data,host 上对应
`$buckyos_root/data/home/$username/.local/share/$appid`,不用于承载 session-bin 或依赖缓存 upper。
OpenDAN 在该 App 数据目录下继续按 AgentId 隔离 AgentRootFS：
`/home/$username/.local/share/$appid/agents/$agent_id`。

Agent 创建向导的 `permission_mode` 默认 `read_write`，可选 `read_only`。对应用户 home 的读写 / 只读挂载；AgentRoot 始终可读写，用户选择只读仍可保存 Session、记忆、工作区和技能。实际 Docker Worker 路径带 `/opt/buckyos/data` 前缀：

| 目录 | 容器路径 | 宿主路径 | Agent 权限 |
| --- | --- | --- | --- |
| Owner home | `/opt/buckyos/data/home/<owner>/` | `$BUCKYOS_ROOT/data/home/<owner>/` | `permission_mode` 为 `read_write` 时读写，`read_only` 时只读 |
| AgentRoot | `/opt/buckyos/data/home/<owner>/.local/share/<agent_id>/agents/<agent_id>/` | `$BUCKYOS_ROOT/data/home/<owner>/.local/share/<agent_id>/agents/<agent_id>/` | 始终读写 |
| Agent 私有 storage | `/opt/buckyos/storage/` | `$BUCKYOS_ROOT/storage/apps/<runtime_key>/` | 始终读写，`runtime_key` 为 AppInstanceId 派生的目录键 |

Agent 的 storage 不挂载宿主全局 `storage/`；其他 Worker 的既有 storage 映射不变。`BUCKYOS_ROOT` 本身可为符号链接，其下 Agent 使用的 home、App 数据、日志与私有 storage 挂载根不能经符号链接重定向。模板声明不能把只读 Owner home 重新开放为可写目录。API 访问还需经过用户与 runtime App 两侧的授权校验。`debug_jarvis.sh` / `opendan --dev` 直接运行在宿主进程环境中，文件访问受宿主进程权限约束，不提供 Docker 的只读挂载隔离。

当应用关闭兼容性开关后，应用使用buckyos-sdk访问cyfs://,所有的到cyfs://的自动mount都会消失，本地文件读写只有/tmp/会成功

## 单机版特例

单机版(桌面版)不会运行cyfs FUSE daemon,所以直接映射到本地文件系统：
```
cyfs://$this_zone_id/ => $buckyos_root/data/下
```

桌面安装支持的和当前host-node的Home目录的融合，是通过link实现的，比如建立一个软链接
$buckyos_root/data/home/$username/Documents => c:/users/alice/Documents

## 系统服务视角(kernel/frame service)

服务通常需要通过buckyos-base的API把逻辑path映射到本机path(host-node-path)
服务不在docker里，所以可以直接通过目录转换访问本地目录，要切换到sdk上，方便后续对接cyfs 的fs api
用户通常不会手工管理服务，但系统有考虑面向单个服务提供“重新安装按钮“，用于修复一些问题


### 服务常用的目录

/config/services/$service_name/* :服务保存配置的在system_config
$buckyos_service_data => cyfs://$zone_id/var/$service_name/ => $buckyos_root/data/var/$service_name : 服务有读写权限,系统卸载的时候会删除
$buckyos_service_cache => cyfs://$zone_id/cache/$service_name/ => $buckyos_root/data/cache/$service_name : 服务有读写权限,系统卸载的时候会删除
$buckyos_service_home => cyfs://$zone_id/srv/$service_name/=> $buckyos_root/data/srv/$service_name : 服务有读写权限,系统卸载的时候不会删除

服务也可以使用host-node-path,使用下面两个(通常不鼓励使用):

$buckyos_service_local_cache => /tmp/buckyos/$service_name
$buckyos_service_local_data => /opt/buckyos/local/$service_name

### RDB instance 数据分区

服务和 App 通过 `buckyos-api::rdb_mgr` 获取关系数据库连接串时，spec 中的 `partitions` 决定库的
物理目录和生命周期：

| 分区 | Kernel/Frame service | AppService | 备份、卸载与软重置语义 |
| --- | --- | --- | --- |
| `user_data`（默认） | `$buckyos_root/data/$appid` | `$buckyos_root/data/$user/$appid` | 属于用户/Zone 数据；保持既有 RDB 路径，不自动搬迁 |
| `local` | `$buckyos_root/local/$service_name` | 不允许 | 本机运行状态；不备份，随 `local/` 删除，缺失时自举空库 |
| `cache` | `$buckyos_root/data/cache/$service_name` | `$buckyos_root/data/cache/$user/$appid` | 可重建派生数据；不备份，卸载或软重置时删除 |
| `storage` | `$buckyos_root/storage/$appid` | 不允许 | 内核基础设施持久存储；不进入用户备份，不随普通卸载删除 |

`user_data` 为兼容已部署数据库，使用 `data/$appid`，不等同于上文
`$buckyos_service_data` 的 `data/var/$service_name`。RDB 的 `cache` 分区固定使用 `data/cache/**`，
不使用旧 runtime cache helper 的 `cache/**`。SQLite 文件必须位于所选分区的 base dir 内；
`../`、绝对路径覆盖和符号链接逃逸都会被拒绝。

## 内核服务视角

内核服务基本只和node-host-fs打交道
内核服务也可以使用buckyos_service_local_cache，buckyos_service_local_data

### 内核服务目录，这些目录通常只被内核服务访问,是System级别(Zone级别的)
$buckyos_root/data/srv/library/ => cyfs://$zone_id/srv/library/ : zone内共享的资料库
$buckyos_root/data/srv/publish/ => cyfs://$zone_id/srv/publish/ : zone级别的分享数据
$buckyos_root/storage : 内核基础设施在本机的持久化存储(dcfs chunks、named_store,未来可能包含dRDB数据),卸载不会删除
$buckyos_root/etc : 内核配置区,覆盖安装和卸载时的操作是逐个文件定义的
$buckyos_root/security : 启动配置、私钥和私有启动模板，不向应用容器默认开放


## 安装卸载整理

- 更新：二进制目录 $buckyos_root/bin/apps and service
- 更新时不删除： 默认都不删除
- 软重置: 通过control panel实现，这个是强业务逻辑
- 覆盖安装：覆盖安装的特点是 “如果不存在就复制“
  - 减少rootfs/bin/ 目录外的文件，这意味着应用本身还依赖"必须存在的外部数据" (rootfs是buckyos_root模版) 
- 卸载时不删除 
  - $buckyos_root/data/home/ (用户个人数据)
  - $buckyos_root/data/srv/ (服务持久数据 + zone共享数据)
  - $buckyos_root/storage (内核基础设施在各host-node上的持久化存储)
  - RDB `storage` 分区（位于上述 storage 目录）
  - 根据 $buckyos_root/bin/applist.json 决定保留哪些host-node-local-app的数据
- 卸载时删除
  - $buckyos_root/bin/ (二进制文件,删除前先读取applist.json)
  - $buckyos_root/data/var/ (服务运行数据)
  - $buckyos_root/data/cache/ (服务缓存数据)
  - $buckyos_root/local/ (服务本地数据)
  - RDB `cache` 和 `local` 分区；RDB `user_data` 按所在 data 子目录的既有规则处理
  - $buckyos_root/etc/ (系统配置,删除前自动执行身份备份)
  - $buckyos_root/logs/ (日志)
  - /tmp/buckyos/ (应用和服务的临时数据,在$buckyos_root外)

## 软重置
- 重新安装所有服务（更新二进制文件+重置服务）
  - 删除 $buckyos_service_data
  - 删除 $buckyos_service_cache
  - 保留 $buckyos_service_home
  - 删除 RDB `local`/`cache` 分区，保留 `user_data`/`storage` 分区；服务必须能从空库自举
- 删除所有应用和应用的数据
- 保留分布式系统的拓扑 （内核数据保留）
- 保留分布式存储（DFS+KV+RDB)
- 保留用户数据

执行细节：待定
