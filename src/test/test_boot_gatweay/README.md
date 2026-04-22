# boot_gateway debug tests

参考 `~/cyfs-gateway/tests/process_chain_debug/` 的写法，用 `cyfs_gateway debug` 直接验证当前仓库里的 `src/rootfs/etc/boot_gateway.yaml`。

## 运行方式

在仓库根目录执行：

```bash
uv run src/test/test_boot_gatweay/run_debug_tests.py
```

脚本会优先尝试：

- `$BUCKYOS_ROOT/bin/cyfs-gateway/cyfs_gateway`
- `/opt/buckyos/bin/cyfs-gateway/cyfs_gateway`
- 源码树下常见的 `cyfs-gateway` 构建路径

也可以显式指定 binary：

```bash
uv run src/test/test_boot_gatweay/run_debug_tests.py --binary ~/cyfs-gateway/src/rootfs/bin/cyfs-gateway/cyfs_gateway
```

## 说明

- runner 每次会创建临时目录，并复制当前仓库的 `src/rootfs/etc/boot_gateway.yaml`
- `node_gateway_info.json` 由 runner 在测试时直接构造，不依赖运行态 `src/rootfs/etc/node_gateway_info.json`
- 目前覆盖：
  - 本地 app 转发
  - 远端 app 转发
  - root host / host prefix / `/kapi` / `/sso_callback` / `/ndm`
  - `/.well-known/*` 与 `/1.0/identifiers/*`
  - 非法 host reject
  - `/kapi/kevent/*` 早转发
