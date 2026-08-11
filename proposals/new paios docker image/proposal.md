# PAIOS Docker Image v2 需求

## 1. 背景与目标

当前 BuckyOS 中每个 Agent 运行在独立的 Docker 容器中，容器基于统一镜像，面向 bash 环境。过去容器是**无状态**的（纯运行环境，不挂载卷），Agent 的状态管理完全依赖 host 文件系统的 bind mount（如用户的 MyDocuments）。

随着意图引擎的推进，Agent 开始需要**在自己的 environment 中编写和运行工具**（TS 脚本、Python 脚本），这对容器的能力提出了根本性的新要求。

### 1.1 核心目的

- **为 Agent Environment 服务**：让 Agent 能在自己的隔离环境中编写、安装、运行工具。
- **从 C 端体验路径出发**：最先出效果的场景是 Agent 为用户编写场景化脚本（文档格式转换、数据处理、定时抓取等）。

### 1.2 语言选型策略

| 场景 | 推荐语言 | 理由 |
|------|----------|------|
| 长期运行的服务（service、spider、daemon） | TypeScript (Deno) | 内存占用可控，服务类库生态成熟 |
| 一次性/工具性任务（PPT 生成、Excel 处理、图片/视频处理） | Python (uv) | 文档格式、媒体处理生态更完整 |

---

## 2. 容器有状态化：Agent Volume

### 2.1 现状问题

过去容器无卷挂载，每次启动都是干净环境。Agent 现在需要：安装依赖、保存自己编写的工具、维护工具运行产物。纯靠 host 文件系统 bind mount 在 Windows/macOS 上会带来复杂性。

### 2.2 方案

- 每个 Agent 根据其**唯一 Agent ID**拥有一个独立的 **Docker Volume**。
- 容器仍使用**统一镜像**，但启动时挂载 Agent 专属卷。
- 卷内采用 **OverlayFS 两层结构**：
  - **底层（只读）**：开发者/系统提供的原始文件（Package 层）。
  - **上层（读写）**：Agent 运行时产生的数据和安装的依赖（Data 层）。

### 2.3 用户可见性问题

Agent 看到的 root 路径在 host 裸文件系统上不可直接访问。

**当前结论**：不强制将 Agent Volume 映射回 host 文件系统。用户通过 BuckyOS 的 **Files 工具**（Web 文件浏览器）来查看 Agent 文件——Files 本身也是 Docker 容器，可以直接挂载 Agent 的 Volume（只读或读写），对用户呈现为额外的目录，体验一致。

---

## 3. 依赖管理与环境隔离

Agent 编写的工具会引入依赖（Python pip 包可能触发 native building，npm 的 node_modules 膨胀问题），需要在容器内部做好隔离。

### 3.1 Python 侧

- 镜像预装 **uv**（及 uvx）。
- Agent 使用 uv 管理 virtualenv，解决 Python 自身的环境隔离。
- 依赖安装在 Agent Volume 内，不污染基础镜像。

### 3.2 TypeScript 侧

- 镜像预装 **Deno**。
- Deno 自带依赖管理和沙箱，天然解决 node_modules 膨胀问题。
- 如需 npm 包，也通过 Deno 的 npm 兼容层处理。

---

## 4. Agent 内部目录规划与工具生命周期

### 4.1 工具分类

| 类型 | 作用域 | 存储位置 | 说明 |
|------|--------|----------|------|
| 临时工具 | 当前 Session | Session 目录 | 只在当前 Teamwork Session 中可见，Session 结束后可清理 |
| 持久工具 | 全局 | Agent 长期存储目录 | 所有 Teamwork Session（包括历史 Session）均可见 |

### 4.2 工具可见性

在 Teamwork Session 中，工具以路径映射的方式暴露给 Agent。当前策略：**全量暴露，不做访问限制**——Agent 可以看到所有已安装的工具描述，是否使用由 Agent 自行决定。这在 Session 粒度上不会造成实质干扰。

### 4.3 脚本的三种形态

一旦允许 Agent 在自己的环境中编写脚本，脚本的形态会自然分化：

