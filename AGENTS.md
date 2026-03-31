# AGENTS

## Commands

### 开发常用脚本

**注意下面脚本都推荐在src目录下运行**

```bash
# 修改代码后重新构建buckyos
uv run buckyos-build.py
# 指定模块构建
un run buckyos-build.py -s <module-name>
# 跳过web UI构建
uv run src/buckyos-build.py --skip-web
# 重新构建cyfs-gateway 并安装
cd ../../cyfs-gateway/src/ && uv run buckyos-build.py && uv run buckyos-install.py

# 使用上一次build的结果（如果有)覆盖安装后,启动buckyos
uv run start.py
# 使用devtest环境，全新安装并启动. 
uv run start.py --all
# 使用生产环境，全新安装并启动（会进入待激活状态)
uv run start.py --reinstall release

# 检查当前buckyos的运行状态
uv run check.py

# 停止bcukyos
uv run stop.py

```



## 目录结构

本段只提供「从哪找什么」的地图，不替代各模块内的 canonical 文档。

- **仓库根**：`pyproject.toml`、`uv.lock`（`uv run` 拉起 `buckyos-devkit`）、`devenv.py`（开发环境）、以及 `build.py`、`check.py` 等辅助脚本；与 **`src/`** 并列的协作与产品材料见下。
- **`doc/`**：跨模块说明、CI、架构与各子域文档（如 `doc/arch`、`doc/control_panel`、`doc/sdk` 等）；具体协议与规范以子目录为准。
- **`harness/`**：Harness Engineering 的规则、技能、任务模板与检查单。
- **`product/`**：产品与界面规划类材料（图表、说明等），**不是**可运行源码的主承载。
- **`proposals/`**：提案入口；流程上通常与「proposal → 实现 → PR」衔接。
- **`test/`**：仓库级测试与脚本（与 `src/` 内的测试目录区分）。
- **`src/`**：**日常开发、构建与安装命令的默认工作目录**（见上文 Commands）。
  - **Rust workspace**：`Cargo.toml` / `Cargo.lock`，crate 为主要服务端与库代码的载体。
  - **`kernel/`**：内核侧守护与服务（如 `node_daemon`、`scheduler`、`kmsg`、`task_manager` 等）。
  - **`frame/`**：框架层业务与服务（如 `control_panel`、`opendan`、`repo_service`、`msg_center` 等），与 `kernel/` 分工协作。
  - **`apps/`**：面向安装/交付的应用与系统测试应用（如 `control_panel` 应用侧、`sys_test`）。
  - **脚本与配置**：`buckyos-build.py`、`buckyos-install.py`、`start.py`、`stop.py`、`make_config.py`；`bucky_project.yaml` 常作为构建模块清单入口。
  - **`tools/`**：CLI、Agent、打包等开发与运维工具。
  - **`rootfs/`、`dev_configs/`、`patches/`**：根文件系统相关、开发配置与补丁。

## 处理规则

从proposal开始，到PR结束

## py 开发支持脚本编写
- 用buckyos-devkit 库来获得多平台支持

## 常见术语

## 1. 文档目标

本文件不是项目百科，也不是实现现状清单。

它只定义两件事：

- AI / 人在本仓库中协作开发时，应该以什么信息作为输入
- 一个任务在进入编码后，最低需要走完什么闭环才算完成

具体实现细节、路由、RPC、页面、历史背景，尽量下沉到模块自己的 canonical 文档中，不在这里重复展开。


## 3. 基本原则

- Git / 文件系统优先：优先依据仓库中的代码、文档、脚本与目录结构工作，而不是外部平台语义。
- 文档先于编码：没有足够清晰的文档输入时，不要直接扩写实现假设。
- 测试属于任务本身：写完代码不等于完成任务，至少要补到可验证状态。
- 组合优于发明：优先复用已有模块、类型、脚本、依赖和既有模式。
- 边界优先：修改前先确认这个需求属于哪个模块，避免跨边界误改。

## 4. 信息输入优先级

处理任务时，默认按以下顺序建立上下文：

1. 当前代码路径与可运行脚本
2. 模块 canonical 文档
3. 模块本地规则文件
4. 历史文档

## 6. AI 开发前的最小检查

开始实现前，至少完成以下检查：

- 任务属于哪个模块
- 当前事实来源是哪份 canonical 文档
- 关键入口文件是什么
- 是否已有现成实现、类型、脚本或依赖可复用
- 任务的完成标准是什么
- 最低测试要求是什么

如果这些问题答不出来，不要急着开始大改。

## 7. Developer Loop 最小闭环

任何默认开发任务，至少按下面的闭环推进：

1. 读取任务相关文档与代码入口
2. 确认最小改动面
3. 实现
4. 运行最相关的测试 / 构建 / lint / 验证脚本
5. 失败则诊断并修复
6. 输出结果与证据

“完成”至少意味着：

- 代码已改
- 相关校验已执行，或明确说明为什么没法执行
- 风险点已指出

## 8. 测试与验证要求

对本仓库中的任务，默认分三层看待：

### 8.1 第一层：局部校验

- Rust：`cargo test`、`cargo build`
- Web：`pnpm build`、`pnpm lint`
- 只要任务改动影响到该层，就优先跑最小相关校验

### 8.2 第二层：单点 / 开发态验证

如果任务影响运行时行为，优先补单点验证，例如：

- 服务是否能启动
- RPC 是否能跑通
- 页面是否能加载
- 关键 HTTP / Files 路径是否可用

### 8.3 第三层：更高成本验证

例如集成环境、跨节点环境、真实部署验证。

这类验证当前在本文件中不强制展开；如果没执行，需要在结果中明确说明。

## 9. control_panel 常用命令

### 9.1 Workspace / system

- `cd src && uv run ./buckyos-build.py`
- `cd src && uv run ./buckyos-build.py --no-build-web-apps`
- `cd src && uv run buckyos-install`
- `cd src && cargo test -- --test-threads=1`

### 9.2 control_panel backend

- `cd src && cargo run -p control_panel`
- `cd src && cargo build -p control_panel`
- `cd src && cargo test -p control_panel`

### 9.3 control_panel web

- `cd src/frame/control_panel/web && pnpm install`
- `cd src/frame/control_panel/web && pnpm dev`
- `cd src/frame/control_panel/web && pnpm build`
- `cd src/frame/control_panel/web && pnpm lint`

### 9.4 本地部署流

- `cd src && uv run ./buckyos-build.py -s control_panel control_panel_web`
- `cd src && uv run buckyos-install`
- `systemctl restart buckyos`

## 10. 实施约束

### 10.3 共享规则

- 优先最小改动，不做无关重写
- 不新增依赖，除非现有方案明显不够
- 改协议、字段、命名时，必须检查前后端和文档是否联动

## 11. 输出要求

完成任务后，至少应能回答：

- 改了什么
- 为什么这样改
- 跑了什么验证
- 还有什么风险或未验证项

如果是较大任务，还应说明：

- 主要改动入口文件
- 是否影响文档、协议、共享类型、依赖




