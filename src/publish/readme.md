# Desktop Packaging

当前仓库只保留本地桌面安装包脚本：

- `make_local_osx_pkg.py`
- `make_local_win_installer.py`
- `make_local_deb.py`

这三套脚本共享同一个配置源：

- `src/bucky_project.yaml`

统一入口 `make_local_pkg.py` 会先把它转换成一份中间 manifest JSON，
平台脚本优先消费这份 manifest；只有直接单独调用平台脚本且未传 `--manifest`
时，才会回退读取 `bucky_project.yaml`。

manifest 会固化：

- `apps.buckyos.*`
- `publish.macos_pkg.apps.*`
- `publish.win_pkg.apps.*`
- 如本地存在 `../cyfs-gateway/src/bucky_project.{yaml,yml,json}`，会把其中 `cyfs-gateway` app 的安装项并入 `buckyos`

然后把目录布局、`data_paths`、`clean_paths`、组件列表和默认安装目标固化进最终安装包与安装脚本。安装时不会再回仓库实时读取 `bucky_project.yaml`。
打包和本地安装时会过滤 `.DS_Store`、`__pycache__` 这类无关文件/目录。

## Common Flow

### 准备 BUCKYOS_BUILD_ROOT
0. 注意依赖，buckyos的local安装包依赖cyfs-gateway和BuckyOSApp,src目录结构固定。安装脚本不会去做git的任何操作
1. cyfs-gateway 使用 `buckyos-build` / `buckyos-install` 构建，构建目录在 `BUCKYOS_ROOT`，BuckyOSApp 使用 `pnpm run tauri build` 构建，构建目录在 `$RUST_BUILD/release/bundle/...`
2. 在本仓库内用 `uv run ./src/buckyos-build.py` 和 `uv run buckyos-install` 准备好 `BUCKYOS_BUILD_ROOT` 下的发布目录。
3. 运行 `uv run ./src/make_config.py release --rootfs <staged_rootfs>` 生成发布配置。
### 制作安装包
4. 调用对应平台的 `make_local_*` 脚本执行 `build-pkg`。
5. 如需校验，调用对应脚本的 `verify-pkg`。
6. 版本号规则是 src/VERSION下的内容作为主版本，然后 +buildYYMMDD

默认 staging 根目录由 `BUCKYOS_BUILD_ROOT` 决定：

- macOS / Linux 默认 `/opt/buckyosci`
- Windows 默认 `C:\opt\buckyosci`

## Platform Commands

macOS:

```bash
uv run ./src/publish/make_local_osx_pkg.py build-pkg aarch64 0.5.1+build260115 \
  --app-publish-dir /opt/buckyosci \
  --out-dir ./publish
```

Windows:

```powershell
python .\src\publish\make_local_win_installer.py build-pkg amd64 0.5.1+build260115 `
  --app-publish-dir C:\opt\buckyosci `
  --out-dir .\publish
```

Linux:

```bash
uv run ./src/publish/make_local_deb.py build-pkg amd64 0.5.1+build260115 \
  --app-publish-dir /opt/buckyosci \
  --out-dir ./publish
```

## Unified Entry

仓库根目录只保留统一入口：

- `make_local_pkg.py`

它会先执行 Common Flow 里的通用准备步骤：

- 清理并重建 `BUCKYOS_BUILD_ROOT`
- 构建并安装 `cyfs-gateway`、`buckycli`、`buckyos`
- 执行 `uv run ./src/make_config.py release --rootfs <staged_rootfs>`
- 按约定在 BuckyOS 同层目录查找并构建桌面端项目：
  `../cyfs-gateway`、`../BuckyOSApp`
- 桌面端构建产物优先从用户环境变量 `RUST_BUILD` 读取；若未设置，则读取 `~/.cargo/config.toml` 的 `[build].target-dir`；再回退到 `/tmp/rust_build`
- 如需覆盖默认约定，可用 `--desktop-app` 直接指定桌面端产物

然后再自动识别当前 OS/Arch，并转调 `src/publish/` 下对应的子脚本：

- macOS -> `make_local_osx_pkg.py`
- Windows -> `make_local_win_installer.py`
- Linux -> `make_local_deb.py`

常用命令：

```bash
uv run ./make_local_pkg.py prepare-root
uv run ./make_local_pkg.py build-pkg
uv run ./make_local_pkg.py build-pkg 0.6.0+build260317 --build-root /opt/buckyosci --out-dir ./publish
uv run ./make_local_pkg.py show-manifest
uv run ./make_local_pkg.py show-manifest --out /tmp/buckyos-pkg-manifest.json
uv run ./make_local_pkg.py verify-pkg ./publish/<pkg-file>
uv run ./make_local_pkg.py show-target
```

说明：

- `build-pkg` 不传版本号时，默认使用 `src/VERSION + buildYYMMDD`
- `build-pkg` 默认先执行 `prepare-root`；如果 staging 已经准备好，可加 `--skip-prepare`
- 默认会在 `../BuckyOSApp` 执行 `pnpm run tauri build`，并从 Rust 构建目录 `release/bundle/...` 复制产物到 `BUCKYOS_BUILD_ROOT/BuckyOSApp/`
- 如果不想自动构建桌面端，可加 `--skip-desktop-app-build`
- 如果桌面端产物不在约定目录，可用 `--desktop-app` 明确指定
- `verify-pkg` 会统一转调当前平台对应的子脚本
- `sync-scripts` 会在 macOS / Windows 上同步安装脚本模板
- `show-manifest` 会输出按安装项目分组的通用 JSON 清单，其中 `module_items` / `data_items` / `clean_items` 都带 `target_dir_name`
- manifest 里的 `source_rootfs` / `source_path` 语义是 staging 后的 `BUCKYOS_BUILD_ROOT` 路径；原工程来源只保留在 `project_source_*` 元数据里
