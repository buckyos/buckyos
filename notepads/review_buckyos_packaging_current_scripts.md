# BuckyOS 打包脚本实现差距 Review

本文按 `notepads/impl_buckyos_packaging.md` 对当前打包脚本做只读检查，范围包括：

- `make_local_pkg.py`
- `src/bucky_project.yaml`
- `src/publish/make_local_win_installer.py`
- `src/publish/make_local_osx_pkg.py`
- `src/publish/make_local_deb.py`
- `src/publish/deb_template/DEBIAN/*`
- `src/publish/macos_pkg/scripts/*`
- `src/publish/win_pkg/scripts/*`

结论：当前实现已经有一部分 `modules => overwrite`、`data_paths => .buckyos_installer_defaults` 的雏形，但离实现文档的目标还有明显差距。最大问题集中在统一入口/manifest 公共解析、rpm 缺失、组件配置校验不足、hook 机制没有按规范实现、平台安装/静默安装行为不稳定，以及若干配置字段或脚本文件处于“读了但没真正生效”或“留下文件但不会执行”的状态。

## 1. 统一入口与产物目标

### 1.1 缺少 `--format rpm` 和 rpm 平台脚本

现状：

- `make_local_pkg.py` 的目标脚本映射固定为 Linux -> `make_local_deb.py`，没有 `--format` 参数和 rpm 分支，见 `make_local_pkg.py:501-505`、`make_local_pkg.py:926-945`。
- 仓库中没有 `src/publish/make_local_rpm.py`，也没有 rpm template/spec。

应修改：

- 在 `make_local_pkg.py build-pkg/verify-pkg/show-target/show-plan` 增加 `--format deb|rpm`，Linux 默认 `deb`。
- 新增 `src/publish/make_local_rpm.py` 和 rpm template/spec 生成逻辑。
- `detect_target` 需要根据 Linux `--format` 选择 deb/rpm 脚本。
- 实现文档中已明确 `--format` 默认值为 `deb`；代码实现时应保持这个用户体验。

理由：

- 实现文档要求 Fedora rpm 是目标产物，且 `python make_local_pkg.py build-pkg --format rpm` 是标准入口。
### 1.2 默认 staging root 以当前实现为准

现状：

- 当前默认 root 是 `/opt/buckyosci` / `C:\opt\buckyosci`，见 `make_local_pkg.py:479-484`。
- 实现文档原先要求默认 Linux/macOS 为 `/opt/buckyos`，Windows 为 `C:\opt\buckyos`，这与现有本地打包体验不一致。

应修改：

- 不改脚本默认值，继续保留 `/opt/buckyosci` / `C:\opt\buckyosci` 作为默认 staging root。
- 实现文档已更新为当前默认值。

理由：

- 默认路径影响版本解析、staging 输入、用户本地命令体验，应优先保证现有开发者本地流程一致。

### 1.3 架构 canonical 值不一致

现状：

- `make_local_pkg.py` 对 Linux 的 `aarch64/arm64` 归一化为 `arm64`，见 `make_local_pkg.py:487-493`。
- deb 输出文件名也使用 `arm64`，会生成 `buckyos-linux-arm64-{version}.deb`，见 `src/publish/make_local_deb.py:488-489`。
- 实现文档原先的 canonical arch 是 `amd64|aarch64`，已调整为 `amd64|arm64`。

应修改：

- 外层入口、内部 manifest 和产物文件名统一使用 canonical `arm64`。
- rpm 平台脚本内部再将 `arm64` 映射为 rpm arch `aarch64`。

理由：

- 产物命名、show-plan、CI 产物匹配都应使用统一的用户可见架构名，避免同一架构在不同命令中显示为 `arm64` / `aarch64`。

### 1.4 产物文件名不符合规范

现状：

- Windows 输出 `buckyos-win-{architecture}-{version}.exe`，见 `src/publish/make_local_win_installer.py:561`、`src/publish/make_local_win_installer.py:1259-1260`。
- macOS 输出 `buckyos-apple-{architecture}-{version}.pkg`，见 `src/publish/make_local_osx_pkg.py:704`。
- 产物架构名统一为 `arm64` 后，macOS/rpm 仍需要跟随调整。

应修改：

- Windows 改为 `buckyos-windows-amd64-{version}.exe`。
- macOS 改为 `buckyos-macos-{arch}-{version}.pkg`。
- deb/rpm 文件名使用 canonical arch，即 `amd64|arm64`。

理由：

- 实现文档把这些文件名列为目标产物，后续 CI/CD 和用户文档会依赖。

### 1.5 `show-plan` 缺失，内部 manifest 公共解析不足

现状：

- 外层只有 `show-manifest`，没有 `show-plan`，见 `make_local_pkg.py:1102-1128`、`make_local_pkg.py:1134-1146`。
- 当前临时 JSON 顶层是 `platforms` / `install_projects`，见 `make_local_pkg.py:436-445`。这套结构可以继续保留，不再要求改造成额外的扁平文件计划。

应修改：

- 沿用当前内部 manifest 结构：`module_items` 表示 `overwrite`，`data_items` 表示 `preserve_existing`，`clean_items` 表示卸载/清理规则。
- 不新增另一套用户可见或内部长期维护的展开计划。
- `platform/format/arch/version/staging_root` 属于 build context，由外层参数和平台脚本入口决定，不强制写入 manifest 顶层。
- `show-plan` 打印当前内部 manifest；`show-manifest` 可保留为兼容别名。
- 将 manifest 加载、校验、路径解析、item 语义转换抽成公共模块，保证 Windows/macOS/deb/rpm 对 manifest 的解释一致。

理由：

