# BuckyOS 企业部署指南

本文面向企业运维、平台工程和应用交付团队，介绍在企业自有或托管的 Linux 服务器上更新 BuckyOS、部署应用、管理配置与访问权限，以及迁移数据和验收发布的流程。公网、企业内网和通过 VPN 访问的环境均可采用，网络入口按实际部署调整。

本文对应仓库当前 Beta 2.2 实现。应用通过标准 `.pikg` 包交付，不依赖特定企业、业务系统或私有安装脚本。测试与生产使用同一套流程，但分别配置服务器、Zone、身份、访问策略和业务参数；测试通过后，将同一份已验证的构建产物发布到生产。

以下操作以**已安装并激活 BuckyOS 的环境**为起点。首次部署系统时，先按仓库 [入门说明](../../README.md#getting-started) 完成依赖准备、系统安装和 Zone 激活，再配置企业域名、网关与服务托管。源码启动不会自动注册系统服务；文中的 `buckyos.service` 示例要求目标机已配置该服务。首次初始化命令不能作为已有环境的日常更新命令使用。

## 1. 部署前准备

### 环境与交付物

- **目标环境**：确认服务器系统、CPU 架构、BuckyOS 版本、实际运行根目录和服务管理方式。运行根目录通常是 `/opt/buckyos`；容器应用还需具备与包要求匹配的容器运行环境。
- **网络入口**：确认 Zone 的域名解析、网关路由和证书可用。SSH 地址用于系统文件部署，Zone API 入口用于应用管理，两者分别配置。公网入口使用 HTTPS；企业内网或 VPN 环境应能从运维终端访问目标网关。
- **身份与归属**：准备有应用管理权限的目标 Zone 运维身份，并明确应用安装归属的用户。应用发布者 DID、安装 owner 和执行部署的运维身份是不同概念，不应混用。
- **构建环境**：系统源码构建依赖见 [README](../../README.md#getting-started)。应用使用自身项目的构建工具与锁文件；本文以 Node.js、pnpm 和项目内的 `buckyos` npm 包为例。只有更新系统文件或通过 SSH 上传包时才需要 `ssh` / `scp`。
- **发布记录与备份**：记录系统版本、CLI 版本、应用版本、包摘要、目标环境和安装归属。系统更新、配置变更或数据迁移前，保存可恢复的配置、身份材料和业务数据备份，并明确停机窗口与恢复负责人。

普通运维身份位于 `~/.buckyos`。`~/.buckycli` 中的开发身份只有在目标 Zone 开启开发模式时才能使用，不应作为企业生产部署的前置条件。

`buckyos-devkit` 的 `buckyos-build` / `buckyos-install` 负责系统工程构建和文件部署；`buckyos-tool` 的命令名是 `buckyos`，负责 PIKG 打包、应用安装、状态查询和生命周期管理。应用安装语法为 `buckyos app install`。

### 示例参数

文中的 `example.com` 域名、`192.0.2.10` 地址、DID、用户名、版本和文件路径均为占位示例，执行前必须替换为企业实际值。测试和生产应分别保存参数，切换环境后重新核对目标与身份。

| 参数 | 示例 | 用途 |
| --- | --- | --- |
| `TARGET_SERVER_IP` | `192.0.2.10` | 需要 SSH 操作时使用的服务器地址 |
| `TARGET_SSH_USER` | `deploy` | 有相应文件和服务管理权限的 SSH 用户 |
| `TARGET_ZONE` | `corp.example.com` | 已激活的目标 Zone |
| `TARGET_ENDPOINT` | `https://corp.example.com` | 运维终端可访问的 Zone 网关入口 |
| `TARGET_IDENTITY` | `did:bns:operator` | 已配置到运维终端、获目标 Zone 授权的身份 |
| `APP_DID` | `did:bns:portal.example` | 从应用包元数据取得的应用 DID |
| `APP_OWNER` | `appowner` | 目标 Zone 中的应用安装归属用户 |
| `APP_PIKG` | `/path/to/release/portal-1.0.0.pikg` | 本次发布的已验证应用包 |

## 2. 按需更新 BuckyOS

仅发布应用时跳过本节。系统二进制、配套 Deno 和容器镜像的系统及 CPU 架构必须与目标节点匹配；`--remote` 只负责远程复制，不会自动交叉编译。

### 构建系统发布产物

在构建机的 `buckyos/src` 目录执行，`uv` 使用上级工程声明的 devkit 依赖。发布构建可显式提供同一批 SDK/Tool 发行物的四个绝对路径：npm 包、发布清单、Deno 可执行文件和 SBOM。清单中的版本及摘要必须与实际文件一致。

```bash
cd /path/to/project/buckyos/src
export BUCKYOS_SDK_TOOL_ARTIFACT=/path/to/release/buckyos.tgz
export BUCKYOS_SDK_TOOL_RELEASE_MANIFEST=/path/to/release/release-manifest.json
export BUCKYOS_SDK_TOOL_DENO=/path/to/release/deno
export BUCKYOS_SDK_TOOL_SBOM=/path/to/release/sbom.cdx.json

# Linux x86_64；ARM64 使用 aarch64，并准备对应架构的产物及工具链。
uv run buckyos-build.py amd64
```

未指定预构建产物时，当前入口会从同级 `buckyos-websdk` 源码构建 SDK/Tool；相关依赖及路径配置见 [源码构建说明](../../src/readme.md)。选择预构建产物模式后，四项输入必须齐全。构建入口将系统版 Tool 和各模块组装到 `rootfs`，不要绕过入口而遗漏配套 Tool。

若本次需要更新 `cyfs-gateway`，应在其工程中另行构建和部署；`buckyos-install` 不会执行 `publish` 配置中的依赖打包流程。将系统和网关更新纳入同一份发布与恢复计划。

### 更新目标服务器

以下示例在构建机执行，适用于目标机已配置 `buckyos.service`、安装目录为 `/opt/buckyos`，且 SSH 用户支持免密 sudo 的环境：

```bash
TARGET_SERVER_IP=192.0.2.10
TARGET_SSH_USER=deploy

ssh "${TARGET_SSH_USER}@${TARGET_SERVER_IP}" 'sudo -n systemctl stop buckyos.service'
```

确认服务及旧系统进程已停止，目标目录是现有安装目录且非空，并完成备份后，再执行文件更新。下面的命令仅在安装成功后启动服务：

```bash
uv run buckyos-install --remote "$TARGET_SERVER_IP" \
  --ssh-user "$TARGET_SSH_USER" --sudo --target-rootfs /opt/buckyos &&
ssh "${TARGET_SSH_USER}@${TARGET_SERVER_IP}" 'sudo -n systemctl start buckyos.service' &&
ssh "${TARGET_SSH_USER}@${TARGET_SERVER_IP}" 'sudo -n systemctl is-active buckyos.service'
```

以 root 连接时可去掉 `--sudo` 和服务命令中的 `sudo -n`。安装连接参数支持 `--ssh-port`、`--ssh-key` 和可重复的 `--ssh-option`；单独执行的 SSH/SCP 命令也应配置相同的端口、密钥和连接选项。使用其他根目录或服务管理方式时，按实际环境调整。

安装命令的行为需要注意：

- 默认按 `bucky_project.yaml` 的 `apps.*.modules` 覆盖文件，包含二进制及部分 `etc` 文件；部署前检查是否涉及企业定制配置。
- 安装不会自动停止、启动或重启 BuckyOS，也不是原子切换。复制失败时保持服务停止，完成修复或恢复一致的旧版本后再启动。
- 日常更新不使用 `--all` 或 `reinstall`：它们会执行清理、数据初始化和模块安装，可能删除身份、配置及运行数据。目标目录不存在或为空时，默认更新也会进入重装路径。

启动后确认 Zone API、核心服务和网关可用。`systemctl is-active` 只代表 systemd 单元处于运行状态，不能代替服务健康检查。

## 3. 构建与验证应用包

应用开发和首次生成 `dapp_meta` 的方法见 [应用开发指南](app-dev-quickstart.md)。应用项目应锁定与目标系统匹配的 `buckyos` SDK/CLI 版本，并提交依赖锁文件。

打包前更新 `dapp_meta/app.json` 中的应用版本及 `dapp_meta/pikg.json` 中的产物来源、镜像版本和包文件名，核对其他元数据中的版本引用。包版本以元数据为准，不能只修改根 `package.json` 或重命名旧包。正常升级保持应用 DID 不变。

以下为使用 pnpm、已有 `dapp_meta` 的应用示例，在构建机执行；应用源码构建命令按项目替换：

```bash
cd /path/to/enterprise-app
pnpm install --frozen-lockfile
npx --no -- buckyos --version --verbose
pnpm run build

# 先完成应用编译；Docker 应用还需构建元数据引用的目标架构镜像。
npx --no -- buckyos pikg build ./dapp_meta
npx --no -- buckyos pikg pack ./dapp_dist

APP_PIKG=./dapp_dist/portal-1.0.0.pikg
npx --no -- buckyos pikg info "$APP_PIKG"
sha256sum "$APP_PIKG"
```

若项目构建脚本已包含 PIKG 构建与打包，不必重复执行。`pikg` 消费已有构建产物，不替应用执行源码或容器镜像构建。`--version --verbose` 的 `executable` 应指向项目内 CLI；`npx --no --` 用于避免临时下载其他版本。macOS 可用 `shasum -a 256` 计算摘要。

在测试 Zone 安装该包，完成自动化测试、浏览器验收和关键业务流程验证。涉及数据库迁移或启动校验时，使用按企业数据管理要求准备的副本演练升级及恢复，不直接操作生产数据。通过后保存包、摘要、元数据和验证记录，向生产交付**同一个 PIKG 文件**，不重新构建。

## 4. 在目标 Zone 安装或更新应用

### 选择 CLI 执行位置

可以在具备身份和网络访问能力的运维终端直接安装本地 PIKG，Tool 会将包暂存到目标服务，无需先复制到服务器。下面以已安装项目依赖的运维终端为例：

```bash
cd /path/to/enterprise-app
APP_PIKG=/path/to/release/portal-1.0.0.pikg
TARGET_ZONE=corp.example.com
TARGET_ENDPOINT=https://corp.example.com
TARGET_IDENTITY=did:bns:operator
APP_DID=did:bns:portal.example
APP_OWNER=appowner
INSTALL_POLICY=normal

bucky() {
  npx --no -- buckyos --zone "$TARGET_ZONE" \
    --endpoint "$TARGET_ENDPOINT" --identity "$TARGET_IDENTITY" "$@"
}

bucky --version --verbose
bucky auth whoami
bucky app list
bucky pikg info "$APP_PIKG"
sha256sum "$APP_PIKG"
```

核对目标 Zone、身份、应用 DID、安装归属和包摘要后继续。`--zone`、`--endpoint`、`--identity`、`--yes` 等全局参数放在模块名前，也可用预先配置的 `--profile` 选择环境。

如需在目标机执行，可先用 SCP 上传同一份 PIKG，再将函数中的 `npx --no -- buckyos` 替换为系统配套的 `/opt/buckyos/bin/buckyos`，重新设置目标机上的包路径和身份。系统版 Tool 自带 Deno，无需安装 npm CLI；仅上传 `.pikg` 不会提供 npm CLI。目标机本地网关入口可按实际配置使用 `http://127.0.0.1:3180`，该地址不能在运维终端上直接代表远程服务器。

两种 CLI 发行方式不会互相更新。部署前核对实际执行文件和版本，避免因旧 CLI 与 Zone 协议不匹配导致安装失败。

### 选择安装信任策略

示例使用 `normal`。企业应准备可通过所选策略校验的 AppDoc、发布身份和名称解析证据；本地包摘要校验成功不代表发布身份已获得目标 Zone 的信任。

`local-developer` 仅用于目标 Installer 明确允许的开发测试场景，并需显式指定。正式发布校验失败时，应修复发布材料或信任配置，不直接降级为开发策略。安装策略、CLI 开发身份和 Zone 开发模式是不同机制。

### 首次安装

在 `app list` 中确认目标归属下尚无该应用后，生成并检查安装计划：

```bash
bucky app fetch "$APP_PIKG" --owner "$APP_OWNER" \
  --policy "$INSTALL_POLICY" --plan ./app.install-plan.json
bucky app install "$APP_PIKG" --policy "$INSTALL_POLICY" \
  --plan ./app.install-plan.json --dry-run
```

核对计划中的 owner、部署目标、卷、服务和安装参数。出现 `PLAN_INPUT_REQUIRED` 时，按 `bucky command describe app fetch` 的 schema 提供 `target` / `install_params`，通过全局 `--input` 重新生成计划。计划不应包含明文密码、令牌或私钥，使用协议支持的密钥引用。

预检符合预期后提交：

```bash
bucky --yes app install "$APP_PIKG" --policy "$INSTALL_POLICY" \
  --plan ./app.install-plan.json
bucky app get "$APP_DID"
bucky app status "$APP_DID"
```

首次安装缺少计划会返回 `PLAN_REQUIRED`。计划绑定本次包、目标和配置，各环境分别生成；出现 `PLAN_STALE` 时重新获取并检查计划，不手改指纹。首次安装计划不能覆盖已安装应用。

### 更新已有应用

使用新版本 PIKG 更新，不带首次安装计划：

```bash
bucky app install "$APP_PIKG" --policy "$INSTALL_POLICY" --dry-run
```

检查预检结果的安装归属、目标实例和版本后再执行：

```bash
bucky --yes app install "$APP_PIKG" --policy "$INSTALL_POLICY"
bucky app get "$APP_DID"
bucky app status "$APP_DID"
```

`app install <PIKG>` 会识别已有安装并进入升级流程，沿用已有安装归属。`app upgrade` 查询 Catalog 更新，不用于指定本次交付的 PIKG。不要依赖相同版本包强制重装，每次正常发布应更新版本号。同一应用存在多个 owner 的安装时，应先从 `app list` / `app get` 核实实际实例，不能把应用 DID 当作唯一安装实例。

安装默认等待任务完成。使用 `--no-wait` 时保留任务 ID，通过 `bucky task get <task_id>` / `bucky task wait <task_id>` 跟踪。超时不代表服务端任务已取消，应先查询状态再决定是否重试。

## 5. 配置与用户访问权限

### 企业业务配置

应用包与环境配置分别管理。首次安装、更换应用 DID 或安装归属后，按应用公开的配置说明初始化目标环境；不要假定 PIKG 内含企业业务参数，也不要直接复用测试环境的密钥和外部服务账号。

使用 system-config 保存应用设置的项目，常见路径为 `users/{owner}/apps/{appId}/settings`。实际键名与默认值以应用实现为准；其中 `appId` 是规范 App ID，不是带 `did:` 前缀的 DID。企业域名、回调地址、数据库连接、第三方服务及模型服务配置应逐项核对。

优先使用应用提供的管理界面或配置工具。通过 `system-config` 修改 JSON 文档时，先读取并备份现值，在保留未修改字段的基础上合并变更，再校验、写回和回读确认。`system-config set` / `set-file` 写入的是整个键值，不是 JSON 字段级合并；协调同一配置的并发修改，避免覆盖他人的更新。密钥通过受控渠道或密钥引用提供，不写入源码、示例配置或发布日志。

若应用在启动时读取配置，变更后需重启并检查日志及健康状态；若支持动态加载，则按应用说明验证生效。配置备份与业务数据备份分别维护。

### 授权企业用户访问

应用用户访问策略独立于 `dapp_meta/app.json` 的 `permissions`；后者声明应用需要的系统权限，不能代替用户授权。首次安装、更换 App ID 或安装归属后，应检查应用实例的访问策略。根据企业需求开放指定用户或用户组，不默认向访客开放。

以应用 owner 的用户身份登录 Control Panel，读取当前可用性策略后修改。通过 API 管理时，使用有效的 Control Panel 用户会话，以标准 kRPC 封装向 `/kapi/control-panel` 调用：

1. `apps.availability.get`：传入实际 `app_instance_id`，保存现有策略及 revision。实例 ID 从安装结果或 `app get` 获取，例如 `portal.example.bns.did@appowner`。
2. `apps.availability.set`：提交完整规则及刚读取的 `expected_revision`。以下仅演示允许 `users` 组，需替换实例 ID 和 revision，并保留企业仍需使用的其他规则。

```json
{
  "app_instance_id": "portal.example.bns.did@appowner",
  "expected_revision": 1,
  "group_rules": [
    { "group_id": "users", "effect": "allow" }
  ],
  "user_rules": []
}
```

`group_rules` 和 `user_rules` 是完整替换；不要把示例空数组用于覆盖已有的用户规则。`users` 面向普通用户，若只允许部分员工使用，应配置对应用户或用户组。只有业务明确需要匿名访问时才配置 `guest`；仅授权 `guest` 不会同时授权已登录用户。

服务端要求应用 owner 的 Control Panel 用户会话来修改可用性策略，普通运维 CLI 身份或管理员身份不能直接替代该会话。服务端负责版本检查和审计，不直接改写 system-config 中的访问策略。

修改后调用 `apps.availability.check`，传入实例 ID 和待验证的 `user_id`，再分别用应允许和应拒绝的账号验证应用入口与实际访问。访问策略调整无需重装应用。

## 6. 数据迁移与恢复

仅升级代码通常不需要复制业务数据。需要迁移时，将数据操作安排在应用安装或更新完成后的独立停机窗口，不假定升级过程会持续保持停机。

```bash
bucky app stop "$APP_DID"
bucky app status "$APP_DID"
```

确认所有相关实例、容器、后台任务及其他写入者已停止后，再备份和复制数据。不能只依赖 stop 任务返回成功，也不要只杀进程而让调度器重新拉起应用。

1. **确认数据范围**：根据安装计划、实际挂载和应用配置定位持久目录、数据库、附件及对象存储。目录约定见 [运行时路径说明](../path_usage.md)，应用自定义目录以实际配置为准；包解压目录、镜像层和仓库种子数据不能代替持久数据。
2. **取得一致备份**：保留目标数据备份，源端停写或使用数据库支持的一致性备份方式。SQLite 应使用一致性备份或在停写后完整处理数据库及相关 WAL 文件，恢复时避免与目标旧 WAL/SHM 混用；其他数据库使用对应的备份恢复机制。
3. **恢复业务数据与配置**：按应用的数据集边界迁移数据库、文件和附件，恢复所需属主与权限。system-config 设置、密钥和外部服务配置不会随应用数据目录自动迁移，应按目标环境单独恢复与核对。测试演练的数据副本按企业要求脱敏。
4. **校验版本兼容性**：确认数据 schema 与将启动的版本兼容。新版本启动可能自动迁移数据库；恢复旧代码不等于恢复旧数据，应事先演练对应的恢复路径。

完成数据检查后启动应用：

```bash
bucky app start "$APP_DID"
bucky app status "$APP_DID"
```

发生故障时，先保留日志、安装任务 ID、发布产物和备份。按已演练的流程恢复兼容的系统或应用版本、配置和数据；不要假定安装旧版本 PIKG 就能完成降级。明确恢复点之后新增数据的处理方式，再恢复流量及后台任务。

## 7. 发布验收

将以下结果纳入本次发布记录：

- **系统与应用**：Zone API、核心服务、网关、应用实例状态正常，实际运行版本及包摘要与发布记录一致，启动日志无未处理的迁移或配置错误。
- **网络与身份**：通过目标环境实际域名验证 DNS、HTTPS、登录和页面/API 访问，确认普通员工、指定用户组及未授权用户的行为符合企业策略。
- **业务与集成**：完成关键业务流程，验证所用外部服务、回调和后台任务。应用提供健康端点时检查其就绪状态，具体路径及成功条件以应用说明为准。
- **数据与恢复**：迁移后核对关键记录、文件及附件，确认备份、恢复步骤和运行监控可用。

任一关键验收失败时，保留现场并按恢复计划处理。测试环境通过后，生产使用同一构建产物，重新生成目标环境的安装计划并核对配置与访问策略；已有安装按升级流程执行。

## 参考资料

- [BuckyOS 入门与系统初始化](../../README.md#getting-started)、[SDK/CLI 构建说明](../../src/readme.md)
- [应用开发与 PIKG 打包](app-dev-quickstart.md)、[PIKG 示例](../../test/app_installer_test/pikg_samples/README.md)
- [系统构建入口](../../src/buckyos-build.py)、[系统安装目录规则](../../src/bucky_project.yaml)、[运行时路径说明](../path_usage.md)
- [Control Panel 服务与应用管理 API](../control_panel/Control_Panel_Service.md)
- [BuckyOS Devkit](https://github.com/buckyos/buckyos-devkit)、[SDK/CLI 源码与命令说明](https://github.com/buckyos/buckyos-websdk/tree/main/cli)

命令参数以部署所用版本的 `buckyos command describe <module> <verb>` 为准。
