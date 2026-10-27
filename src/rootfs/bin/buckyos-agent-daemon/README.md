# BuckyOS Agent Daemon

`buckyos_agent_daemon` 是独立的 Linux/macOS/Windows 当前用户执行端，复用 Node Executor 的命令、文件和操作账本。macOS 桌面功能要求 macOS 14 或更新版本；Windows 使用当前登录用户桌面中的 Win32 截图和输入，API 最低要求为 Windows 10 1703（build 15063），Windows 10/11 各版本仍需实机验证。

```sh
buckyos_agent_daemon run --url-stdin
buckyos_agent_daemon doctor --json
buckyos_agent_daemon stop --executor <executor_id>
buckyos_agent_daemon pause --executor <executor_id>
buckyos_agent_daemon resume --executor <executor_id>
```

接入 URL 由控制面 Settings → Cluster Manager → Nodes/Devices 签发。桌面截图/输入需单独授权，默认禁用。默认前台运行，退出即下线，不安装启动项，不读取完整 Node 身份，也不监听 3184。Linux UI 固定为 `unsupported`。

macOS 请在当前登录用户的终端运行，不使用 sudo。系统设置 → 隐私与安全性中为实际运行进程/启动宿主授予屏幕录制和辅助功能权限；用 `doctor --json` 检查当前进程是否就绪。输入与截图在 daemon 同一进程内完成，没有另一个后台 helper 身份。打包产物尚未经过 Developer ID 签名或公证，移动或更新二进制后应重新核验权限。

Windows 在当前用户的 PowerShell 终端解压 ZIP 并运行 `./buckyos_agent_daemon.exe run --url-stdin`。不支持 Session 0 服务；提升权限不能替代交互会话，也不授予跨用户或安全桌面访问。命令后端优先使用 PATH 中的 PowerShell 7，否则使用系统 Windows PowerShell，不需要 Bash。用 `./buckyos_agent_daemon.exe doctor --json` 检查会话和桌面是否就绪；锁屏、安全桌面、切换用户或无法验证前台目标时不发送输入。Windows 当前仅支持主显示器的 visual/auto 观察、click、Unicode type、keypress、按行 scroll 和 delay 等待；semantic/hybrid、drag、像素滚动不支持。未完成真实 Agent 桌面任务验收。

`pause`/`resume` 提交本地控制请求；在线时，`status --json` 的 `instances[].capabilities.ui.state` 通常在下一次心跳（约 5 秒）反映 `paused`。请求已提交不是原动作取消确认；查询原操作确认执行事实。恢复后旧观察无效，必须重新观察。Ctrl-C、stop、控制面撤销 URL 都可停止接入。

Windows 动作检测到人工键鼠接管时自动暂停，需用户在本机 `resume` 并重新观察。进程崩溃后若无法恢复 Unicode 输入的释放记录，`resume` 会继续保持 dirty 和暂停，需用户介入；不保证所有未知输入都能自动恢复，不删除协调文件或重放原动作来绕过。

Windows 输入监视线程出现心跳断档后会持续禁用桌面能力，休眠、调试暂停或长时间调度停顿也可能触发。此时 `resume` 无法恢复，需停止并重启 daemon，重新检查 readiness 并观察；查询原操作，不重放未知输入。详细阈值与限制见 Agent Daemon 协议文档。

Windows 数据默认保存在 `%LOCALAPPDATA%\buckyos-agent-daemon`；macOS 使用 `~/Library/Application Support/buckyos-agent-daemon`；Linux 使用 `$XDG_DATA_HOME/buckyos-agent-daemon` 或 `~/.local/share/buckyos-agent-daemon`。可用全局 `--data-dir` 覆盖；更新保留其中 `ledger/` 以查询原操作，不重放 unknown/partial 动作。

## 下载

GitHub Actions 为 Linux x86_64（静态 musl）/ARM64、macOS Apple silicon/Intel、Windows x64/ARM64 构建 release 包。稳定链接（始终指向最新版本）：

- 列表与校验：<https://github.com/buckyos/buckyos/releases/tag/agent-daemon-latest>（`latest.json` 列出当前版本与每个 target 的 SHA-256）
- 归档：`https://github.com/buckyos/buckyos/releases/download/agent-daemon-latest/buckyos-agent-daemon-<target>.tar.gz`（Windows 为 `.zip`），`<target>` 取 `x86_64-unknown-linux-musl`、`aarch64-unknown-linux-gnu`、`aarch64-apple-darwin`、`x86_64-apple-darwin`、`x86_64-pc-windows-msvc`、`aarch64-pc-windows-msvc`
- 固定版本：`https://github.com/buckyos/buckyos/releases/download/agent-daemon-v<version>/buckyos-agent-daemon-<version>-<target>.tar.gz`

`<version>` 为从实际二进制提取的 BuckyOS 包版本 `<src/VERSION>+build<YYMMDD>`，例如 `0.7.0+build261026`；归档、`manifest.json` 和 `latest.json` 使用同一版本。`--version`、`doctor` 和 `status` 报告包含 channel 等信息的完整运行时版本。同一基础版本在同一编译日重建不会自动递增，已发布的固定版本不会覆盖。

Linux 可用启动脚本自动下载、校验并前台运行；脚本不把 URL 写入文件，`--url-stdin` 让接入 URL 不进入命令行：

```sh
curl -fsSLO https://github.com/buckyos/buckyos/releases/download/agent-daemon-latest/start-buckyos-agent-daemon.sh
sh start-buckyos-agent-daemon.sh run --url-stdin
sh start-buckyos-agent-daemon.sh --download-only   # 只下载到 ~/.cache/buckyos-agent-daemon
```

`BUCKYOS_AGENT_VERSION` 固定版本，`BUCKYOS_AGENT_RELEASE_URL` 指向私有 HTTPS 镜像目录。归档内 `service.sh` 可把 daemon 安装为显式的 systemd user service（`service.sh install --binary FILE --sha256 HASH --url-file FILE`，另有 `upgrade`/`stop`/`uninstall`），不启用 lingering，不自动重启。macOS 与 Windows 解压归档后直接运行。所有产物仅有 SHA-256 校验，没有代码签名或公证。

在仓库根执行 `uv run src/tools/package_agent_daemon.py --target x86_64-pc-windows-msvc --output dist/agent-daemon` 打包 Windows ZIP；ARM64 target 为 `aarch64-pc-windows-msvc`，需对应 MSVC/Windows SDK 与 Rust target。macOS 使用 `--target aarch64-apple-darwin`（Intel 为 `x86_64-apple-darwin`），输出 tar.gz。核对归档 `.sha256` 与 `manifest.json` 内二进制摘要后解压运行。构建与完整协议见 [Agent Daemon](https://github.com/buckyos/buckyos/blob/main/doc/arch/agent_daemon.md)。SDK 的 `ExecutorCommandStream.computer` 可使用桌面协议；草案中的 `buckyos computer` CLI 和 `--present` 模型图片桥接尚未交付。

Windows 独立打包入口默认静态链接 MSVC CRT，清单中的 `requirements.windows_crt` 为 `static`。普通 Cargo 开发构建默认使用动态 CRT，需目标架构的 Microsoft Visual C++ v14 Redistributable；VC runtime DLL 并非 Windows 自带。使用 `--binary` 打包已有 `.exe` 时不会核验其运行库依赖，清单标为 `prebuilt_unverified` 并列出 VC++ v14 runtime 要求，由调用者核对并提供预构建产物所需运行环境。