- “从安装包安装项目”和“从某个本地路径安装项目”本质上共享同一套安装项目语义。问题不在于 manifest 结构本身，而在于当前公共解析/校验不足，导致平台脚本重复解释。
### 1.6 版本 fallback 逻辑应删除

现状：

- 默认版本解析只接受包含 `+build` 的版本，见 `make_local_pkg.py:529-536`。
- `_default_version()` 从 `src/VERSION` 生成版本但当前 build-pkg 流程没有使用，见 `make_local_pkg.py:515-519`。

应修改：

- 默认版本仍从 `{staging}/buckyos/bin/node-daemon/node_daemon --version` 获取，并要求版本号包含 `+build`。
- 未传版本且无法从 node_daemon 获取时必须失败。
- 删除未使用的 `_default_version()` 和 `src/VERSION` fallback 路径。

理由：

- 基础的 `buckyos-base` 包保证 `node_daemon` 会输出带 `+build` 的版本号。`_default_version()` 和 `src/VERSION` 是 `bucky_project.yaml` 没有加入 `version` 字段前的临时实现，继续保留会制造第二套版本来源。
## 2. 配置校验与配置内容

### 2.1 `src/bucky_project.yaml` 当前不满足三组件要求

现状：

- macOS 声明了 `BuckyOSApp`、`buckycli`、`buckyos`，但 `BuckyOSApp` 和 `buckycli` 是 `optional: false`，见 `src/bucky_project.yaml:135-152`。
- Windows 只声明了 `BuckyOSApp` 和 `buckyos`，缺少 `buckycli`，见 `src/bucky_project.yaml:153-167`。
- `default_selected` 字段没有声明。
- Windows 的 `system_service: true,` 被 YAML 解析成字符串 `"true,"`，见 `src/bucky_project.yaml:165`。

应修改：

- Windows/macOS 都声明 `BuckyOSApp`、`buckyos`、`buckycli` 三个组件。
- 三个组件均设置为 `optional: true`、`default_selected: true`。
- `system_service` 改成合法布尔值 `true`。
- Windows/macOS 的 `default_target` 按实现文档重新明确最终安装路径，而不是临时 staging 位置。
- 新增 `publish.linux_pkg.apps`，让 Linux deb/rpm 也通过 `publish` 段声明平台安装包组件。当前 Linux 声明 `buckyos` app，`buckycli` 通过 `apps.buckyos.modules.buckycli` 随 service payload 安装。
- Linux 没有交互式组件选择，`optional/default_selected` 只用于配置结构一致性和 bool 校验，不驱动包管理器安装行为。

理由：

- 实现文档要求三个桌面安装器组件均可选且默认选中，静默安装默认全装。
- Linux 段缺失是历史发展原因：先有单独 deb 打包脚本，再引入 `bucky_project.yaml`、devkit 统一编译/安装，以及 Windows/macOS 打包脚本。现在统一 review 时应补上 Linux deb/rpm 配置边界。
- Windows 安装包当前默认位置为 `C:\BuckyOS`，很可能是脚本写死，没有使用配置中的 `default_target`；后续应让默认安装位置的来源在文档和脚本中一致。
### 2.2 配置字段校验不足

现状：

- `load_app_layout()` 在 `apps.buckyos` 不存在时会默默使用空 map，见 `src/publish/make_local_deb.py:100-118`、`src/publish/make_local_osx_pkg.py:132-150`、`src/publish/make_local_win_installer.py:142-160`。
- `type` 没有限定为 `app|bundle`。
- `optional` 使用 `bool(value)`，字符串 `"false"` 会变成 `True`，见 `src/publish/make_local_osx_pkg.py:278`、`src/publish/make_local_win_installer.py:266`。
- `default_selected` 完全没有进入 dataclass，也没有被解析，见 `src/publish/make_local_osx_pkg.py:254-262`、`src/publish/make_local_win_installer.py:240-248`。
- 外层 manifest 当前也没有固定校验 Windows/macOS 三组件个数；这本身可以接受，但仍缺少已声明组件的字段合法性校验。

应修改：

- 在外层生成内部 manifest 前做完整校验：YAML 可解析、`apps.buckyos` 存在、`modules/data_paths/clean_paths` 类型正确。
- 不按固定组件个数做失败校验；只校验已声明组件的字段合法、`type` 枚举合法、bool 可解析。
- Linux manifest 明确排除 `BuckyOSApp`。
- `default_selected` 进入 manifest 并驱动 Windows/macOS 平台安装器选择状态；Linux 上仅用于结构一致性和 bool 校验。

理由：

- 当前很多错误会延迟到打包中途才失败，或者被 `bool()` 错误解释，和实现文档的“平台脚本 MUST 校验”不一致。
- 组件个数不是稳定约束，后续某个平台可能只打其中一部分组件；真正需要收紧的是组件类型、bool、target、source 和 app layout 是否真实存在。
- `load_app_layout()` 在 `apps.buckyos` 不存在时默默使用空 map 是错误的，应改为读取真实 app layout，缺失时直接失败。
### 2.3 外部 `cyfs-gateway` 合并是隐式输入

现状：

- `make_local_pkg.py` 会自动读取兄弟目录 `../cyfs-gateway/src/bucky_project.*` 并把 `cyfs-gateway` 合并进 `buckyos`，见 `make_local_pkg.py:332-352`。

应修改：

