## 开发环境与pkg的更新

通过git clone得到的开发环境默认是一个独立环境，未设置source-service

### 最简单的单机环境

用现有脚本完成构建后立刻运行，不需要使用任何pkg-system的设施来发布/更新

### 使用有身份的自编译环境(nightly-channel,使用真实身份）

核心是与nightly-SN的版本保持同步，应使用nightly环境下SN兼容的版本进行开发测试
git checkout到正确的版本，然后编译运行
该环境下可以发布 app(流程与release环境相同)

## 标准的应用开发环境

确认repo-server已配置成开发模式（git clone的版本默认就是开发环境）
构建时要注意app sub pkg-name必须是完整版的：nightly-apple-x86_64.$pkg_name#version

### 构建 （TODO：app 标准开发目录，考虑到全平台编译工具的完善，默认使用linux，windows和osx平台需要用户手工build好复制到target目录)

1. 构建全平台2进制 --> 构建对应的Docker
2. 基于docker image 构建全平台的image sub_pkg
3. 如果是系统应用，还需要基于二进制再构建全平台的bin sub_pkg

为了提高效率，在测试阶段可以只针对开发环境构建一个平台的pkg

### 测试

1. 注意更新app_name.app_doc.json 的版本
2. 通过 repository service 发布 app 并使新索引在 Zone 内生效
3. 使用 `buckyos app install` 安装应用（如已安装可跳过）
   4.2 zone内已经运行的app会变成新版本

### 发布

根据不同的source-service的规则，提交新的app_doc.
发布的时候有2个channel (nightly / release),这两个环境是独立的。 不依赖nightly新功能的应用开发者，只需要基于release channel进行构建和发布就好了。
在提交app_doc之前，应确保已经获得了 可提交资格（已认证开发者)

## buckyos系统的构建与发布

### AIOS 镜像 CI

`.github/workflows/build-all.yml` 在推送到 `main` 和手动触发时运行。`detect-image-changes`
先从 R2 读取上一次成功发布的镜像指针 `paios-images/main/aios.json`（由导出 job 写入，
记录镜像的源码提交），再用 `git diff <指针 revision> HEAD -- <路径列表>` 判断 AIOS 是否需要重建。
以下情况都会重建：手动触发、指针不存在、指针 revision 不是当前提交的祖先。因为比较基准是上次成功
发布的镜像而不是上一次 push，镜像构建失败后，下一次 push 即使没改 AIOS 路径也会重新构建。
指针内容另存为 artifact `paios-image-pointers`，后续 job 用这份快照，避免并发 run 期间指针变化。

路径列表在 `Detect paios image changes` 步骤的 `aios_paths` 中，包括 `build_aios`、`publish/aios/`、
OpenDAN 及其本地 Rust 依赖、`src/tools/buckyos-agent/`、Rust 补丁、Cargo 清单与锁文件、版本号、
工程配置、两个 workflow 和 `.github/scripts/paios_image*`。新增本地源码依赖时需要同步更新。
外部仓库的 `main` 更新不会触发本仓库的路径检测，需要手动触发重新构建。

`build-aios.yml` 在 Ubuntu runner 上准备两个架构的 musl C/C++ 工具链，执行
`bash ./build_aios --prepare-context "$RUNNER_TEMP/aios-context" --image paios/aios`
编译 Rust 二进制并导出两个架构的 Docker 构建上下文，再由 `docker/build-push-action`
（`provenance: false`、`sbom: false`，使返回的 digest 是单架构 image manifest）分别构建推送，
最后用两个 digest 合并多架构 manifest。

`--prepare-context <dir>` 要求输出目录尚不存在，生成 `<dir>/amd64/`、`<dir>/aarch64/`
及 `metadata.env`（版本、源码提交、构建时间、各架构 OpenDAN SHA-256，以及工作区干净时的
`immutable_tag=<version>-<提交前 12 位>`；CI 要求它非空）。此模式不调用 Docker，不能与 `--push`
同时使用；本地仍可使用 `build_aios --push` 直接构建发布（不推不可变 tag）。

标签：`<version>-<rev12>-<arch>`（不可变，正式包锁定的就是它）、`<version>-<arch>`、`latest-<arch>`，
以及 `<version>`、`latest` 多架构 manifest。`latest-*` 供开发环境使用。

推送完成后 `export` job 对每个架构 pull by digest、打不可变 tag、`docker save | gzip -n`，
按 tar 的 sha256 上传到 R2 `buckyos-installer/paios-images/aios/<arch>/<sha256>.tar.gz`（已存在则跳过），
生成清单片段（artifact `paios-image-aios`），并在 main 上更新指针 `paios-images/main/aios.json`。
脚本是 `.github/scripts/paios_image_export.sh` 和 `.github/scripts/paios_images.py`。

仓库需要 Actions secrets `DOCKERHUB_USERNAME`、`DOCKERHUB_TOKEN`（`paios/aios`、`paios/exttool`
的推送权限）以及 `R2_ACCESS_KEY_ID`、`R2_SECRET_ACCESS_KEY`（`buckyos-installer` 桶读写）。
AIOS 发布流程通过 concurrency 串行执行，避免同时更新同一组标签。手动触发的 `enable_tests`
控制各平台的 Cargo 测试；AIOS 保留生成的 Dockerfile 中的镜像构建检查。

### ExtTool 镜像 CI

ExtTool 和 AIOS 分别判断是否需要构建，规则相同，指针是 `paios-images/main/exttool.json`，
路径列表 `exttool_paths` 包括 `build_exttool`、`publish/exttool/`、`src/VERSION`、`build-all.yml`、
`build-exttool.yml` 和两个导出脚本。

