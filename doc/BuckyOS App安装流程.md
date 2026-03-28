# BuckyOS 应用安装流程

> 本文档的首要目的是列出系统必须支持的内核接口，并围绕这些接口梳理应用的完整生命周期。  
> 应用的发布与获取基于 RepoService 的通用内容原语（store / collect / pin / serve / announce），PKG 是 Content Network 中的一种 NamedObject 类型。  
> 每个关键步骤标注了产生的**证明（Proof）**类型，这些证明构成内容网络的传播关系图，是经济模型和信任模型的基础数据。

---

## 1. 应用类型

BuckyOS 内核视角下，应用分为三类：

### 1.1 静态网页应用（Static Web App）

本质是一组文本 Web 文件夹，由一个子域名指向该文件夹。无后端逻辑、无需运行服务，所有业务逻辑通过前端 JS 脚本实现。

### 1.2 Docker 应用（Standard Docker App）

最广泛支持的应用类型，也是连接 BuckyOS 与现有应用生态的主要桥梁。

- **不集成 SDK 的情况**：BuckyOS 将应用视为普通 HTTP upstream。系统负责流量路由——非公开应用的流量经网关转发时必须携带系统级 Session Token，从而获得 BuckyOS 认证体系的保护。应用内部可能仍保留自有身份系统（如 WordPress），此时存在系统级认证与应用级认证两套体系并存的问题。
- **集成 SDK 的情况**：应用可复用 BuckyOS SSO 联合登录，不再自建身份系统。通过 SDK API 判断用户登录状态，未登录时弹出 SSO 对话框。集成 SDK 的应用倾向于无状态设计，使用系统提供的资源分配函数获取存储实例。
- **数据持久化**：未集成 SDK 的传统应用通过 Data Mount 将容器内数据落盘到 Docker 外部。
- **协议扩展**：系统默认仅支持 HTTP 访问；若应用需要其他协议，需与系统网关对接。

Docker 应用的能力向下兼容静态网页应用。

### 1.3 Agent 应用（Agent App）

- **Agent Environment**：每个 Agent 的核心是一组配置文件（Agent Environment）。初始化时从开发者发布的包中 fork 一份。fork 后通过文件 include 链接上游；若 Agent 自演化过程中切断了 include 路径，则脱离上游更新，实现独立进化。
- **用户感知**：Agent 的安装语义为"领养"（adopt），领养后可切断与原始构造环境的联系。
- **运行时（Runtime）**：Agent Environment 需要一个 Runtime 才能运行。系统默认提供 OpenRuntime（一个容器）。不同 Agent 的权限本质上是 Environment 指定配置后在 Runtime 中运行的结果。
- **多实例特性**：与普通应用（一个 image 一个实例）不同，Agent 应用一个 image 可对应多个实例，每个实例拥有独立的 Agent Environment 根目录。
- **权限与数据挂载**：用户可将系统中的各类数据以只读或读写方式 mount 到 Agent Environment，赋予不同级别的能力。

---

## 2. 核心概念

### 2.1 PKG 作为 NamedObject

PKG 是 Content Network 中的一种 NamedObject。它遵循与所有内容（视频、音乐、文档等）相同的 RepoService 原语，没有特殊地位。

| 概念 | 说明 |
|------|------|
| **PKG** | 带 metadata 的文件夹 zip 包，是一个 NamedObject，打包后得到 content_id |
| **content_id** | PKG 的唯一内容寻址标识 |
| **content_name** | 基于 BNS 的全网唯一域名标识，格式如 `did:bns:app1.alice` |
| **Install Spec** | 安装配置清单，描述运行时权限、端口、数据挂载等决策项；PKG 本身纯静态，所有安装选项由 Install Spec 决定 |
| **Scheduler（调度器）** | 负责应用实例的目标状态管理与容器编排 |

### 2.1.1 package-lib 实现类型（参考 cyfs-ndn package-lib）

以下类型对应 package-lib 中的实际实现，可与上述概念对应：

