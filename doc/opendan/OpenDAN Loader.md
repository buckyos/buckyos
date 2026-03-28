# OpenDAN 演化为 Agent Loader 需求文档（简化版）

> 版本：0.1-draft

---

## 1. 核心改动

OpenDAN 从"默认加载 Jarvis 的系统服务"变为"由外部驱动的 Agent Loader"。实际改动点只有两个：

1. **启动时传入 AgentInstanceName**：OpenDAN 不再内置默认 Agent，而是通过外部参数指定要加载的 Agent Instance，并通过 BuckyOS SystemConfig 接口读取该实例的配置信息。
2. **提示词文件通过 ConfigMerger 实现上游整合**：Agent Instance 的提示词等资源默认继承（include）上游 Agent Package 的内容，通过 `buckyos_kit::ConfigMerger` 实现合并；Instance 可以局部 override，也可以完全切断与上游的关联（自演化）。

---

## 2. 角色与边界

| 组件 | 职责 |
|------|------|
| **BuckyOS** | 管理 Agent Package 安装、Agent Instance 生命周期，通过 AppLoader 启动 OpenDAN |
| **OpenDAN** | 解析启动参数，加载并运行指定的单个 Agent Instance |

核心原则：

- 一个 OpenDAN 进程只服务一个主 Agent Instance
- Agent 的存在由系统（BuckyOS）定义，不由 Runtime 决定
- 无外部配置时，OpenDAN 不启动任何默认 Agent (空跑，应该直接退出)

---

## 3. 关键概念

- **Agent Package**：作者发布的只读软件包（提示词、behaviors、skills、配置等）
- **Agent Instance**：用户"领养"后的实例，拥有独立 ID、数据目录、可变状态
- **Agent Environment**：Instance 的读写运行目录，承载演化结果
- **include 机制**：Environment 通过 ConfigMerger 引用 Package 内容，而非物理复制
- **自演化**：Instance 在 Environment 中写入本地内容，局部或全部脱离 Package 默认行为

---

## 4. 启动输入

### 4.1 命令行参数

```bash
opendan \
  --agent-id jarvis \
  --agent-env /opt/buckyos/data/home/$owner/agents/jarvis \
  --agent-bin /opt/buckyos/bin/buckyos_jarvis
```
buckyos_jarvis 是Agent Jarvis的pkg-name

环境变量中包含以 `appid=jarvis` 方式构造的 `APP_SESSION_TOKEN`，用于 `buckyos_api_runtime` 的 login。

### 4.2 配置解析优先级

```
CLI 参数 > 环境变量 > 实例配置文件
```

---

## 5. 目录模型

### Package 目录（只读）

```
package_root/<agent_package_id>/
  meta/ prompts/ behaviors/ skills/ assets/ defaults/
```

### Instance Environment 目录（读写）

```
agent_env_root/<agent_instance_id>/
  config/ prompts/ behaviors/ memory/ cache/ workspace/ state/ logs/
```

Runtime 对 Package 只读，对 Environment 完全读写。所有自定义与演化内容写入 Environment，不回写 Package。

---

## 6. ConfigMerger 加载规则

1. **默认继承**：Environment 中的资源默认 include Package 对应资源
2. **局部覆盖**：Environment 中存在本地 override 时，优先使用本地内容
3. **升级兼容**：Package 升级后，未 override 的资源自动继承新版本；已 override 的保持本地版本
4. **可切断**：Instance 可以选择完全脱离上游，在 Environment 中独立维护所有资源

---

## 7. 生命周期

1. **安装** → 获得 Agent Package
2. **领养/实例化** → 创建 Agent Instance（分配 ID、创建 Environment、初始化 include）
3. **启动** → BuckyOS 通过 AppLoader 拉起 OpenDAN，传入 Instance 参数
4. **运行** → OpenDAN 在单 Agent 上下文中运行
5. **升级** → Package 更新，影响 include 资源，不破坏本地 override
6. **演化** → Agent 生成新内容写入 Environment

---

## 8. 容器化兼容

当前以本地进程模式运行，设计上需兼容未来 Docker 容器化：

- 每个 Agent Instance 可运行在独立容器中
- 不同 Agent 可绑定不同版本 Runtime
- 容器内挂载 Package（只读）+ Environment（读写）
- CLI/env 启动协议可直接迁移为容器 entrypoint

---

## 9. 验收要点

- 无外部配置时 OpenDAN 不启动，返回明确错误
- 可通过参数切换加载不同 Agent Instance
- Runtime 能正确区分并挂载 Package（只读）和 Environment（读写）
- ConfigMerger 能正确解析 include/override 关系
- Package 升级不覆盖已演化的本地内容
- 启动日志记录当前 Instance、Package、目录路径、override 状态