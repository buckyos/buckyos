# BuckyOS Agent Daemon

`buckyos_agent_daemon` 是独立的 Linux/macOS 当前用户执行端，复用 Node Executor 的命令、文件和操作账本。macOS 桌面功能要求 macOS 14 或更新版本。

```sh
buckyos_agent_daemon run --url-stdin
buckyos_agent_daemon doctor --json
buckyos_agent_daemon stop --executor <executor_id>
buckyos_agent_daemon pause --executor <executor_id>
buckyos_agent_daemon resume --executor <executor_id>
```

接入 URL 由控制面 Settings → Cluster Manager → Nodes/Devices 签发。桌面截图/输入需单独授权，默认禁用。默认前台运行，退出即下线，不安装启动项，不读取完整 Node 身份，也不监听 3184。Linux UI 固定为 `unsupported`。

macOS 请在当前登录用户的终端运行，不使用 sudo。系统设置 → 隐私与安全性中为实际运行进程/启动宿主授予屏幕录制和辅助功能权限；用 `doctor --json` 检查当前进程是否就绪。输入与截图在 daemon 同一进程内完成，没有另一个后台 helper 身份。打包产物尚未经过 Developer ID 签名或公证，移动或更新二进制后应重新核验权限。

`pause`/`resume` 发送本地控制信号；在线时，`status --json` 的 `instances[].capabilities.ui.state` 通常在下一次心跳（约 5 秒）反映 `paused`。信号发送成功不是原动作取消确认；查询原操作确认执行事实。恢复后旧观察无效，必须重新观察。Ctrl-C、stop、控制面撤销 URL 都可停止接入。

macOS 数据默认保存在 `~/Library/Application Support/buckyos-agent-daemon`；Linux 使用 `$XDG_DATA_HOME/buckyos-agent-daemon` 或 `~/.local/share/buckyos-agent-daemon`。可用全局 `--data-dir` 覆盖；更新保留其中 `ledger/` 以查询原操作，不重放 unknown/partial 动作。

在仓库根执行 `uv run src/tools/package_agent_daemon.py --target aarch64-apple-darwin --output /tmp/agent-daemon-dist` 打包（Intel 使用 `x86_64-apple-darwin`）。核对归档 `.sha256` 与 `manifest.json` 内二进制摘要后解压运行。构建与完整协议见 [Agent Daemon](https://github.com/buckyos/buckyos/blob/main/doc/arch/agent_daemon.md)。SDK 的 `ExecutorCommandStream.computer` 可使用桌面协议；草案中的 `buckyos computer` CLI 和 `--present` 模型图片桥接尚未交付。
