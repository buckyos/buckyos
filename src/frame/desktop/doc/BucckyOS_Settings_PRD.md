# 《BuckyOS Settings PRD》



---

## 1. 文档概述

### 1.1 背景

BuckyOS 已进入首个面向 2C Public Research 版本的发布阶段。此前 Settings 更偏向原型或隐性能力承载，但在 V1 发布阶段，Settings 需要承担更明确的产品职责：

1. 作为系统能力的保底入口，承接尚未做完专用 UI 的配置能力。
2. 作为系统信息、隐私状态、网络连接状态和调试信息的统一可视化面板。
3. 作为工程支持和高级用户的 Debug 入口，便于快速定位问题。

### 1.2 文档目标

本文定义 BuckyOS Settings 在 V1 阶段的：

- 产品定位
- 信息架构
- 各模块功能范围
- 关键交互规则
- UI 设计原则
- 数据与权限抽象
- V1 交付边界与后续演进方向

### 1.3 核心结论

Settings 在 V1 阶段不是“最精致的体验入口”，而是：

> **系统能力的全集视图 + 功能兜底层 + 调试入口**

其设计优先级为：

1. 能用
2. 能找到
3. 不容易误操作
4. 再考虑精细体验

---

## 2. 产品定位与设计原则

### 2.1 Settings 的定位

Settings 是系统的 **Fallback Layer（能力兜底层）**。

当某个组件或能力：

- 还没有想明白完整交互
- 没时间做专用页面
- 还在快速迭代
- 或更适合高级用户/工程师查看

则可先落到 Settings 中，优先解决“有没有”，再解决“好不好”。

### 2.2 Settings 的职责

#### 2.2.1 功能兜底

所有系统能力都应至少有一个 Settings 入口，哪怕它只是一个基础开关、表单、只读信息区或 JSON 视图。

#### 2.2.2 全量可观测

用户和工程师应能在 Settings 中看到系统重要配置和状态的全貌。

#### 2.2.3 Debug 入口

Settings 需要支持：

- 系统信息复制
- 日志导出
- 关键配置查看
- 网络/证书/域名状态查看
- 开发者诊断工具入口

### 2.3 哪些内容不应该长期放在 Settings

高频、强任务驱动、属于主流程的能力，长期应从 Settings 中拿走，进入专门页面。例如：

- Agent 主操作入口
- Message / Chat 主路径入口
- 高频 AI 能力选择入口
- 高频 Storage 主流程入口

Settings 中保留的内容更偏向：

- 低频配置
- 系统级配置
- 高级配置
- 调试配置
- 未成熟能力的临时承接

### 2.4 开发范式

建议 BuckyOS 未来所有新能力遵循以下上线顺序：

```text
Step 1：先提供 Settings 入口，保证能力可达
Step 2：验证能力本身是否成立
Step 3：再视需要做专用 UI
```

---

## 3. 目标用户

### 3.1 普通用户

关注：

- 当前系统版本
- 明暗模式、语言、字体大小等基础外观设置
- 哪些内容是公开的
- 自己的数据会被谁看到
- 出问题时如何把信息发给工程师

### 3.2 高级用户

关注：

- 集群/网络状态
- 域名、DID、证书、SN 转发
- 日志和配置文件查看
- 应用/Agent/设备权限的可见性

### 3.3 工程支持 / 开发者

关注：

- 系统关键配置文件
- 系统诊断结果
- 日志打包下载
- 常用 CLI 指令
- System Tester / API 调试能力

---

## 4. 范围定义

### 4.1 V1 范围内

本次 PRD 覆盖以下五个分区：

1. General（通用设置）
2. Appearance（外观，基于 Session）
3. Cluster Manager（集群管理，原 Zone Manager）
4. Privacy（隐私与访问控制）
5. Developer Mode（开发者模式）

### 4.2 V1 暂不展开或仅预留坑位

以下方向在口述中有提及，但不属于本次 PRD 重点：

- Agent 专用设置页
- Message 通道专用设置页
- AI 能力/Provider 专用设置页
- Storage 专用设置页
- Monitor / 实时监控页

这些模块后续可独立成页，Settings 中仅保留必要入口或只读信息。

