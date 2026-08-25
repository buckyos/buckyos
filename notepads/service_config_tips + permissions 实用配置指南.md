# `service_config_tips + permissions` 实用配置指南

> 文档状态：指导性设计稿，供后续 CodeAgent 完善实现并整理为 BuckyOS SDK 文档。
>
> 面向版本：BuckyOS beta 2.2。该版本允许 breaking change，本文不要求保留旧字段兼容层。
>
> 当前实现基线：2026-08-25 仓库代码。本文会明确区分“目标 SDK 语义”和“当前代码行为”；第三方开发者最终只能依赖已经进入正式 SDK 并通过测试的部分。

## 1. 这两个配置分别解决什么问题

第三方开发者把一个现成的 self-host Docker 镜像改造成 BuckyOS App 时，通常只真正关心六件事：

1. 镜像哪些目录需要持久化，默认应该放到哪里。
2. 镜像要读写用户文件、共享媒体库还是只读自己的 AppData。
3. 镜像开放哪些服务，哪些服务应通过 ZoneGateway 暴露。
4. 镜像需要哪些环境变量、数据库、密钥和初始化参数。
5. 镜像是否需要访问互联网、局域网、IoT 设备或 BuckyOS 系统服务。
6. 用户拒绝某项权限或配置后，App 是否仍能正常工作。

`service_config_tips` 和 `permissions` 应分别回答：

- `service_config_tips`：**这个镜像怎样接入 BuckyOS 才能工作**。它声明容器路径、建议存储位置、服务端口、环境变量和运行需求，是 InstallPlan 的配置输入。
- `permissions`：**这个 App 获准访问哪些超出自身沙箱的资源**。它声明权限上限，安装时用户实际批准的权限必须是该声明的子集。

不要用 `permissions` 表示 Docker 端口或普通 AppData，也不要用 `service_config_tips` 绕过用户授权去访问 Home、Zone Library、LAN 或 WAN。

## 2. 第三方开发者的最短决策路径

拿到一个 Docker 镜像后，按下面顺序判断即可：

1. 找出镜像文档中的 `VOLUME`、数据库文件、配置文件、上传目录和媒体目录。
2. 把 App 自身状态放到标准 AppData；如果只有一个主要持久目录，优先写成 `null`。
3. 把可重建内容放到 `local_cache_mount_points`，不要混进 AppData。
4. 只有当 App 的核心功能确实要浏览或修改用户文件时，才映射 User Home 并申请 `user/home`。
5. 只有共享媒体或共享文件场景才申请 `zone/library`、`zone/public`。
6. 声明容器内监听端口；HTTP 服务通常交给 ZoneGateway 做 HTTPS，不要在镜像里重复申请公网宿主端口。
7. 只有 App 主动访问公网时才申请 `wan`；入站 Web 暴露和出站联网是两件事。
8. 不要使用 `--privileged`、Docker socket、任意宿主绝对路径或 `--network host` 作为普通适配手段。
9. 不要把密码、token、private key 的值写入 `app.json`。
10. 用默认 InstallPlan 检查最终挂载、权限和环境变量，而不是只检查 AppDoc 能否通过 JSON schema。

## 3. 配置从 AppDoc 到容器的生命周期

推荐把配置链路理解为三层：

```text
AppDoc
  service_config_tips + permissions
            │
            │ Inspect：展开建议值、匹配权限、生成默认选择
            ▼
InstallPlan
  install_params + service_spec_config + plan_fingerprint
            │
            │ 用户确认后冻结
            ▼
AppServiceSpec / NodeExecutionSpec
            │
            │ scheduler + node-daemon
            ▼
container mounts / env / ports / runtime grants
```

关键规则：

- AppDoc 只声明能力需求和建议值，不能直接授予权限。
- 建议路径必须在 Inspect 阶段完成变量展开、规范化和权限分类。
- 用户修改挂载、权限、环境变量或服务暴露后，必须重新生成 InstallPlan。
- 展开后的最终配置必须进入 `plan_fingerprint`；Deploy 阶段不能再次读取环境并得到不同结果。
- Prepare/Deploy 只消费已批准 Plan 中冻结的 `service_spec_config`。

## 4. `data_mount_points` 的目标语义

### 4.1 基本模型

`data_mount_points` 的设计模型是：

```text
"容器内路径" -> "建议的持久化来源路径"
```

例如：

```json
{
  "data_mount_points": {
    "/data": null,
    "/srv": {
      "mount_point_name": "${BUCKYOS_USER_HOME_DIR}",
      "access": "read_write",
      "reason": {
        "en": "Browse and manage the installing user's files.",
        "zh-CN": "浏览和管理安装用户的文件。"
      }
    }
  }
}
```

这里的 map key 永远是**容器内绝对路径**；value 描述 Installer 应向用户推荐的来源位置。

### 4.2 `null` 的标准含义

当 value 为 `null` 时，Installer 必须把该容器路径映射到当前 App 实例的标准 AppData 目录，并默认使用 `read_write`：

```json
{
  "data_mount_points": {
    "/var/lib/myapp": null
  }
}
```

其逻辑结果为：

```text
container:/var/lib/myapp
    -> current app instance's standard AppData
```

标准 AppData：

- 按 `owner_user_id + app_id` 隔离。
- 默认持久化并参与备份/迁移策略。
- App 读写自身 AppData 不需要额外的 `permissions` 条目。
- App 卸载时由用户决定是否同时删除，升级不得清空。

使用建议：

- 一个镜像只有一个主要持久目录时，优先使用 `null`。
- 多个容器路径都写 `null` 会让它们看到同一个 AppData 根；只有确实希望共享同一目录时才这样做。
- 数据库、配置、上传文件需要分目录时，使用 `${BUCKYOS_DATA_DIR}/database`、`${BUCKYOS_DATA_DIR}/config` 等建议路径。

