# Jarvis Runtime

- `agent/`：Jarvis 的运行配置、身份提示词、行为模板与多语言资源。
- `dapp_meta/`：应用元数据与 pikg 打包配置，`agent` 子包从 `../agent` 读取源码。
- `dapp_dist/`：生成的 pikg 和中间产物，不纳入版本控制。

在 `src/` 下运行 `uv run buckyos-build.py -s jarvis_runtime` 构建。此模块无需
编译，devkit 直接打包 `agent/`，将 pikg 放入 `rootfs/data/cache/`，供安装流程使用。

在 `src/` 下运行 `./debug_jarvis.sh` 可直接使用 `agent/` 中的源码启动开发调试。