---

## 5. 术语定义

### 5.1 Session

> Session = 一个配置作用域 + 一个交互上下文

说明：

- 一个用户可以拥有多个 Session。
- Session 不是用户身份本身，而是“当前环境/当前设备/当前访问形态”的抽象。
- 外观配置绑定到 Session，而不是绑定到用户全局。

#### Session 类型

- Shared Session：跨设备共享配置
- Device Session：仅绑定当前设备的配置

### 5.2 Cluster

> Cluster = 系统的部署与连接单元

即使桌面版当前只有单节点，仍按集群模型呈现，以便未来自然扩展为多节点集群。

### 5.3 Zone

> Zone = 一个对外可识别的服务入口或网络身份单元

它以 DID 进行标识，并承载域名、连接方式、证书等信息。

### 5.4 Public / Shared / Private

- Public：对外公开，全网可访问
- Shared：当前 Zone / 系统内其他用户可访问
- Private：默认仅当前用户可访问

---

## 6. 信息架构

### 6.1 顶层结构

```text
Settings
├── General
├── Appearance
├── Cluster Manager
├── Privacy
└── Developer Mode
```

### 6.2 Privacy 子结构

```text
Privacy
├── Public Access
├── Messaging Access
├── Data Visibility
├── App & Agent Access
└── Device & Capability Permissions
```

### 6.3 Developer Mode 子结构

```text
Developer Mode
├── Mode Switch
├── System Diagnostics
├── Config Explorer
├── Logs
├── CLI Helpers
└── System Tasks / System Tester
```

---

## 7. 通用交互与 UI 原则

### 7.1 平台布局

#### Desktop

- 左侧为 Settings 导航
- 右侧为当前模块内容区
- 适合展示大块只读信息、卡片、树状结构与日志下载操作

#### Mobile / 浏览器接入

- 使用分层列表
- 结构上类似传统系统设置页
- 每个分区点入后再看详情

### 7.2 统一交互原则

1. **Read-first**：默认只读优先。
2. **Safe-first**：高风险配置默认关闭或隐藏在高级项中。
3. **Copy-first**：关键系统信息支持一键复制。
4. **Advanced collapsed**：复杂或工程向信息默认折叠。
5. **User language first**：能用用户语言解释，就不直接暴露底层技术模型。

### 7.3 视觉提示模型

对权限和风险的说明使用统一颜色表达：

- 绿色：可信 / 系统默认 / 风险低
- 黄色：需注意 / 存在额外权限 / 可能影响隐私
- 红色：高风险（V1 预留）

---

## 8. 模块一：General（通用设置）

### 8.1 模块定位

General 是 Settings 的默认入口页面，用于展示系统的基础信息、静态状态和诊断入口。

### 8.2 用户目标

用户在此页面主要想完成以下事情：

1. 确认当前运行的是哪个版本。
2. 了解设备与系统环境信息。
3. 在出问题时一键复制系统信息给工程师。

### 8.3 边界

General 展示的是静态或半静态信息，不承担实时监控职责。  
CPU 实时利用率、内存实时波动、网络吞吐等动态指标属于 Monitor 范畴，不放在 General。

### 8.4 信息结构

```text
General
├── Software Info
├── Device Info
├── System Snapshot
└── Support / Debug
```

### 8.5 功能需求

#### 8.5.1 Software Info

展示字段：

- BuckyOS Version
- Build Version / Commit ID（可选）
- Release Channel（Stable / Beta / Dev）
- Last Update Time（如可获取）

操作：

- Check for Updates
- Update Now（当存在新版本时）
- Auto Update Toggle（若已支持）

#### 8.5.2 Device Info

桌面版重点展示当前运行设备的静态信息：

- OS 类型（Windows / macOS / Linux）
- OS 版本
- CPU 型号 / 核心数
- 总内存
- 总存储容量

#### 8.5.3 System Snapshot

展示与当前安装形态和设备概况相关的摘要：

- Install Mode（Desktop / Cluster）
- Node Count
- Storage Used / Total
- 已启用核心模块概览（可选）

#### 8.5.4 Support / Debug

核心动作：