| 概念 | package-lib 类型 | 说明 |
|------|------------------|------|
| **PKG 元数据** | `PackageMeta` | 继承 `FileObject`，含 `version`、`version_tag`、`deps`；`content` 指向 tar.gz 的 chunk_id/chunklist_id |
| **包标识** | `PackageId` | `{ name, version_exp?, objid? }`；格式：`pkg_name`、`pkg_name#version`、`pkg_name#version#objid` |
| **meta_obj_id** | `ObjId` | `PackageMeta.gen_obj_id()` 得到的对象 ID，对应 content_id 的元数据层面标识 |
| **包环境** | `PackageEnv` | `{ work_dir, config, lock_db }`，管理安装目录、索引与加载 |
| **环境配置** | `PackageEnvConfig` | `prefix`、`enable_link`、`enable_strict_mode`、`parent`、`named_store_config_path` 等 |
| **加载结果** | `MediaInfo` | `{ pkg_id, full_path, media_type }`，`load()` 返回的包物理路径信息 |
| **元数据索引** | `MetaIndexDb` | SQLite：`pkg_metas`（metaobjid, pkg_meta, author）、`pkg_versions`（pkg_name, version, metaobjid, tag） |


### 2.2 RepoService 原语与证明

RepoService 是 Content Network 的核心节点组件，管理两样东西：**内容本体**（节点）和**传播关系**（边）。

| 原语 | 语义 | PKG 场景 | 产生的证明 |
|------|------|---------|-----------|
| **store** | 创作者保存自己的内容 | 开发者存入 PKG | 无 |
| **collect** | 收录 meta，建立关系 | 用户得知应用存在 | referral action（若经分享） |
| **pin** | 实体钉在本地 | PKG 下载完成 | download action |
| **unpin** | 释放本地实体 | 删除本地 PKG | 无（历史证明保留） |
| **uncollect** | 彻底移除 | 遗忘该应用 | 无（历史证明保留） |
| **serve** | 响应外部请求 | 向他人提供 PKG | download action |
| **announce** | meta 发布到 BNS | 正式发布应用 | 无 |

### 2.3 四种证明

当前接口层里，“证明”已经落到两类标准对象：

| 领域概念 | 当前对象类型 | 关键字段 | 经济模型用途 |
|---------|-------------|---------|------------|
| **收录证明** | `InclusionProof` | `content_id` / `content_obj` / `curator` / `collection` / `rank` | 收录/背书的凭证 |
| **转发证明** | `ActionObject(action=shared)` | `subject` / `target` / `base_on?` | 追溯传播链 |
| **下载证明** | `ActionObject(action=download)` | `subject` / `target` / `base_on?` | 分发节点贡献凭证 |
| **安装证明** | `ActionObject(action=installed)` | `subject` / `target` / `base_on?` | 真实安装/消费凭证 |

`ActionObject.base_on` 支持把 `shared / download / installed` 串成动作链。

### 2.4 PKG 在节点上的生命周期状态

```
  未知           collect(meta)              pin(content_id)
───────  ─────────────────────►  已收录  ──────────────────►  已持有
                + referral_action (collected)  + download_action (pinned)
                  (若经分享)       持有 meta                    持有 meta + 实体
                                  可浏览详情                    可本地安装
                                  可在线流式安装                 可对外 serve
```

对于开发者自己的 PKG，使用 `store` 直接进入 pinned 状态，origin = local。

---

## 3. 内核接口清单

### 3.1 RepoService 接口（通用内容原语）

这些接口不是 PKG 专用的，是 Content Network 的通用基础设施。

#### 写操作