### 4.3 对象值的含义

当前 `MountPointInfo` 的形状是：

```json
{
  "mount_point_name": "${BUCKYOS_DATA_DIR}/config",
  "access": "read_write",
  "reason": {
    "en": "Persistent application configuration.",
    "zh-CN": "持久化应用配置。"
  }
}
```

字段语义：

| 字段 | 语义 | SDK 要求 |
| --- | --- | --- |
| `mount_point_name` | 建议来源路径表达式 | 必须支持受限环境变量；不能当成人类显示名称 |
| `access` | 容器获得的访问方式 | 只能是 `read_only`、`read_write`、`read_write_append` |
| `reason` | 向用户解释为什么需要该目录 | 至少建议提供 `en` 和 `zh-CN`，不得只重复路径 |

`mount_point_name` 这个名字容易被误解成“挂载点显示名称”。在 beta 2.2 正式冻结 SDK 前，建议 breaking rename 为 `suggested_path`，且不要同时保留两个别名。本文示例继续使用当前字段名，便于和现有 AppDoc 对照。

`read_write_append` 只有在运行时能够强制“只追加、不能覆盖或删除”时才有独立安全意义。当前 Docker mount 最终仍会转成普通 `rw`；正式 SDK 在没有文件系统或 API 层 enforcement 前，不应把它向用户描述成 append-only 保护。

### 4.4 建议路径中的环境变量

建议路径不能直接做任意宿主进程环境变量替换。Installer 应提供一个**受限、确定性、跨平台的模板变量表**，并在 Inspect 阶段展开。

建议冻结以下变量：

| 模板变量 | 逻辑资源 | 是否需要额外权限 |
| --- | --- | --- |
| `${BUCKYOS_DATA_DIR}` | 当前 App 实例的标准 AppData | 否 |
| `${BUCKYOS_APP_CACHE_DIR}` | 当前 App 在目标 Node 上的可清理缓存 | 否 |
| `${BUCKYOS_USER_HOME_DIR}` | 当前 `owner_user_id` 的整个 Home | `user/home` |
| `${BUCKYOS_ZONE_LIBRARY_DIR}` | Zone 共享资料库/媒体库 | `zone/library` |
| `${BUCKYOS_ZONE_PUBLIC_DIR}` | Zone 公开或发布目录 | `zone/public` |
| `${BUCKYOS_OWNER_USER_ID}` | App 实例 owner 的稳定 user id | 仅允许作为已授权逻辑根下的路径片段 |
| `${BUCKYOS_APP_ID}` | canonical App id | 仅允许作为已授权逻辑根下的路径片段 |
| `${BUCKYOS_APP_INSTANCE_ID}` | canonical App instance id | 仅允许作为已授权逻辑根下的路径片段 |

其中 `${BUCKYOS_OWNER_USER_ID}`、`${BUCKYOS_APP_ID}`、`${BUCKYOS_APP_INSTANCE_ID}` 已有同名 runtime 环境变量；`${BUCKYOS_USER_HOME_DIR}`、`${BUCKYOS_APP_CACHE_DIR}`、`${BUCKYOS_ZONE_LIBRARY_DIR}`、`${BUCKYOS_ZONE_PUBLIC_DIR}` 是建议新增的 Installer 模板变量。

`${BUCKYOS_DATA_DIR}` 在这里表示“标准 AppData 这一逻辑资源”。Inspect 阶段把它解析为宿主侧受控来源；容器启动时同名 runtime 变量仍指向容器内可访问的标准 AppData。开发者不应依赖宿主物理布局。

禁止作为第三方 App 模板变量：

- `${HOME}`、`${USERPROFILE}` 等 node-daemon 进程自身的用户目录。
- 任意未列入白名单的宿主环境变量。
- `${BUCKYOS_ROOT}` 加任意相对路径；这会把整个运行时目录暴露成可猜测的宿主命名空间。
- 能解析到 `/etc`、`/var/run/docker.sock`、设备节点、BuckyOS identity/security 目录或其它 AppData 的表达式。

展开规则必须满足：

1. 未知变量、未设置变量、递归变量和空变量一律使 `config readiness = NOT_READY`。
2. 展开后进行路径规范化，再做授权根分类；不能在字符串替换前只检查前缀。
3. 拒绝 `..`、平台盘符注入、UNC 路径、NUL 和路径分隔符混淆。
4. 变量展开结果及目标 Node/Owner 快照必须进入 Plan fingerprint。
5. 展开只发生一次；node-daemon 不得在 Deploy 时根据另一套进程环境重新展开。
6. 日志和 UI 优先显示逻辑资源名称，同时可在管理员诊断视图显示最终物理路径。

### 4.5 AppData、用户数据与缓存必须分开

| 数据类型 | 推荐字段 | 示例 | 权限 |
| --- | --- | --- | --- |
| App 自身数据库、配置、索引 | `data_mount_points` | `${BUCKYOS_DATA_DIR}/database` | 无额外权限 |
| 用户自己的文件 | `data_mount_points` | `${BUCKYOS_USER_HOME_DIR}` | `user/home` |
| Zone 共享媒体/资料 | `data_mount_points` | `${BUCKYOS_ZONE_LIBRARY_DIR}` | `zone/library` |
| 缩略图、转码、下载缓存 | `local_cache_mount_points` | `${BUCKYOS_APP_CACHE_DIR}/thumbnails` | 无额外权限 |
| 用户现场选择的移动盘/NAS | `external_mount_points` | 安装时选择 | 必须生成针对具体位置的显式授权 |
| 包管理器缓存、可重建运行副本 | `instance_volume` 或 cache | instance volume | 无额外权限，但要有清理语义 |

不要把 App 自身 SQLite 数据库放进 User Home 根，也不要为了让媒体服务读取照片而把整个 User Home 都映射为可写。