- Copy System Info
- Export as JSON（可选）
- Report Issue（可选，后续接入）

建议复制内容：

```json
{
  "buckyos_version": "x.x.x",
  "os": "macOS 14.3",
  "cpu": "Apple M2",
  "memory": "16GB",
  "storage_total": "512GB",
  "storage_used": "120GB",
  "install_mode": "desktop"
}
```

### 8.6 验收标准

- 用户能在 1 次进入内看到当前软件版本。
- 用户能看到当前设备 OS、CPU、内存和存储概况。
- 用户能通过一个按钮完成系统信息复制。
- General 不展示实时监控指标。

---

## 9. 模块二：Appearance（外观，基于 Session）

### 9.1 模块定位

Appearance 是基于 Session 的 UI 与桌面体验配置中心。

其核心差异在于：

> 外观不是全局属性，而是 Session 属性。

### 9.2 核心规则

1. 用户只能修改 **当前 Session** 的配置。
2. 不允许跨 Session 修改配置。
3. 同一 Session 的配置自动同步；不同 Session 的配置相互隔离。

### 9.3 模块结构

```text
Appearance
├── Current Session
├── Session Management
├── Theme & Display
└── Desktop Layout
```

### 9.4 功能需求

#### 9.4.1 Current Session

展示：

- Session Name
- Session Type（Shared / Device）
- 当前运行环境（Desktop / Mobile / Browser 等）

#### 9.4.2 Session Management

支持操作：

1. **Rename Session**
   - 仅修改当前 Session 名称。
   - 若当前为 Shared Session，则名称变化影响所有绑定该 Session 的入口。

2. **Clone to Device Session**
   - 基于当前 Session 克隆一份新配置。
   - 新 Session 仅绑定当前设备。
   - 克隆后不再与原 Session 同步。

用户提示：

> “克隆后，该 Session 将不再与其他设备同步。”

#### 9.4.3 Theme & Display

V1 向用户开放的设置项：

- Theme Mode：Light / Dark
- Language：语言选择
- Font Size：Small / Medium / Large
- Wallpaper：桌面背景选择

V1 暂不开放但保留配置位：

- Opacity / 透明度
- 更多高级视觉参数

#### 9.4.4 Desktop Layout

属于系统自动管理或后续高级设置的内容：

- 窗口位置
- 窗口尺寸
- 桌面布局状态
- Icon Size（后续可考虑开放）

这些信息在 V1 可以作为 Session 配置的一部分存在，但不要求完整 UI 控制能力。

### 9.5 数据模型建议

```json
{
  "session_id": "session_xxx",
  "name": "Leo's MacBook",
  "type": "shared",
  "device_id": null,
  "settings": {
    "appearance": {
      "theme": "dark",
      "language": "zh-CN",
      "font_size": "medium",
      "wallpaper": "wallpaper_01"
    },
    "desktop": {
      "layout": {},
      "window_state": {}
    }
  }
}
```

### 9.6 验收标准

- 桌面与移动端可使用不同 Theme Mode。
- 用户只能修改当前 Session 配置，不能直接编辑其他 Session。
- 系统支持把当前 Session 克隆为设备专属 Session。
- 字体大小至少支持 Small / Medium / Large 三档。

---

## 10. 模块三：Cluster Manager（集群管理）

> 命名说明：本模块原始口述中为 Zone Manager，PRD 中统一使用 **Cluster Manager**，以提升用户理解一致性。

### 10.1 模块定位

Cluster Manager 是用户查看和理解“自己如何被访问”的入口，用于呈现：

- 集群规模
- 节点信息
- Zone / DID 标识
- 域名与接入方式
- DNS / IP / 端口映射
- SN 转发与流量
- 证书状态

### 10.2 模块目标

帮助用户回答：

1. 我有几个节点 / 设备？
2. 我的系统通过什么域名或 DID 被别人访问？
3. 当前流量是否走 SN 转发？
4. 为什么我的访问会慢？
5. 证书是否正常？

### 10.3 模块结构

```text
Cluster Manager
├── Cluster Overview
├── Nodes / Devices
├── Zones
├── Connectivity
├── Certificates
└── Debug & Export
```

