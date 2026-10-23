# OpenDAN 默认会话链接的机器配置缺失（#681）

`DID::to_host_name()` 按进程内的 `web3_bridge` 映射构造域名。OpenDAN 使用 AppService runtime，
此前 runtime 跳过 `machine.json`，容器也未映射该文件，因此空 bridge 表让 BNS DID 落到 `.bns.did`。
宿主 node-daemon 已读取机器配置，不能由宿主转换正常推断容器内同样正常。

修复在普通 Docker App 和 Agent/Script Worker 中默认只读映射宿主 `etc/`，并让 AppService 读取
`machine.json` 机器策略。缺文件使用默认配置，配置格式错误拒绝启动。App 身份仍通过原有环境变量
初始化。`start_config.json` 包含初始化凭据，迁至 security 根；开发 CA 私钥、包含用户私钥和本地
provider token 的开发 boot 模板也移入 security，容器不挂载该目录。生成器保留公开基础 boot 模板，
scheduler 优先读取 security 中的完整私有模板。激活、Control Panel、备份恢复及 SDK 独立激活入口
均同步新路径，provision 的 seed 输入布局不变。旧安装遗留 start_config、CA key 或含 PEM 私钥的
基础模板会阻断实际容器挂载，不自动迁移；命令预览不执行此校验。须迁走凭据并恢复公开模板后部署。
OpenDAN 的 `to_host_name()` 和 MessageHub 的 Agent DID 参数保持原有语义。

回归检查覆盖三类容器的文件挂载与预览、缺文件行为、自定义 bridge 下的 DID 转换以及 App 身份隔离。
部署需更新 node-daemon、重建 aios 镜像并 recreate 已有容器；`docker restart` 不会添加新挂载。
真实浏览器的 DNS、登录和 MessageHub 跳转仍需在受影响 Zone 验收。

同步手册：`buckyos_admin`、`opendan_agent_dev`、`buckyos_dev_release`。`ui_core` 的会话命令、权限和路径
未改变，`buckyos_app_dev` 的打包和 API 使用方式未改变，无需修改。未修改协议、共享类型或依赖声明。

## 第一轮验证（machine.json 单文件挂载）

- `BUCKYOS_MACHINE_CONFIG_DOCKER_IMAGE=alpine:latest cargo test -p buckyos-api -p node_daemon -- --test-threads=1`：
  buckyos-api 的 305 个单元与集成测试通过，包括新增的机器策略回归；三种真实 Docker 挂载验证通过。
  node-daemon 为 128 通过、2 失败、2 忽略，失败分别为现有测试硬编码 `/bin/true`（macOS 无此路径）
  和临时目录符号链接导致的路径比较。
- 使用 `TMPDIR=/private/tmp` 重新执行已编译 node-daemon 测试程序的 `test_app_loader` 测试组：
  42 个通过，包含原先失败的挂载边界测试及三种真实 Docker 只读挂载验证。
- 首次编译因本地忽略的 Cargo.lock 锁定旧版 buckyos-base、缺少 `RPCErrorDetail` 失败；
  `cargo update -p kRPC` 更新既有 `main` 分支依赖后通过编译。未修改依赖声明或加入新依赖。
- `TMPDIR=/private/tmp cargo test -p opendan -- --test-threads=1`：40 个通过、1 个忽略。
  首轮使用系统临时目录时，现有 `agent_state_over_krpc_matches_the_files` 用例受同一 macOS 路径比较问题影响。
- `TMPDIR=/private/tmp cargo test -p libopendan --test xagent_lib work_sessions_take -- --test-threads=1` 和
  `TMPDIR=/private/tmp cargo test -p libopendan --test xagent_cli skills_are -- --test-threads=1`：均通过。
- `./build_aios --image local/aios-issue681 --arch aarch64 --reuse-cargo-target`：成功，包含 WebUI 构建、
  Linux ARM64 Release 编译、镜像封装和二进制校验。镜像为 `local/aios-issue681:latest-aarch64`；
  `docker run --rm --network none --entrypoint /opt/buckyos/bin/opendan/opendan local/aios-issue681:latest-aarch64 --help`
  成功。未修改运行环境的镜像覆盖或重启 Zone；未实测 Windows、Linux amd64、远端浏览器跳转，未跑全 workspace 测试。

## 最终验证（etc 目录挂载与 security 布局）

- `TMPDIR=/private/tmp BUCKYOS_MACHINE_CONFIG_DOCKER_IMAGE=alpine:latest cargo test -p node_daemon test_app_loader -- --test-threads=1`：
  42 通过。包含三类真实 Docker 容器读取机器配置、拒绝写入 etc、无法读取 security，以及旧敏感文件阻断。
- `cargo test -p buckyos-api --test app_service_runtime -- --test-threads=1`：1 通过，验证 bridge 转换与 App 环境身份。
- `cargo test -p scheduler test_boot_schedule -- --test-threads=1`：1 通过，覆盖 security 启动参数、公开基础模板与私有模板优先。
- `cargo check -p node_daemon -p control_panel`：通过。
- 已编译 node-daemon 测试程序的 `active_server::` 测试组：11 通过、1 忽略。
- `uv run python -m unittest discover -s tools/tests -p 'test_*.py'`：46 通过，包含默认和自定义 security 根的备份恢复。
- `deno test --allow-all --no-lock --import-map <local-sdk-map.json> make_config_test.ts active_test.ts`：9 通过。
  临时 import map 将 `buckyos/provision` 指向本地 `buckyos-websdk/dist/provision.mjs`；默认远端 main bundle
  缺少已有 `hashAdminPassword` 导出，未用其通过验证，未修改仓库依赖地址。
- 联动 `buckyos-websdk` 的 provision activation 源码、测试、文档及 dist：Jest 10 通过、provision CLI 5 通过，
  `pnpm run build` 成功。`cyfs-gateway/src/make_sn_config_test.ts` 同步 OOD runtime 路径，使用本地 SDK 的 BNS seed
  定向测试通过；默认远端 SDK 下完整套件 8/14，其余 6 项被同一缺失导出阻断。
- 三个仓库 `git diff --check` 通过。变更尚未提交、推送或部署；未移动已安装环境的文件。
  最终变更未重跑全 workspace 或完整 BuckyOS 构建，未进行真实 Zone 激活和浏览器 MessageHub 验收。
  前轮 ARM64 镜像构建验证仍覆盖本次未再更改的 AppService runtime；新增路径和挂载由宿主侧实现。