- 如果继续需要 `cyfs-gateway`，应在 `src/bucky_project.yaml` 增加 `deps.cyfs-gateway.source: "../cyfs-gateway/src"`，由打包脚本显式处理 `deps` 中声明的组件。
- 不应把 `cyfs-gateway` 直接加入 `apps.buckyos.modules` 作为配置来源；`modules` 在安装包里已经有 `overwrite` 语义，而 `deps` 表示上游项目输入/合并来源。
- 打包脚本不能再隐式扫描未声明的兄弟目录；普通开发者本地打包仍可通过默认 `deps.cyfs-gateway.source` 获得同样便利。

理由：

- 实现文档要求当前仓库的 `src/bucky_project.yaml` 是安装包产品定义来源。隐式读取兄弟仓库会让同一命令在不同工作区产生不同 payload；显式 `deps` 可以保留本地便利，同时让输入来源可审计。
## 3. Payload 规则与文件范围

### 3.1 声明的 module 缺失时会静默跳过

现状：

- deb/macOS 的 `_stage_buckyos_app_root()` 对 `modules` 中不存在的源路径不报错，只是不复制，见 `src/publish/make_local_deb.py:230-246`、`src/publish/make_local_osx_pkg.py:369-381`。
- Windows 对 module 缺失也静默跳过，data_paths 缺失只 warn 后跳过，见 `src/publish/make_local_win_installer.py:455-482`。

应修改：

- `modules` 中声明的 overwrite 路径缺失时，打包必须失败。
- `data_paths` 中声明的 preserve_existing 默认源缺失时，三平台行为统一为失败，除非后续显式引入 optional data path。

理由：

- staging 被定义为完整输入。静默跳过会生成缺文件的安装包，后续 verify 也未必能发现。

### 3.2 app 组件会复制整个目录，未按 `apps.<component>.modules` 收敛

现状：

- macOS 对非 `buckyos` 的目录组件直接 `_copy_dir_contents(src, dst)`，见 `src/publish/make_local_osx_pkg.py:644-649`。
- Windows 对非 `buckyos` 组件也复制整个目录，见 `src/publish/make_local_win_installer.py:1194-1198`。

应修改：

- 对 `type: app` 的组件使用 `apps.<component>.modules/data_paths/clean_paths` 派生文件计划。
- `type: bundle` 才允许把声明的 bundle 源整体作为外源组件复制。

理由：

- 实现文档要求 staging 中未被 `apps.*` 或 `publish.*.apps.*` 声明的文件不得进入安装包。当前 `buckycli` staging 目录里如果混入额外文件，会被整体打进包。

### 3.3 `--extra-bundle` 允许未声明组件进入 macOS 包

现状：

- `make_local_osx_pkg.py build-pkg --extra-bundle` 可注入 YAML 中不存在的 bundle，见 `src/publish/make_local_osx_pkg.py:575-592`、`src/publish/make_local_osx_pkg.py:1419-1424`。

应修改：

- 删除 `--extra-bundle` 参数。
- 需要打入 macOS 包的外部 bundle 必须先声明到 `src/bucky_project.yaml` 的 `publish.macos_pkg.apps`，再进入内部 manifest。

理由：

- 实现文档要求安装包组件来自 `src/bucky_project.yaml` 和内部 manifest，不应从额外命令行临时扩展 payload。
- 该参数可能是早期为了引入 `BuckyOS.app` 添加的兼容入口；现在已有配置文件，继续保留会绕过 payload 声明边界。
## 4. Hook 与脚本发现

### 4.1 Windows 没有实现组件 hook 发现和执行

现状：

- Windows build 只生成/复制 `seed_defaults.ps1`、`ensure_firewall_rules.ps1`、`uninstall_cleanup.ps1` 和 loader 脚本，见 `src/publish/make_local_win_installer.py:1189-1193`、`src/publish/make_local_win_installer.py:1525-1572`。
- NSIS 里没有按 `{component}_{preinstall|postinstall}.ps1/.bat/.cmd` 发现并随组件执行的逻辑。

应修改：

- 增加 Windows hook discovery：按组件 key 查找 `src/publish/win_pkg/scripts/{component}_{step}.ps1|bat|cmd`。
- 将被选择组件的 pre/post hook 打入 installer 并在对应 Section 中执行。
- hook 返回非 0 时设置实现文档定义的 exit code 并失败。

理由：

- 当前脚本只有内置动作，没有通用 hook 机制。后续放置 `buckyos_preinstall.ps1` 也不会生效。

### 4.2 macOS hook 只支持无扩展名，且不应打包 uninstall hook

现状：

- macOS 只查找 `{component}_preinstall`、`{component}_postinstall`、`{component}_uninstall`，见 `src/publish/make_local_osx_pkg.py:674-685`、`src/publish/make_local_osx_pkg.py:1204-1223`。
- 不支持 `.sh` 等扩展；实现文档已调整为 macOS 只支持无扩展名 hook。
- `_materialize_pkg_scripts_from_templates()` 会把 `{component}_uninstall` 复制成 `Scripts/uninstall`，但 macOS Installer 不会在卸载时自动执行这个文件，代码注释也承认 “not auto-executed”，见 `src/publish/make_local_osx_pkg.py:1219-1223`。

应修改：

- macOS hook discovery 保持只查找无扩展名 `{component}_{preinstall|postinstall}`。
- 不要再发现或打包 `{component}_uninstall`；macOS pkg 本期明确不提供卸载入口。
- 新增并维护独立 `uninstall_for_macos` 文档，说明手工停止服务、删除程序文件和可选删除数据的步骤。
- verify-pkg 不应把 `{component}_uninstall` 的存在作为组件脚本必须附加的依据。

理由：