### 4.6 `local_cache_mount_points`

缓存必须满足“删除后可以重建”：

```json
{
  "local_cache_mount_points": {
    "/cache": {
      "mount_point_name": "${BUCKYOS_APP_CACHE_DIR}/media-index",
      "access": "read_write",
      "reason": {
        "en": "Rebuildable thumbnails and media index cache.",
        "zh-CN": "可重新生成的缩略图和媒体索引缓存。"
      }
    }
  }
}
```

缓存不应包含唯一副本、用户上传原件、密钥或不可重建数据库。系统可以在磁盘压力、迁移或软重置时清理缓存。

### 4.7 `external_mount_points`

`external_mount_points` 用于安装时由用户选择的 BuckyOS 管理范围外存储，例如 USB 盘、NAS 映射或管理员预注册的数据集。

目标设计要求：

- AppDoc 只声明容器路径、访问模式和用途，不应硬编码真实宿主路径。
- Installer 使用文件选择器或预注册 Storage Grant 让用户选择来源。
- 批准结果绑定到规范化后的具体资源 identity，而不是只批准一个字符串前缀。
- 默认 `read_only`；`read_write` 必须额外确认。
- 卷消失时 App 应进入可诊断的 degraded/config-blocked 状态，不能静默创建同名空目录。

当前权限常量中还没有足以表达“某个具体外部卷”的稳定 scope。正式 SDK 发布前需要冻结 Storage Grant/外部挂载权限模型。

## 5. `permissions` 实用指南

### 5.1 数据结构

```json
{
  "scope_path": "user/home",
  "required": true,
  "actions": ["read", "write"],
  "exp": null
}
```

| 字段 | 第三方开发者应怎样填写 |
| --- | --- |
| `scope_path` | 使用 SDK 已注册的精确 scope，不自行发明近似字符串 |
| `required` | 用户拒绝后 App 核心功能无法工作才设为 `true` |
| `actions` | 使用该 scope 的最小动作集合；文件类通常为 `read`/`write` |
| `exp` | 当前建议使用 `null`；数值的单位、绝对/相对语义和续期流程尚未冻结 |

`required: true` 表示默认 Plan 必须包含该权限，用户拒绝后安装应明确显示 `CONFIG_BLOCKED` 或“功能不可用”，而不是偷偷降级。它不表示系统可以跳过用户确认自动授权高风险权限。

`required: false` 表示可选增强能力。例如媒体服务可以在不联网时播放本地媒体，那么 `wan` 可以是 optional。

### 5.2 当前已定义的主要 scope

以下 scope 来自 `src/kernel/buckyos-api/src/permission.rs`。表中的典型动作是 SDK 建议，当前代码尚未对每个 scope 的动作词表做完整类型校验。

| scope | 第三方 App 什么时候申请 | 最小动作建议 | 典型风险 |
| --- | --- | --- | --- |
| `user/home` | 浏览、同步、编辑当前 owner 的整个 Home | `read` 或 `read, write` | 可读取/修改用户私人文件 |
| `zone/library` | 访问 Zone 共享媒体和资料库 | 优先 `read` | 可看到多人共享内容 |
| `zone/public` | 读取或发布 Zone 公共内容 | `read`；发布时增加 `write` | 内容可能对更大范围可见 |
| `zone/location` | 使用 Zone/用户位置能力 | `read` | 敏感位置数据 |
| `wan` | App 主动连接公共互联网 | `connect` | 数据外发、供应链访问 |
| `lan` | App 主动访问或发现局域网服务 | `connect` | 横向访问家庭/办公网络 |
| `devices/iot` | 控制或读取 IoT 设备 | `read`/`write`，未来应按设备细分 | 可影响物理设备 |
| `access/all` | 一个实例需要服务 Zone 内所有用户 | `serve` | 扩大用户访问范围 |
| `default/app/{}` | 申请默认 App/短域名入口 | `claim` | 改变 Zone 默认入口 |
| `kapi/aicc` | 调用 AI Center | `call` | 费用、内容外发、模型能力 |
| `kapi/task-manager` | 创建或管理系统任务 | `call` | 后台计算和持久任务 |
| `kapi/workflow` | 调用 Workflow | `call` | 自动化副作用 |
| `kapi/msg-center` | 收发消息 | `call` | 通信隐私和外部发送 |
| `kapi/kmsg` | 使用系统消息队列 | `call` | 后台事件和队列访问 |
| `kapi/repo-service` | 访问 RepoService | `call` | 内容发布、收集和存储 |

对第三方开发者最重要的区分：

- 入站 HTTP/TCP 暴露由 `service_endpoints` 描述。
- App 主动访问公网由 `wan` 描述。
- App 主动访问局域网由 `lan` 描述。
- “有一个 Web 页面”不等于需要 `wan`。

### 5.3 权限与挂载必须成对校验

目标 Installer 必须从**展开后的来源逻辑根**推导权限，而不是相信 App 自己写的说明：

| 最终来源 | 必须存在的权限 | 访问一致性 |
| --- | --- | --- |
| 标准 AppData | 无 | 允许 `read_write` |
| App Cache | 无 | 允许 `read_write` |
| Owner User Home | `user/home` | `read_only` 至少要有 `read`；`read_write` 必须有 `write` |
| Zone Library | `zone/library` | 同上 |
| Zone Public | `zone/public` | 同上 |
| 外部卷 | 对具体 Storage Grant 的授权 | 必须与批准的卷和访问模式完全匹配 |
| 其它 AppData、identity、system config 原始目录 | 禁止 | 不能靠通用 mount 权限放行 |

如果 `/srv` 建议为 User Home，但 AppDoc 没有声明 `user/home`，Inspect 必须返回配置问题；如果只批准了 `read`，最终 mount 不能是 `read_write`。

### 5.4 安装者和网页访问者不是同一个概念