| 接口 | 说明 | 产生的证明 |
|------|------|-----------|
| `POST /kapi/repo` + `method=store` → `ObjId` | 创作者保存内容到 RepoService（origin=local, status=pinned）。**原子操作。** | 无 |
| `method=collect(content_meta, referral_action?) → content_id` | 收录 meta，不下载实体（origin=remote, status=collected）。若携带 `ActionObject(action=shared)` 则校验后落盘。**原子操作。** | referral action |
| `method=pin(content_id, download_action) → bool` | 标记本地持有。前提：Downloader 已将实体下载到本地。校验哈希 + `ActionObject(action=download)` 后落盘。**原子操作。** | download action |
| `method=unpin(content_id, force) → bool` | 释放本地实体，回到 collected。origin=local 需 force。历史证明保留。**原子操作。** | 无 |
| `method=uncollect(content_id, force) → bool` | 彻底移除记录。若 pinned 先 unpin。历史证明保留。**原子操作。** | 无 |

#### 证明管理

| 接口 | 说明 |
|------|------|
| `method=add_proof(proof: RepoProof) → proof_id` | 直接写入一条 proof（如 install action 或外部生成的 `InclusionProof`）。**原子操作。** |
| `method=get_proofs(content_id, filter?) → [RepoProof]` | 查询某内容的所有 proof。filter 当前保留 `proof_type / from_did / to_did / start_ts / end_ts` 这组兼容字段。 |

#### 读操作

| 接口 | 说明 |
|------|------|
| `method=resolve(content_name) → [ObjId]` | 返回本地 Repo 视角下已经 pinned 的对象 ID 列表；不负责从 BNS 拉远端 meta。 |
| `method=list(filter?) → [repo_record]` | 列出本地记录，支持按 status / origin / content_name / owner_did 过滤。 |
| `method=stat() → repo_stat` | 返回本地仓库统计信息。 |

#### 对外服务

| 接口 | 说明 | 产生的证明 |
|------|------|-----------|
| `method=serve(content_id, request_context) → RepoServeResult` | 响应 Zone 内/外请求。仅 pinned 可 serve。免费放行，收费验 receipt。通过后生成 `download_action` 并返回结构化结果 `{ status, content_ref?, download_action?, reject_code?, reject_reason? }`。 | download action |

#### 发布辅助

| 接口 | 说明 |
|------|------|
| `repo.announce(content_id) → bool` | 将 origin=local 的内容 meta 发布到 BNS 智能合约。 |

### 3.2 应用实例化（AppInstaller / apps.*）

Control Panel 通过 `AppInstaller`（`src/frame/control_panel/src/app_installer.rs`）管理应用生命周期，对应 RPC 模块 `apps.*`。底层基于 **spec 驱动**：写入 `users/{uid}/apps/{app}/spec` 或 `users/{uid}/agents/{app}/spec`，调度器（schedule_loop 每 5s）读取 spec 变化并执行实例分配/回收。

| 接口 | 说明 | 返回值 | 产生的证明 |
|------|------|--------|-----------|
| `install_app(spec: AppServiceSpec)` | 写 spec（state=New）到 system_config，触发调度器选点并分配 InstanceReplica。**前提：content_id 在 RepoService 中 status=pinned。** spec 中 app_index 由系统自动分配。 | task_id | **install action**（通过 `repo.add_proof` 落盘） |
| `start_app(app_id)` | 改 spec.state 为 Running，触发调度器分配 app_service_instance_config。 | task_id | 无 |
| `stop_app(app_id)` | 改 spec.state 为 Stopped，触发调度器删除 app_service_instance_config。 | - | 无 |
| `upgrade_app(spec: AppServiceSpec)` | 内部先 stop 当前实例，再覆盖 spec，触发调度器重新分配新版本实例。 | - | install action（新版本） |
| `uninstall_app(app_id, is_remove_data)` | spec.state → Deleted，执行 stop_app，等待调度器 RemoveInstance。 | - | 无 |
| `get_app_service_spec(app_id)` | 查询当前 spec。 | AppServiceSpec | - |
| `get_app_service_instance_config(app_id)` | 查询实例状态（ServiceInstanceReportInfo）。 | ServiceInstanceReportInfo | - |

