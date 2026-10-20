# paios 镜像随版本发布 TODO：镜像清单锁定 + 全量安装包（issue #672）

> 创建日期：2026-10-19
> 版本：v0.5（2026-10-20：用户同意后做了 A6（pandoc 移到 ExtTool）和 A7（三个 Rust 二进制合并），§9.8）。v0.4（2026-10-20：用户确认打包机侧完成后 M2 合入 main，全部 M0–M7 完成）。v0.3（2026-10-20：用户裁决 D1–D10 全按建议、D5 移出 FreeCAD、按 §5.0 顺序直接推 main；按 M0 实测修订 §3.2 / §3.5 的 tar 寻址；开始实施，记录见 §9）。v0.2（2026-10-19：根据 Actions run 39361968276 和 pve-pack-system 源码，修正 §2.4 对打包流水线的描述，补上 Linux onlinepack 这条安装路径，并相应修改 D4、M0、M4 和验收）。v0.1（初稿）。
> 状态：已完成。§4 已裁决；实施记录和与设计不同之处见 §9。
> 依据：[issue #672](https://github.com/buckyos/buckyos/issues/672) 正文与评论（wugren：安装包体验最好、出错少，但包大，镜像不该有 2G；waterflier：docker.io 会限流，确定走安装包方案，2G 的原因待查）；2026-10-19 对 buckyos（main `f31715059`）、buckyos-devkit、sn-business（`CD/cluster_config/ci`）、pve-pack-system（`README.md`、`doc/artifact_pack_entry.md`、`doc/onlinepack.md`、`installer/linux.sh`，HEAD `94f68cbdb`）的源码核对；Actions run [39361968276](https://github.com/buckyos/buckyos/actions/runs/39361968276) 的 job 和 artifact 列表；`doc/CI/BuckyOS的CI发布流程.md`、`doc/CI/prd_buckyos_packaging.md`、`doc/CI/impl_buckyos_packaging.md`、`doc/app_service/paios容器需求.md`、`product/buckyos_desktop_installer/新版本离线安装包的构建思路.md`；本机 `docker image inspect` / `docker history` / 远端 manifest 实测。
> 第 2 节“现状”只以源码和实测为准。

## 0. 给实施 Agent 的约定

- 本 TODO 跨四个仓库，每个任务都标了仓库：
  - **buckyos**：node_daemon、`build_aios` / `build_exttool`、`.github/workflows`、`build.py` / `start.py`、文档。
  - **buckyos-pack-installer**：buckyos 的 `src/publish` 子模块（`.gitmodules`），本机没有检出。deb/rpm/pkg/exe 的平台脚本和 `verify-pkg` 都在这里。
  - **sn-business**：`CD/cluster_config/ci/commands/pack/`，在 PVE 打包机上组装 staging 并调用平台脚本。
  - **pve-pack-system**（内部仓库，不要公开）：`bin/artifact_pack_orchestrator.py` 负责调度打包；`installer/install.sh`、`installer/linux.sh`、`bin/onlinepack.py` 是 Linux 在线安装。
- 合入顺序有硬约束（§5.0）：node_daemon 停止 pull（M2）之前，安装包和 Linux 在线安装都必须已经能把清单和镜像 tar 放到位（M3、M4），否则这段时间里装出来的系统没法用。
- 第三方 Docker 应用的镜像逻辑不在本 TODO 范围内。`prepare_docker_image` 里 pkg tar 优先、pull 兜底的逻辑保持不动。
- 不提交 Docker Hub / R2 凭据。workflow 只引用 secrets。
- 每完成一个 M，在 §9 写实施记录：落点、验证命令和结果、与本文设计不同的地方。

## 1. 目标与范围

issue 提出三点，评论补了一点结论：

1. **正式产品按镜像 id 使用与当前系统版本配套的 paios 镜像，不再总用 latest。** 这靠一个只有正式包才带的清单文件切换：有清单时用清单锁定的镜像；没有清单时用本机的 paios 镜像（开发模式）。**两种模式下 BuckyOS 都不主动 pull，镜像不存在就报错。**
2. **打包时先等 paios 镜像构建完成**（不需要构建时，继承上一个版本的镜像 id），**再生成清单。**
3. **全量（all-in-one）安装包**：把 BuckyOS 依赖的镜像 tar 打进安装包，安装后不需要再从 docker.io 拉镜像。评论已经确定走安装包方案（docker.io 会限流），所以**全量包是本期的默认产物，不是可选项**。
4. 评论提出：查清镜像为什么有 2G，并瘦身。结论见 §2.6，瘦身计划见 §3.8 和 M7。

本轮做：

- 定义镜像清单 `paios_images.json`。node_daemon 实现“锁定模式 / 本机模式”，删掉 worker（`paios/aios`）和 exttool（`paios/exttool`）的自动 pull，缺镜像时从安装包自带的 tar 加载。
- 运行中的 worker 容器要和当前镜像 id 比对，不一致就重建。exttool 卷改成按镜像 id 命名，镜像换了就重新播种。
- CI：镜像加不可变 tag 并导出 tar；用“上次成功发布的镜像 revision”判断要不要重建，不重建就继承；生成清单写进每个平台的 payload。
- 打包：sn-business 按清单取对应架构的 tar 放进 staging；平台脚本的 `verify-pkg` 校验清单和 tar。Linux 在线安装（pve-pack-system `linux.sh`）在安装时按清单从 R2 下载 tar。
- 开发流程：`build.py` 预拉取正确的 tag，`start.py` 在缺镜像时给出提示，更新文档。
- 镜像瘦身（独立一条线，可以并行）。

不做：

- 第三方 Docker 应用的镜像（已经有 pkg tar 加 digest 的机制，见 §2.2）。
- 系统 OTA 时携带镜像。`doc/CI/BuckyOS的CI发布流程.md` 规划了“pkg_list 推到 source-service 后自动升级”，但 node_daemon 里没找到自升级系统二进制的实现（只有 `zone_boot_resolve.rs:304` 的 `check_zone_smooth_upgrade`，管的是 Zone 配置）。将来做 OTA 时，镜像必须作为 pkg 一起下发，记在 §7。
- 容器运行时的安装：Docker / OrbStack / WSLC 仍然由用户自己装，安装器只做检测（`prd_buckyos_packaging.md` §8）。
- 私有 registry 或镜像站：既然不再 pull，就不需要了。

## 2. 现状（2026-10-19 核对）

### 2.1 node_daemon 怎么选、怎么拉 worker 和 exttool 镜像

代码在 `src/kernel/node_daemon/src/app_loader.rs`：

- 常量：`DEFAULT_WORKER_IMAGE_REPO = "paios/aios"`（`:32`）、`DEFAULT_EXTTOOL_IMAGE_REPO = "paios/exttool"`（`:73`）、`DEFAULT_EXTTOOL_VOLUME_NAME = "buckyos-exttool"`（`:65`）、`devenv.json` 的 `aios` / `exttool` 键（`:66-68`）。`:31` 的注释指向 `notepads/paios容器需求.md`，这个路径不存在，实际文件在 `doc/app_service/paios容器需求.md`。
- **镜像引用固定是 `<repo>:latest-<arch>`**：`worker_image_repo()`（`:2075`）的取值顺序是测试用 override → `$BUCKYOS_ROOT/etc/devenv.json` 的 `aios` → 默认值；`worker_image_name()`（`:2083`）在后面拼 `latest-amd64` 或 `latest-aarch64`。exttool 在 `:2119-2130`，规则相同但没有 override。`devenv.json` 每次调用都重新读（`devenv_json_path` `:3643`，`parse_image_repo_from_devenv` `:3626`）。代码里没有 digest、版本 tag，也没有对应的环境变量。
- `prepare_worker_image()`（`:2091`）：只在 `docker images -q <tag>` 为空时才 `pull_docker_image`（`:1659`），pull 失败则 deploy 失败。**tag 已经存在就不再更新**，所以本机的 `latest-*` 可能是任意旧版本。
- `prepare_exttool_volume()`（`:2201`，带进程内锁 `exttool_prepare_lock` `:81`）→ `prepare_exttool_volume_serialised()`（`:2206`）：
  - 镜像缺失就 pull；**pull 失败不算错**，打日志后跳过，worker 不挂 ExtTool 照样启动。
  - 卷不存在时 `docker volume create buckyos-exttool`，再用 `seed_exttool_volume`（`:2154`，即 `docker run --rm -v buckyos-exttool:/opt/buckyos/tools <img> true`）播种。**卷存在后再也不会重新播种**，换了镜像也一样（注释 `:2170-2187` 和 `AGENTS.md:70` 都说要手工 `docker volume rm`）。
  - 卷已经存在时仍然要求镜像在本地，镜像缺了会再 pull 一次（`:2211-2233`），所以播种后也删不掉这个 3.67G 的镜像。
- `run_worker_container()`（`:1223`）：卷存在才挂 `buckyos-exttool:/opt/buckyos/tools:ro`（`:1284-1298`），镜像参数是 `worker_image_name()`（`:1348`）。
- **运行中的 worker 容器不和镜像比对**：`observe_deployment()`（`:469`）给 HostScript 和 Agent 用的 `SubPkgDesc` 没有 `docker_image_digest`；`docker_runtime_matches_target()`（`:3149`）在没有期望 digest 时直接返回 `true`。镜像换了以后，老容器仍然被当作“匹配”。目前之所以没出问题，是因为覆盖安装和 `stop.py` 会 `docker rm -f` 掉 `buckyos-app-*` 容器。
- Windows：`wslc.rs` 把所有 `docker` 调用翻译成 `wslc.exe`（`program()` `:4-15`，`command()` `:37-82` 映射 `images -q` 和若干 `--format` 模板）。`doc/windows_wslc.md:38` 指出，WSLC 的镜像、容器、卷都和**调用用户及权限上下文**绑定，“交互式用户拉取的镜像不能假定在 LocalSystem 服务中可见”。
- 相关测试在 `src/kernel/node_daemon/src/test_app_loader.rs`：`:344` / `:359` 是 devenv 覆盖；`:795` 是 Agent deploy 时 pull `paios/aios:latest-amd64` 和 `paios/exttool:latest-amd64`；`:958` 是自定义 repo；`:1118` / `:1149` 是 host-script 预览和 `latest-aarch64`；`:1336` 是并发准备 exttool 时只 pull 和播种一次；`build_agent_loader`（`:264`）强制 `with_worker_image_repo_override("paios/aios")`。

### 2.2 第三方 Docker 应用已经有离线 tar 机制（可复用）

- `prepare_docker_image()`（`app_loader.rs:961-1041`）的顺序：先 `check_docker_image_exists(name, digest)`；不在就装 pkg，再找 tar（`docker_image_tar_candidates_for_arch` `:2909`：`<app_id>.tar` → `<arch>_docker_image.tar` → 另一个架构的 tar），执行 `docker load -i`（`:1507`），或者把 OCI layout 打成 tar 再 load（`load_docker_image_from_layout` `:1523`）；最后才 `pull_docker_image(name, digest)` 兜底。
- `check_docker_image_exists(name, digest)`（`:1694`）：有 digest 时，`RepoDigests` 里包含 `@digest`，或者 `.Id` 的 hash 部分等于 digest，都算命中。
- 安装时 `frame/control_panel/src/app_install_planner.rs:219-245` 要求 Docker 包必须有 `pkg_objid` 和合法的 sha256 `docker_image_digest`；`app_installer.rs:1651` 把镜像 tar 以 `<app_id>.tar` 打进 PIKG。

结论：“tar 跟着包走、按 digest 校验、加载后打 tag”这套逻辑已经有了，worker 和 exttool 只是没用上。

### 2.3 镜像构建 CI（buckyos 仓库）

- `build-all.yml` 只在 push 到 `main` 和 `workflow_dispatch` 时触发（`:3-12`）。
  - `detect-image-changes`（`:15-97`）用 `git diff $BEFORE $AFTER -- <paths>` 判断：**比较基准是上一次 push，不是上一次成功发布的镜像**。上一次镜像构建失败的话，下一次没有改 aios 路径的 push 就会跳过构建，镜像一直落后于代码。手动触发或找不到 `before` 时强制构建。
  - `build-aios`（`:98-104`）和 `build-exttool`（`:105-111`）按检测结果条件执行。平台构建 `build-windows`（`:112-120`）、`build-linux`（`:121-129`）、`build-macos`（`:130-138`）**不依赖镜像构建**。
  - `upload-payloads`（`:140-168`）只在 main 且仓库为 `buckyos/buckyos` 时运行，把 `buckyos-*` artifact 上传到 R2 桶 `buckyos-installer` 的 `buckyos/` 下。
- `build-aios.yml`：
  - amd64 在 `ubuntu-24.04` 上构建并推送（`:110-126`），tag 为 `:<version>-amd64` 和 `:latest-amd64`。aarch64 在 `ubuntu-24.04-arm` 上原生构建（`:127-169`）。
  - `publish-manifests` 用 `imagetools create` 生成 `:<version>` 和 `:latest` 多架构 manifest（`:170-193`）。concurrency 组 `publish-aios`，不取消进行中的任务（`:14-16`）。
  - job 级 outputs 有 `version`、`revision`、`created`、`aarch64_sha256`、`amd64_digest`（`:22-27`），但 **`on.workflow_call` 没有声明 outputs**，`build-all` 拿不到。
- `build-exttool.yml`（共 84 行）：一个 job 里用 QEMU 构建两个架构（`:45-72`），manifest 在 `:73-84`，没有任何 outputs。
- **版本 tag 会被覆盖**：`VERSION_TAG` 取自 `src/VERSION`（`build_aios:338`、`build_exttool:117`），目前是 `0.7.0`。版本号不变时，每次构建都会覆盖 `paios/aios:0.7.0-amd64`。镜像上没有任何不可变 tag。
- 标签：aios 有 `org.opencontainers.image.{title,description,vendor,version,revision,created}`（`build_aios:640-647`）。**exttool 只有 title、description、vendor**（`publish/exttool/Dockerfile:6-8`），本机看到的 `version="24.04"` 是从 ubuntu 基础镜像继承来的。
- CI 实际构建的 Dockerfile 由 `build_aios` 的 `render_dockerfile()`（`:519-652`）生成。`publish/aios/Dockerfile` 没有被任何脚本引用（`build_aios` 只用了 `publish/aios/` 下的 `entrypoint.sh`、`aicc-tool`、`buckyos`，见 `:30-33`），内容已经和 render 结果不一致（缺 `COPY web`）。
- `build_aios` 还支持本地 `--push` 直接推送 `latest-*` 和版本 tag（`doc/CI/BuckyOS的CI发布流程.md:57-58`），所以 **Docker Hub 上的 `latest-*` 不一定是 CI 构建的**。

### 2.4 安装包流水线（Actions 编译 → R2 → 打包机 / 在线安装）

编译已经全部在 GitHub Actions 上完成。**但 Actions 不产出安装包**：run 39361968276（`f317150`，push）只有 `detect-image-changes`、5 个平台构建和“Upload component payloads to R2”这几个 job；artifact 只有 5 个 payload，`buckyos-{windows-amd64,apple-amd64,apple-aarch64,linux-amd64,linux-aarch64}-0.7.0+build261020`，大小 166–217 MB。payload 变成用户能装的东西，现在有两条路径：

| 路径 | 入口 | 是否用 PVE 打包机 | 产物 |
| --- | --- | --- | --- |
| 传统安装包（artifact pack） | pve-pack-system `bin/artifact_pack_orchestrator.py`：等待 buckyos、cyfs-gateway、BuckyOSApp 三个仓库最新的 `build-all.yml` run 成功 → `update_apps_version.ts` → `fetch-versions` → 回退 builder 快照 → `depoly --cluster ci --apply` 放置 Artifact → sn-business `pack --cluster ci` → 上传 S3/R2（`doc/artifact_pack_entry.md`） | **用**。Linux、Windows、macOS 三台 builder 只做组装、打包和签名，**不编译** | 12 个 deb/rpm/pkg/exe（含签名变体） |
| Linux 在线安装（onlinepack，2026-10-14 新增） | pve-pack-system `bin/onlinepack.py` / `check_and_publish_onlinepack.py` 校验 Actions payload，然后把产品 manifest `buckyos/<version>/manifest.json`、`version.tsv` 和 `install.sh`、`linux.sh` 发布到 R2；用户执行 `curl … \| bash`（`doc/onlinepack.md`） | **不用** | 不生成安装包；`linux.sh` 从 `https://buckyos-installer.tbudr.top` 下载 payload，按 `.install-*.json` 的 `modules` / `data_paths` 安装 |

旧逻辑是 pve-pack-system 的 `bin/pve_orchestrator.py`：在 VM 里拉源码、编译，再调用 `make_local_pkg.py`。它正在被 artifact pack 取代（`doc/artifact_pack_entry.md` 开头的原话是“逐步替代”）。也就是说，旧逻辑和新逻辑都要用打包机，区别只是新逻辑在打包机上不再编译。现在只有 Linux 在线安装完全不经过打包机。

- **buckyos 仓库在 push 时不构建安装包**，只产出平台 payload。以 `build-linux.yml` 为例：`buckyos-install --app=buckyos --all --target-rootfs=$STAGE_ROOT/buckyos`（`:105-111`）→ 用 `node_daemon --version` 取版本（`:115-125`）→ `write-install-descriptor.py` 写 `.install-buckyos.json`（`:128-135`）→ `tar -czf`（`:138-140`）→ `upload-artifact`（`:142`）。macOS 对应 `build-macos.yml:67-104`，Windows 对应 `build-windows.yml:111-125`。产物名为 `buckyos-{linux|apple|windows}-<arch>-<version>.tar.gz`。
- 传统安装包在打包机上由 **sn-business 的 `pack`** 生成（`CD/cluster_config/ci/commands/pack/README.md`），调度方是 pve-pack-system 的 artifact pack：
  - `artifact_sources.json` 选择 `build-all.yml` 在 main 上 push 产生的 artifact；`apps_version.json` 固定版本（目前是 `0.7.0+build260904`）。
  - `update_apps_version.ts:107-139` 要求**同一 repo/workflow 的组件必须来自同一次 workflow run、同一个版本**。
  - 打包机把 artifact 部署到 `/opt/<artifact>/latest`。`post_pve-linux-builder.sh` 的 `assemble_stage` 把 buckyos payload 和 cyfs-gateway payload 拼成 `$WORK/staged-<arch>/buckyos`，然后调用 `src/publish/make_local_deb.py`、`make_local_rpm.py` 的 `build-pkg` 和 `verify-pkg`。macOS 和 Windows 由 `post_pve-macos-builder.sh`、`post_pve-windows-builder.ps1` 处理（含签名变体）。结果由 `create_package_result.py` 打成 `package-result.zip`。
  - `package-kit/buckyos/src/bucky_project.json` 是 `update_apps_version.ts` 在 artifact 对应 commit 上从 `src/bucky_project.yaml` 自动生成的，不要手工维护。
- **安装包只收录声明过的路径**：`impl_buckyos_packaging.md` §5 规定未声明的文件不进安装包；`write-install-descriptor.py` 也会校验声明的路径在 staging 中存在。`modules` 走覆盖安装，`data_paths` 走“已存在则保留”。
- **devkit 安装模块目录的方式是先 `rmtree` 再 `copytree`**（`buckyos-devkit/src/install.py:292-300`）。源路径不存在时 `copytree` 或 `shutil.copy` 会直接失败，所以**声明了的模块在 `src/rootfs` 里必须存在**。已有先例：`src/rootfs/bin/buckyos_filebrowser-img/readme.txt`。
- `bin/` 是根 package env（`node_daemon/src/service_pkg.rs:35`，`new_package_env(get_buckyos_system_bin_dir())`）。服务按名字从这里加载。
- 现有的安装脚本都不 pull、也不 load 镜像：deb 的 `Depends` 里有 `docker.io | docker-ce | moby-engine`；macOS preinstall 检测 `docker info`；Windows 检测 docker（`impl_buckyos_packaging.md` §9.1）；覆盖安装前 `stop.py` 会删除 `buckyos-app-*` 容器（`impl_buckyos_packaging.md` §8.3）。
- sn-business 的 tar 解包 transform `transforms/extract-single-tar-gz.ts` 有 8 GiB 的解包上限（`MAX_EXTRACTED_BYTES`）。
- **Linux 在线安装**（pve-pack-system `installer/linux.sh`，298 行）：
  - 装好依赖（缺 docker 时 apt 装 `docker.io`，yum 依次尝试 `moby-engine`、`docker-ce`、`docker-engine`）。
  - 下载 `buckyos/<version>/manifest.json`，按其中的 `size` 下载并校验 buckyos 和 cyfs-gateway 两个 payload。
  - 按 payload 内 `.install-<component>.json` 里声明的 `modules` 安装，`data_paths` 放进 `.buckyos_installer_defaults`。
  - 然后执行 `systemctl enable --now docker.service`，停掉旧服务并跑 `stop.py`，删除 `bin/` 后复制新文件。
  - **全程不处理任何镜像**。现在装完后依赖 node_daemon 首次 deploy 时从 docker.io pull，issue 里说的限流问题正出现在这条路径上。
  - R2 的公开域名是 `https://buckyos-installer.tbudr.top`（`config/onlinepack.toml` 的 `base_url`）。

### 2.5 开发流程

- 仓库根目录的 `build.py`：`default_docker_images()`（`:115-120`）预拉取的是 **`paios/aios:latest`（多架构 tag）**，而 node_daemon 找的是 `latest-<arch>`，所以这次预拉取对 worker 没用；exttool 不预拉取。`ensure_docker_images()`（`:262-303`）支持 `--refresh-images` 和 `--skip-docker`。
- `./build_aios --local-test` 会写 `$BUCKYOS_ROOT/etc/devenv.json` 的 `aios` 键（`AGENTS.md:62-70`）；`start.py --all` 会保留这个文件（`_preserve_devenv_config`）。本机当前内容是 `{"aios": "local/aios-test"}`。
- `src/diagnose_jarvis_orbstack.sh:419-420` 写死了 `latest-*`。
- 版本：`node_daemon --version` 的输出是 `0.7.0+build<YYMMDD>.<branch>.<commit12> (nightly)`（`buckyos-base/src/buckyos-kit/src/channel.rs:63-71`）。CI 从来没有设置 `CHANNEL`，所以所有 CI 构建都是 nightly。release 和 nightly 的配置差别只在 `make_config.ts` 的 `release` / `nightly` 组（`:1124-1146`，域名分别是 `buckyos.ai` / `buckyos.io`）。
- 本机 Docker 是 26.1.5，overlay2，经典镜像存储（不是 containerd image store）。

### 2.6 镜像体积（实测）

| 镜像 | 解压后 | 压缩后（registry 层合计） | `docker save` tar |
| --- | --- | --- | --- |
| `paios/aios:latest-amd64`（rev `835b5bd`） | 1,384.6 MB | 467.8 MB（aarch64 为 457.1 MB） | 1,421 MB；gzip -6 为 458 MB；zstd -3 为 429 MB；zstd -19 --long 为 326 MB |
| `paios/exttool:latest-amd64` | 3,673 MB（本机旧版，2026-04 构建） | **1,122 MB**（远端 2026-10 构建）；aarch64 为 132 MB（FreeCAD 只是占位） | 约等于压缩后大小 |

“2G”的来源：**线上发布的 aios 镜像没有 2G**。可能来自以下几处：

1. demo 热修镜像 `demo-buckyos/aios-20261016-*` 有 1.94 GB。它是在旧 demo 镜像上叠加的，每轮都 `COPY` 一份二进制再 `chmod`，每次大约多出 77 MB。
2. 本地构建（`--local-test` / `--load`，buildx 用 docker driver）时，`RUN chmod 755`（`build_aios:615`）会把 82.6 MB 的二进制完整复制一份，所以 `local/aios-test` 有 1.47 GB。
3. Docker Desktop 在 containerd image store 下显示的大小是解压加压缩合计，约 1.85 GB。（推断，未核实。）
4. 下载量：在 amd64 上首次跑 Agent 要拉 aios 468 MB 加 exttool 1,122 MB，**合计约 1.6 GB**，这一点最接近大家说的“2G”。

aios 各层：apt 层 976 MB / 306 MB（`render_dockerfile` `:538-547`）占了约 70%。按包拆开看：

- ffmpeg 及其依赖 375 MB。其中 **libllvm20 143.5 MB 加 mesa-libgallium 43.4 MB**，是经 libavdevice → libsdl2 → libgbm/libgl1 拉进来的 GPU 栈。用 ldd 核对过，ffmpeg 并不链接它们。
- pandoc 196 MB。`src/` 里没有任何调用，只出现在容器需求文档里。
- Debian 的 npm 107 MB，连带 354 个包（libssl-dev、eslint、webpack 等），而且 Node 18 已经 EOL。
- git 带进来的 perl 73 MB；tesseract 22 MB；locales 17 MB。

其他层：deno 106 MB；`uv python install 3.12` 98.5 MB（Ubuntu 24.04 自带的 python3 已经是 3.12）；uv 38 MB；三个 Rust 二进制合计约 82 MB；`:572` 的 deno check 留下了 4 MB 的 `/root/.cache/deno`。没有发现残留的缓存或构建产物。另外 `build_aios --runtime-only` 目前实际上不起作用（`WORKSPACE_PACKAGES` 为空，只影响 `:1408` 的一行日志）。

**exttool 才是真正的大头**：解压后的 FreeCAD 1.0.1 conda AppImage 有 3.21 GB / 982 MB，里面有 LLVM 19 和 20、libclang、FreeCAD 本身用不到的 pythonocc，以及 VTK、PCL、OpenVINO、OpenCV 等。`src/` 里没有任何地方使用 FreeCAD。一个 amd64 节点上目前要占约 6.9 GB（镜像 3.67 GB 加卷 3.25 GB）。镜像里还有约 390 MB 的内容（uv 的 Python、uv/deno 二进制、ubuntu 基础层）根本不会进到卷里。

### 2.7 结论

| issue / 评论的要求 | 现状 | 本设计 |
| --- | --- | --- |
| 按镜像 id 用配套镜像 | 固定 `latest-<arch>`，只要 tag 存在就用 | 清单锁定，用不可变 tag 加身份校验（§3.2–§3.3） |
| 没有正式配置时用本机镜像 | 用 devenv 或默认 repo 的 `latest-<arch>` | 保留，作为本机模式（§3.1） |
| 不主动 pull，缺了就报错 | worker 缺了就 pull；exttool pull 失败则降级 | 两种模式都删掉 pull，锁定模式从包内 tar 加载（§3.4） |
| 等镜像构建完再生成清单；不构建就继承 | 平台构建和镜像构建互不依赖；基准是上次 push | 用 R2 指针记录上次成功的镜像；检测基于指针 revision；平台构建依赖“清单解析” job（§3.5） |
| all-in-one 安装包 | 安装包里没有镜像；Linux 在线安装也不处理镜像 | 清单和目标架构的 tar 进 `bin/paios-images/`。传统安装包在打包时放进去；在线安装在安装时从 R2 下载（§3.6） |
| 为什么 2G | — | §2.6；瘦身见 §3.8 |
| （附带问题）老容器不重建、exttool 卷不刷新 | 已确认存在 | 容器带镜像 id 标签比对；卷按镜像 id 命名（§3.4） |

## 3. 设计

### 3.1 两种模式

| | 锁定模式（pinned） | 本机模式（local） |
| --- | --- | --- |
| 何时进入 | `$BUCKYOS_ROOT/bin/paios-images/paios_images.json` 存在 | 该文件不存在 |
| 谁会处于这个模式 | artifact pack 产出的所有安装包，以及 Linux 在线安装，nightly 和将来的 release 都算（D2） | git clone 后用 `start.py`、`buckyos-install` 装的开发环境 |
| aios / exttool 引用 | 清单中当前架构的 `ref`（不可变 tag），并校验身份 | `<repo>:latest-<arch>`，repo 来自 `etc/devenv.json` 或默认值（和现在一样） |
| 本地缺镜像或身份不符 | 从 `bin/paios-images/` 下的 tar 校验 sha256，然后 `docker load`，再校验身份；仍不符则报错 | 直接报错，错误信息给出 `docker pull <ref>`、`uv run build.py`、`./build_aios --local-test` 三种处理方式 |
| pull | **永不** | **永不** |
| 清单存在但无法解析，或缺少当前架构 | 报错，**不退回本机模式** | — |
| `etc/devenv.json` 覆盖 | 见 D7（建议：覆盖优先，打 warn 日志） | 和现在一样 |

为什么用“清单是否存在”来切换，而不是看 channel 或编译开关：CI 从来不设 `CHANNEL`（§2.5），nightly 和 release 都要锁定镜像；开发环境安装时 devkit 会先 `rmtree` 再 `copytree` 这个目录（§2.4），只要仓库里不放清单，开发安装就自然回到本机模式。即使有人在装过安装包的机器上跑了 `start.py --all`，也会自动变成本机模式，这正是开发场景需要的行为。

### 3.2 清单文件 `paios_images.json`

**位置（D1）**：`bin/paios-images/`。

- 仓库里新增 `src/rootfs/bin/paios-images/README.md`，说明这个目录在正式包里放清单和 tar，开发环境里为空。
- `src/bucky_project.yaml` 的 `apps.buckyos.modules` 加一项 `paios_images: bin/paios-images/`，走覆盖安装，跟系统版本一起替换。deb `preinst` 删除 `/opt/buckyos/bin/` 时，旧版本的 tar 会被一起清掉。
- 没有放在 `etc/` 的原因：`etc/` 下要做成模块就得是一个具体文件，而开发 rootfs 里没有这个文件，devkit 的 `shutil.copy` 会失败；另外 `etc/devenv.json` 是个人配置，两者放在一起容易混淆。

**格式（v1）**：所有平台的 payload 里放同一份文件，两个架构都列出；安装包只带目标架构的 tar。

```json
{
  "schema": "buckyos.paios-images/v1",
  "buckyos_revision": "<40 位 hex：本次 buckyos 构建的 commit>",
  "generated_at": "2026-10-19T12:00:00Z",
  "images": {
    "aios": {
      "repo": "paios/aios",
      "version": "0.7.0",
      "revision": "<40 位 hex：镜像构建时的 buckyos commit，继承时可能早于 buckyos_revision>",
      "created": "2026-10-19T11:00:00Z",
      "platforms": {
        "amd64": {
          "ref": "paios/aios:0.7.0-<rev12>-amd64",
          "image_id": "sha256:<config digest>",
          "manifest_digest": "sha256:<registry 上该架构 image manifest 的 digest>",
          "rootfs_layers_sha256": "sha256:<对 RootFS.Layers 用 \\n 连接后的 sha256>",
          "tar": {
            "file": "aios-amd64-<image_id 前 12 位>.tar.gz",
            "sha256": "<tar 文件 sha256>",
            "size": 480000000,
            "path": "paios-images/aios/amd64/<tar sha256>.tar.gz",
            "url": "https://buckyos-installer.tbudr.top/paios-images/aios/amd64/<tar sha256>.tar.gz"
          }
        },
        "aarch64": { "...": "同上" }
      }
    },
    "exttool": { "...": "结构同 aios" }
  }
}
```

规则：

- `schema` 必须完全相等，未知字段忽略（向前兼容）。`platforms` 的键只能是 `amd64` 或 `aarch64`，和 `PlatformArch` 一一对应：macOS arm64 用 `aarch64`，Windows 只有 `amd64`。
- `tar.file` 只能是文件名：不能含 `/`、`\`、`..`，按清单所在目录解析。`tar.path` 是相对安装源根（`https://buckyos-installer.tbudr.top`）的对象路径，必须以 `paios-images/` 开头，Linux 在线安装用它拼自己的 base URL（支持 `BUCKYOS_DOWNLOAD_BASE_URL` 换域名）；`tar.url` 是 `path` 的完整地址，给打包流程用。运行时永远不下载。
- R2 对象按 **tar 的 sha256** 寻址，不按 image id：M0 实测 `docker save` 不是字节稳定的（§9.1），按 image id 寻址时“已存在就跳过”会让清单里的 sha256 和 R2 上的文件对不上。
- `images` 至少要有 `aios`。exttool 缺失按 D6 处理。
- Rust 和 Python 共用一份 fixture：`src/kernel/node_daemon/src/testdata/paios_images.v1.json`。

### 3.3 镜像身份与校验

运行时用 `ref` 作为 `docker run` 的参数，便于人看 `docker images` 时认出来。但只认 tag 不够，因为 tag 可能被人改指向。所以每次 deploy 前都要做 `verify_pinned_image(ref, expected)`：

1. `docker image inspect <ref>`，取 `.Id`、`.RepoDigests`、`.RootFS.Layers`、`.Config.Labels`。
2. 满足以下任一条件即匹配：
   - `.Id` 等于 `image_id` 或 `manifest_digest`；
   - `RepoDigests` 中包含 `@<manifest_digest>`；
   - `rootfs_layers_sha256` 相等，并且 label `org.opencontainers.image.revision` 等于清单中的 `revision`。

第三条兜底的原因：containerd image store（新装 Docker Engine 29 或 Docker Desktop 可能默认启用）和 WSLC 在 `docker load` 之后报告的 `.Id`，可能既不是 config digest，也不是 registry 上的 manifest digest。M0 会实测这几种运行时；如果都能稳定拿到 config digest，可以不实现第三条，但 CI 仍然要把 `rootfs_layers_sha256` 写进清单，以后要用时不必再改 CI。

本机模式不做身份校验，只看 tag 是否存在，然后记下实际的 `.Id`，供容器比对用（§3.4）。

### 3.4 node_daemon 行为

新增 `src/kernel/node_daemon/src/paios_images.rs`，app_loader 只调用它的接口：

- `load_manifest(root: &Path) -> Result<Option<PaiosImagesManifest>>`：文件不存在返回 `Ok(None)`，解析失败返回 `Err`。写法参照 `sn_zone_info.rs:11-25`（`NotFound` 映射为 `None`）。和 `devenv.json` 一样每次调用都读，文件很小。
- `resolve(kind: Aios | Exttool, arch, root) -> Result<ResolvedImage>`，其中 `ResolvedImage { reference, pinned: Option<PinnedPlatform>, tar_path: Option<PathBuf> }`。优先级：`devenv.json` 覆盖（按 D7）> 清单 > 默认值。测试用的 `worker_image_repo_override` 仍然最优先。
- `ensure_present(&ResolvedImage) -> Result<String /* 实际 image id */>`：
  - 锁定模式：先校验，匹配就返回；不匹配或缺失时，流式计算 tar 的 sha256 并和清单比对，然后 `docker load -i <tar>`，再校验一次；仍然失败就报 `ExecuteError`。tar 缺失时的错误信息是“安装不完整：缺少 `bin/paios-images/<file>`，请重新安装同版本的 BuckyOS”。
  - 本机模式：tag 存在就返回 `.Id`，不存在就报错并给出提示。
  - **两种模式都不调用 `pull_docker_image`。**
  - 每个 `reference` 一把进程内互斥锁，参照 `exttool_prepare_lock`，避免并发 deploy 重复 load。

app_loader 的改动：

1. `worker_image_name()` / `exttool_image_name()` 改为返回 `resolve(...).reference`。`prepare_worker_image()` 改为调用 `ensure_present`，删掉 pull。
2. exttool：
   - 卷名改为 `buckyos-exttool-<image id 前 12 位>`，两种模式都这样命名。锁定模式下 id 从清单取，不需要镜像在本地；本机模式从 `ensure_present` 返回的 id 取。
   - 卷已存在并且有播种完成标记时，**直接跳过，不要求镜像在本地**。
   - 播种：`docker volume create` 之后执行 `docker run --rm -v <vol>:/opt/buckyos/tools --entrypoint /bin/sh <exttool> -c 'touch /opt/buckyos/tools/.buckyos-seeded-<id12>'`。Docker 在容器启动前就把镜像内容复制到空卷里，所以命令跑完、标记文件存在，就说明复制已经完成。
   - 检查标记：用 aios 镜像以只读方式挂载卷，执行 `docker run --rm -v <vol>:/t:ro --entrypoint /bin/sh <aios> -c 'test -f /t/.buckyos-seeded-<id12>'`。
   - 卷存在但没有标记（上次播种中断）时，删掉卷重新播种。
   - 锁定模式下播种成功后是否删除 exttool 镜像，见 D8。
   - 拿不到 exttool（缺 tar 或校验失败）时按 D6 处理。
3. `run_worker_container()`：挂载上面得到的卷名；新增 label `buckyos.exttool_volume=<卷名>`；`buckyos.image_digest` 写入 `ensure_present` 返回的实际 worker image id（`docker_runtime_labels` `:2329` 目前只在 desc 带 digest 时才写）。
4. `observe_deployment()` 处理 HostScript 和 Agent 时：
   - 复制一份 desc，把 `docker_image_digest` 设成当前 worker 镜像的 `.Id`。这一步只 inspect，不 load：镜像不存在时按现有逻辑返回 `NotExist`。这样就复用了 `docker_runtime_matches_target()`（`:3149`）现有的“label / RepoDigests / 镜像 id”三重比对。
   - 同时比较 `buckyos.exttool_volume` 标签是否等于当前期望的卷名，不相等也算不匹配。
   - 不匹配时走 Docker 应用 digest 不符的现有处理路径。M0 要确认 reconcile 在这种情况下会停掉老容器再按新镜像重建。
5. `preview_host_script_deploy()`（`:2634`）和 `preview_agent_deploy()`（`:2662`）：去掉 `docker pull`。锁定模式下改为输出 `docker load -i <tar>`（如果需要的话）、`docker volume create <vol>` 和播种命令。
6. 删掉 worker 和 exttool 路径上所有 `pull_docker_image` 调用。Docker 应用的 `prepare_docker_image` 不动。
7. 修正 `:31` 注释中的文档路径。

**启动预加载**：

- 锁定模式下，node_daemon 启动后用 `tokio::spawn` 在后台执行 `ensure_present(aios)`、`ensure_present(exttool)` 和播种，不阻塞启动。可以放在 `node_daemon.rs` 里内核服务和应用开始调度之前。
- 和 deploy 共用同一组锁。
- Docker 不可用时按指数退避重试（上限 5 分钟），日志里打出耗时。
- 目的：首次加载 1.4 GB 的 tar、播种几个 GB 的卷要花几分钟，不应该卡在用户第一次打开 Agent 的时候。

**WSLC**：

- 由 node_daemon 自己执行 load，镜像就落在 node_daemon 进程的用户上下文里，避开了 §2.1 提到的上下文问题。这也是 D3 建议“安装器不 load”的主要原因。
- 如果用到新的 `--format` 模板（`{{json .RootFS.Layers}}`、`{{json .Config.Labels}}`），要在 `wslc.rs::command()` 和 `project()` 里补映射，并在 `test_wslc.rs` 里加用例。

**旧镜像和旧卷的回收**（D9，放在 M6）：只在锁定模式下做。node_daemon 记录自己 load 过的 image id（`$BUCKYOS_ROOT/local/node_daemon/paios_images_state.json`）。启动预加载完成后，删除状态里不再被清单引用、也没有容器在用的镜像，以及所有不是当前卷名、也没有容器在用的 `buckyos-exttool*` 卷（包括旧的 `buckyos-exttool`）。

### 3.5 CI：构建、导出、继承、生成清单（buckyos 仓库）

1. **不可变 tag 和完整标签**（`build_aios`、`build_exttool`、两个 workflow）：
   - 在现有 tag 之外，再推一个 `paios/<name>:<VERSION>-<rev12>-<arch>`，`rev12` 是构建 commit 的前 12 位。CI 里工作区不会是 dirty，`build_aios` 遇到 dirty 时不生成这个 tag。
   - `latest-*` 和 `<VERSION>-*` 继续推，供开发环境使用。
   - exttool 的 Dockerfile 加上 `IMAGE_VERSION`、`SOURCE_REVISION`、`BUILD_CREATED` 三个 build arg 和对应的 OCI label（照抄 `build_aios:640-647`）；`metadata.env` 加上 `revision` 和 `created`。
2. **build-push-action 加 `provenance: false` 和 `sbom: false`**，让返回的 `digest` 是单架构 image manifest，而不是带 attestation 的 index。M0 要确认现在返回的到底是哪一种。
3. **导出 tar 并上传 R2**：`build-aios.yml` 和 `build-exttool.yml` 各加一个 `export` job，依赖构建 job，在 `ubuntu-24.04` 上运行。对每个架构：
   1. `docker pull --platform linux/<arch> paios/<name>@<digest>`，再用 `docker image inspect` 取 `image_id` 和 `RootFS.Layers`。
   2. `docker tag <image_id> <ref>`，然后 `docker save <ref> | gzip -n > <file>`，记录 sha256 和 size。gzip 是兼容性底线；改用 zstd 的前提是 M0 确认各运行时（含 WSLC）的 `docker load` 都支持。
   3. 上传到 R2 的 `buckyos-installer/paios-images/<name>/<arch>/<tar sha256>.tar.gz`，按内容寻址，已存在就跳过。
   4. 生成片段 `paios-image-<name>.json`（结构就是清单里 `images.<name>` 那一段），作为 Actions artifact 上传。
   5. 两个架构都成功、并且 `github.ref == refs/heads/main && github.repository == buckyos/buckyos` 时，才把片段写到指针 `paios-images/main/<name>.json`。
   - 这两个 reusable workflow 要新增 secrets `R2_ACCESS_KEY_ID` 和 `R2_SECRET_ACCESS_KEY`，`build-all.yml` 负责透传。
4. **变化检测改为和指针比较**（`build-all.yml` 的 `detect-image-changes`）：
   1. 先读指针 `paios-images/main/<name>.json`，得到上次成功发布的 `revision`。
   2. 指针不存在，或者 `git merge-base --is-ancestor <revision> HEAD` 不成立，就判定 `changed=true`。
   3. 否则用 `git diff --quiet <revision> HEAD -- <原有路径列表>` 判断。
   4. `workflow_dispatch` 仍然强制构建。
   5. 指针内容作为 artifact `paios-image-pointer-<name>` 存下来。后面的 job 用这份快照，不再重新读 R2，以免并发 run 期间指针被改，造成检测和继承用的不是同一个版本。
   - 这样就修好了 §2.3 的问题：上次失败后没有重新构建、镜像落后于代码。
5. **新 job `resolve-paios-images`**：
   - `needs: [detect-image-changes, build-aios, build-exttool]`，条件是 `if: ${{ !cancelled() && !contains(needs.*.result, 'failure') }}`，因为镜像构建被跳过也是正常情况。
   - 对每个镜像：本次构建了就用本次 run 的片段 artifact，否则用指针快照（继承）。
   - 组装 `paios_images.json`，写入 `buckyos_revision = github.sha`。
   - 校验：两个架构都在、字段齐全、对每个 `tar.url` 发 HTTP HEAD 返回 200。
   - 上传 artifact `paios-images-manifest`。注意名字**不能以 `buckyos-` 开头**，否则会被 `upload-payloads` 的匹配模式选中。
   - 不是 main 分支、不是 `buckyos/buckyos` 仓库（fork）时跳过，payload 不带清单，也就是本机模式。
6. **平台 payload 写入清单**：
   - `build-linux`、`build-macos`、`build-windows` 在 `build-all.yml` 里加 `needs: resolve-paios-images`。
   - 三个 workflow 在 `buckyos-install` 之后、`write-install-descriptor.py` 之前，下载 `paios-images-manifest`，复制到 `$STAGE_ROOT/buckyos/bin/paios-images/paios_images.json`。
   - artifact 不存在（fork 的情况）就跳过。
   - 代价：aios 有改动的那次 run，平台构建要等镜像构建完，墙钟时间会变长。没有改动的 run 只多一个几秒钟的 resolve job。这是保证“payload 和镜像严格配套”的必要代价，不要为了提速再把它们拆开。
7. **脚本**：新增 `.github/scripts/paios_images.py`，提供三个子命令：`fragment`（由 inspect 结果、tar 信息、metadata.env 生成片段）、`assemble`（由片段和指针生成清单）、`validate`。单测放在 `.github/scripts/test_paios_images.py`，参照同目录的 `test_install_descriptor.py`，并使用 §3.2 的 fixture。

### 3.6 打包与在线安装（sn-business + buckyos-pack-installer + pve-pack-system）

**sn-business**：

- 新增 `CD/cluster_config/ci/commands/pack/package-kit/stage_paios_images.py`（python3，三台打包机上都有）：
  - 参数：`--stage <stage>/buckyos --arch <amd64|aarch64> --cache <dir> [--allow-unpinned]`。
  - 读取 `<stage>/bin/paios-images/paios_images.json`。清单缺失时报错退出，除非传了 `--allow-unpinned`。CI 打出来的包必须是锁定的。
  - 对每个镜像的目标架构：如果缓存 `<cache>/<sha256>.tar.gz` 不存在，就从 `tar.url` 下载（带重试和断点续传）；校验 sha256 和 size；复制到 `<stage>/bin/paios-images/<file>`。
  - 缓存目录：Linux 和 macOS 用 `/opt/buckyosci/paios-image-cache`，Windows 用 `C:\opt\buckyosci\paios-image-cache`。每个镜像每个架构保留最近 4 个版本。
- 三个打包脚本在 `assemble_stage` 之后调用它，架构映射如下：
  - `post_pve-linux-builder.sh`：amd64 → `amd64`，aarch64 → `aarch64`。
  - `post_pve-macos-builder.sh`：amd64 → `amd64`，arm64 → `aarch64`。
  - `post_pve-windows-builder.ps1`：amd64 → `amd64`。
- `create_package_result.py` 在结果中加入 `paios_images` 字段（每个镜像的 `ref` 和 `image_id`），方便追溯某个安装包带的是哪个镜像。
- `bucky_project.json` 由 `update_apps_version.ts` 从 yaml 重新生成，新模块会自动带上，不需要手改。

**buckyos-pack-installer（`src/publish` 子模块）**：

- `verify-pkg`：payload 中有 `bin/paios-images/paios_images.json` 时，检查目标架构每个镜像的 `tar.file` 都存在并且 sha256 一致。新增参数 `--require-paios-images`，sn-business 打包时传入；缺清单或缺 tar 就失败。
- 安装器**不执行** `docker load`（D3）。安装前已有的“停止服务和容器”逻辑不变。
- 安装包体积和耗时（在 M0 实测的基础上处理）：
  - deb、rpm、pkg 对已经 gzip 过的 tar 再做 xz 压缩，没有收益，只会更慢。如果打包或安装耗时明显增加，就为这类大文件调整压缩参数（例如 deb 改用 `-Zzstd`），或者在 tar 里不再压缩、交给包格式统一压缩（代价是安装后磁盘占用变成解压后的大小，不推荐）。
  - **Windows NSIS 的风险**：NSIS 生成的安装包超过 2 GB 时不可用（这是 NSIS 的已知限制，M0 要用一个 2.5 GB 的假 payload 验证）。按 §2.6 的数据，Windows amd64 包要加上约 458 MB（aios）和约 1.1 GB（exttool），再加上 payload 本身，很接近这条线。处理方式见 D10。
- 可选：安装前检查磁盘剩余空间是否不少于“payload + 两倍镜像解压后大小”，不够时按各平台的依赖检查方式提示用户。

**pve-pack-system（Linux 在线安装）**：

- `installer/linux.sh` 解出 buckyos payload 之后、复制到 `$BUCKYOS_ROOT` 之前：
  1. 如果 payload 里有 `bin/paios-images/paios_images.json`，就按本机架构（`x86_64` → `amd64`，`aarch64` → `aarch64`）取出每个镜像的 `tar.url`、`sha256`、`size`。
  2. 用现有的 `download()` 写法（curl 加 https 限定和重试）下载到 `$WORK`，校验 size 和 sha256，然后放进 `package/bin/paios-images/<file>`，跟其他模块一起安装。
  3. 校验失败就中止安装，不留下半装的 `bin/`。
- 这一步是**安装时从 R2 下载**，不是运行时从 docker.io pull，和“BuckyOS 不主动 pull”的原则不冲突，也避开了 docker.io 限流。代价是在线安装要多下载约 1.6 GB（amd64，§2.6；D5 落地后约 0.6 GB），要在安装输出里打印进度和总大小。
- `linux.sh` 只用 `tar.path`（必须以 `paios-images/` 开头）拼当前的下载 base URL，不使用 `tar.url`，所以不会去访问别的域名。CI 上传时 key 就是 `tar.path`（§3.5-3），两边一致。
- `bin/onlinepack.py` / `check_and_publish_onlinepack.py` 发布前，对所选 buckyos payload 里清单列出的每个 tar 做一次带认证的 R2 HEAD，校验 size，都存在才发布 manifest 和 `version.tsv`。这和现在对 payload 的 HEAD 校验是同一种做法。这一步需要先解出 payload 里的清单。如果不想在发布时下载整个 payload（`doc/onlinepack.md` 写明发布流程不下载 payload），可以让 CI 把清单另外作为一个独立的小文件上传到 R2，例如 `buckyos/<payload 名>.paios_images.json`。M4 决定用哪种。
- `--stage` 模式（`STAGE_DIR`）也要走同样的下载和校验，这样它的输出才能和真实安装一致。

### 3.7 开发流程调整（buckyos 仓库）

- `build.py`：`default_docker_images()` 改为 `paios/aios:latest-<arch>` 和 `paios/exttool:latest-<arch>`。如果 `$BUCKYOS_ROOT/etc/devenv.json` 里有对应的覆盖，说明用的是本地构建的镜像，就跳过该镜像的预拉取。filebrowser 的预拉取保持不变。
- `start.py`：启动后做一次预检查。如果处于本机模式，并且 `latest-<arch>`（或 devenv 覆盖的 repo）不存在，就打印醒目的 warning 和 `docker pull ...` 命令。**`start.py` 自己不 pull。**
- `make_local_pkg.py`：按 D2 新增 `--paios-images local`，从本机已有的镜像生成清单和 tar（inspect 加 save），用于开发者打离线试用包。默认不带，也就是本机模式。
- `src/diagnose_jarvis_orbstack.sh`：清单存在时按清单的 `ref` 诊断。
- 文档：
  - `AGENTS.md:62-70`：devenv 语义、不再自动 pull、exttool 卷按 id 命名后不再需要手工删卷。
  - `doc/CI/BuckyOS的CI发布流程.md` 中 AIOS 和 ExtTool 两节：tag、指针、检测基准、导出、清单。
  - `doc/CI/impl_buckyos_packaging.md` §3.3、§5、§15.2：新模块和 `verify-pkg`。
  - `doc/app_service/rust_app_loader.md:287-290, 503-510`：worker 镜像规则。
  - `doc/windows_wslc.md`：镜像改为由 node_daemon 从包内 tar 导入。
- DV 和测试机：升级到新 node_daemon 之后，开发环境要手工 `docker pull paios/aios:latest-<arch> paios/exttool:latest-<arch>` 一次（或者跑 `uv run build.py`）。要在 M2 的 PR 描述里写清楚。

### 3.8 镜像瘦身（独立一条线，M7）

目标：把全量包控制在 Windows NSIS 的限制以内，并减少节点磁盘占用。和锁定改造并行推进，互不阻塞。但 Windows 全量包能不能出，取决于这里的结果（D10）。

aios 按风险从低到高（估算基于 §2.6 的实测）：

| # | 改动 | 节省（解压 / 压缩） |
| --- | --- | --- |
| A1 | apt 安装后删掉 GPU/LLVM 栈（直接 `rm` 掉 libllvm20、mesa-libgallium、libgl1-mesa-dri、libglx-mesa0 的 .so，避免 apt 状态出现未满足依赖），或者改用静态 ffmpeg | 187 / ~55 MB（静态 ffmpeg 估计 215 / ~70 MB） |
| A2 | 删掉 `uv python install 3.12`（`render_dockerfile:554`）和 `python3-venv`；`entrypoint.sh:354` 的 `uv venv` 直接用系统的 3.12 | 101 / ~35 MB |
| A3 | apt 的 `npm` 换成官方 tarball，并顺带离开已经 EOL 的 Node 18 | ~96 / ~14 MB |
| A4 | `locales` + `locale-gen` 换成 `C.UTF-8` | ~20 / ~5 MB |
| A5 | 在 `:572` 的 RUN 里清掉 deno 缓存；把 `:615-616` 的 `RUN chmod` 改成 `COPY --chmod=755`（本地和 demo 构建能省 82.6 MB） | 4 MB，外加本地构建的 82.6 MB |
| A6 | pandoc 移到 ExtTool 或按需安装。这是产品决定，要同步修改 `paios容器需求.md` §6.3，用户同意前不做（2026-10-20 用户同意，移到 ExtTool，§9.8） | 196 / ~38 MB |
| A7 | 三个 Rust 二进制合并成一个，按调用名分派 | ~45 / ~18 MB（工作量中等） |

A1–A5 做完后，aios 预计约 980 MB / 355 MB；再加上 A6、A7，约 740 MB / 300 MB。

exttool：

| # | 改动 | 节省 |
| --- | --- | --- |
| E1 | **FreeCAD 改成可选**，单独做成镜像或工具包，不进默认 exttool（D5） | 每个节点约 3.2 GB 解压 / 1 GB 压缩；全量包减少约 1 GB |
| E2 | 两阶段构建，种子层改用 busybox（保留 `sh`，§3.4 的播种标记要用） | ~390 MB / ~140 MB |
| E3 | 不做 E1 的话，裁剪 FreeCAD 目录（重复的 LLVM、clang、pythonocc、OpenVINO、PCL），然后重新测 `freecadcmd` | >1 GB |

另外：删掉已经不用的 `publish/aios/Dockerfile`，或者在文件头注明“以 `build_aios` 的 `render_dockerfile` 为准”，避免以后再改错文件。让 `build_aios --runtime-only` 要么真正生效，要么删掉这个参数。

## 4. 待用户决定（2026-10-19 已裁决：全部按“建议”列执行，D5 选择移出 FreeCAD）

| # | 问题 | 建议 |
| --- | --- | --- |
| D1 | 清单放在哪里 | `bin/paios-images/paios_images.json`，tar 放同一目录（理由见 §3.2）。备选：`etc/paios_images.json` 加一个单独的 tar 目录，但需要改 devkit 让它能处理“可缺失的模块” |
| D2 | “正式包”指哪些 | artifact pack 打出来的所有安装包，以及 Linux 在线安装（nightly 和 release），都是锁定模式；本地 `make_local_pkg.py` 默认是本机模式，传 `--paios-images local` 时用本机镜像生成锁定清单 |
| D3 | 镜像什么时候进 Docker | 安装器只把 tar 放到磁盘上，由 node_daemon 启动时在后台 load，首次 deploy 时也会兜底 load。理由：WSLC 的镜像和用户上下文绑定；安装时 Docker 可能还没启动；用户 `docker image prune -a` 之后还能离线自愈。备选：在 postinstall 里尽力 load 一次（只在 Linux 和 macOS 上，失败不报错），可以和建议方案叠加 |
| D4 | 不带镜像的安装方式怎么办 | 传统安装包本期只出全量包，不出“在线包”（BuckyOS 不再 pull，不带 tar 的安装包装完就没法用）。Linux 在线安装（onlinepack，已经上线）保留，改成安装时从 R2 下载 tar（§3.6）。`BuckyOS的CI发布流程.md:96-98` 里“在线包 / 完整包”两版设想，相应改成“离线全量安装包 + Linux 在线安装，两者装完都带同一份清单和 tar” |
| D5 | FreeCAD 是否移出默认 exttool | 建议移出（§3.8 E1）。`src/` 里没有使用 FreeCAD，它占了 exttool 约 90% 的体积，也是 Windows 包能否压到 2 GB 以内的关键。移出后单独提供可选的工具镜像或包，安装方式另行设计 |
| D6 | exttool 拿不到时怎么办 | aios 拿不到时 deploy 失败。exttool 拿不到时维持现在的降级行为：worker 不挂 ExtTool 也照样启动，但要打 error 日志，并把原因写进 deploy 结果，不能悄悄忽略。锁定模式下出现这种情况，说明安装包不完整 |
| D7 | 锁定模式下，`etc/devenv.json` 里的覆盖是否生效 | 生效（显式写了覆盖就是开发者的意图），并打 warn 日志。这样在装了安装包的机器上也能用 `build_aios --local-test` 调试 |
| D8 | tar 和 exttool 镜像在加载或播种之后是否保留 | 保留 tar，用于离线自愈，代价是 amd64 上多占约 1.6 GB 磁盘（D5 落地后约 0.6 GB）。锁定模式下 exttool 播种成功后**删除 exttool 镜像**（可以随时从 tar 重建），省下约 3.7 GB。aios 镜像必须保留 |
| D9 | 是否自动回收旧版本的镜像和卷 | 做，只在锁定模式下做，只回收 node_daemon 自己 load 过的镜像（§3.4），放在 M6，不阻塞主线 |
| D10 | Windows 全量包超过 NSIS 2 GB 时怎么办 | 先做瘦身（D5 加 A1–A5），预计 Windows 包能降到 1 GB 左右。如果仍然超限，退而求其次：发一个 zip，里面是 `exe` 加同目录的 `paios-images/`，由安装器复制过去。不考虑安装时联网下载，因为这违背 issue 的出发点 |

## 5. 实施计划

### 5.0 合入顺序（硬约束）

M0 → M1 → M3 → M4（传统安装包和 Linux 在线安装两部分都要完成）→ M2（与 M5 在同一个 PR）→ M6；M7 随时可以并行。

原因：M3 和 M4 合入之后，旧版 node_daemon 会忽略清单和 tar，没有任何副作用；M2 合入后，node_daemon 不再 pull，此时如果安装包或在线安装还没有把 tar 放到位，新装的机器就跑不起 Agent。在线安装直接用最新的 Actions payload，不经过打包机，所以 `linux.sh` 的改动必须先上线到 R2。M5 里的 `build.py` 修复必须和 M2 一起合入，否则开发机会失去唯一的镜像来源。

### M0：实测与核对（不改行为）[buckyos、sn-business]

- [~] 在下面四种环境里，用 CI 方式（pull by digest、tag、`docker save | gzip`）做一个 aios tar，`docker load` 之后记录 `.Id`、`RepoDigests`、`RootFS.Layers`，看是否等于 config digest；并确认 `docker run <ref>` 和 `docker image inspect` 的输出可用：
  - Ubuntu 24.04 上的 Docker Engine（经典存储）；
  - 开启了 containerd image store 的 Docker Engine；
  - macOS 上的 OrbStack 和 Docker Desktop；
  - Windows 上的 WSLC（`load -i` 能否直接吃 `.tar.gz`，能否吃 zstd）。
  - 结论决定 §3.3 第三条兜底要不要实现，以及 tar 用 gzip 还是 zstd。
- [x] 确认 `docker/build-push-action@v7` 现在返回的 `digest` 是 image manifest 还是 index，以及加上 `provenance: false` 之后的结果。
- [x] R2 桶 `buckyos-installer` 的公开域名已经确定是 `https://buckyos-installer.tbudr.top`（pve-pack-system `config/onlinepack.toml`）。要确认的是：新前缀 `paios-images/` 能公开读取；1 GB 级的对象经过 CDN 下载正常，并且支持 Range（用于断点续传）。
- [x] 实测各平台 payload 的压缩后大小，估算全量包大小；用一个 2.5 GB 的假 payload 验证 NSIS 的限制。
- [x] 确认 `observe_deployment` 返回“容器存在但身份不匹配”时，reconcile 会停掉老容器并重建（找到具体代码路径并写进 §9）。
- [x] 确认 `bin/` 根 package env（`service_pkg.rs:35`）不会把 `bin/paios-images/` 当成 pkg 去解析。
- 产出：实测结果写进 §9.1；有影响设计的，先更新 §3 再动手。

### M1：清单格式、解析与目录占位 [buckyos]

- [x] `src/rootfs/bin/paios-images/README.md`；在 `src/bucky_project.yaml` 的 `apps.buckyos.modules` 里加 `paios_images: bin/paios-images/`。
- [x] `src/kernel/node_daemon/src/paios_images.rs`：实现结构体、`load_manifest`、`resolve`，以及字段规则（§3.2）。此时**还不接入 app_loader**。
- [x] fixture `src/kernel/node_daemon/src/testdata/paios_images.v1.json`。单测覆盖：文件不存在；解析失败；schema 不一致；缺当前架构；`tar.file` 带路径时被拒绝；未知字段被忽略；devenv 优先级（D7）。
- [x] `.github/scripts/paios_images.py` 的 `fragment`、`assemble`、`validate` 子命令，以及 `.github/scripts/test_paios_images.py`，与 Rust 共用同一份 fixture。
- 验证：`cargo test -p node_daemon paios_images -- --test-threads=1`；`uv run python -m unittest .github/scripts/test_paios_images.py`（按 `test_install_descriptor.py` 的实际运行方式调整）；`buckyos-install --app=buckyos --all --target-rootfs=<tmp>` 能把 README 装进 `bin/paios-images/`。

### M2：node_daemon 锁定模式 / 本机模式，去掉 pull [buckyos]（与 M5 同一个 PR）

> 2026-10-20：M2+M5+M6 已实现并通过本机 DV（分支 `paios-images-m2`，fbf42ab8b）；用户确认打包机侧完成后合入 main（merge 09694b564，§9.3）。

- [x] 按 §3.4 改造 app_loader：`worker_image_name`、`prepare_worker_image`、exttool 的卷名、播种、标记和跳过逻辑、`run_worker_container` 的标签、`observe_deployment` 的身份比对、两个 preview 函数，并删掉 worker 和 exttool 路径上的 pull。
- [x] 启动后台预加载（锁定模式），和 deploy 共用锁，Docker 不可用时退避重试。
- [x] WSLC：补上 inspect 的格式映射和 `test_wslc.rs` 用例（只在 Windows 上编译）。
- [x] 修正 `app_loader.rs:31` 的注释路径。
- [x] 测试（`test_app_loader.rs`，复用 `:795`、`:1336` 那种记录命令的写法）：
  - 锁定模式，镜像已存在且身份匹配：不执行 load，不执行 pull。
  - 锁定模式，镜像缺失：先校验 tar 的 sha256，再 `docker load`，再校验身份。
  - tar 的 sha256 不一致：报错，不执行 load，不执行 pull。
  - load 之后身份仍不符：报错。
  - tar 缺失：报错，错误信息里有文件名。
  - 本机模式，镜像缺失：报错，错误信息里有 `docker pull` 提示，并且全部命令里**没有** `pull`。
  - 容器的 `.Image` 和当前 worker 镜像 id 不一致：判定为不匹配。
  - exttool 卷名带 id；卷存在且有标记：不检查 exttool 镜像；卷存在但没有标记：重新播种。
  - D6 的降级路径。
  - devenv 覆盖在锁定模式下优先，并且打 warn 日志。
  - 改写 `:795`、`:958`、`:1118`、`:1149`、`:1336` 里依赖 pull 的旧断言。
- 验证：`cargo test -p node_daemon -- --test-threads=1`；`cargo test -- --test-threads=1`（和 `dev-tests.yml` 的单测入口一致）。
- 本机 DV：
  - 手工放一份清单和 tar 到 `/opt/buckyos/bin/paios-images/`，然后 `docker rmi` 掉 aios，重启 BuckyOS → 能从 tar 恢复，Jarvis 能对话。
  - 删掉清单 → 进入本机模式，按 `devenv.json`（`local/aios-test`）运行。
  - 注意本机现在的 `devenv.json` 里是 `{"aios": "local/aios-test"}`，测锁定模式前先备份并移走这个文件，测完恢复。

### M3：CI 导出、继承、清单写入 payload [buckyos]

- [x] `build_aios`、`build_exttool`：不可变 tag；exttool 补上 build arg、label 和 `metadata.env` 字段（§3.5-1）。
- [x] `build-aios.yml`、`build-exttool.yml`：`provenance: false`、`sbom: false`；新增不可变 tag；新增 `export` job（pull by digest、inspect、tag、save、上传 R2、片段 artifact、写指针）；新增 R2 secrets。
- [x] `build-all.yml`：`detect-image-changes` 改为和指针比较，并保存指针快照 artifact；新增 `resolve-paios-images` job；三个平台构建加 `needs`，透传 R2 secrets。
- [x] `build-linux.yml`、`build-macos.yml`、`build-windows.yml`：下载 `paios-images-manifest`，写入 `$STAGE_ROOT/buckyos/bin/paios-images/paios_images.json`（artifact 不存在时跳过）。
- 验证：
  - 首次运行时还没有指针 → 必须构建，并且会写出指针。
  - 只改 `doc/` → 不构建，清单继承指针，`images.aios.revision` 早于 `buckyos_revision`。
  - 改 `publish/aios/` → 构建，清单指向新的 id。
  - 人为让一次 aios 构建失败 → 下一次不改 aios 的 push 仍然会构建。
  - 下载某个平台的 payload artifact，解包后能看到 `bin/paios-images/paios_images.json`，并且能通过 `paios_images.py validate`。

### M4：打包取 tar、安装包校验、在线安装下载 tar [sn-business、buckyos-pack-installer、pve-pack-system]

- [x] sn-business：`package-kit/stage_paios_images.py`；三个打包脚本调用它；`create_package_result.py` 记录 `paios_images`；在 README 里说明缓存目录和网络要求（能访问 R2）。
- [x] buckyos-pack-installer：`verify-pkg` 支持 `--require-paios-images`，sn-business 打包时传入；处理 M0 发现的压缩和体积问题；Windows 按 D10 处理。
- [x] 更新 buckyos 主仓库里的子模块指针。
- [x] pve-pack-system：`installer/linux.sh` 按 §3.6 下载、校验并安装 tar（真实安装和 `--stage` 两种模式）；`onlinepack.py` 发布前对 tar 做 HEAD 校验，并决定清单从哪里读（从 payload 里解出，或用 CI 单独上传的清单小文件，见 §3.6）；在 `tests/` 里补对应用例（参照 `tests/test_artifact_pack_orchestrator.py` 的写法）。artifact pack 的调度（`artifact_pack_orchestrator.py`）不需要改动，它调用的 sn-business `pack` 已经在上面处理了。
- 验证：
  - 通过 artifact pack 跑一次完整打包（`python3 bin/artifact_pack_orchestrator.py --config config/artifact-pack.toml --force`，或者只跑 sn-business 的 `pack --cluster ci`），五种安装包都包含清单和对应架构的 tar（`verify-pkg` 通过）；记录各安装包的大小。
  - Linux 在线安装：在新的 Ubuntu 24.04 amd64 和 arm64 上用 `install.sh` 各装一次，`$BUCKYOS_ROOT/bin/paios-images/` 下有清单和对应架构的 tar，sha256 一致。

### M5：开发流程与文档 [buckyos]（与 M2 同一个 PR）

- [x] `build.py`、`start.py`、`make_local_pkg.py --paios-images local`、`diagnose_jarvis_orbstack.sh`（§3.7）。
- [x] `src/tools/tests/` 下补充或修改 `build.py` 镜像列表和 `start.py` 预检查的测试（参照 `test_start_entrypoint.py` 的写法）。
- [x] 按 §3.7 列出的文档逐一更新。
- 验证：`uv run python -m unittest discover -s tools/tests -p 'test_*.py'`（在 `src/` 下运行）；在没有任何 paios 镜像的机器上跑 `uv run build.py`，之后 Agent 可以正常运行。

### M6：旧镜像和旧卷回收 [buckyos]（取决于 D9）

- [x] 状态文件 `local/node_daemon/paios_images_state.json`；预加载完成后执行回收（§3.4）；旧的 `buckyos-exttool` 卷在没有容器使用时被删除。
- 验证：从版本 A 覆盖安装到版本 B（镜像 id 不同），重启后 A 的镜像和卷被删掉，B 正常运行；开发者自己 `docker pull` 下来的镜像不会被删。

### M7：镜像瘦身 [buckyos]（可以并行）

- [x] aios A1–A5（§3.8）。每改一项，在镜像里跑一遍 ffmpeg 转码、imagemagick、pdftotext、tesseract、`uv venv`、deno、node 的冒烟命令，并跑通一次 Jarvis 对话的 DV。
- [x] 按 D5 处理 exttool：E1，或者 E2 加 E3。
- [x] 删掉或标注 `publish/aios/Dockerfile`；处理 `--runtime-only`。
- [x] A6（pandoc 移到 ExtTool）、A7（三个 Rust 二进制合并成一个，按调用名分派）。
- 验证：在 §9 记录改动前后的 `docker image inspect` 大小和 registry 压缩后大小。

## 6. 验收

1. **离线全量安装**：在 Ubuntu 24.04 amd64 上用 deb 新装，同时在 `/etc/hosts` 里把 `registry-1.docker.io`、`auth.docker.io`、`production.cloudflare.docker.com` 指到 `0.0.0.0`。激活 → Jarvis 对话 → systest 脚本应用运行，全部成功；`docker images` 显示清单中的不可变 tag；node_daemon 日志里没有任何 pull。
2. 同样的验收在 macOS arm64（pkg + OrbStack）和 Windows amd64（exe + WSLC 或 Docker Desktop）上各做一次。另外用 Linux 在线安装（`install.sh`）在 Ubuntu 24.04 上做一次：安装阶段只访问 R2，不访问 docker.io；装完后按第 1 条在屏蔽 docker.io 的情况下验收。
3. **自愈**：`docker rmi` 掉 aios 镜像后重启 BuckyOS，能自动从 tar 恢复；删掉 exttool 卷后重启，会重新播种。
4. **篡改**：改动 tar 中的一个字节 → 报错，错误信息可读，并且没有发起 pull。
5. **本机模式**：删掉清单，本地也没有 `latest-<arch>` → 报错，带 `docker pull` 提示；有 `latest-<arch>` 时能正常运行。
6. **升级**：从版本 A 覆盖安装到版本 B（镜像 id 不同）→ worker 容器按 B 的镜像重建，exttool 换成新卷；做完 M6 之后，A 的镜像和卷被回收。
7. **CI**：M3 的五条验证全部通过；sn-business 产出的每个安装包的 `package-result.zip` 里都能查到所带的镜像 id。
8. **单测**：`cargo test -- --test-threads=1`、`.github/scripts` 下的单测、`src/tools/tests` 下的单测全部通过。
9. **体积**：§9 记录每个安装包的大小；Windows 安装包在 NSIS 限制以内，或者已经按 D10 的备选方案发布。

## 7. 风险与已知坑

- **NSIS 2 GB 限制**（§3.6、D10）：Windows 全量包要靠瘦身才稳妥。
- **镜像 id 的语义在不同运行时下不一致**（§3.3）：M0 必须实测，不要凭印象实现。
- **首次启动慢**：load 1.4 GB 的 tar，再播种几个 GB 的卷，要几分钟。预加载只能把这段时间移到开机后，不能消除。Agent 在这段时间里 deploy 会等锁，UI 上可能显示“启动中”的时间更长，必要时在 M2 之后补一个状态提示。
- **磁盘占用**：tar 加 Docker 存储，再加 exttool 卷。D5 和 D8 决定最终占用多少，最好在安装包的依赖或空间检查里提示。
- **指针竞争**：并发 run 的问题由“detect 时保存指针快照”解决（§3.5-4）。指针只能由 main 分支上的 CI 写。开发者本地 `build_aios --push` 只会改 Docker Hub 上的 tag，不影响指针，也就不会污染正式包。
- **fork 和 PR**：没有 R2 和 Docker Hub 凭据，payload 不带清单。这类构建本来就不会进 sn-business 打包。
- **未来做 OTA 时**：如果以后用 pkg 升级系统二进制，`bin/paios-images/` 必须作为同一次升级的一部分下发，tar 也要跟着走；否则 node_daemon 会拿着新清单去找不存在的 tar，然后报错。到时候在 OTA 设计里引用本文。
- **WSLC 用户上下文**：用户换了运行 node_daemon 的账号或权限级别，原来 load 的镜像就看不到了。按本设计会从 tar 重新 load，结果正确，只是要多等一会儿。
- **开发者习惯变化**：M2 之后不再自动 pull。PR 描述、`AGENTS.md` 和 `start.py` 的 warning 都要写清楚。
- **在线安装的下载量和 R2 流量**：每次在线安装（包括升级）都要从 R2 下载约 1.6 GB 的 tar（amd64）。升级时如果清单里的 `image_id` 和已装版本相同，`linux.sh` 可以复用 `$BUCKYOS_ROOT/bin/paios-images/` 里已有的、sha256 一致的文件，不再下载。注意 `linux.sh` 会删掉 `bin/`，要在删除前先把可复用的 tar 移到 `$WORK`。这是 M4 的可选优化。
- **在线安装用的是最新的 Actions payload**，不经过打包机的人工确认。CI 生成的清单一旦有错（比如指向一个不存在的 tar），会直接影响在线安装的用户。所以 `onlinepack.py` 发布前的 tar HEAD 校验是必做项，不是可选项。

## 8. 参考入口

- issue：https://github.com/buckyos/buckyos/issues/672
- 运行时：`src/kernel/node_daemon/src/app_loader.rs`（`:32`、`:73`、`:961`、`:1223`、`:1659`、`:1694`、`:2075-2253`、`:2329`、`:2634`、`:2662`、`:2909`、`:3149`、`:3626-3647`）、`src/kernel/node_daemon/src/wslc.rs`、`src/kernel/node_daemon/src/test_app_loader.rs`、`src/kernel/node_daemon/src/sn_zone_info.rs:11-25`
- CI：`.github/workflows/build-all.yml`、`build-aios.yml`、`build-exttool.yml`、`build-{linux,macos,windows}.yml`、`.github/scripts/write-install-descriptor.py`、`test_install_descriptor.py`
- 镜像构建：`build_aios`（`:338`、`:519-652`、`:1246-1334`）、`build_exttool`、`publish/aios/`、`publish/exttool/`
- 打包：`src/bucky_project.yaml:91-162`、`make_local_pkg.py`、`src/publish`（buckyos-pack-installer）、`doc/CI/impl_buckyos_packaging.md`、`doc/CI/prd_buckyos_packaging.md`
- sn-business：`CD/cluster_config/ci/{artifact_sources.json,apps_version.json,update_apps_version.ts,apps.ts}`、`commands/pack/{README.md,post_pve-*-builder.*,package-kit/}`
- pve-pack-system：`README.md`、`doc/artifact_pack_entry.md`、`doc/onlinepack.md`、`bin/artifact_pack_orchestrator.py`、`bin/onlinepack.py`、`bin/check_and_publish_onlinepack.py`、`installer/{install.sh,linux.sh}`、`config/onlinepack.toml`；旧入口 `bin/pve_orchestrator.py`
- Actions run 实例：https://github.com/buckyos/buckyos/actions/runs/39361968276（只有编译和 payload，没有安装包）
- devkit：`buckyos-devkit/src/install.py:292-300, 484-516`
- 开发：`build.py:100-120, 262-303`、`src/start.py`、`AGENTS.md:62-70`、`src/diagnose_jarvis_orbstack.sh`
- 设计文档：`doc/CI/BuckyOS的CI发布流程.md`、`doc/app_service/paios容器需求.md` §6.3、`doc/app_service/rust_app_loader.md`、`doc/windows_wslc.md`、`product/buckyos_desktop_installer/新版本离线安装包的构建思路.md`

## 9. 实施记录

（实施后填写。每个 M 一节：落点、验证命令和结果、与 §3 设计不同的地方、遗留问题。）

### 9.1 M0 实测结果（2026-10-20）

用到的脚本和原始输出在会话草稿区，没有进仓库；结论如下。

| 项 | 结果 | 对设计的影响 |
| --- | --- | --- |
| registry 上 `paios/aios:latest-amd64` 的形态 | 是带 attestation manifest 的 **index**（`627cb262…` 是 amd64 image manifest，`35ba75b8…` 是 attestation）。`build-push-action` 现在输出的 `digest` 是这个 index | §3.5-2 的 `provenance: false`、`sbom: false` 必须加，已加 |
| Docker Engine 26.1.5（经典存储，本机） | 按 CI 方式 pull by digest → tag → `docker save \| gzip -n`（458,226,988 字节，save+gzip 45 s）→ rmi → `docker load`：`.Id` = config digest `7329ae7f…`，`RepoDigests` 为空，`RootFS.Layers` 与 pull 时一致，revision label 保留，`docker run <ref>` 正常 | 第一条规则（`.Id == image_id`）可用 |
| Docker Engine 29.9.1（`docker:29-dind`，默认 containerd image store） | 同一个 tar load 后 `.Id` = `sha256:15a94a4b…`，是 **tar 内 index 引用的 manifest 的 digest**，既不是 config digest 也不是 registry manifest digest；`RepoDigests` 是 `paios/aios@sha256:15a94a4b…`；`RootFS.Layers` 与 revision label 一致；`docker images` 显示 2.95 GB（解压+压缩合计，印证 §2.6 第 3 条“2G”来源） | §3.3 第三条兜底（`rootfs_layers_sha256` + revision）**必须实现**，已实现 |
| `docker save` 是否字节稳定 | 同一镜像两次 `docker save \| gzip -n` 的 sha256 不同 | R2 对象 key **改为按 tar 的 sha256 寻址**（`paios-images/<name>/<arch>/<tar sha256>.tar.gz`），否则“已存在就跳过”会让清单里的 sha256 与 R2 上的文件不一致 |
| zstd | `docker load -i x.tar.zst` 在 26.1.5 经典存储和 29.9.1 containerd 存储上都成功；zstd -3 为 429 MB，比 gzip 小 6% | OrbStack、Docker Desktop、WSLC 未测，按 §3.5-3 保持 gzip |
| R2 公开域名 | 用 pve-pack-system 的 R2 凭据往 `paios-images/m0-probe.json` 写入、经 `https://buckyos-installer.tbudr.top` 公开读取成功（`cf-cache-status: DYNAMIC`，不缓存），随后删除。现有 216 MB payload 的 HEAD 有 `Content-Length`、`Accept-Ranges: bytes`，Range 请求返回 206 | 新前缀可公开读；1 GB 级对象的 CDN 下载留到 M3 第一次真实上传后验证 |
| 平台 payload 压缩后大小（run 39361968276） | windows-amd64 168 MB、apple-amd64 166 MB、apple-aarch64 158 MB、linux-amd64 206 MB、linux-aarch64 203 MB | 全量包 ≈ payload + aios tar 458 MB + exttool tar（瘦身后见 §9.8） |
| NSIS（3.11，Ubuntu 包） | 单个 2.5 GiB 文件：`makensis` 直接失败 `failed creating mmap`（lzma 和 `SetCompress off` 都一样）；单个 1.1 GiB 文件成功。多文件合计超 2 GiB 的结果见 §9.8 | D10：Windows 全量包只能靠瘦身压到 2 GB 以内 |
| `observe_deployment` 不匹配后的 reconcile | `run_item.rs::ensure_run_item_state`：容器身份不匹配且镜像存在时 `observe_deployment` 返回 `Stopped`（镜像不存在返回 `NotExist`）→ `start_run_item` → `start_agent` / `start_host_script` 先 `deploy()`，再 `stop_agent()`（`docker rm -f`）后按新镜像 `run_worker_container` | §3.4 第 4 条的思路成立，不需要新增 reconcile 分支 |
| `bin/` 根 package env | `service_pkg.rs::new_system_package_env` 按 pkg 名 `load`，不扫描目录 | `bin/paios-images/` 不会被当成 pkg |
| devkit 安装 | `buckyos-install --app=buckyos --all --target-rootfs=<tmp>` 后 `bin/paios-images/` 只有 README.md | 开发安装自然处于本机模式 |

### 9.2 M1：清单格式、解析与目录占位

- 落点：`src/rootfs/bin/paios-images/README.md`；`src/bucky_project.yaml` 的 `apps.buckyos.modules.paios_images`；`src/kernel/node_daemon/src/paios_images.rs`（`load_manifest` / `resolve` / `parse_image_inspect` / `matches_pinned` / `verify_tar` / 本机模式缺镜像提示）；fixture `src/kernel/node_daemon/src/testdata/paios_images.v1.json`（aios amd64 一项用的是 M0 实测的真实值）；`.github/scripts/paios_images.py`（`platform` / `fragment` / `assemble` / `validate` / `validate-fragment`）与 `.github/scripts/test_paios_images.py`，两边共用 fixture。
- 与设计不同：`resolve` 自己读 `<root>/etc/devenv.json`，不再经 app_loader 的 devenv 解析；`tar` 多了 `path` 字段（§3.2）。
- 验证：`cargo test -p node_daemon paios_images -- --test-threads=1` 10 项通过；`cargo test -p node_daemon -- --test-threads=1` 119 项通过；`python3 -m unittest discover -s .github/scripts -p 'test_*.py'` 13 项通过；`uv run buckyos-install --app=buckyos --all --target-rootfs=<tmp>` 后 `bin/paios-images/` 只有 README.md。

### 9.3 M2 + M5 + M6：node_daemon 锁定模式 / 本机模式、开发流程、旧镜像回收

- node_daemon：`paios_images.rs` 增加 `PaiosDocker`（复用 app_loader 的 docker 执行和测试 runner）：`ensure_image`（按 ref 加锁；锁定模式校验身份，不符或缺失时先流式校验 tar 再 `docker load` 再校验；本机模式只看存在；都不 pull）、`prepare_exttool`（卷名 `buckyos-exttool-<id12>`、`.buckyos-seeded-<id12>` 标记、无标记重建、锁定模式播种后 `image rm` 种子镜像、拿不到时 error 日志降级）、`worker_expectation`（状态检查用的镜像 id + 可挂载卷，真实 docker 下缓存 30 s，load/播种后失效）、`collect_garbage`（D9）和 `spawn_preload`（锁定模式启动即后台预加载，5 s 起指数退避到 5 min，成功后回收）。`local/node_daemon/paios_images_state.json` 记录 node_daemon 自己 load 过的 ref。
- app_loader：删掉 worker / exttool 的 pull、`DEFAULT_*` 常量和 devenv 解析（devenv 改由 `paios_images::resolve` 读）；`worker_image_name()` 返回 `Result`；`run_worker_container` 挂载 `buckyos-exttool-<id12>`，标签写实际镜像 id 和 `buckyos.exttool_volume`；`observe_deployment` 对 HostScript / Agent 用当前镜像 id 复用原有三重比对，再比对 ExtTool 卷标签，镜像不在本地时返回 `NotExist`；preview 改为 `image inspect` / `load -i` / 卷创建 / 播种命令。新增 `with_paios_images_root()` 测试钩子，测试全部改为不依赖本机 `/opt/buckyos`。
- 与设计不同：WSLC 不需要新增格式映射——身份检查用已有的 `{{json .}}` 投影，`load`、`volume rm`、`image rm` 都是原样透传。D6 里“把原因写进 deploy 结果”没有单独做：降级在 node_daemon 日志里是 error，deploy 本身成功。
- 开发流程：`build.py` 预拉取 `paios/aios`、`paios/exttool` 的 `latest-<arch>`（运行时 `devenv.json` 覆盖的跳过）；`start.py` 启动后在本机模式下检查这两个镜像，缺了打印醒目的 `docker pull` 提示，自己不拉；`make_local_pkg.py build-pkg --paios-images local` 把本机 `latest-<arch>` 打成 `paios/<name>:local-<id12>-<arch>` 并写清单（离线测试包用）；`diagnose_jarvis_orbstack.sh` 有清单时按清单 ref 诊断并列出 ExtTool 卷。文档：`AGENTS.md`、`doc/CI/impl_buckyos_packaging.md`（§5、§15.2）、`doc/app_service/rust_app_loader.md` §9.1.1、`doc/windows_wslc.md`、cmd_skill `buckyos_dev_release`。
- 验证：`cargo test -p node_daemon -- --test-threads=1` 126 项通过（新增锁定模式 load / 篡改 / 缺 tar / 身份不符 / 本机模式缺镜像不 pull / ExtTool 播种与标记与重建 / D6 / D8 / 状态检查期望 / 回收）；`src/` 下 `uv run python -m unittest discover -s tools/tests -p 'test_*.py'` 通过（新增 `test_build_paios_images.py` 和 start 预检查用例）。
- 本机 DV（2026-10-20，Docker 26.1.5 经典存储，zone test.buckyos.io，3 个 Worker：Agent tmgr1、xman@bob 和脚本应用 systest；`buckyos-build.py -s node_daemon` 后安装）：
  1. 本机模式（`devenv.json` 的 `aios=local/aios-test`）：deploy 播种 `buckyos-exttool-d59491422db1` 用时 0.5 s；容器标签 `buckyos.image_digest`、`buckyos.exttool_volume` 正确；75 s 内没有重建循环；`start.py` 没有缺镜像警告。
  2. 锁定模式（移走 `devenv.json`，用同一批本机镜像生成清单和 tar）：预加载 0.2 s 完成（镜像已在且身份匹配，不 load），随后回收删掉了旧的 `buckyos-exttool` 卷；三个 Worker 用 `paios/aios:dv-…` 运行，`check.py` 为 Running。
  3. 自愈：`docker rmi` 掉锁定 tag → 下一轮状态检查 `NotExist` → 三个应用共用一次 load（含 541 MB sha256 校验共 8.3 s）→ 容器重建。没有 pull。
  4. 篡改：改 tar 一个字节后再 rmi → 每个应用报 `bin/paios-images/aios-amd64-….tar.gz is corrupted (size … sha256 …, expected …)`，没有 pull，原有容器保持运行；恢复 tar 后自动 load 恢复。据此加了按 size/mtime/期望 sha256 缓存校验结果，避免损坏时每轮重算 sha256。
  5. 真实 CI 产物：从 R2 公开域名下载 run 39385674475 的清单和 amd64 tar（aios 458 MB、exttool 11 MB，HEAD 和 sha256 均一致），放进 `bin/paios-images/` 后重启：预加载 12.2 s 从 tar 载入 `paios/aios:0.7.0-b91b891ca3ff-amd64`（`.Id` 等于清单的 config digest）；回收删掉了上一轮 node_daemon load 的 `paios/aios:dv-…` tag；三个 Worker 改用 CI 镜像运行，Agent 端口 HTTP 200，容器内 `/opt/buckyos/tools` 有 `.buckyos-seeded-d59491422db1`，`freecadcmd` 输出新的占位说明。
  6. 运行中恢复 `devenv.json`（D7）：约 30 s 内三个 Worker 自动按 `local/aios-test` 重建（无需重启），日志有覆盖告警；告警原来每轮都打，已改为每个进程只打一次。
  - DV 后已还原：删除清单和 tar、状态文件和测试 tag，`devenv.json` 原样恢复，zone 停止（与开始前一致）。未做：Jarvis 实际 LLM 对话（镜像内运行时没有改动，只验证了 Agent 服务可访问）；containerd 存储、OrbStack、Docker Desktop、WSLC 上的 node_daemon 实跑。
- 合入（2026-10-20）：用户确认打包机侧完成后 `git merge --no-ff` 进 main（09694b564，无冲突）。合入后并行跑 `cargo test -p node_daemon` 时 main 上原有的 `exttool_prepare_lock_is_process_wide_and_exclusive` 偶发失败：它用 `try_lock` 假定锁空闲，而并行的 ExtTool 播种测试正持有这把锁（上面的验证用的是 `--test-threads=1`，所以没暴露）；改为 `blocking_lock` 等锁（3db8ee701），并行连跑 5 次 126 项都通过。CI 见 §9.4 run 6。

### 9.4 M3：CI 导出、继承、清单写入 payload

- 落点：
  - `build_aios` / `build_exttool`：`--prepare-context` 的 `metadata.env` 增加 `immutable_tag=<VERSION>-<rev12>`（工作区 dirty 时为空，CI 检查非空）；`build_exttool` 增加 `revision`、`created`，并把 `IMAGE_VERSION` / `SOURCE_REVISION` / `BUILD_CREATED` 传给 Dockerfile 写 OCI label。
  - `build-aios.yml` / `build-exttool.yml`：`provenance: false`、`sbom: false`；多推一个 `<immutable_tag>-<arch>`；新增 `export` job 调 `.github/scripts/paios_image_export.sh`（pull by digest → tag → `docker save | gzip -n` → `paios_images.py platform` → R2 按 sha256 上传，已存在跳过 → `fragment`），上传 artifact `paios-image-<name>`；main 上写指针 `paios-images/main/<name>.json`。R2 用 runner 自带的 AWS CLI（`AWS_REQUEST_CHECKSUM_CALCULATION=when_required`）。
  - `build-all.yml`：`detect-image-changes` 总是 checkout，先用 R2 凭据读两个指针（校验后存 artifact `paios-image-pointers` 作为快照），再按指针 revision 做 `merge-base --is-ancestor` + `git diff`；路径列表加上两个导出脚本。新增 `resolve-paios-images`（总是运行；非 `buckyos/buckyos` main 时输出 `pinned=false`；main 上 detect 或任一镜像构建不是 success/skipped 就失败；构建了用本次 fragment，跳过了用指针快照；跑 `test_paios_images.py`、`assemble`、`validate --check-urls`，上传 `paios-images-manifest`）。三个平台构建 `needs` 它且要求 success，传入 `paios_images` 输入。`upload-payloads` 给每个 `buckyos-*.tar.gz` 旁边放一份 `<payload>.paios_images.json`（§3.6 末尾的二选一，选了“CI 另外上传清单小文件”）。
  - `build-{linux,macos,windows}.yml`：新增 `paios_images` 输入；为 true 时下载 manifest、复制到 `$STAGE_ROOT/buckyos/bin/paios-images/paios_images.json` 并 `validate`，位置在 `buckyos-install` 之后、`write-install-descriptor.py` 之前。
- 与设计不同：本地 `build_aios --push` 不推不可变 tag（避免开发机覆盖 CI 的 tag）；主线上镜像构建失败时平台构建不再继续（以前互不依赖），保证 main 上传到 R2 的 payload 一定带清单。
- 验证：actionlint 1.7.7 只剩 `build-windows.yml` 原有的 shellcheck 报错（与本改动无关）。
- CI 实跑（2026-10-20）：
  1. run 39385674475（b91b891ca，还没有指针）：两个镜像都构建、导出到 R2 并写了指针；`resolve-paios-images` 组装清单并 HEAD 校验 4 个 tar；5 个平台构建全部成功；Linux amd64 payload 里有 `bin/paios-images/paios_images.json` 且 `.install-buckyos.json` 声明了该模块；`upload-payloads` 把 `<payload>.paios_images.json` 旁挂清单传到了 R2。
  2. run 39389340246（47a6e822d，只改子模块）：detect 判定两个镜像都 “unchanged since published revision b91b891ca”，不构建，清单从指针快照继承（`images.*.revision` 早于 `buckyos_revision`）。
  3. run 39390683744（559883fde，M7 改了 `build_aios`、`publish/aios/`）：只重建 aios（exttool 继承），指针更新为 559883fde；CI 导出的瘦身后 aios tar amd64 370 MB（原 458 MB）、aarch64 360 MB（原 443 MB）。
  - **发现并修复的问题**：run 2、run 3 的 `upload-payloads` 被跳过——它没有写状态函数，隐式 `success()` 会沿依赖链看到被跳过的镜像构建（`resolve-paios-images` 依赖它们），只要镜像没变就不上传 payload。改为 `!cancelled()` 加上对四个直接依赖结果的显式判断。修复前这两次 run 的 payload 没有进 R2（R2 上仍是 run 1 的同版本 payload，onlinepack 会因 Last-Modified 对不上而拒绝发布，直到下一次成功上传）。
  4. run 39400318783（3c9b09169，即上面的修复；改了 `build-all.yml`，它在两个镜像的路径列表里）：两个镜像都重建，`Upload component payloads to R2` 成功；R2 上 `buckyos-linux-amd64-0.7.0+build261020.paios_images.json` 的 `buckyos_revision` 和两个镜像 revision 都是 3c9b09169，指针同步更新（aios amd64 370 MB / aarch64 359 MB，exttool 10.9 / 10.4 MB）。
  5. run 39411084310（7f23502b9，只改 notepads）：`build-aios`、`build-exttool` 都是 skipped，清单从指针继承，`Upload component payloads to R2` 照常运行并成功；R2 旁挂清单的 `buckyos_revision` 是 7f23502b9，两个镜像 revision 仍是 3c9b09169。上面的上传被跳过问题已确认修复。
  6. run 39429150151（3db8ee701，M2 合入后）：M2 改了 `build_exttool` 的注释，exttool 重建、aios 继承；全部 job 成功，payload 上传；R2 旁挂清单 `buckyos_revision` 3db8ee701，aios 3c9b09169，exttool 3db8ee701。
  - “上一次镜像构建失败后下一次仍会构建”没有人为制造失败来验证，依据是指针只在导出成功后写入。

### 9.5 M4：打包取 tar、安装包校验、在线安装下载 tar

- buckyos-pack-installer（`src/publish` 子模块）：`package_common.verify_paios_images()`（按文件名定位，兼容 NSIS 展开后目录被打平）+ `package_arch_from_name()`；deb / rpm / macOS pkg / Windows exe 的 `verify-pkg` 都加了 `--require-paios-images`。rpm 用 `rpm2cpio | cpio` 只解出 `bin/paios-images/`；macOS 用 bsdtar 只解出 `buckyos.pkg/Payload` 里的 `bin/paios-images/`；deb、exe 用已有的解包目录。payload 带清单时即使不传该参数也会校验 tar。
- sn-business：`package-kit/stage_paios_images.py`（读 staging 里的清单 → 缓存 `<cache>/<name>/<arch>/<sha256>.tar.gz`，HTTP Range 续传，校验 size+sha256 → 复制到 `bin/paios-images/`，每个镜像每个架构保留 4 份）；三个 builder 脚本在组装 staging 后调用，`verify-pkg` 传 `--require-paios-images`；`create_package_result.py --paios-images <summary>` 把每个架构的 ref / image_id / revision 写进 `package-result.json` 的 `paios_images`；README 补了缓存目录和网络要求。macOS arm64 包本来就跳过 `verify-pkg`（交叉架构不能跑 Mach-O 检查），这次没有改。
- pve-pack-system：`installer/linux.sh` 解出 payload 后、停旧服务前按本机架构用 `tar.path` 拼当前 base URL 下载 tar，校验 size+sha256，`$BUCKYOS_ROOT/bin/paios-images/` 里已有同 sha256 的文件直接复用；校验失败在动旧安装之前中止。`bin/onlinepack.py` 新增 `paios_images()`：读 `buckyos/<payload>.paios_images.json` 旁挂清单，对该架构每个 tar 做 HEAD 并比对 size，没有旁挂清单或不一致就拒绝发布；`check_and_publish_onlinepack.py` 用带认证的 R2 GET/HEAD。核对过的 ref / image_id 记在发布回执的 payload 记录里。`doc/onlinepack.md` 已同步。
- 验证：pack-installer `python3 -m unittest discover -s tests` 56 项通过；sn-business `package-kit` 6 项通过；pve-pack-system 117 项通过（新增 `tests/test_online_paios_images.py`，其中 `linux.sh --stage-dir` 用假 curl 做了端到端安装、复用和篡改中止）。
- 本机模拟打包机 Linux builder（2026-10-20）：取 run 39385674475 的 `buckyos-linux-amd64-0.7.0+build261020` payload（带清单）→ `stage_paios_images.py` 从 R2 下载 437 MB + 10 MB 并校验 → pack-installer `make_local_deb.py build-pkg`（项目定义用 b91b891ca 的 `bucky_project.yaml`，没有叠加 cyfs-gateway payload）→ `verify-pkg --require-paios-images` 通过，deb 596 MB。没有在干净 VM 上安装这个 deb（§6 验收 1 需要打包机产物 + 真实 VM）。
- 部署：打包机上的 `/root/sn-business`（含子模块）和 pve-pack-system 都不会自动更新，需要在打包机上 `git pull` 后才生效；Linux 在线安装的 `linux.sh` 要等打包机下一次 `check_and_publish_onlinepack.py` 才会上传到 R2。这两步完成并验证之前不能合入 M2；2026-10-20 用户确认已完成，随后合入 M2（§9.3）。

### 9.8 M7：镜像瘦身

- exttool（E1 + E2，随 M3 一起推送 b91b891ca）：FreeCAD 默认不再打包（`FREECAD_APPIMAGE_URL_*` 默认为空，`freecadcmd` 是说明怎样自行构建带 FreeCAD 镜像的占位命令）；Dockerfile 改成两阶段，最终镜像只有 `busybox:1.36.1` + `/opt/buckyos/tools`。本机 `docker image inspect` 从 3,673 MB 降到 33.6 MB；CI 导出的 tar：amd64 10.9 MB、aarch64 10.4 MB（原来 amd64 压缩后 1,122 MB）。E3 不需要了。
- aios（A1–A5）：apt 层装完后在同一层删掉 `libLLVM*`、`libgallium-*`、`libGLX_mesa*` 和 `dri/`（ldd 核对过 ffmpeg 等工具不链接它们，ffmpeg 只链接很小的 `libxcb-dri3`）；去掉 `uv python install 3.12` 和 `python3-venv`，`uv venv` 离线用系统 Python 3.12.3；Debian 的 nodejs 18 + npm 换成官方 Node `v24.21.0` tarball（按 `SHASUMS256.txt` 校验，去掉 include 和文档）；`locales` + `en_US.UTF-8` 换成 `C.UTF-8`；deno check 后清 `/root/.cache/deno`；二进制和 entrypoint 改用 `COPY --chmod=755`。删掉了没人引用的 `publish/aios/Dockerfile`，去掉了不起作用的 `--runtime-only`。A6、A7 后来也做了，见下。
- 大小（本机构建 `local/aios-slim` 对比 CI 的 `paios/aios:0.7.0-b91b891ca3ff-amd64`）：解压后 1,068 MB 对 1,385 MB；`docker save | gzip -1` 408 MB 对 508 MB（CI 的 gzip -6 基线是 458 MB，按比例约 370 MB）。
- 冒烟（`--network none`）：ffmpeg 转码 / 抽帧、imagemagick 缩放旋转、tesseract 加载 eng 数据、pdftotext 抽取手写 PDF 文本、pandoc、`uv venv --offline`（系统 Python）、node 24 + `npm init`、deno、UTF-8、`xagent --help` 全部通过；基线镜像同样的脚本也通过（基线镜像没有字体，`convert -annotate` 本来就不可用，所以冒烟不依赖字体）。zone DV：`devenv.json` 指向 `local/aios-slim` 后三个 Worker 正常运行，Agent 端口 HTTP 200，opendan 日志里只有启动期网关未就绪的登录重试（基线一样）。DV 后 `devenv.json` 已恢复、zone 已停止。
- A6（2026-10-20 用户同意）：pandoc 不再是 aios 的 apt 包，改成 ExtTool 工具包——`publish/exttool/install_pandoc.sh` 下载官方静态发行版 3.12.1（版本和两个架构的 sha256 固定在脚本里，与 GitHub release 的 asset digest 一致），只保留二进制放在 `tools/store/pandoc/bin/`，`tools/bin/` 下的 `pandoc`、`pandoc-lua`、`pandoc-server` 都是指向它的相对符号链接（`tools/bin` 本来就在 Worker 的 PATH 上）。`build_exttool` 把脚本复制进构建上下文。`paios容器需求.md` §6.3 和 `NewOpenDANRuntime.md` 的 bin 分层表已同步。ExtTool 不可用（D6）时 Worker 照常启动，只是没有 pandoc。注意 `publish/*` 在 `.gitignore` 里，新脚本要 `git add -f`。
- A7：`libopendan` 的 `src/bin/xagent.rs` 主体移到库模块 `xagent_cli`（入口 `xagent_cli::main()`，`xagent` 二进制只剩一行），`agent_tool_cli_dev` 增加 `main()` 和 `handles_invocation_name()`；`opendan` 的 `main()` 先按 argv[0] 的文件名分派：`xagent` → xagent，`agent_tool` 或 agent_tool 按名提供的工具 → agent_tool，其他 → 加载器。镜像里只有 `opendan` 一个文件，`agent_tool`、`xagent` 是指向它的符号链接；`build_aios` 只编译 `-p opendan`。因为 Linux 的 `current_exe()` 会解析符号链接，独立运行的 xagent 给会话生成 `agent-session` 包装脚本时改为优先用同目录的 `xagent`（加载器本来就用 `beside_exe("xagent")`）。主机 payload 里的三个二进制不变（Windows 上不适合用符号链接）。
- A6 + A7 的大小（本机 `local/aios-slim` 对比 A1–A5 后的 CI 镜像，amd64）：aios 解压后 1,068 MB → 820 MB（二进制 82.6 MB → 35.6 MB，pandoc 约 200 MB）；`docker save | gzip -6` 370 MB → 313 MB。exttool 33.6 MB → 199 MB，gzip -6 10.9 MB → 46 MB。两个 tar 合计 381 MB → 359 MB：A7 省约 18 MB 压缩后体积；A6 主要是让 aios 镜像变小，总量只少约 3 MB（官方静态 pandoc 158 MB，Debian 的 pandoc 加数据包 196 MB）。
- A6 + A7 验证：`cargo test -p agent_tool_cli_dev -p opendan -p libopendan` 416 项通过（新增调用名分派的单元测试）。断网容器冒烟：模拟 node_daemon 播种 ExtTool 卷后挂进 aios 容器，pandoc 3.12.1 完成 md→docx→gfm、html、epub、pptx 转换，`pandoc-lua` 执行 Lua，`pandoc-server --help` 正常；ffmpeg、imagemagick、pdftotext、tesseract、node 24、deno、`uv venv --offline` 照常可用；`xagent new --msg` 在容器里建会话并驱动（无 zone，模型调用按预期失败），会话 `.runtime/bin/agent-session` 内容是 `exec "/opt/buckyos/bin/opendan/xagent"`，实际调用 `agent-session sessions` 和 `llm_understand_media --help` 都由正确的入口处理；`opendan --dev` 启动后 HTTP 200、WebUI 可访问，进程名仍是 `opendan`（`stop.py` 按名字杀 `xagent` / `agent_tool` 不受影响）。没有在真实 zone 上做 DV：当时本机 zone 正被使用（07:34 刚启动），没有打断它。
- 全量包估算（amd64，D5 + A1–A5 之后）：payload 约 170–210 MB + aios tar 约 370–410 MB + exttool tar 11 MB ≈ 0.6 GB，Windows 安装包远低于 NSIS 的 2 GB 上限（§9.1：总数据超过 2 GiB 时 makensis 直接崩溃）。D10 的 zip 兜底方案不需要。