一个 App 实例由 `owner_user_id` 安装，并不表示之后每个网页访问者都应该看到 owner 的 Home。

- mount 发生在 App 实例级，通常固定绑定 `owner_user_id`。
- 网页访问者由 session token 标识，App 必须转发真实用户 token 调用 BuckyOS API。
- 需要服务多个用户的 App 不应简单把多个 Home 全部 bind mount 进一个容器。
- 多用户文件访问更适合通过带 ACL 的 Files/DFS API，而不是通过共享宿主目录绕开授权。

`access/all` 只扩大谁可以使用 App，不自动授予这些访问者读取 owner Home 的权利。

## 6. `service_config_tips` 其它关键字段

### 6.1 `service_endpoints`

普通 Web App 推荐：

```json
{
  "service_endpoints": {
    "www": {
      "protocol": "http",
      "inner_port": 80,
      "required": true,
      "description": {
        "en": "Main web interface.",
        "zh-CN": "主 Web 界面。"
      },
      "expose": {
        "route": {
          "type": "web"
        },
        "scope": "",
        "allow_guest": false
      }
    }
  }
}
```

规则：

- `inner_port` 是容器内监听端口，不是宿主端口。
- 普通 HTTP 服务使用 `protocol: http`，由 ZoneGateway 做 HTTPS/TLS 终止。
- `required: true` 表示禁用该 endpoint 会导致配置不可用。
- `allow_guest` 默认应为 `false`；只有明确设计为匿名服务时才开启。
- `route.type: web` 使用 ZoneGateway 域名路由。
- `route.type: port` 用于 SSH、游戏协议等非 HTTP 服务；`preferred_port` 只是建议，必须检查冲突并允许 Installer 调整。
- 一个镜像可以声明多个命名 endpoint，例如 `www`、`ssh`、`metrics`。监控/管理端口不要默认对外暴露。

同时提供 Web 和 SSH 的示例：

```json
{
  "service_endpoints": {
    "www": {
      "protocol": "http",
      "inner_port": 3000,
      "required": true,
      "expose": {
        "route": {
          "type": "web"
        },
        "scope": "",
        "allow_guest": false
      }
    },
    "ssh": {
      "protocol": "tcp",
      "inner_port": 22,
      "required": false,
      "expose": {
        "route": {
          "type": "port",
          "preferred_port": 2222
        },
        "scope": "",
        "allow_guest": false
      }
    }
  }
}
```

### 6.2 `bash_envs`

当前声明形状：

```json
{
  "bash_envs": {
    "TZ": {
      "required": false,
      "description": {
        "en": "Timezone used by the application.",
        "zh-CN": "应用使用的时区。"
      }
    }
  }
}
```

第三方 App 常见环境变量分三类：

1. 系统可自动派生：App URL、owner id、data dir、service port。应由 BuckyOS 注入，用户不手填。
2. 普通设置：时区、语言、日志级别、功能开关。Installer 可以让用户填写。
3. 密钥：管理员密码、数据库密码、API token、OAuth secret。必须进入 secret store，通过 `value_from` 注入，不能明文写进 AppDoc、InstallPlan 日志或普通 system-config。

正式 SDK 需要为 `BashEnvInfo` 增加并冻结：

- 可选 `default` 或 `default_from`。
- `secret: true/false`。
- `value_from` 的受控来源类型。
- 格式、枚举、长度等输入约束。
- 升级时保留、重置和重新确认语义。

在这些字段落地前，不要把 secret value 写入 `app.json`，也不要假设声明了 `bash_envs` 就一定会被当前 node-daemon 注入。

### 6.3 `rdb_instances`

`rdb_instances` 适合使用 BuckyOS SDK 的 App 申请托管 SQLite/PostgreSQL：

```json
{
  "rdb_instances": {
    "main": {
      "backend": "sqlite",
      "version": 1,
      "schema": {
        "sqlite": "CREATE TABLE IF NOT EXISTS items (...)"
      },
      "connection": "sqlite://$appdata/$instance.db"
    }
  }
}
```

注意：未经适配的第三方 Docker 镜像通常期待 `POSTGRES_HOST`、`POSTGRES_PASSWORD` 等环境变量。仅声明 `rdb_instances` 不会自动把任意镜像改造成托管数据库客户端。未来需要明确的 database binding，把托管实例安全地转成该镜像支持的 env/file secret。

不要把一个完整 `docker-compose.yml` 机械翻译成单个 AppDoc。多容器依赖、Redis、后台 worker、初始化 job 和数据库迁移都需要正式的 component/dependency 模型。

### 6.4 `instance_volume`

```json
{
  "instance_volume": {
    "mode": "disabled"
  }
}
```

建议语义：

- `required`：App/Agent 需要私有可写执行卷才能运行，例如自更新工作副本、包管理器环境。
- `optional`：有卷时提升体验，但丢失后可以恢复。
- `disabled`：纯二进制或普通不可变第三方镜像，不使用私有执行卷。
- `ephemeral_contents`：列出重置时允许丢失的相对路径。
- `quota_mib`：目前只是软配额设计，不能向开发者承诺已强制执行。

self-host App 的业务数据应优先进入 AppData，不应只放在 opaque Docker named volume 中，否则备份、迁移、删除确认和用户可见性都难以统一。

### 6.5 `runtime_caps`

`runtime_caps` 应表达结构化、可审计的运行能力，例如 GPU、硬件转码、受控设备访问，而不是任意 Docker flags。

当前字段只是 `capability -> enabled/disabled` 字符串。正式 SDK 前至少需要：

- capability registry 和稳定名称。
- required/optional 语义。
- 目标 Node 能力匹配。
- 用户授权和运行时执行。
- 无能力时的降级行为。

`req_capbilities` 用于选择“目标 Node 有没有这个能力”；`runtime_caps` 用于“最终运行时授予什么能力”。两者不能混为一谈。