**调度器视角**（`doc/arch/scheduler.md`）：输入 `users/*/apps|agents/*/spec`（非 Static app）；Step2 处理 `New`→选点+InstanceReplica、`Deleted`→RemoveInstance；输出写入 `nodes/{node}/config.apps`；node-daemon 读 config 收敛实例。

---

## 4. 应用生命周期流程

### 4.1 开发者发布

```
打包 PKG (zip + metadata)
        │
        ▼
  repo.store(pkg_path)              ← origin=local, status=pinned
        │                              证明：无（自己的内容）
        ▼
  repo.announce(content_id)          ← meta 发布到 BNS
        │
        ▼
  (可选) 社交传播 meta               ← 群组、分享链接等
```

store 完成后，开发者节点的 RepoService 即可通过 serve 响应其他节点请求。

### 4.2 用户发现与安装

```
用户通过任意渠道获得 pkg_meta
        │
        │  若经社交分享：分享者创建 referral_action
        │
        ▼
  repo.collect(app_pkg_meta, referral_action?)
        │  meta 入库, status=collected
        │  ✎ proof 落盘：shared action（记录"从谁那里得知"）
        ▼
  构造 AppServiceSpec (UI 决策，基于app_pkg_meta)
        │  权限、端口、数据挂载等
        │  用户可浏览应用详情
        ▼ (用户决定安装)
  (若收费) 链上付费 → 获得 receipt
        │
        ▼
  downloader.download(app_pkg_meta)
        │  协议层从网络拉取 PKG 实体
        │  传输完成时生成 download_action
        ▼
  repo.pin(app_pkg_meta, download_action)
        │  哈希校验, status=pinned
        │  ✎ proof 落盘：download action（记录"从哪个节点下载"）
        │
        ▼
  AppInstaller::install_app(AppServiceSpec)
        │  写 users/{uid}/apps|agents/{app}/spec (state=New)
        │  ✎ proof 落盘：install action（通过 repo.add_proof）
        │
        ▼
  scheduler schedule_loop（每 5s）
        │  Step2: New → 选点 + InstanceReplica → nodes/{node}/config.apps
        ▼
  node_daemon 读 nodes/{node}/config
        │  收敛实例，启动容器
        ▼
      应用运行
```

**一次完整安装产生的证明链：**

```
referral_action    →  download_action    →  install_action
(谁推荐给我的)        (从谁那里下载的)        (我确实安装了)
```

这三条动作边可通过 `ActionObject.base_on` 串成链，再结合内容本体的 meta（owner 信息），用于经济模型追踪传播路径。

### 4.3 应用升级

升级的本质：同一个 content_name 下出现了新版本的 content_id。

```
  上层通过 BNS/索引源获得最新 meta
        │  若通过 UI 构造 AppServiceSpec，需继承旧配置 + 补充新字段
        ▼ (发现新版本)
  repo.collect(new_pkg_meta)               ← 新版 meta 入库, status=collected
        │                                    ✎ 可能有新的 referral_action
        ▼
  downloader.download(new_pkg_meta)      ← 下载新版 PKG 实体
        │
        ▼
  repo.pin(new_pkg_meta, download_action) ← 新版实体就绪
        │                                     ✎ download action 落盘
        ▼
  AppInstaller::upgrade_app(new_AppServiceSpec)
        │  内部：stop_app → 覆盖 spec → 触发调度器
        │  ✎ install action 落盘（新版本的安装证明）
        ▼
  scheduler schedule_loop
        │  New → 选点 + InstanceReplica → nodes/{node}/config.apps
        ▼
  node_daemon 读 config 收敛实例
        │
        ▼
      新版本运行
```

**设计原则**：

- 新旧版本在 RepoService 中始终共存（content_id 不同，各自独立记录）
- 每个版本的证明链独立存在，不会因升级而覆盖
- 用户可 100% 回退到任意已 pin 过的版本，应用开发者无权阻止回退
- `repo.resolve(content_name)` 可列出本地已经 pinned 的版本 `ObjId`

### 4.4 应用卸载

卸载分为四个递进层面，每一层都是独立的用户决策：