1. **轻量脚本**：一次性执行的小工具（如格式转换）。
2. **重型工具**：Agent 投入大量 token 打造的复杂工具（如文档处理流水线）。
3. **服务**：长期运行的后台进程（如定时抓取 Spider、数据同步服务等）。

这三种形态对容器的进程管理、生命周期控制、端口暴露等都有不同要求。

---

## 5. Script Service 模式：Agent 编写 BuckyOS 服务

### 5.1 场景

Agent 编写的服务分两种：

- **自用服务**：在 Agent 自己的容器内运行，仅 Agent 自己调用。
- **BuckyOS 系统服务**：挂载到 BuckyOS 服务体系，供系统或其他应用调用。

BuckyOS 的服务严格运行在独立 Docker 容器中，因此 Agent 写好的系统服务需要**单独启动一个容器**。

### 5.2 运行机制

1. Agent 编写代码后，将其放置到 host 级别的指定目录（通过 mount 映射）。
2. 用户（或 Agent）通过 `bucky-cli` 指令启动服务。
3. 启动时**复用 PAIOS 同一镜像**，但进入 **Script Service 模式**（而非 Agent 模式）。
4. 容器的 entrypoint 变为执行指定目录下的脚本。
5. 可选为该服务分配独立 Volume（用于安装依赖等）。

### 5.3 与现有服务架构的关系

BuckyOS 当前架构：一个应用 = 一个容器 = 一个极小的 Linux 镜像（Rust musl 编译产物 + 最小运行时）。PAIOS 镜像相对较大（携带 Python/Deno 运行时），但可以作为**脚本类服务的通用容器**复用。

---

## 6. 脚本包分发：面向脚本开发者的新能力

### 6.1 核心思路

从 Agent 写服务的能力中，自然延伸出一个正交的通用能力：**脚本开发者无需自己构建 Docker 镜像，只需发布脚本包**。

### 6.2 工作流程

1. 开发者发布一个**脚本包**（TS 或 Python 项目，包含依赖声明）。
2. BuckyOS 安装脚本包时，将其解压到固定目录。
3. 系统基于 PAIOS 镜像创建容器，进入 Script Service 模式。
4. 首次启动时自动安装依赖（依赖安装到该服务的专属 Volume）。
5. 后续启动直接运行（Volume 中已有依赖缓存）。

### 6.3 价值

- 降低 BuckyOS 应用开发门槛：脚本语言开发者不需要了解 Docker，不需要发布容器镜像。
- 复用 PAIOS 镜像：脚本包共享同一个基础运行环境，节省磁盘和拉取成本。
- Agent 可用、人也可用：Agent 自动生成的工具和人工编写的脚本包走同一套机制。

---

## 7. 镜像能力总结

| 能力 | v1（现状） | v2（目标） |
|------|-----------|-----------|
| 基本运行环境 | ✅ bash 环境 | ✅ bash 环境 |
| 状态管理 | ❌ 无状态，依赖 host mount | ✅ Agent 专属 Volume + OverlayFS |
| Python 支持 | 基础 Python | ✅ uv/uvx 预装，virtualenv 隔离 |
| TypeScript 支持 | 基础 Node | ✅ Deno 预装，内建依赖管理 |
| Agent 写工具 | ❌ | ✅ 临时工具 + 持久工具 |
| Agent 写服务 | ❌ | ✅ 自用服务 + BuckyOS 系统服务 |
| 容器运行模式 | Agent 模式（单一） | ✅ Agent 模式 / Script Service 模式 |
| 脚本包分发 | ❌ | ✅ 脚本包安装 → PAIOS 容器运行 |
| Files 工具集成 | 有限 | ✅ 通过 Volume 挂载直接浏览 Agent 文件 |

---

## 8. 演进路径

```
本地裸运行 → Docker 容器化（v1，纯运行环境）
  → 有状态容器（v2，Agent Volume）
    → Agent 编写工具/服务
      → Script Service 模式（BuckyOS 系统服务）
        → 脚本包分发（通用能力，降低开发门槛）
```

这是一条由需求驱动的自然演进路径：从一个"让 Agent 能写脚本"的小起点出发，推导出对 PAIOS 镜像的整体重构，并附带产出了面向脚本开发者的通用分发能力。