### 10.4 功能需求

#### 10.4.1 Cluster Overview

展示：

- Cluster Mode（Single Node / Multi Node）
- Node Count
- Zone Count
- 当前 Active Zone（如适用）

说明：即使桌面版只有单节点，也沿用 Cluster 视角。

#### 10.4.2 Nodes / Devices

每个节点展示：

- Device Name
- Device ID
- Online / Offline
- 所属 Zone

V1 桌面版通常只会显示一个节点，但 UI 结构应按多节点扩展设计。

#### 10.4.3 Zones

每个 Zone 展示：

- Zone DID
- DID Method（如 did:web、did:bns，未来可扩展）
- Owner DID
- Zone 基本属性

规则：

- Owner 必须使用 BNS DID。
- Zone 自身的 DID Method 可不是 BNS，可为 did:web 或未来其他 DID Method。

#### 10.4.4 Connectivity

展示与当前接入方式相关的信息：

- 使用的是 BNS 二级域名还是用户自定义域名
- Zone 的域名接入方式
- 是否通过 SN 转发
- 当前 SN Region（如香港 / 北美）
- SN 流量使用情况（若可获得）
- 当前 DNS 解析信息
- 当前 IP 能力（IPv4 / IPv6）
- 是否具备直连条件
- 是否开启端口映射

V1 目标是“让用户看懂为什么会快 / 慢 / 能不能连”，不要求复杂操作。

#### 10.4.5 Certificates

展示证书相关信息：

- 当前证书来源：
  - Auto（域名 NS 托管到 SN，自动签发）
  - Custom（用户自行配置）
- Domain
- Issuer
- Expiry Date
- Valid / Expired 状态
- X.509 原始信息（高级/只读）

#### 10.4.6 Debug & Export

核心动作：

- Copy Cluster Info
- Copy DNS Info（可选）
- Copy Network Path（可选）

建议复制内容：

```json
{
  "zone_did": "did:web:example.com",
  "owner": "did:bns:alice",
  "sn_region": "NA",
  "relay": true,
  "ipv6": false,
  "port_mapping": false,
  "certificate": {
    "type": "auto",
    "expiry": "2026-01-01"
  }
}
```

### 10.5 交互要求

- 高级技术信息默认折叠。
- 用户需要能快速看到当前是“直连”还是“经 SN 转发”。
- 用户需要能快速知道“是不是因为地域原因而慢”。

### 10.6 验收标准

- 用户可以看到节点数、Zone 数和当前主要接入信息。
- 用户可以查看 Zone DID、Owner DID 和 DID Method。
- 用户可以看到是否走 SN、SN 区域、IPv6 状态和端口映射状态。
- 用户可以查看证书来源与有效期。
- 用户可以一键复制集群调试信息。

---

## 11. 模块四：Privacy（隐私与访问控制）

### 11.1 模块定位

Privacy 是用户理解并控制“谁能访问我、谁能看到我的数据、哪些能力被暴露”的入口。

在 V1，Privacy 的核心策略不是复杂权限编辑，而是：

> **先让用户看见，再逐步开放控制。**

### 11.2 模块结构

```text
Privacy
├── Public Access
├── Messaging Access
├── Data Visibility
├── App & Agent Access
└── Device & Capability Permissions
```

### 11.3 设计原则

1. 所有对外暴露的表面必须可见。
2. 所有风险说明尽量使用用户语言。
3. 对复杂能力的控制在 V1 以只读或灰置为主。
4. RBAC/ACL 作为底层模型存在，但不直接把复杂规则暴露给普通用户。

---

### 11.4 子模块 A：Public Access（公开访问）

#### 11.4.1 目标

让用户明确知道哪些入口是“无需认证即可访问”的。

#### 11.4.2 功能需求

V1 展示的对外域名：

1. `public.<zone_id>`
   - 默认公开
   - 无需认证即可访问
   - 必须高可见

2. `home.<zone_id>`（未来）
   - 类似私域/朋友圈入口
   - V1 仅可作为预留概念或占位说明，不要求开放配置

#### 11.4.3 UI 文案建议