ExtTool 不需要 Rust 或 musl 工具链。workflow 执行
`bash ./build_exttool --prepare-context "$RUNNER_TEMP/exttool-context" --image paios/exttool`，
导出两个架构的 Dockerfile、FreeCAD 安装脚本及 `metadata.env`（版本、源码提交、构建时间、
`immutable_tag` 和各架构 FreeCAD 下载地址），用 QEMU 构建两个架构，标签、导出和指针规则与 AIOS 相同。

镜像分两阶段：Ubuntu 阶段预热 uv / Deno 缓存并安装工具包，最终镜像只有 busybox 加
`/opt/buckyos/tools`（约 34 MB）。FreeCAD 是可选工具，默认不打包，`freecadcmd` 是说明如何启用的占位命令；
需要时用 `FREECAD_APPIMAGE_URL_AMD64` / `FREECAD_APPIMAGE_URL_AARCH64` 指定 AppImage 后自行构建。

### 镜像清单与平台 payload

`resolve-paios-images` 在镜像构建之后运行：本次构建了的镜像用本次的清单片段，没构建的继承指针快照，
组装成 `paios_images.json`（`buckyos_revision` 是本次提交），校验每个 tar 的公开 URL 可访问且大小一致，
上传为 artifact `paios-images-manifest`。main 上任一镜像构建失败时它会失败，平台构建也不再继续；
非 `buckyos/buckyos` 的 main（fork、手动触发其他分支）时不生成清单，payload 处于本机镜像模式。

Linux、macOS、Windows 构建依赖它，在 `buckyos-install` 之后把清单放到
`$STAGE_ROOT/buckyos/bin/paios-images/paios_images.json`。`upload-payloads` 上传 payload 时，
在每个 `buckyos-*.tar.gz` 旁边放一份 `<payload>.paios_images.json`，供在线安装发布流程读取。
镜像 tar 不进 payload：传统安装包由打包机按清单从 R2 取对应架构的 tar，Linux 在线安装在安装时从 R2 下载。
清单格式和 node_daemon 的锁定模式见 `notepads/paios-image-release-todo.md` §3。

我们构建的发布目标有

- deb/rpm 全新安装包 (最好是一个下载安装脚本，总是可以在最新版上)

  - 该最新版的URL也可以支持一些出厂硬件在首次激活前的自动更新
  - 两种安装方式：离线全量安装包（deb/rpm/pkg/exe，带目标架构的 paios 镜像 tar）和 Linux 在线安装（`curl … | sh`，安装时从 R2 下载 payload 和镜像 tar）。两者装完都带同一份 `bin/paios-images/paios_images.json` 和 tar，BuckyOS 运行时不再从 docker.io 拉取 paios 镜像
- buckyos desktop 安装包
- 将buckyos的pkg_list发布到source-service上，当前channel且启用了自动更新的运行中系统，会很快更新到新版本上

### 日常开发循环

常规逻辑可以使用最简单的单机环境进行开发和验证，可以通过删除 $BUCKYOS_ROOT/data/system_config 让系统从boot流程开始。该开发循环用到的所有pkg-env都处于开发模式，不会启用自动升级
但手工向该repo-server发布pkg是可行的,发布的pkg会按正常逻辑影响zone内的所有node

完成基本的开发测试后，要进行进一步验证。为了防止受到外界环境的影响，buckyos的系统开发验证过程一定使用的是独立环境。我们提供了一些简洁的虚拟机模板，用来模拟一些典型的集群环境（这些集群环境里包含刚刚构建的独立SN），并有基于这些典型集群环境的测试用例。

系统开发者可以基于这些典型环境设计用例，或进行开发。当需要在这些典型环境中部署刚刚编译的软件时，就需要手工向zone repo 发布pkg:

- 运行脚本，会准备待pack的目录，可以进行一些打包前的检查
- 继续运行脚本，完成所有的pkg的pack
- 执行发布脚本，发布选中的packed pkg到本地repo，并调用pub index让其立刻生效

### buckyos nightly build

系统开发者完成日常开发循环后，提交代码。合并到buckyso 当前nightly分支的代码会触发自动构建。自动构建按如下步骤进行（用户也可在本地环境手工触发自动构建）

- 完成所有支持平台的二进制编译和root fs准备
- 基于构建结果构建deb/rpm安装包
- 基于构建结果构建buckyos desktop安装包
- 基于构建结果构建全平台的pkg
- 按流程启动典型环境（OOD一般是linux的虚拟机），并部署刚构建的二进制包
- 在典型环境中运行所有的测试用例
- 验证通过后，本次CI构建完成，开始发布
  - 创建新的github nightly release
  - 将安装包发布到nightly的下载服务器（用脚本进行安装会得到新版本）
  - 将pkg推送到nightly的source-service,所有运行在nightly环境系统都会触发自动升级

#### always run

我们有多个运行在nightly 环境的测试机，会从用户的角度，不断的模拟一些常见动作。

- 各个支持平台的自动安装
- 打开自动更新的模拟生产系统，不断的运行一些always run测试用例

在nightly环境发布后，上述测试如果失败会给出自动警告，说明该版本有问题。手工验证后可以撤销某个nightl build.

从维护成本的角度，我们不实现真正意义上的“回滚”，而是通过发布一个修正了问题的新的nightly版本来实现同样的效果

### 正式发布

正式发布通常都是选择发布一个指定的nightly版本
选择需要正式发布的nightly版本后，相关脚本可以自动化的运行并完成发布工作。

- 创建新的github release
- 将安装包发布到release的下载服务器（用脚本进行安装会得到新版本）
- 将pkg推送到release的source-service,所有运行在nightly环境系统都会触发自动升级

我们在release环境也有always run用于实时监控release版本的系统是否在正常工作。