- 只支持无扩展名符合当前 macOS pkg 脚本习惯，减少同一平台多种 hook 命名带来的歧义。
- 当前 `buckyos_uninstall` 看起来被“打包”，但实际上不会随系统卸载执行，也不会被安装到用户可直接调用的位置，属于空转。既然 macOS pkg 设计上不提供卸载入口，就应删除这条误导性的打包路径，把卸载说明放入独立文档。
### 4.3 macOS 通用 `preinstall/postinstall/uninstall` 文件不会被当前打包流程使用

现状：

- `src/publish/macos_pkg/scripts/preinstall`、`postinstall`、`uninstall` 只是调用 `buckyos_*`，但构建逻辑只读取组件前缀文件，见 `src/publish/make_local_osx_pkg.py:674-685`。
- 这三个通用文件不会被 `_materialize_pkg_scripts_from_templates()` 复制到任何组件的 `Scripts/preinstall|postinstall`。

应修改：

- 删除这些不再生效的 wrapper。
- macOS 卸载步骤写入独立 `uninstall_for_macos` 文档，不通过 pkg `Scripts/uninstall` 或通用 wrapper 暗示可自动执行。

理由：

- 这些文件目前只是资源目录里的闲置脚本，容易让维护者误以为它们会被 pkgbuild 执行。

### 4.4 Linux 没有实现组件 hook 拼接

现状：

- deb 只把 `modules` 和 `data_paths` 自动块写入 `preinst/postinst`，见 `src/publish/make_local_deb.py:371-406`。
- 没有从 `src/publish/deb_template/DEBIAN/` 或 `src/publish/linux_deb/scripts/` 按组件发现 hook，也没有拼接到 maintainer scripts。

应修改：

- 增加 Linux hook discovery。
- 将组件 preinstall 拼接进 deb `preinst` / rpm `%pre`，postinstall 拼接进 deb `postinst` / rpm `%post`。
- 拼接顺序使用内部 manifest 的组件顺序。

理由：

- 实现文档明确 Linux 也支持组件 hook script。当前 auto-generated block 不是通用 hook。

### 4.5 `--no-sync-scripts` 与 Windows 内置脚本调用容易脱节

现状：

- Windows build 默认调用 `sync_win_scripts()` 写入 repo 下的 `seed_defaults.ps1` 等脚本，见 `src/publish/make_local_win_installer.py:1136-1138`。
- 但当前仓库里 `src/publish/win_pkg/scripts/` 只提交了 loader 脚本。若传 `--no-sync-scripts`，installer 仍然会调用 `${var_name}\scripts\seed_defaults.ps1` 和 `ensure_firewall_rules.ps1`，见 `src/publish/make_local_win_installer.py:964-967`。

应修改：

- 删除 `--no-sync-scripts` 参数。
- Windows 固定脚本应作为模板或构建生成物稳定进入临时 build 目录，NSIS 中引用的脚本必须总是存在。
- 如果后续仍需要“不同步到 repo 工作区”的能力，应通过“生成到临时目录”实现，而不是让 installer 调用可能不存在的脚本。

理由：

- 当前 `--no-sync-scripts` 名义上禁用同步，但 installer 逻辑仍依赖被同步生成的文件，属于半空转参数；CI 流程也没有使用该参数，继续保留只会增加分支复杂度。
## 5. Windows exe 差距

### 5.1 配置的 `default_target` 没有驱动默认安装目录

现状：