### 6.6 `container_param` 与 `start_param`

普通第三方 App 不应依赖这两个自由字符串字段。

高风险 Docker 参数包括：

- `--privileged`
- `--network host`
- `-v /:/host`
- `-v /var/run/docker.sock:/var/run/docker.sock`
- `--device` 指向任意宿主设备
- `--pid host`、`--ipc host`
- 添加不受控 capability

这些需求应变成 typed runtime capability，并同时做权限、目标能力和平台策略检查。`container_param` 只能作为本地开发/受信系统 App 的临时逃生口，不能成为公开 App Store 的常规配置方式。

`start_param` 必须明确究竟是 argv、entrypoint override 还是 shell 字符串；在语义和转义规则冻结前，第三方 SDK 不应推荐使用。

### 6.7 目前缺失但 self-host App 很关心的字段

后续 schema 需要评估加入：

- healthcheck/readiness：路径、协议、超时、启动宽限期。
- shutdown：优雅停止时间和信号。
- run-as：容器 UID/GID、目录 ownership 初始化策略。
- secret binding：secret store 到 env/file 的绑定。
- dependency/component：Web、worker、database、cache 等组件关系。
- resource request/limit：CPU、内存、磁盘、GPU。
- backup/reset policy：哪些目录备份、卸载保留、软重置删除。
- migration/init job：升级前后数据库迁移和幂等初始化。

例如 FileBrowser 镜像以非 root 用户运行。只创建宿主目录但不处理 ownership，可能导致镜像获得 `rw` mount 后仍无法写入。不能继续用 `container_param --user ...` 掩盖这个问题。

## 7. 常见 self-host 场景矩阵

| 场景 | 持久数据 | 常见额外权限 | 网络/端口 | 主要注意事项 |
| --- | --- | --- | --- | --- |
| 无状态 Web/API | 无或 AppData | 通常无 | HTTP Web route | `instance_volume` 可 disabled |
| File Browser | AppData 保存配置/DB；User Home 保存文件 | `user/home` | HTTP Web route | Home 权限与 `/srv` mount 必须绑定 |
| 媒体服务器 | AppData 保存配置；Library 保存媒体；cache 保存转码 | `zone/library`，联网元数据时可选 `wan` | HTTP Web route | 媒体默认只读，硬件转码用 typed cap |
| 照片管理 | AppData/DB；Library 或 User Home 保存原图；cache 保存缩略图 | `user/home` 或 `zone/library`；可选 `wan` | Web + worker | 原图与索引不能放同一个清理策略 |
| Git 服务 | AppData 保存仓库和配置 | 可选 `wan` | Web + TCP SSH | 不需要 User Home；SSH 端口冲突需重算 Plan |
| 下载器/同步器 | AppData 保存状态；User Home/Library 保存结果 | `wan` + `user/home`/`zone/library` | Web | 写权限和出站联网都是高风险 |
| 密码管理器 | AppData 或托管 DB | 通常无，邮件通知时可能 `wan` | Web | secret、备份、TLS/header 配置最重要 |
| Home Automation | AppData 保存配置 | `lan`、`devices/iot` | Web + discovery | 不应用 `--network host` 代替受控发现能力 |
| 监控面板 | AppData；可能无业务文件 | `lan`，按需 KAPI | Web | 只读原则；metrics 管理端口不外露 |
| 备份工具 | AppData 保存任务；Home/Library/外部卷保存源和目标 | `user/home`、`zone/library`、外部 Storage Grant、可选 `wan` | Web/后台任务 | 权限最高，应逐目录、逐动作授权 |

## 8. 完整示例

### 8.1 只有自身数据的普通 Web App

该 App 不读取用户文件，也不主动联网：

```json
{
  "permissions": [],
  "service_config_tips": {
    "data_mount_points": {
      "/var/lib/myapp": null
    },
    "service_endpoints": {
      "www": {
        "protocol": "http",
        "inner_port": 8080,
        "required": true,
        "expose": {
          "route": {
            "type": "web"
          },
          "scope": "",
          "allow_guest": false
        }
      }
    },
    "instance_volume": {
      "mode": "disabled"
    }
  }
}
```

### 8.2 FileBrowser：`/srv` 访问安装用户 Home

FileBrowser 镜像使用 `/srv` 保存被浏览文件，使用 `/database` 和 `/config` 保存自身状态。目标配置应为：

```json
{
  "permissions": [
    {
      "scope_path": "user/home",
      "required": true,
      "actions": [
        "read",
        "write"
      ],
      "exp": null
    }
  ],
  "service_config_tips": {
    "data_mount_points": {
      "/srv/": {
        "mount_point_name": "${BUCKYOS_USER_HOME_DIR}",
        "access": "read_write",
        "reason": {
          "en": "FileBrowser needs access to the installing user's Home to browse, upload, rename, and delete files.",
          "zh-CN": "FileBrowser 需要访问安装用户的 Home，以浏览、上传、重命名和删除文件。"
        }
      },
      "/database/": {
        "mount_point_name": "${BUCKYOS_DATA_DIR}/database",
        "access": "read_write",
        "reason": {
          "en": "Persistent FileBrowser database owned by this App instance.",
          "zh-CN": "当前 App 实例专用的 FileBrowser 持久数据库。"
        }
      },
      "/config/": {
        "mount_point_name": "${BUCKYOS_DATA_DIR}/config",
        "access": "read_write",
        "reason": {
          "en": "Persistent FileBrowser configuration owned by this App instance.",
          "zh-CN": "当前 App 实例专用的 FileBrowser 持久配置。"
        }
      }
    },
    "service_endpoints": {
      "www": {
        "protocol": "http",
        "inner_port": 80,
        "required": true,
        "expose": {
          "route": {
            "type": "web"
          },
          "scope": "",
          "allow_guest": false
        }
      }
    },
    "instance_volume": {
      "mode": "disabled"
    }
  }
}
```