- Public：Accessible without authentication
- Home：Coming soon / 未来提供

#### 11.4.4 验收标准

- 用户可以明确看到 `public.<zone_id>` 是公开入口。
- 用户不会误以为所有域名都必须登录后访问。

---

### 11.5 子模块 B：Messaging Access（消息接入）

#### 11.5.1 背景

Message Hub 存在一个可公开调用的消息投递接口，允许外部向当前 Zone 投递消息。

这也是原生 Message Tunnel 能力的基础之一。

#### 11.5.2 功能需求

展示一个开关项：

- Allow External Messages / Send Message API

V1 策略：

- 默认 ON
- UI 可展示为灰置，不允许关闭
- 配说明文案：该能力是核心消息功能所必需

#### 11.5.3 用户可理解表达

不要要求用户理解 API 细节，而是用用户语言表达：

> “是否允许别人直接给你发送消息。”

#### 11.5.4 关闭后的影响（用于说明）

若未来开放关闭：

- Message Hub 原生能力会受限
- 原生 Message Tunnel 可能无法正常使用
- 仅能依赖 Telegram / WhatsApp 等外部消息通道

#### 11.5.5 验收标准

- 用户可以看到系统存在“外部消息投递”这一能力。
- 用户能理解该能力当前处于开启且不可关闭状态。

---

### 11.6 子模块 C：Data Visibility（数据可见性）

#### 11.6.1 定位

这是 RBAC / ACL 的用户视角投影，目标是回答：

> “我的数据会被谁看到？”

#### 11.6.2 数据暴露规则

1. **Private**
   - 默认仅当前用户可见

2. **Shared**
   - 放入 Shared Folder 的数据可被当前系统内其他用户访问

3. **Public**
   - 放入 Public Folder 的数据可被外部访问

#### 11.6.3 UI 表达要求

必须从“用户文件夹视角”展示，而不是裸露系统路径模型。

推荐展示名称：

- Public Folder
- Shared Folder
- Private Data

#### 11.6.4 用户提醒

- 放进 Public Folder：等同于全网可见
- 放进 Shared Folder：等同于当前系统中其他用户可见

#### 11.6.5 验收标准

- 用户能清楚区分 Private / Shared / Public 三类数据。
- 用户能知道哪些目录的数据会被别人看到。

---

### 11.7 子模块 D：App & Agent Access（应用与 Agent 访问）

#### 11.7.1 应用访问模型

默认规则：

- 应用默认只访问当前用户自己的数据
- 这类应用为绿色状态

特殊情况：

- 某些应用为多用户共享安装，可访问多个用户数据
- 这类应用为黄色状态

系统应用说明：

- 系统应用可访问用户全部数据
- 但在用户视角中标记为绿色可信

#### 11.7.2 额外权限

关注点：应用是否申请访问超出传统应用数据目录之外的权限。

V1 当前典型例子：

- 系统 File Browser 申请访问 Home 分区 / 更大范围文件系统

#### 11.7.3 Agent 权限

Agent 本质上也是应用，但通常拥有更大工作权限。

V1 说明：

- Agent 可访问当前用户的数据，以完成任务
- 非管理员情况下，不能访问其他用户数据
- 如果是管理员能力，则需要显式提示管理员和 Agent 可能看到更多数据

#### 11.7.4 风险标识建议

- 系统应用 / 系统发行：绿色
- 第三方共享安装且可访问多用户数据：黄色
- 高风险模型：红色（V1 预留）

#### 11.7.5 验收标准

- 用户能看到系统应用为何拥有更大访问权限。
- 用户能识别哪些应用只访问自己的数据，哪些应用可访问多用户数据。
- 用户能看到 Agent 拥有数据读取能力这一事实。

---

### 11.8 子模块 E：Device & Capability Permissions（设备与能力权限）

#### 11.8.1 定位

该子模块承接传统 OS 隐私权限模型，但在 BuckyOS 中同时覆盖本地设备与 IoT 设备。

#### 11.8.2 范围

- Camera
- Microphone
- 未来的屏幕录制 / 输入设备
- IoT 摄像头与其他设备
- Docker / 容器化应用等高级系统能力

