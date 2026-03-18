# Desktop Packaging

当前仓库只保留本地桌面安装包脚本：

- `make_local_osx_pkg.py`
- `make_local_win_installer.py`
- `make_local_deb.py`

这三套脚本共享同一个配置源：

- `src/bucky_project.yaml`

脚本会在打包时读取：

- `apps.buckyos.*`
- `publish.macos_pkg.apps.*`
- `publish.win_pkg.apps.*`

然后把目录布局、`data_paths`、`clean_paths`、组件列表和默认安装目标固化进最终安装包与安装脚本。安装时不会再回仓库实时读取 `bucky_project.yaml`。

## Common Flow

### 准备 BUCKYOS_BUILD_ROOT
0. 注意依赖，buckyos的local安装包依赖cyfs-gateway和BuckyOSApp,src目录结构固定。安装脚本不会去做git的任何操作
1. cyfs-gateway使用buckyos-build/buckyos-install构建，构建目录在BUCKYOS_ROOT, BuckyOSApp使用pnpm run tauri build构建，构建目录在 `$RUST_BUILD/release/bundle/...`
2. 用 `buckyos-build` 和 `buckyos-install` 准备好 `BUCKYOS_BUILD_ROOT` 下的发布目录。
3. 运行 `make_config.py release --rootfs <staged_rootfs>` 生成发布配置。
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
python3 ./src/publish/make_local_osx_pkg.py build-pkg aarch64 0.5.1+build260115 \
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
python3 ./src/publish/make_local_deb.py build-pkg amd64 0.5.1+build260115 \
  --app-publish-dir /opt/buckyosci \
  --out-dir ./publish
```

## Unified Entry

仓库根目录只保留统一入口：

- `make_local_pkg.py`

它会先执行 Common Flow 里的通用准备步骤：

- 清理并重建 `BUCKYOS_BUILD_ROOT`
- 构建并安装 `cyfs-gateway`、`buckycli`、`buckyos`
- 执行 `make_config.py release --rootfs <staged_rootfs>`
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
python3 ./make_local_pkg.py prepare-root
python3 ./make_local_pkg.py build-pkg
python3 ./make_local_pkg.py build-pkg 0.6.0+build260317 --build-root /opt/buckyosci --out-dir ./publish
python3 ./make_local_pkg.py verify-pkg ./publish/<pkg-file>
python3 ./make_local_pkg.py show-target
```

说明：

- `build-pkg` 不传版本号时，默认使用 `src/VERSION + buildYYMMDD`
- `build-pkg` 默认先执行 `prepare-root`；如果 staging 已经准备好，可加 `--skip-prepare`
- 默认会在 `../BuckyOSApp` 执行 `pnpm run tauri build`，并从 Rust 构建目录 `release/bundle/...` 复制产物到 `BUCKYOS_BUILD_ROOT/BuckyOSApp/`
- 如果不想自动构建桌面端，可加 `--skip-desktop-app-build`
- 如果桌面端产物不在约定目录，可用 `--desktop-app` 明确指定
- `verify-pkg` 会统一转调当前平台对应的子脚本
- `sync-scripts` 会在 macOS / Windows 上同步安装脚本模板