如果产品只允许浏览、不允许修改文件，应同时改为：

```json
{
  "scope_path": "user/home",
  "required": true,
  "actions": [
    "read"
  ],
  "exp": null
}
```

以及：

```json
{
  "mount_point_name": "${BUCKYOS_USER_HOME_DIR}",
  "access": "read_only",
  "reason": {
    "en": "Browse the installing user's files without modifying them.",
    "zh-CN": "只浏览安装用户的文件，不进行修改。"
  }
}
```

### 8.3 媒体服务器：配置可写、媒体只读、缓存可删除

```json
{
  "permissions": [
    {
      "scope_path": "zone/library",
      "required": true,
      "actions": [
        "read"
      ],
      "exp": null
    },
    {
      "scope_path": "wan",
      "required": false,
      "actions": [
        "connect"
      ],
      "exp": null
    }
  ],
  "service_config_tips": {
    "data_mount_points": {
      "/config": null,
      "/media": {
        "mount_point_name": "${BUCKYOS_ZONE_LIBRARY_DIR}/media",
        "access": "read_only",
        "reason": {
          "en": "Read media stored in the Zone library.",
          "zh-CN": "读取 Zone 资料库中的媒体文件。"
        }
      }
    },
    "local_cache_mount_points": {
      "/cache": {
        "mount_point_name": "${BUCKYOS_APP_CACHE_DIR}/transcode",
        "access": "read_write",
        "reason": {
          "en": "Rebuildable thumbnails and transcoding cache.",
          "zh-CN": "可重新生成的缩略图和转码缓存。"
        }
      }
    },
    "service_endpoints": {
      "www": {
        "protocol": "http",
        "inner_port": 8096,
        "required": true,
        "expose": {
          "route": {
            "type": "web"
          },
          "scope": "",
          "allow_guest": false
        }
      }
    }
  }
}
```

### 8.4 Git 服务：HTTP + SSH，不访问用户 Home

```json
{
  "permissions": [
    {
      "scope_path": "wan",
      "required": false,
      "actions": [
        "connect"
      ],
      "exp": null
    }
  ],
  "service_config_tips": {
    "data_mount_points": {
      "/data": null
    },
    "service_endpoints": {
      "www": {
        "protocol": "http",
        "inner_port": 3000,
        "required": true,
        "expose": {
          "route": {
            "type": "web"
          },
          "scope": "",
          "allow_guest": false
        }
      },
      "ssh": {
        "protocol": "tcp",
        "inner_port": 22,
        "required": false,
        "expose": {
          "route": {
            "type": "port",
            "preferred_port": 2222
          },
          "scope": "",
          "allow_guest": false
        }
      }
    }
  }
}
```

Git 仓库属于该 App 的业务数据，放 AppData 即可；不要因为仓库也是“文件”就申请整个 `user/home`。

### 8.5 Home Automation：局域网和设备权限

```json
{
  "permissions": [
    {
      "scope_path": "lan",
      "required": true,
      "actions": [
        "connect"
      ],
      "exp": null
    },
    {
      "scope_path": "devices/iot",
      "required": false,
      "actions": [
        "read",
        "write"
      ],
      "exp": null
    }
  ],
  "service_config_tips": {
    "data_mount_points": {
      "/config": null
    },
    "service_endpoints": {
      "www": {
        "protocol": "http",
        "inner_port": 8123,
        "required": true,
        "expose": {
          "route": {
            "type": "web"
          },
          "scope": "",
          "allow_guest": false
        }
      }
    }
  }
}
```

如果镜像依赖 mDNS、SSDP、广播或 host network，当前 schema 还不能安全、精确地表达；不要直接塞进 `container_param`。应先补 typed discovery/network capability。

## 9. 默认 InstallPlan 应怎样生成

目标算法：

1. Resolver 得到可信 AppDoc，确定 `owner_user_id`、目标 Node、OS/arch 和 App instance identity。
2. Installer 为目标 Node 构造受控模板变量表。
3. 遍历 `data_mount_points`：
   - value 为 `null`：选择标准 AppData，`read_write`。
   - value 为对象：展开 `mount_point_name`，采用声明的 `access`。
4. 遍历 `local_cache_mount_points`，映射到标准 App Cache 或其子目录。
5. 遍历 `external_mount_points`；没有具体 Storage Grant 时标记需要用户选择，不能创建猜测路径。
6. 对展开路径进行规范化、安全检查和逻辑根分类。
7. 从逻辑根和 access 推导所需权限，和 AppDoc 声明及用户批准项做交叉校验。
8. 自动选择 `required` 权限作为默认 Plan 的待批准权限；optional 权限只作为选项展示。
9. 自动启用 required endpoint，应用默认 expose 建议；端口冲突时要求重新选择。
10. 检查 required env/secret/database binding、目标能力和运行限制。
11. 生成最终 `service_spec_config`，把展开后的 mount、权限、env binding、endpoint 和目标快照纳入 fingerprint。
12. 用户确认后冻结 Plan；任何修改都重新 Inspect。

以下情况必须让 `config readiness` 变成 `NOT_READY`：

- 必需 mount 无法解析或指向未批准的资源。
- 变量未知、展开失败或路径越界。
- mount 要求 `read_write`，但只批准了 `read`。
- required permission 被拒绝。
- required endpoint 被禁用或端口无法分配。
- required env/secret/database binding 没有值。
- 目标 Node 缺少 required runtime capability。
- 外部卷尚未选择或已经离线。

## 10. 安装 UI 应向用户展示什么

第三方开发者提供 `reason`，但风险结论必须由系统生成。安装确认页至少展示：