#### 11.8.3 IoT 设备共享模型

IoT 设备在系统中可能存在两种使用方式：

1. **Direct Access**
   - App 直接访问某个设备

2. **Data Routed Access**
   - 设备数据先写入某个目录（如 Shared），App 再读取数据

两种模型都需要在隐私视角中可见。

#### 11.8.4 高权限应用

例如通过 Docker 运行的 Home Assistant 等第三方服务，可能申请较多系统能力。

V1 目标：

- 让用户知道“哪些应用权限很高”
- 不强求立即提供精细控制

#### 11.8.5 控制策略

V1 以可见性为主：

- 查看：支持
- 开关：仅少量支持
- 精细权限控制：暂不要求

#### 11.8.6 验收标准

- 用户可看到哪些 App 正在使用哪些设备。
- 用户可看到哪些 App 拥有高权限能力。
- IoT 设备的访问方式可区分为 Direct 与 Data Routed 两类。

---

## 12. 模块五：Developer Mode（开发者模式）

### 12.1 模块定位

Developer Mode 是系统底层状态与配置的“只读可观测层 + 调试工具入口”。

目标不是让普通用户修改系统，而是让开发者、高级用户和工程支持快速获取内部信息。

### 12.2 核心策略

1. 默认只读
2. 高风险写入能力默认关闭
3. 关键内部信息可见
4. 调试信息可导出

### 12.3 模块结构

```text
Developer Mode
├── Mode Switch
├── System Diagnostics
├── Config Explorer
├── Logs
├── CLI Helpers
└── System Tasks / System Tester
```

### 12.4 功能需求

#### 12.4.1 Mode Switch

展示开发者模式开关：

- V1 默认为 OFF
- 当前版本不允许普通用户打开写入模式
- 界面明确说明：当前为只读模式

#### 12.4.2 System Diagnostics

基于内置脚本（如 `check.py`）生成系统诊断结果。

行为建议：

- 用户进入该页面时自动执行一次
- 将脚本结果以可读方式展示
- 可补充简要分析或状态标签

#### 12.4.3 Config Explorer

展示系统关键配置文件内容。

V1 范围：

- SystemConfig
- Gateway 关键配置文件
- 一些本地关键配置文件

UI 形式：

- 左侧：配置树 / 配置列表
- 右侧：内容查看

SystemConfig 为树状结构：

- 支持逐层展开
- 点击节点后在右侧查看具体内容

安全要求：

- 不得直接显示私钥、密钥材料、敏感 Token
- 对敏感字段进行过滤或以“Hidden for security”替代

#### 12.4.4 Logs

Developer Mode 中的日志以系统调试日志为主，与用户态 Task/Event 日志区分开。

核心动作：

- Download Logs（打包下载）

行为要求：

- 默认只打包最近一段时间的日志，而非全量历史日志
- 打包逻辑由底层统一实现
- 下载后便于用户发给工程师做问题排查

#### 12.4.5 CLI Helpers

显示若干常见的 BuckyOS CLI 命令，帮助拥有宿主机权限的高级用户进行本地诊断或操作。

示例（实际命令以后续实现为准）：

```bash
buckyos status
buckyos restart
buckyos logs
buckyos doctor
```

V1 原则：

- 仅展示命令
- 不在 Settings 中直接执行

#### 12.4.6 System Tasks / System Tester

预留系统级调试工具入口，用于：

- API 调试
- 调用内部系统能力
- 观察返回结果

V1 可允许页面存在但内容为空或仅保留入口。

### 12.5 验收标准

- 用户能看到开发者模式当前为只读。
- 用户能查看 SystemConfig 和 Gateway 配置的非敏感内容。
- 用户能触发日志打包下载。
- 页面可展示系统诊断结果。
- CLI 区仅展示命令，不执行命令。

---

## 13. 跨模块数据与权限抽象

### 13.1 可写性分层

V1 对 Settings 内能力按可写性分层：

1. **只读**
   - 大多数系统信息、隐私状态、集群状态、开发者信息

2. **有限可写**
   - 外观配置
   - Session 命名与克隆
   - 更新相关入口

