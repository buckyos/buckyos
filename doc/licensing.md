# 许可证与发行材料

本仓库自有代码采用 `AGPL-3.0-only`；标准全文在根目录 `LICENSE`。本次调整范围内的公开关联仓库自有代码采用 `Apache-2.0`；私有仓库不纳入此开源声明。根目录 `NOTICE` 列出第三方源码例外；依赖、复制的第三方源码、字体和独立工具保留原许可证，不能批量改写其许可证或版权声明。

## BuckyOS 发行包

构建入口 `src/buckyos-build.py` 从根目录生成 `src/rootfs/licenses/buckyos/`，安装到 `$BUCKYOS_ROOT/licenses/buckyos/`。该目录包含 `LICENSE`、`NOTICE`、本说明和源码版本记录。独立 Agent Daemon 包也携带这些材料；`build_aios` 将其放入镜像的 `/opt/buckyos/licenses/buckyos/`。

源码版本记录标明打包工作区的 Git 提交及是否有修改，不自动证明传入的现成二进制与该工作区匹配（`scope: packaging_checkout`、`binary_source_verified: false`）。干净提交的源码链接用于定位该工作区；有未提交修改的构建必须另外提供实际修改后的源码，不能用提交链接冒充对应源码。二进制发布者还需核对构建来源，保存当次 Cargo/npm/Python 锁定结果、Git 依赖实际提交、补丁和构建脚本，并按适用许可证提供完整对应源码。GitHub 的单仓库自动源码压缩包不包含所有 Git 依赖和子模块。

对外提供修改后的网络服务时，应在用户可见位置提供该运行版本的对应源码获取入口，满足 AGPLv3 第 13 条；链接必须覆盖实际修改，不能只指向滚动的 `main`。安装器协议不限制各开源许可证授予的复制、修改、分发等权利。

## 扩展工具发行包

`publish/exttool/ext-core/build.sh` 在裁剪运行文件前收集各组件的版权与许可证文本、精确构建元数据、源码、配方和补丁。`store/ext-core/LICENSES/` 中的组件清单区分二进制下载地址与源码材料，不把二进制 URL 标为源码。

材料校验通过后，每个平台同时生成：

- `ext-core-<version>-<platform>.tar.gz`：运行文件和许可证材料。
- `ext-core-<version>-<platform>-corresponding-source.tar.gz`：匹配的源码与构建材料。

`manifest.json` 的平台项通过 `corresponding_source` 记录源码包路径、大小与 SHA-256。运行包和源码包携带相同的 `source-manifest.json`，发布流程校验二者一致，并将二进制与源码包一起上传到同一版本的发行位置。获取入口见运行包中的 `store/ext-core/LICENSES/SOURCE_CODE.md`。

重新分发时，必须保留许可证与版权声明，并同时提供与所分发二进制一致的源码包；只转发原上游首页或一个二进制下载链接不足以替代适用的源码提供义务。源码包需要按实际配方保存第三方源码、补丁及构建配置，不能用当前 `main` 或不匹配版本代替。已发布的旧工具包不会因本次脚本修改自动补齐，需重新构建、核验并发布新版本。

MPL 部分保留文件级授权及源码提供安排；LGPL 部分还需满足适用的替换、修改和重新链接要求。GPL/AGPL 独立工具不因聚合分发而改用 BuckyOS 或安装器的许可证。

扩展包的在线项（`ext-manifest.json` 的 `online`）不在我们的分发范围内：包里只写上游的官方地址、SHA-256 和大小，包装好以后由节点自己从上游下载到 `$BUCKYOS_ROOT/tools/online/<id>/`，运行包、`.pikg`、离线包和镜像都不带它，源码包和 `LICENSES/` 里也没有它的材料。目前只有 ext-core 的 `chromium`（Chrome for Testing，归档里带闭源的 Widevine CDM）是在线项；它的使用条款是节点的 owner 与 Google 之间的事。

## 发布核对

先检查许可证与包元数据，再执行相关打包测试和实际归档检查。确保运行包中的许可证、来源记录和发行的对应源码包完整匹配。禁止发布缺少必需源码、许可证或补丁材料的扩展工具包。

当前保留官方 Pandoc 二进制。其完整静态依赖源码（与官方二进制对应的 Haskell 依赖闭包和构建计划）尚未补齐；owner 于 2026-11-15 明确接受这一缺口，记为 `publish/exttool/ext-core/official-sources.json` 里 Pandoc 组件的 `accepted_source_gaps`（写明缺口、接受人、日期），不再阻止发行，并随每个对应源码包的清单一起发布。没有被明确接受的缺口仍记在 `unresolved_source_requirements`，会阻止最终归档和发行；不会将只含主仓库的源码标为完整。
