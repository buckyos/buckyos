# Windows 上使用 WSL containers

BuckyOS 的 Windows node-daemon 可通过 `wslc.exe` 直接管理 Linux 容器，不需要 Docker Desktop 或在 WSL 发行版内安装 Docker Engine。App、Agent 和 HostScript 共用现有 AppLoader，应用包仍使用 Docker/OCI 镜像描述。

## 准备环境

需要 Windows 11、硬件虚拟化和包含 `wslc.exe` 的 WSL。微软文档给出的最低 WSL 版本为 2.9.3，本次验证使用 3.0.3.0。Windows 本身的版本号与 WSL 软件包版本号不同。

在管理员 PowerShell 执行，按系统提示重启：

```powershell
wsl --install --no-distribution
wsl --update
wsl --version
wslc version
wslc run --rm hello-world
```

在 PVE 虚拟机内测试时，需要向 Windows 暴露嵌套虚拟化能力。WSLC 不要求安装 Ubuntu 等发行版。`devenv.py` 在 Windows 默认安装 `Microsoft.WSL`，并提示完成上述初始化。

## 选择运行时

Windows 在找到 `wslc.exe` 时优先使用 WSLC；查找范围包含 PATH 和 `%ProgramFiles%\WSL\wslc.exe`。未找到时使用 Docker。Linux/macOS 默认继续使用 Docker。

可以在启动 BuckyOS 的同一个 PowerShell 中显式选择：

```powershell
$env:BUCKYOS_CONTAINER_RUNTIME = "wslc"
uv run src/start.py --skip-update
```

该命令只重启已经安装的 BuckyOS，不会构建或更新 node-daemon。首次使用需要先构建并安装包含本改动的版本。需要 Docker 时把变量设为 `docker`；未知值会报错。停止脚本读取同一变量：

```powershell
uv run src/stop.py
```

WSLC 的会话、镜像、容器和卷与调用用户及权限上下文关联。请让 node-daemon、镜像导入和 stop.py 使用同一个 Windows 用户及相同的管理员权限级别；交互式用户拉取的镜像不能假定在 LocalSystem 服务中可见。正式安装包的 paios 镜像（`bin/paios-images/`）因此由 node-daemon 在自己的上下文里从包内 tar 执行 `load` 导入，安装器不导入；换了运行用户或权限级别后，node-daemon 会从 tar 重新导入。手动执行 `wslc load -i <tar>` 恢复时，必须在 node-daemon 的运行账号、相同权限级别的 PowerShell 中执行；管理员窗口中 `wslc image ls` 可见的镜像不代表 node-daemon 可见。不要在已有容器运行时切换运行时；先用原运行时停止，两个运行时的镜像和命名卷分别保存。

## 运行行为

- 容器生命周期、镜像加载/拉取/打标签、标签身份校验和健康状态沿用 AppLoader；WSLC 的 JSON 数组/JSON 流会转换成已有查询代码需要的字段。
- WSLC 的错误信息随系统语言本地化，AppLoader 不解析错误文本：`image inspect` / `image rm` 失败后，用镜像列表（`images -q <ref>`，sha256 引用用 `images --quiet --no-trunc`）确认镜像不存在，才按缺失处理；列表查询也失败或列表中仍有该镜像时，按原命令失败上报。
- Windows 路径直接绑定到容器，命名卷用于实例工作目录及共享 ExtTool；空卷由镜像内容初始化。
- 容器通过 `host.wslc.internal` 访问 Windows 宿主。AppLoader 将其写入 `BUCKYOS_HOST_GATEWAY` 和 kevent 地址，不传 Docker 专用的 `--add-host ...:host-gateway`。
- 服务端口仍按 AppLoader 的端口映射发布。WSLC 默认绑定 Windows loopback，供宿主上的 NodeGateway 访问。
- `stop.py` 只清理所选运行时中名称以 `buckyos-app-` 开头的容器，保留命名卷。
- `container_param` 会直接交给所选运行时；Docker 特有参数不保证被 WSLC 支持，错误会直接上报。

## 已验证范围及限制

Windows 11 / WSL 3.0.3.0 上验证容器启动、状态与标签查询、镜像加载和打标签、Windows 空格路径只读绑定、命名卷持久化及空卷初始化；通过 Windows 编译的测试程序运行真实 WSLC 适配模块和 AppLoader 命令执行函数，双向网络连接、实际 stop.py 清理也已验证；Windows 重启后可重新启动 WSLC，并读回原命名卷内容。完整 Windows 安装、Zone 激活和 Jarvis/LLM 对话尚未进行端到端验证。测试镜像从开发机导入，不能据此认定测试机直连 Docker Hub 拉取成功。

WSLC 3.0.3.0 上复现了 Alpine/musl 解析 `host.wslc.internal` 失败的上游问题；Ubuntu/glibc 能解析。BuckyOS 的 `paios/aios` 和 `paios/exttool` 基于 Ubuntu 24.04。使用 Alpine 的自定义应用应关注上游修复。

参考：

- [微软 WSL container 文档](https://learn.microsoft.com/en-us/windows/wsl/wsl-container)
- [WSLC 架构与用户会话模型](https://devblogs.microsoft.com/commandline/wslc-architecture-deep-dive/)
- [musl 域名解析问题 #41769](https://github.com/microsoft/WSL/issues/41769)

在具备 Rust 构建环境的 Windows 上，先加载 `alpine:latest` 和 `ubuntu:24.04`，然后在 `src/` 执行实机测试：

```powershell
$env:BUCKYOS_CONTAINER_RUNTIME = "wslc"
cargo test -p node_daemon windows_wslc_lifecycle -- --ignored --test-threads=1
```

测试创建并清理以 `buckyos-app-wslc-test-` / `buckyos-wslc-test-` 命名的资源，使用动态本地端口，不依赖在线 Zone。
