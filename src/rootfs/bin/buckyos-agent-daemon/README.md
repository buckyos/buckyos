# BuckyOS Agent Daemon

`buckyos_agent_daemon` 是独立的 Linux 当前用户执行端，复用 Node Executor 的命令、文件和操作账本。

```sh
buckyos_agent_daemon run --url-stdin
buckyos_agent_daemon doctor --json
buckyos_agent_daemon stop --executor <executor_id>
```

接入 URL 由控制面 Settings → Cluster Manager → Nodes/Devices 签发。默认前台运行，退出即下线，不安装启动项，不读取完整 Node 身份，也不监听 3184。Linux UI 固定为 `unsupported`。

构建与完整协议见 [Agent Daemon](../../../../doc/arch/agent_daemon.md)。
