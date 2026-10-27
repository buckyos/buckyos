# OOD 冷启动等待 system-config（#687）

Issue：[buckyos/buckyos#687](https://github.com/buckyos/buckyos/issues/687)。

## 根因

`keep_system_config_service` 只确认进程状态；启动链固定等待两秒后调用空实现
`wait_sysmte_config_sync`，在 system-config 尚未监听时就注册设备。首次写入失败直接使
node-daemon 退出，后续服务编排无法继续。kRPC 和 SystemConfigClient 又把网络与业务错误
统一转换成字符串，不能安全地对所有错误重试。

## 修复

- 创建带设备 bootstrap token 的本机客户端后，读取 `boot/config` 验证真实 RPC 可用。
  成功和 `KeyNotFound` 均放行；首次激活仍由原有 boot scheduler 生成配置。
- 就绪检查、设备文档写入、设备信息写入各有 60 秒总期限，单次请求最多 3 秒，退避从
  200 毫秒增长到最多 2 秒。仅网络故障和超时重试；业务、认证、权限和解析错误立即报告。
  超时错误包含操作、次数和最后失败原因。
- 只重复写入相同的设备文档或已生成的设备信息，不重启服务进程、不重复采集设备信息、
  不删除身份或业务数据；成功日志明确表示 RPC 就绪，不宣称复制同步或整机启动完成。
- 配套修改 `buckyos-base/src/kRPC/src/lib.rs` 保留 HTTP 传输错误及其分类。
  SystemConfigClient 的 RPC 错误转换保留 `Timeout` / `TransportError`；既有 Control Panel
  瞬态错误处理同步接收新的传输错误类型。Runtime 和 xllm 的穷尽错误分类同步处理新增分支。

## 联动与验证

此次修改不增加依赖，不改变 RPC 请求/响应、存储、scheduler 接入或 WebSDK 协议。
Rust 错误类型有新增枚举分支；发布时必须先合入配套 buckyos-base 修改，再由 BuckyOS
解析 `main` 分支依赖。测试使用临时 Cargo patch 指向同级 base 源码，未提交本机路径依赖。

`buckyos_admin` 手册已补充启动等待、超时定位和整机验收边界；`xllm` 的参数、命令与结果
协议不变，无需修改手册，其它 cmd_skills 的命令、路径与协议不受影响。

已通过：

- 在 buckyos-base 根目录执行
  `cargo test -p kRPC --manifest-path src/Cargo.toml -- --test-threads=1`：62 项通过。
- 在 buckyos/src 执行
  `cargo test --config /tmp/buckyos-687-local-base.toml -p node_daemon system_config_boot_tests -- --test-threads=1`：
  8 项通过。真实 loopback 覆盖 0/3/10/30 秒延迟监听、首次激活缺失配置、永久不监听、
  挂起请求、单次超时恢复、认证/权限失败、就绪后注册断线与注册权限失败。
- `cargo test --config /tmp/buckyos-687-local-base.toml -p node_daemon -p buckyos-api -- --test-threads=1 --skip node_daemon::system_config_boot_tests`：
  buckyos-api 339 项通过；node-daemon 其余 148 项通过、3 项原有 ignored。
- `cargo test --config /tmp/buckyos-687-local-base.toml -p agent_tool --lib rpc_http_timeout -- --test-threads=1`：
  2 项通过。
- `cargo test --config /tmp/buckyos-687-local-base.toml -p system_config bootstrap_assertion_is_explicit_and_read_only_except_for_own_device -- --test-threads=1`：
  bootstrap 权限回归通过。
- 两个仓库的 `git diff --check` 通过。

未对运行中的 Zone 执行停止、重装或冷启动；真实已有身份冷启动、首次激活完整服务编排、
多 OOD、跨平台 DV、全 workspace 测试及完整安装包构建未执行。