- App 名称、发布者、App DID、版本和目标 Node。
- 每个 container path 最终映射到的逻辑资源。
- AppData、User Home、Zone Library、External Volume 的明显分组。
- 每个 mount 的 read-only/read-write 状态。
- required 与 optional 权限，拒绝后的具体影响。
- 入站 Web/TCP 暴露与出站 WAN/LAN 权限分栏。
- 用户需要提供的普通 env、secret 和数据库绑定。
- 运行能力，如 GPU、IoT、硬件设备、局域网发现。
- 数据在卸载、升级、备份和 reset 时的保留策略。

不要只显示开发者提供的 `reason`；恶意或粗糙的 AppDoc 可能淡化风险。系统应显示标准风险文案，例如“可删除 Home 中的文件”“可向公网发送数据”。

## 11. 当前实现与目标语义的差距

下面是 2026-08-25 代码基线，不代表最终 SDK 承诺。

| 能力 | 当前行为 | 目标差距 |
| --- | --- | --- |
| default InstallParams | `default_install_params()` 只预选 required 权限 | 没有从 mount tips 生成默认 mount |
| `data_mount_points` | 只有调用方显式放入 `InstallParams.data_mount_points` 才进入最终配置 | `null` 和建议路径都没有按本文语义自动 materialize |
| `external_mount_points` | 同样只接受显式选择 | 没有 Storage Grant/文件选择器语义 |
| `local_cache_mount_points` | 会从 tips 自动构造默认配置 | 和 data/external 行为不一致 |
| `mount_point_name` | helper 会把它当 target path，但当前主要用于 local cache；data/external 的对象信息未生成默认选择 | 应统一成为建议路径表达式 |
| 环境变量展开 | mount 建议路径没有展开 | 需要受限、确定性的 Inspect-time expansion |
| `null` | 只是“没有 MountPointInfo” | 尚未实现“标准 AppData”语义 |
| 路径安全 | node-daemon 拒绝 `..` 并把 target 限制在 `BUCKYOS_ROOT` 下 | 仍允许猜测 root 内其它敏感目录；缺少逻辑资源 allowlist |
| mount 与权限 | planner 校验权限选择是 AppDoc 声明的精确子集 | 没有校验 `user/home`、`zone/library` 与实际 mount 的对应关系 |
| access 与 actions | mount access 和 permission actions 独立 | 没有阻止 `read` 权限配 `read_write` mount |
| `read_write_append` | node-daemon 当前映射为普通 Docker `rw` | 尚未提供 append-only enforcement |
| `bash_envs` | InstallParams 值会写入 `ServiceSpecConfig` | 当前 app loader 未看到把它们注入容器的实现，也没有 secret binding |
| `runtime_caps` | 从 AppDoc 复制到最终配置 | 当前 node-daemon 未执行这些 capability |
| `instance_volume.mode` | 配置会进入 spec | worker 当前仍总是创建/挂载 instance volume，mode 未形成执行约束 |
| `container_param` | 会经过 shell-word splitting 后直接追加到 `docker run` 参数 | 对公开第三方 App 权限过大，缺少 policy/capability gate |
| `start_param` | 被保存到 spec | 当前 AppLoader 没有消费路径 |
| `exp` | PermissionItem 可以保存数值 | 没有稳定的期限单位、续期和 enforcement 契约 |
| managed RDB | SDK 可以按 spec 解析实例连接 | 未适配的第三方镜像不会自动得到它期待的数据库 env/secret |
| healthcheck/run-as | AppDoc tips 没有稳定 typed 字段 | self-host 镜像的健康检查和非 root 写权限无法可靠表达 |

主要代码入口：

- `src/kernel/buckyos-api/src/app_doc.rs`：`ServiceConfigTips`、`MountPointInfo`、endpoint、env、instance volume schema。
- `src/kernel/buckyos-api/src/permission.rs`：当前 permission scope 常量。
- `src/kernel/buckyos-api/src/app_install.rs`：`InstallParams`、`InstallPlan`、readiness。
- `src/frame/control_panel/src/app_install_planner.rs`：default params、permission selection、Plan 生成。
- `src/frame/control_panel/src/app_install_deployer.rs`：tips + InstallParams 到 `ServiceSpecConfig` 的合成。
- `src/frame/control_panel/src/app_install_driver.rs`：mount path 安全检查、Plan verify。
- `src/kernel/node_daemon/src/app_loader.rs`：最终 Docker mount、env、instance volume、container params。
- `src/tools/buckyos-tool/modules/pikg_protocol.ts`：AppDoc/PIKG schema 校验。
- `doc/App 安装协议.md`：AppDoc schema 和 InstallPlan 协议。
- `doc/arch/11_env_contract.md`：当前 runtime 环境变量契约。

## 12. 后续 CodeAgent 改进清单

### P0：先把 mount 与权限闭环做正确

- [ ] 冻结 `null = standard AppData` 的协议和测试。
- [ ] 冻结 mount 模板变量白名单、展开阶段和跨平台行为。
- [ ] `default_install_params`/planner 能从 tips 生成默认 data/cache mount。
- [ ] 把展开后的 target、owner、access 和权限批准结果纳入 Plan fingerprint。
- [ ] 对 AppData、User Home、Zone Library、Zone Public、External Storage 做逻辑资源分类。
- [ ] 按逻辑资源自动要求对应 permission scope。
- [ ] 校验 permission actions 与 mount access 一致。
- [ ] 禁止映射其它用户 AppData、identity/security、Docker socket和任意系统目录。
- [ ] Deploy 只使用 Plan 中冻结的已展开路径，不二次展开。
- [ ] default plan、recompute、upgrade 必须使用同一套 mount materializer。

### P0：收紧危险的容器启动入口

- [ ] 普通公开 App 禁止使用 `container_param` 直接增加高风险 Docker flags。
- [ ] 将 GPU、设备、host discovery、额外 capability 变成 typed runtime grants。
- [ ] 对系统 App/本地开发 policy 的逃生口做明确审计和 warning。
- [ ] `start_param` 在语义冻结前从公开第三方 schema 移除，或改成结构化 argv。