**第一层：停止实例**

```
  AppInstaller::stop_app(app_id)          ← 改 spec.state 为 Stopped，调度器删除 instance config
```

**第二层：删除实例**（常规卸载，Control Panel 主入口）

```
  AppInstaller::uninstall_app(app_id, is_remove_data)
        │  spec.state → Deleted，执行 stop_app，等待调度器 RemoveInstance
        │  is_remove_data 控制是否清理应用数据目录
        ▼
  调度器 RemoveInstance → 清理 nodes/{node}/config 中的实例配置
```

此时 PKG 仍在 RepoService 中（pinned 状态），可随时通过 install_app 重新创建实例。

**第三层：释放 PKG 实体**（释放存储空间）

```
  repo.unpin(content_id)                   ← 删除本地实体，回到 collected
```

meta 仍在，用户仍"知道"这个应用。如需重新安装，只需再走 download → pin 流程。**历史证明保留。**

**第四层：彻底遗忘**

```
  repo.uncollect(content_id)               ← 连 meta 都删除
```

从节点上彻底移除该应用的 objects 记录。**历史证明仍保留在 proofs 表中**，经济结算和信任模型仍可回溯。

> 卸载过程中，Install Spec 中挂载的外部引用目录（如用户 home 数据）不会被删除。共享目录中的应用子目录需用户主动清理。

---

## 5. 内容分发网络（Content Network）

### 5.1 RepoService 在网络中的角色

每个 OOD 节点运行一个 RepoService。不同角色使用同一组接口，区别仅在使用姿势和产生的证明类型：

| 角色 | 主要操作 | 主要产生的证明 |
|------|---------|--------------|
| **创作者** | store + announce | 无（自己的内容） |
| **收录者** | collect + add_proof(InclusionProof) | collection proof |
| **传播者** | 创建 `ActionObject(action=shared)` 分享给他人 | shared action |
| **消费者** | collect + pin + install_app | shared / download / installed action |
| **渠道商** | collect + pin + serve | download action（作为 serve 端） |

### 5.2 传播机制

meta 的构造与传播解耦，传播路径可后置选择：

- **BNS / 上层索引源**：默认发现路径。当前 `repo.resolve(content_name)` 只看本地 pinned 版本，不承担远端发现职责
- **社交传播**：将 meta 发送到群组，订阅者收到后自主 collect。分享时创建 `ActionObject(action=shared)`
- **渠道分发**：渠道商 collect + pin 后，通过自己的 serve 提供下载，可附加分成逻辑

### 5.3 付费与收据

| 概念 | 说明 |
|------|------|
| **模型** | 仅两种：免费 和 付费买断。不支持订阅制——用户直接向创作者购买，没有平台中间商 |
| **Receipt** | 链上智能合约签发的购买记录，或发行者私钥签发的购买证明 |
| **校验粒度** | 基于 content_name（非 content_id），同名内容升级不需重新购买 |
| **免费内容** | serve 时同样产生 download action，用于生态激励和传播统计 |

### 5.4 Content Network 与 Control Panel Content Sharing 的区别

这是两个平行的系统：

| | Control Panel Content Sharing | Content Network |
|---|---|---|
| **本质** | 系统文件管理的延伸 | 围绕 RepoService 构建的协议化网络 |
| **协议化** | 非协议化，随系统能力升级 | 基于 cyfs:// 协议、签名验证与 BNS |
| **传播关系** | 不记录 | 通过四种证明完整记录传播链 |
| **典型场景** | 分享目录给好友，实时看到更新 | 正式发版，通过 NamedObject 体系分发 |
| **网络组件** | 无 | RepoService 是唯一对外网络组件 |

用户在同一页面中可能同时看到"分享"和"发布"两个操作，但底层走完全不同的体系。例如：开发者在目录中自由构建静态前端应用，通过 Content Sharing 与好友实时共享预览，确认完成后点击 Publish 触发 `repo.store` + `repo.announce` 进入 Content Network 发行流程。

---

