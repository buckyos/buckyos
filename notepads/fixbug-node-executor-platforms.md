# Node Executor 多平台构建修复

## 根因

GitHub Actions run `39646697595` 中，Windows 构建因 `node_executor` 无条件引用 Unix API 报 40 个编译错误；macOS 构建因 Linux 专有的 `prctl`、`close_range` 报 7 个错误。`cfg!(target_os = "linux")` 只生成布尔值，不能阻止编译器检查 `libc::geteuid()`。

当前实现只允许 Linux root daemon 提供 exec/read/write/edit，文件 worker 的路径校验还依赖 `/proc/self/fd`，不能只屏蔽降权调用后在其它平台执行。Windows 的账本初始化另有 `File::open(directory).sync_all()` 不适用的问题。

## 修复

- 使用 `#[cfg]` 隔离 Linux 执行后端，把 worker 实现移到 `worker_linux.rs`，共享任务和回复类型保持不变。
- 非 Linux 的启动入口拒绝执行；Command Stream 保留 open_stream/describe/query/cancel，执行、文件操作和工作目录解析返回 Capability。
- 隔离 Unix 退出信号、passwd 和 hostname API；Windows hostname 使用 COMPUTERNAME。
- 账本目录同步复用 `libopendan` 的平台模式：Unix 保留目录 fsync，非 Unix 不执行目录 fsync，文件本身仍同步。
- Linux 集成测试按平台编译；新增可移植控制接口、能力拒绝和非 Linux worker 无副作用测试。
- 同步 `doc/arch/node_exector.md` 和 `buckyos_admin` 命令手册的平台说明。未改变协议、配置结构或依赖。

## 验证

- `cargo test -p node_executor --locked -- --test-threads=1`：Linux root 环境 14 项通过；其中不支持执行的分支在 root 下提前返回。
- 将同一 platform 测试可执行文件以 nobody 身份运行，临时目录设为 `/tmp`：2 项通过，实际覆盖能力拒绝分支及账本无操作/上传记录。
- `cargo check -p node_daemon --locked`：通过。
- `git diff --check`：通过。
- `cargo check -p node_executor --lib --target x86_64-pc-windows-msvc --locked`：阻断于 ring/aws-lc-sys 的 C 工具链，当前 Linux 环境缺少 MSVC `lib.exe`，未完成 Windows 检查。
- Windows/macOS 原生构建和测试、完整系统打包与在线 Zone DV 未在本机执行，需由对应平台 CI 验证。