### P1：让常见第三方镜像真正可运行

- [ ] 把批准后的 `bash_envs` 注入容器，并补变量名/value 限制。
- [ ] 增加 secret store binding，所有日志/Plan summary 做 redact。
- [ ] 增加 system-derived env/default/value_from。
- [ ] 增加 run-as/UID/GID 和 mount ownership 初始化策略。
- [ ] 落实 `instance_volume.mode` 和 quota/cleanup 语义。
- [ ] 增加 typed healthcheck/readiness/shutdown contract。
- [ ] 为托管 RDB 增加第三方容器 env/file binding。
- [ ] 外部卷使用具体 Storage Grant，不接受未经授权的路径字符串。

### P1：补安装体验

- [ ] 安装 Inspection 输出结构化 mount options、permission-to-mount 关系和标准风险等级。
- [ ] required mount/env/permission 缺失时生成可操作的 config issue。
- [ ] optional 权限明确展示拒绝后的功能降级。
- [ ] 端口冲突、卷离线、权限过期都能 recompute Plan。
- [ ] upgrade 默认继承已批准配置，但新增权限或扩大访问时必须重新确认。

### P2：支持复杂 self-host 组合

- [ ] component/dependency 模型支持 web、worker、init/migration job。
- [ ] 托管 Redis/PostgreSQL 等依赖有稳定 binding。
- [ ] 多组件独立 endpoint、healthcheck、resource limit。
- [ ] 备份、迁移、reset、卸载数据保留策略进入 typed schema。
- [ ] Files/DFS ACL API 成熟后，减少整目录 bind mount 的使用。

### 建议测试矩阵

- [ ] `null` 确定性映射到正确 owner/app 的 AppData。
- [ ] 同一个 App 给 alice/bob 安装时绝不交叉挂载。
- [ ] `${BUCKYOS_USER_HOME_DIR}` 正确要求 `user/home`。
- [ ] `read_only + read` 通过；`read_write + read` 阻塞。
- [ ] 在 append-only enforcement 落地前，`read_write_append` 不得产生误导性授权结果。
- [ ] 未知变量、空变量、递归变量阻塞。
- [ ] `..`、绝对宿主路径、Windows drive/UNC、软链接逃逸被拒绝。
- [ ] 其它 AppData、identity/security 和 Docker socket 被拒绝。
- [ ] default、recompute、upgrade 生成相同展开结果。
- [ ] 目标 Node/owner 变化导致旧 fingerprint 失效。
- [ ] external volume 离线不创建同名空目录。
- [ ] required permission 被拒绝时不 Deploy。
- [ ] secret 不出现在日志、错误、Plan summary 和持久化明文中。
- [ ] `instance_volume.disabled` 确实不创建 named volume。
- [ ] 公开 App 的危险 `container_param` 被拒绝。
- [ ] Linux/macOS/Windows 的逻辑路径映射行为一致。

## 13. SDK 发布前的第三方开发者检查表

### 数据与目录

- [ ] 每个持久目录都已声明。
- [ ] App 自己的数据使用 AppData；可重建数据使用 cache。
- [ ] 只有确实需要时才访问 User Home/Zone Library。
- [ ] 读权限能满足时没有申请写权限。
- [ ] 没有宿主绝对路径、Docker socket或其它系统目录。
- [ ] 升级、卸载、reset、备份时的数据语义明确。

### 权限

- [ ] 每个 permission 都能对应到一个真实功能。
- [ ] `required` 只用于核心功能。
- [ ] `actions` 使用最小集合。
- [ ] 入站服务暴露没有误写成 `wan`。
- [ ] 多用户访问没有通过共享 owner Home 绕过 ACL。

### 网络与服务

- [ ] 声明了真实 container port。
- [ ] Web 服务让 ZoneGateway 终止 TLS。
- [ ] `allow_guest` 默认关闭。
- [ ] SSH/TCP 等端口允许 Installer 调整冲突。
- [ ] 管理、metrics、debug 端口没有默认公开。

### 配置与密钥

- [ ] AppDoc 不包含密码、token、private key。
- [ ] required env 有安全的输入或派生来源。
- [ ] 第三方数据库依赖不是只有一条无法消费的 `rdb_instances` 声明。
- [ ] 非 root 镜像的 UID/GID 和目录 ownership 已验证。

### 运行安全

- [ ] 不使用 `--privileged`、host network、Docker socket。
- [ ] GPU、IoT、USB、LAN discovery 使用 typed capability。
- [ ] 有可验证的 healthcheck/readiness。
- [ ] 在默认 InstallPlan 下实际启动并完成读写测试。

## 14. 本文的核心结论

1. `service_config_tips` 是**可确认配置的声明与建议**，不是最终运行配置。
2. `data_mount_points` 的 key 是容器路径；`null` 表示标准 AppData，对象值表示可展开的建议来源路径。
3. AppData 不需要特殊权限；User Home、Zone Library、Zone Public 和外部存储必须显式授权。
4. mount 来源、access 与 permission scope/actions 必须由 Installer 做强绑定校验。
5. 环境变量必须在 Inspect 阶段按白名单确定性展开，并进入 Plan fingerprint。
6. Web 入站暴露、WAN 出站、LAN 访问是三个独立维度。
7. self-host Docker 的业务数据、缓存、密钥、数据库和执行卷必须分别建模。
8. 自由 Docker 参数不是正式 SDK 能力；高风险需求必须转换成 typed capability。
9. 当前代码已经有 AppDoc、InstallPlan 和 loader 的基本骨架，但 mount 默认值、权限耦合、env/secret 和 runtime grant 仍需补齐后才能形成可信的第三方 SDK。
