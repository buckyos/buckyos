# OpenDAN 容器化依赖清单

用于构建 Agent Runtime 基础镜像的依赖整理。OpenDAN 正在准备容器化，本文档汇总运行时直接依赖与 Agent 工作空间常用工具。

---

## 1. 运行时直接依赖（必须）

这些是 opendan 二进制在运行 `exec_bash` 等工具时**直接调用**的系统命令，缺一不可。

| 依赖 | 用途 | 代码位置 |
|------|------|----------|
| **tmux** | exec_bash 通过 tmux 创建 per-session 的 shell 环境，执行命令、捕获输出、维持 cwd | `agent_bash.rs` |
| **bash** | 默认 `/bin/bash`，exec_bash 的 shell 入口；可通过 `AgentWorkshopConfig.bash_path` 覆盖 | `workshop.rs` |

### tmux 使用方式

- `tmux -V`：版本检查
- `tmux new-session -d -s <name> -c <cwd>`：创建 session
- `tmux has-session -t <name>`：检查 session 是否存在
- `tmux send-keys -t <target> <command>`：发送命令
- `tmux capture-pane -t <target> -p -S <lines>`：捕获输出
- `tmux display-message -p -t <target> "#{pane_current_path}"`：获取 pane 当前目录
- `tmux list-sessions -F <format>`：列出 session（用于 GC）

---

## 2. Agent 工作空间常用工具（推荐）

Agent 通过 `exec_bash` 执行任意 shell 命令。以下工具在典型 Agent 任务中会被用到，建议预装到基础镜像。

| 依赖 | 用途 | 说明 |
|------|------|------|
| **Python3** | 运行 Python 脚本、pip 包 | 建议最新 LTS；`python3`、`pip3` 可用 |
| **Node.js** | 运行 JS/TS 项目、npm/pnpm | 建议最新 LTS；`node`、`npm` 可用 |
| **git** | 版本控制、diff、clone、commit | Jarvis do.yaml 中明确提到 `git diff` |


---

## 3. 基础系统工具（通常已包含）

以下命令在 behaviors 提示词或工具逻辑中被引用，一般 base image（如 `ubuntu:24.04`）已自带：

- `cat`、`ls`、`grep`、`mkdir`、`cd`
- `tee`、`printf`（exec_bash 脚本中使用）
- `dirname`（exec_bash 脚本中使用）

---

## 4. 可选 / 按需安装

| 依赖 | 用途 |
|------|------|
| **pnpm** | 若 Agent 需要管理 pnpm 项目 |
| **curl** / **wget** | 下载资源 |
| **jq** | JSON 处理 |


---

## 5. 当前 AIOS Dockerfile 对比

`publish/aios/Dockerfile` 当前内容：

```dockerfile
FROM ubuntu:24.04
ARG TARGET_ARCH
WORKDIR /opt/buckyos
RUN mkdir -p /opt/buckyos/bin/opendan
COPY opendan /opt/buckyos/bin/opendan/opendan
ENV AIOS_TARGET_ARCH="${TARGET_ARCH}"
EXPOSE 3180
ENTRYPOINT ["/opt/buckyos/bin/opendan/opendan"]
```

**缺失项：**

- `tmux`：exec_bash 会直接失败
- `bash`：ubuntu 通常有，但若使用 minimal base 需确认
- Python3、Node.js、git：Agent 工作流常用，建议预装

---

## 6. 建议的基础镜像 Dockerfile 示例

```dockerfile
FROM ubuntu:24.04

ARG TARGET_ARCH

# 运行时必须
RUN apt-get update && apt-get install -y --no-install-recommends \
    tmux \
    bash \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Agent 工作空间常用
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 \
    python3-pip \
    python3-venv \
    nodejs \
    npm \
    git \
    && rm -rf /var/lib/apt/lists/*


WORKDIR /opt/buckyos
RUN mkdir -p /opt/buckyos/bin/opendan

COPY opendan /opt/buckyos/bin/opendan/opendan

ENV AIOS_TARGET_ARCH="${TARGET_ARCH}"

EXPOSE 3180

ENTRYPOINT ["/opt/buckyos/bin/opendan/opendan"]
```

**说明：**

- Ubuntu 24.04 的 `nodejs` 可能较旧，若需最新 LTS 可改用 NodeSource 或 nvm
- Python3 默认已较新，可按需升级
- `ca-certificates` 用于 HTTPS 请求（如 pip、npm registry）

---