- NSIS 初始化时把所有组件目录都设为 `$BestInstallDrive\buckyos\`，见 `src/publish/make_local_win_installer.py:857-862`。
- `publish.win_pkg.apps.*.default_target` 只在 dry-run 打印中出现，实际默认路径不使用它。

应修改：

- 内部 manifest 中每个组件的 `target` 应来自 `default_target`。
- NSIS 默认目录页应使用组件 target 初始化。
- `buckycli` 按实现文档固定到当前用户 home 下 `.buckycli` 并更新 PATH。

理由：

- `default_target` 是当前最明显的“配置字段存在但安装时不生效”。

### 5.2 `optional` 在 Windows 中基本只影响描述文本

现状：

- Windows Section 总是没有 `/o`，即全部默认选中；`optional` 只追加描述文字，见 `src/publish/make_local_win_installer.py:907-999`。
- 没有 `default_selected` 字段。

应修改：

- 根据 `optional/default_selected` 生成 NSIS Section flags。
- 若后续存在非 optional 组件，应显式锁定；本期三个组件按文档都应 optional true + default selected。

理由：

- 现在 YAML 写 `optional: false` 或 `optional: true` 对可选性本身没有稳定控制。

### 5.3 依赖检查不是按已选组件执行，也没有 Retry/Open/Cancel 和稳定 exit code

现状：

- Docker/VC++ 在 `.onInit` 执行，早于组件选择，见 `src/publish/make_local_win_installer.py:866-892`。
- Docker 缺失和 VC++ 缺失使用 `MessageBox MB_YESNO`，没有 Retry；Engine 不可用、端口占用也没有 Retry，见 `src/publish/make_local_win_installer.py:870-876`、`src/publish/make_local_win_installer.py:925-930`。
- 脚本里没有 `SetErrorLevel`。

应修改：

- 依赖检查移动到组件选择之后，只检查被选组件所需依赖。
- 图形安装实现 Open/Retry/Cancel 流程。
- 静默安装使用 `IfSilent` 分支，不弹窗、不打开浏览器，写 `%TEMP%\{installer_filename}.log`，并 `SetErrorLevel` 到文档定义的 code。

理由：

- 当前静默安装和图形安装错误路径都不符合实现文档。尤其是静默环境下没有稳定 exit code。

### 5.4 Windows 缺少 `buckycli` 组件和 PATH 更新

现状：

- YAML 没有 `publish.win_pkg.apps.buckycli`。
- NSIS 没有独立安装 buckycli 到用户 home `.buckycli` 和写 PATH 的逻辑。

应修改：

- 补齐 Windows `buckycli` publish 组件。
- 在 NSIS 中按固定行为安装到当前用户 `.buckycli`，并更新 HKCU PATH。

理由：

- 实现文档把 `buckycli` 列为 Windows 三个可选组件之一。

## 6. macOS pkg 差距

### 6.1 Docker 检查与实现文档不一致

现状：

- Distribution XML 检查 `/Applications/OrbStack.app`、固定 docker 路径和 `/var/run/docker.sock`，见 `src/publish/make_local_osx_pkg.py:501-515`。
- 实现文档要求 `command -v docker` 和 `docker info`，且允许远程 Docker context，不要求 `/var/run/docker.sock`。
- `buckyos_preinstall` 中已经有 `docker info`，见 `src/publish/macos_pkg/scripts/buckyos_preinstall:25-31`，但 Distribution 会先用 socket 规则拦截。

应修改：

- Distribution 检查不要要求 `/var/run/docker.sock`。
- 最终以 `docker info` 结果判断 Engine 可用。
- 检查应尽量只对选择了 `buckyos` 的安装生效。

理由：

- 当前会误拒绝远程 Docker context 或非 socket 的 Docker 环境。

### 6.2 组件 optional/default_selected 与文档不一致

现状：

- macOS `optional: false` 会生成 `required="true" enabled="false"`，见 `src/publish/make_local_osx_pkg.py:535-545`。
- 当前 YAML 的 `BuckyOSApp` 和 `buckycli` 是 required，见 `src/bucky_project.yaml:135-147`。

应修改：

- 配置改为三组件 `optional: true`、`default_selected: true`。
- 代码读取 `default_selected`，不要无条件 `start_selected="true"`。

理由：

- 实现文档要求三个组件均可选且默认选中。

### 6.3 macOS 最终安装路径与文档不一致

现状：

- `BuckyOSApp` 先安装到 `/Library/Application Support/BuckyOS/BuckyOS.app`，再 postinstall 复制到 `~/Applications/BuckyOS.app` 或 `/Applications/BuckyOS.app`，见 `src/publish/macos_pkg/scripts/BuckyOSApp_postinstall`。
- `buckycli` 复制到 `$HOME_DIR/buckycli`，不是 `.buckycli`，也没有 PATH 更新，见 `src/publish/macos_pkg/scripts/buckycli_postinstall`。

应修改：

- 在内部 manifest 或平台组件配置中区分 pkg staging path 和真实 final target，避免把 `default_target` 当临时位置使用。
- `BuckyOSApp` 最终安装位置固定为 `/Applications/BuckyOS.app`。
- 不再通过 postinstall 判断当前控制台用户并复制到 `~/Applications/BuckyOS.app`。
- `BuckyOSApp` 的用户身份和配置文件与 app bundle 安装位置分离；macOS 非 App Store/pkg 形态使用用户域配置目录，例如 `~/Library/Application Support/BuckyOSApp/`，与 Windows `%APPDATA%` 语义对应。未来 App Store/sandbox 形态可迁移到 app container 或 app group。
- `buckycli` 改到当前用户 home 下 `.buckycli`，并补 PATH 更新。

理由：

- 当前配置 `default_target` 与实际最终位置不一致，属于配置语义空转。
- `/Applications` 是 pkg、未来 App Store 安装和常见 dmg drag-install 的共同用户预期位置；放在这里更容易被 macOS Apps/Launchpad、Finder 和 Spotlight 发现。
- `~/Applications` 只对某个用户可见，不适合当前同时安装 LaunchDaemon/system service 的管理员 pkg 语义。
- ssh/CD 静默安装时 `installer` 通常以 root 执行，控制台用户可能不存在，或与 ssh 用户不一致；依赖 console user 会导致安装路径、owner 和权限行为不稳定。
- `BuckyOSApp` 即使是 BuckyOS Service 的 GUI，也不需要通过 `~/Applications` 表达 per-user 身份；GUI 的用户身份和配置应放在用户配置目录，而不是 app bundle 路径。
### 6.4 macOS 卸载入口应从 pkg 能力中移除

现状：

- `buckyos_uninstall` 只删除 service 的 modules/clean_paths，不处理 `BuckyOSApp` 和 `buckycli` 的最终用户目录。
- 没有询问是否删除 `apps.buckyos.data_paths`。
- `buckyos_uninstall` 当前作为 pkg `Scripts/uninstall` 打入时不会被 macOS Installer 自动执行，也不会安装到系统路径供用户调用，见 `src/publish/make_local_osx_pkg.py:1219-1223`。

应修改：

- 实现文档不再要求 macOS pkg 支持卸载，也不要求提供随 pkg 安装的卸载脚本。
- 删除 `{component}_uninstall` 的发现、materialize 和 verify 逻辑。
- 提供独立 `notepads/uninstall_for_macos.md` 文档，说明手工停止 service、删除 overwrite payload、clean_paths、`BuckyOSApp` 和 `buckycli` 最终位置。
- 文档中默认保留 `data_paths`，把删除用户数据作为明确的可选步骤。

理由：

- 标准 macOS pkg 不提供类似 Windows installer 的明显卸载入口。当前把 `buckyos_uninstall` 放进 pkg `Scripts/uninstall` 只会造成“看起来已实现、实际不会执行”的空转；设计上明确不提供 pkg 卸载入口更清晰。

## 7. Linux deb/rpm 差距

### 7.1 deb preinst 删除 `bin/` 是有意的覆盖安装清理策略

现状：

- `preinst` 在 auto-generated modules block 前直接 `rm -rf "$BUCKYOS_ROOT/bin/"`，见 `src/publish/deb_template/DEBIAN/preinst:4-10`。

应修改：

- 保留删除整个 `$BUCKYOS_ROOT/bin/` 的逻辑。
- 将该行为写入实现文档，明确它是 Linux 覆盖安装时的程序文件根目录清理，用于清掉新包已移除的旧 modules。
- 限定删除范围只能是 `overwrite` modules 所在的程序区域，例如 `/opt/buckyos/bin/`，不得扩展到 `data_paths`、配置文件或其他用户数据路径。
- rpm `%pre` 应采用同等策略，避免 deb/rpm 覆盖安装行为分叉。

理由：

- 删除整个 `bin/` 是为了避免新安装包移除某些 modules 后，覆盖安装无法清理旧版本残留的程序文件。
- 这类删除只作用于 BuckyOS service 的程序文件区域，不影响 `preserve_existing` 数据和用户配置，因此可以作为覆盖安装前的固定清理动作保留。
### 7.2 deb postinst 首次安装可能失败

现状：

- `postinst` 在 `set -e` 下执行 `systemctl stop buckyos.service`，没有 `|| true`，见 `src/publish/deb_template/DEBIAN/postinst:45`。

应修改：

- 改为 `systemctl stop buckyos.service >/dev/null 2>&1 || true`，或移到 preinst 中作为覆盖安装停止动作。

理由：

- 首次安装时 service 可能不存在，`systemctl stop` 非 0 会导致 postinst 失败。

### 7.3 deb verify 覆盖不足

现状：

- deb verify 只检查包存在、可解包、data_paths 是否在 defaults 和包大小，见 `src/publish/make_local_deb.py:505-566`。
- 没检查 control Depends、Architecture、BuckyOSApp 排除、未声明 payload、maintainer script、systemd unit 行为。

应修改：

- `verify-pkg` 增加实现文档 15.2 的基础检查：metadata、依赖声明、payload 白名单、BuckyOSApp 排除、defaults 位置、hook/script 拼接结果。

理由：

- 现在很多实现文档约束即使破坏也不会被 verify 捕获。

### 7.4 rpm 完全缺失

现状：

- 没有 rpm 脚本、spec、Requires、pre/post/preun/postun、arch 映射。

应修改：

- 新增 `make_local_rpm.py`，在 Ubuntu 24.04 rpm 工具链下生成 Fedora rpm。
- 版本转换为 rpm 合法 `Version/Release`，但产物文件名保留外层 `{version}`。

理由：

- 这是实现文档明确的目标格式。

## 8. 空转/半空转清单

这些点建议优先清理，否则后续维护者很容易误判“配置了就生效”。

处理时需要先分类：

- 删除型空转：旧入口、旧 fallback、不会被平台安装器执行的脚本、CI 不使用且会制造分支的参数，应先删除或停止打包。
- 接线型半空转：配置字段已经存在或即将补齐，但当前没有驱动实际安装行为，应保留字段并接到真实逻辑上。

建议把本清单作为改造第一阶段处理。先删除确定不要的逻辑，再在精简后的代码上修正平台行为，最后从已经正确的逻辑中抽取跨平台公共模块。

1. `publish.*.apps.*.default_selected`
   - 当前 YAML 没声明，脚本 dataclass 也没有字段。macOS 无条件 `start_selected="true"`，Windows 无条件默认选中。

2. Windows `publish.win_pkg.apps.*.default_target`
   - build 阶段解析/打印，但 NSIS 默认安装目录统一改成 `$BestInstallDrive\buckyos\`，未按配置生效，见 `src/publish/make_local_win_installer.py:857-862`。

3. Windows `optional`
   - 只影响描述文案，不控制 Section 是否 optional/required，见 `src/publish/make_local_win_installer.py:990-999`。

4. macOS 通用 `scripts/preinstall`、`scripts/postinstall`、`scripts/uninstall`
   - 当前构建只读取组件脚本，这三个 wrapper 不会被 pkgbuild 附加执行；卸载说明改由独立 `uninstall_for_macos` 文档承载。

5. macOS `{component}_uninstall`
   - 被复制成 pkg `Scripts/uninstall`，但 Installer 不会自动执行；本期 macOS pkg 不提供卸载入口，应删除这条发现和打包路径。

6. `generate_welcome_html()`
   - `src/publish/make_local_osx_pkg.py:421-458` 定义了生成 welcome HTML 的函数，但 build 流程没有调用。

7. `_default_version()`
   - `make_local_pkg.py:515-519` 定义了从 `src/VERSION` 生成版本的函数，但当前 build-pkg 默认只走 node_daemon 解析。

8. Windows `--no-sync-scripts`
   - 禁止生成脚本后，installer 仍然调用 `seed_defaults.ps1` 和 `ensure_firewall_rules.ps1`；CI 没有使用该参数，应删除。

9. Windows `system_service: true,`
   - YAML 中是字符串，代码靠 `.rstrip(",")` 特判兼容。应改为合法 bool，去掉脚本里的脏兼容。

10. macOS/Windows 非 `buckyos` app 组件的 `apps.<component>.modules`
    - `apps.buckycli.modules` 声明了 `buckycli`，但平台打包对 buckycli 目录是整体复制，没有按 modules 文件计划执行。

## 9. 抽取跨平台共通打包模块

现状：

- Windows、macOS、deb 三个平台脚本分别解析 `src/bucky_project.yaml`、`publish.*.apps`、`apps.buckyos.modules/data_paths/clean_paths`，但字段校验、bool 解析、缺失路径处理和组件语义不完全一致。
- `module_items/data_items/clean_items` 的语义已经在三个平台中反复出现，但当前由各平台脚本各自解释，导致同一需求在不同平台容易出现行为漂移。
- 架构归一化、产物命名、版本解析、payload 白名单、defaults 目录、hook 发现、manifest 生成和 verify 基础检查也有明显的重复逻辑。

应修改：

- 分析 `make_local_win_installer.py`、`make_local_osx_pkg.py`、`make_local_deb.py`，把三平台共通逻辑抽成统一引入的 Python 模块，例如 `src/publish/package_common.py` 或 `src/publish/packaging_common/`。
- 公共模块负责：项目 YAML 加载与字段校验、bool/type/default_target 解析、`deps` 显式输入处理、平台组件筛选、内部 manifest 生成、`module_items/data_items/clean_items` 语义转换、source path 校验、canonical arch 与产物命名规则。
- 公共模块负责提供统一的 hook discovery 框架和 verify 基础检查框架；平台脚本只补充各自的脚本扩展名、安装器格式、元数据格式、依赖检测 UI、service 注册和平台专有安装动作。
- 平台脚本不再各自重新解释 manifest。新增 rpm 时必须复用同一公共模块，不能复制 deb 逻辑后再局部改写。
- 公共模块的行为需要有脚本级测试覆盖，至少覆盖配置校验、路径展开、missing module 失败、data_paths defaults 生成规则、platform component filtering、arch 映射和产物命名。

理由：

- 这次 review 中很多问题不是单个平台缺功能，而是同一配置字段在不同平台被不同方式解释，例如 `optional/default_selected/default_target`、`apps.<component>.modules`、hook 文件和 data_paths。
- 抽出公共模块可以把“需求只实现一次”变成结构性约束，降低后续加入 rpm、调整 manifest 语义或修改配置字段时的平台分叉风险。
- 过早抽取会把历史兼容、闲置脚本和半接线字段一起固化进公共层；应先删除不要的逻辑并修正平台行为，再从正确逻辑中提炼公共模块。
- 平台脚本仍然保留各自安装器生成逻辑，抽公共模块不会抹平平台差异，只是把输入解析、校验和文件计划这些本应一致的部分固定下来。

## 10. 建议改造顺序

1. 先处理空转/半空转清单：删除确定不要的旧入口、fallback、无效 hook/wrapper、未使用参数和脏兼容；对应该接线的字段建立明确修改点。

   处理记录：commit `c2d19e8c`。修改文件和行号：`make_local_pkg.py` 删除 `VERSION_FILE` / `_default_version()` / 外层 `--no-sync-scripts` 入口（原约 `41`、`515-519`、`944`、`992`），`src/bucky_project.yaml:181` 修正 Windows `system_service` 为合法 bool，`src/publish/make_local_osx_pkg.py` 删除未使用 welcome 生成、`--extra-bundle`、macOS uninstall hook materialize/sync（原约 `421-458`、`575-592`、`1219-1223`、`1354-1361`、`1420-1424`），`src/publish/make_local_win_installer.py` 删除 `--no-sync-scripts` 并去掉 `system_service` 逗号兼容（当前相关逻辑已收敛到 `242-259`、`301-309`），删除整文件 `src/publish/macos_pkg/scripts/preinstall`、`postinstall`、`uninstall`、`buckyos_uninstall`。

2. 修入口和配置基础：`--format`、canonical arch、产物命名、`show-plan`、补齐 Windows/macOS/Linux publish 配置、加入 `default_selected`，收紧 bool/type/default_target 校验。

   处理记录：commit `4fe751a2`。修改文件和行号：`make_local_pkg.py:90-108` 增加 bool/type 校验，`make_local_pkg.py:378-474` 生成三平台 publish manifest 和 `default_selected`，`make_local_pkg.py:538-566` 支持 Linux `--format deb|rpm` 与 canonical `arm64`，`make_local_pkg.py:981`、`1053`、`1139`、`1156` 增加入口参数，`make_local_pkg.py:1196`、`1215` 增加 `show-plan`；`src/bucky_project.yaml:133-184` 补齐 Linux/macOS/Windows publish 配置和三组件默认选择；`src/publish/make_local_osx_pkg.py:270-328`、`344-373` 解析 `default_selected` 与类型/布尔校验，`src/publish/make_local_osx_pkg.py:664` 改 macOS 产物名；`src/publish/make_local_win_installer.py:242-259`、`280-320`、`345-370` 增加同类校验和 `default_selected`，`src/publish/make_local_win_installer.py:633`、`1334-1335` 改 Windows 产物名；`src/publish/make_local_deb.py:124-159` 收紧 `apps.*` layout 校验。

3. 修正各平台真实行为：payload 白名单、module/data path 缺失失败、非 `buckyos` app 组件按 modules 收敛、Windows 静默分支和 exit code、macOS Docker 检查改 `docker info`、deb preinst/postinst 修正。

   处理记录：commit `54c53a44`。修改文件和行号：`src/publish/make_local_deb.py:256-269` 声明 module 缺失失败，`src/publish/deb_template/DEBIAN/postinst:45` 首装 stop service 容错；`src/publish/make_local_osx_pkg.py:398-401` 增加 CLI pkg staging target，`src/publish/make_local_osx_pkg.py:431-444` module 缺失失败，`src/publish/make_local_osx_pkg.py:496-498` 移除 Distribution 全局 Docker socket 检查，`src/publish/make_local_osx_pkg.py:580-607` 非 bundle app 按 modules/data_paths 收敛；`src/publish/macos_pkg/scripts/BuckyOSApp_postinstall:4-20` 固定 `/Applications/BuckyOS.app`，`src/publish/macos_pkg/scripts/buckycli_postinstall:28-49` 安装到 `~/.buckycli` 并写 PATH；`src/publish/make_local_win_installer.py:517-547` module/data 缺失失败，`src/publish/make_local_win_installer.py:389-403`、`933-940` 默认安装目录来自配置，`src/publish/make_local_win_installer.py:922-1049` 静默分支和稳定 exit code，`src/publish/make_local_win_installer.py:1107-1114` buckycli 写入 PATH，`src/publish/make_local_win_installer.py:1301-1317` app 组件按 manifest modules 收敛。

4. 补 hook：Windows/macOS/Linux 都按文件命名规则发现；Linux 拼接到 maintainer scripts；删除 macOS 无效 uninstall hook 打包方式，并维护独立 `uninstall_for_macos` 文档。

   处理记录：commit `1332a5c2`。修改文件和行号：`src/publish/make_local_win_installer.py:46` 增加 Windows hook 扩展名，`src/publish/make_local_win_installer.py:422-487` 增加 hook 发现、打包和执行失败处理，`src/publish/make_local_win_installer.py:1049-1060`、`1116-1127` 在组件 Section 中执行 pre/post hook，`src/publish/make_local_win_installer.py:1327` 将发现到的 hook 写入 payload；`src/publish/make_local_deb.py:390-456` 增加 Linux 组件顺序、hook 发现和拼接逻辑，`src/publish/make_local_deb.py:476-493` 写入 preinst/postinst 自动块；`src/publish/deb_template/DEBIAN/preinst:4-6`、`src/publish/deb_template/DEBIAN/postinst:18-20` 增加 hook marker；`src/publish/make_local_osx_pkg.py:1161-1165` 校验 macOS pre/post hook 必须是普通文件，macOS uninstall hook 已在 `c2d19e8c` 删除并由 `notepads/uninstall_for_macos.md` 承载。

5. 在正确逻辑上抽取跨平台共通模块：统一配置加载、字段校验、platform component filtering、manifest 生成、路径展开、arch/产物命名、hook discovery 框架和基础 verify 框架。

   处理记录：commit `de9de32e`。修改文件和行号：新增 `src/publish/package_common.py:24-37` 公共 YAML/JSON 加载，`40-58` bool/type 校验，`61-78` arch 和产物命名，`81-125` manifest/component item 读取，`128-143` hook discovery；新增 `src/publish/tests/test_package_common.py:16-87` 覆盖 bool/type、arch/命名、manifest helper 和 hook discovery；`src/publish/make_local_deb.py:29`、`175-177`、`345`、`357`、`526` 接入公共模块；`src/publish/make_local_osx_pkg.py:35`、`180-182`、`281`、`603` 接入公共模块；`src/publish/make_local_win_installer.py:31`、`188-190`、`292`、`378`、`661`、`1382`、`1445` 接入公共模块。

6. 统一 manifest 解析：公共模块处理 `module_items/data_items/clean_items` 语义；module/data path 缺失直接失败。

   处理记录：commit `0d149320`。修改文件和行号：`src/publish/package_common.py:128-153` 增加统一 relpath 规范化和 manifest `source_path` 解析；`src/publish/make_local_deb.py:97-103`、`642`、`673` 使用公共 source 解析并让本地 update 遇到缺失 source 失败；`src/publish/make_local_osx_pkg.py:102-108`、`1010`、`1041` 同步使用公共 source 解析和缺失失败；`src/publish/make_local_win_installer.py:87-88`、`167-168`、`195-196` 为 Windows layout 增加 source map，`551-585` staging 时使用 manifest `source_path`；`src/publish/tests/test_package_common.py:90-107` 增加 source_path 映射优先级测试。

7. 新增 rpm，并复用公共模块。

   处理记录：commit `5da54baf`。修改文件和行号：新增 `src/publish/make_local_rpm.py:181-228` 按 manifest/common source 解析 staging module/data 并在缺失时失败，`257-275` 实现 canonical arch 到 rpm arch 与 `Version/Release` 转换，`336-356` 复用公共 hook discovery 查找 Linux rpm/deb hook，`400-510` 生成 rpm spec、Requires、`%pre/%post/%preun/%postun` 和 systemd 服务行为，`525-620` 接入 rpmbuild、输出规范 rpm 文件名并保留外层版本，`622-659` 增加 rpm 基础 verify；新增 `src/publish/tests/test_make_local_rpm.py:13-45` 覆盖 rpm arch/version 转换和 spec 关键契约。

8. 最后扩展 verify-pkg，让它覆盖 metadata、payload 白名单、组件、hook、依赖声明和 defaults 规则。

   处理记录：commit `4a86aaab`。修改文件和行号：`src/publish/package_common.py:156-188` 增加 payload path 规范化、prefix 匹配和未声明 payload 检查 helper；`src/publish/make_local_deb.py:553-670` 增加 Linux 依赖、payload 白名单、组件和 hook 检查 helper，`672-817` 扩展 deb verify 覆盖 control metadata、Depends、payload defaults/白名单、桌面组件排除、preinst/postinst hook marker 和 systemd 片段；`src/publish/make_local_rpm.py:294-374` 增加 rpm 侧同类依赖/payload/组件检查，`473-489` 增加 rpm hook 期望检查，`749-856` 扩展 rpm verify 覆盖 metadata、Requires、payload list、scriptlets，并在 `900-903` 接入 `--project/--manifest`；`src/publish/tests/test_package_common.py:104-127`、`src/publish/tests/test_make_local_rpm.py:48-94` 增加 payload 白名单与 defaults 规则测试。
