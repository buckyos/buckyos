# Cargo 单线程测试检查（2026-09-26）

在 `src/` 执行 `cargo test --workspace --no-fail-fast -- --test-threads=1`。
首次运行 2,286 项通过、0 项失败、36 项忽略，没有复现编译错误或过期测试失败。
本机缺少 CI 已要求安装的 `ripgrep`，运行前已补齐。

## 原因与修正

- `slog_daemon/src/upload.rs` 的 6 个响应校验、超时默认值测试在历史提交
  `f60eca0f` 中统一增加了 `#[ignore]`。它们只调用本地函数，不需要网络、凭据
  或运行中的服务；显式执行后全部通过，因此去掉忽略标记，恢复默认覆盖。
- AGENTS 与中英文 README 的测试命令遗漏了 CI 已使用的 `--test-threads=1`，
  已统一。部分测试会修改进程环境或全局状态，仍按单线程运行。
- 未发现此次需要删除的失效测试；未修改生产逻辑、依赖或锁文件。

## 验证与范围

- `cargo test -p slog_daemon upload::tests -- --ignored --test-threads=1`：
  恢复前显式执行 6 项，全部通过。
- 修改后完整单线程复跑：83 个测试目标，2,292 项通过、0 项失败、30 项忽略，
  测试执行耗时合计约 180 秒，退出码为 0。
- `rustfmt --edition 2021 --check src/kernel/slog_daemon/src/upload.rs` 与
  `git diff --check` 通过。
- 其余忽略项为手动集成、凭据/本地数据依赖、性能基线或文档示例，本次未执行。
  完整系统打包与部署未验证；本次没有生产代码修改。

本机完整运行日志：`/tmp/buckyos-cargo-test-initial.log`、
`/tmp/buckyos-cargo-test-final.log`。