3. **暂不开放可写**
   - 高风险系统配置写入
   - 核心消息暴露开关
   - 大部分精细化权限控制

### 13.2 信任与风险抽象

对用户呈现的风险主要分为：

- 系统可信能力（绿）
- 第三方高权限或共享能力（黄）
- 高风险未受信能力（红，后续）

### 13.3 复制/导出能力

以下模块应支持一键复制或导出：

- General：Copy System Info
- Cluster Manager：Copy Cluster Info
- Developer Mode：Download Logs

---

## 14. 非功能需求

### 14.1 性能

- Settings 首屏打开应快速可达。
- 大部分只读信息应在可接受时间内完成加载。
- System Diagnostics、日志打包等较慢动作允许异步加载，但需要清晰状态反馈。

### 14.2 安全

- 不得在 UI 中暴露私钥、密钥材料、敏感 Token。
- 高风险写操作必须具备额外保护机制；V1 默认不开放。
- 日志打包需遵循敏感信息过滤策略（如底层已有规则，则沿用底层规则）。

### 14.3 可理解性

- 面向普通用户的描述尽量避免直接出现 RBAC、ACL、X.509 等术语。
- 当必须出现底层术语时，应使用折叠区或高级说明。

### 14.4 多端一致性

- Desktop 为主设计面。
- Mobile / Browser 接入保留同样的信息结构，但允许简化布局。
- 同一配置模型在不同端展示方式可不同，但含义必须一致。

### 14.5 国际化

- Appearance 中需支持语言切换。
- Settings 文案需要从一开始具备国际化键值管理能力。

---

## 15. V1 交付建议

### 15.1 P0（必须完成）

- General 全部基础信息展示与 Copy System Info
- Appearance 的 Session 视图与基础外观项（明暗 / 语言 / 字体 / 壁纸）
- Cluster Manager 的概览、Zone、连接方式、证书只读视图
- Privacy 的可视化说明（尤其是 Public / Shared / App / Agent）
- Developer Mode 的只读开关、诊断、配置查看、日志下载

### 15.2 P1（建议完成）

- Session 克隆为 Device Session
- SN 区域和流量使用展示
- 更多证书详情
- Copy Cluster Info
- Device & Capability 的基础展示

### 15.3 P2（后续）

- 更精细的权限控制
- 开放部分高阶配置写入
- System Tester 实际联调能力
- Home 域名相关策略
- 更多可视化解释和引导文案

---

## 16. 未来演进方向

### 16.1 Settings 的进一步拆分

随着专用页成熟，以下模块可逐步从 Settings 中移出：

- Agent Center
- AI / Provider Center
- Storage Center
- Messaging Center
- Monitor / 实时系统监控

### 16.2 Privacy 的进一步细化

- 更细粒度的 App / Agent 权限审批
- 面向多用户系统的更完整共享策略
- 更强的设备授权机制

### 16.3 Developer Mode 的进一步开放

- 内部构建可开启写入能力
- API 调试能力完整化
- 更丰富的诊断脚本与修复建议

---

## 17. 风险与待确认问题

### 17.1 命名

- 产品正式命名是 BuckyOS 还是 BuckyOS，需要后续统一。
- Cluster、Zone、Session 的用户可见命名是否需要继续简化，需要设计与品牌统一。

### 17.2 Session 规则

- Shared Session 的默认命名策略需产品确认。
- Device Session 克隆后的回退路径是否需要提供。

### 17.3 隐私控制开放边界

- Messaging Access 何时允许真正关闭。
- 哪些高权限应用需要在 V1 提供可控开关。

### 17.4 证书与网络信息可得性

- SN 区域、流量使用、DNS 细节、证书原始信息的底层读取接口需研发确认。

### 17.5 日志脱敏策略

- 日志打包的过滤范围和保留时长需研发与安全共同确认。

---

## 18. 附录：推荐页面顺序

建议用户在 Settings 中看到的页面顺序如下：

```text
1. General
2. Appearance
3. Cluster Manager
4. Privacy
5. Developer Mode
```

排序原则：

- 先放最通用、最无风险的内容
- 再放外观与环境相关能力
- 然后展示网络与隐私
- 最后放开发者模式

---


