# 第三方应用开发：从 HTTP Server 到本地 DV 测试

给开发者和 Coding Agent 的最短路径：**安装 SDK → 编写应用 → 打包 PIKG → 安装到 DV → 改代码、升版本、重新打包升级。** 适用于 Beta 2.2；应用包扩展名为 `.pikg`。

## 1. 在应用项目中安装 SDK

准备 Node.js ≥ 22.13、npm 和可用的 Docker。以下应用命令均在你自己的项目根目录执行。

```bash
npm install buckyos
npx buckyos --version
npx buckyos command describe pikg init
```

SDK 与 CLI 来自同一个 `buckyos` npm 包；提交项目 lockfile。已有 TypeScript 项目继续使用原来的框架、构建命令和目录结构。

## 2. 把 TypeScript HTTP Server 打包成 PIKG

照常编写 HTTP Server，用 Dockerfile 将 Node runtime、编译后的 JS 和运行依赖装进镜像。服务监听 `0.0.0.0:3000`，镜像启动命令直接运行服务。Docker 镜像的 Linux 架构须与 DV 节点匹配。

```bash
npm run build
docker build -t local/app1:0.1.0 .
npx buckyos pikg init . --name app1 --owner did:bns:devtest --kind docker --source local/app1:0.1.0 --version 0.1.0
```

`pikg init` 只生成打包元数据。将 `dapp_meta/app.json` 的 `service_config_tips` 设为下面的 HTTP 端点声明；端口须与 Server 一致：

```json
{
  "service_endpoints": {
    "www": {
      "protocol": "http",
      "inner_port": 3000,
      "required": true,
      "expose": { "route": { "type": "web" }, "scope": "", "allow_guest": false }
    }
  }
}
```

```bash
npx buckyos pikg build ./dapp_meta
npx buckyos pikg pack ./dapp_dist
npx buckyos pikg info ./dapp_dist/app1-0.1.0.pikg
```

提交 `dapp_meta/`；将 `dapp_dist/` 加入 `.gitignore`。SDK 工具消费已构建的镜像，不替应用执行 TypeScript 或 Docker 构建。`did:bns:devtest` 仅用于此本地测试示例，正式发行应使用自己的 Owner DID。

## 3. 从源码启动本地 BuckyOS DV

系统环境有两条路径：使用 nightly 系统安装包，并完成激活和测试配置；或从源码构建、直接生成预设的 DV 配置。**快速迭代阶段推荐源码 DV**。npm 的 `buckyos` 是 SDK/CLI，不是 BuckyOS 系统安装包。

以下为 macOS/Linux shell 示例；使用专门的开发机或 VM，不与正式运行的 BuckyOS 共用运行目录。先按仓库 [README](../../README.md#getting-started) 和 [devenv.py](../../devenv.py) 准备 Rust、Python 3.12、uv、Node/pnpm、Deno ≥ 2.2、tmux、Docker 等构建依赖。两个仓库放在同一父目录：

```bash
git clone https://github.com/buckyos/cyfs-gateway.git
git clone https://github.com/buckyos/buckyos.git
cd cyfs-gateway/src
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git" buckyos-build
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git" buckyos-install --all
cd ../../buckyos/src
uv run tools/prepare_sdk_tool_distribution.py --work-dir ../.cache/sdk-tool
uv run buckyos-build --app=buckyos
uv run start.py --all
uv run check.py
```

这里复用 CI 的“准备 SDK/Tool 发行物 → devkit 构建”路径。直接运行 `buckyos-build.py` 则必须先提供它要求的四个 `BUCKYOS_SDK_TOOL_*` 输入，不能省略。

`--all` 使用 `dev` 配置：Zone 为 `test.buckyos.io`，Owner 为 `devtest`，无需手工激活。检查须显示已激活且核心服务可用。该域名及通配子域名用于回环测试，预期解析到 `127.0.0.1`；在开发机浏览器访问 `http://test.buckyos.io`。若在 VM 中运行，应在 VM 内访问，或另行配置宿主机解析和转发。

环境维护命令均在 `buckyos/src` 执行：

| 目的 | 命令 / 操作 |
| --- | --- |
| 只重启，不更新文件 | `uv run start.py --skip-update` |
| 用上一次构建结果更新系统并重启 | `uv run start.py` |
| 更新 BuckyOS 源码后的运行环境 | 停止环境，更新源码，重跑上面的发行物准备和构建，再执行 `uv run start.py` |
| 重新生成 DV 配置 | `uv run start.py --all`，会重建配置和运行状态，但保留 `data/home`、`data/srv`、`storage` 等持久数据 |
| 得到完全干净的 BuckyOS 数据环境 | `uv run stop.py` 后，将专用测试运行目录整体移到备份位置，再执行 `uv run start.py --all`；不是仅执行 `--all` |

运行目录默认为 `/opt/buckyos`，以 `BUCKYOS_ROOT` 和实际进程路径为准。彻底重建后应用需重新安装；Docker 镜像缓存不会因此清空。若进程由系统服务托管，先停止对应系统服务，避免自动拉起。

## 4. 把 PIKG 安装到 DV

回到应用项目，配置一个显式指向 DV 的 CLI profile：

```bash
npx buckyos config set zone --value test.buckyos.io --profile-name dv
npx buckyos config set default_protocol --value http:// --profile-name dv
```

Coding Agent 使用下面的 Node 片段获取 DV 会话，保存到本机文件；将 `.dv-session-token` 加入 `.gitignore`。这是源码 `dev` 配置的预设账号 `devtest` / `bucky2025`，只用于本地 DV；会话过期或环境重建后重新执行。

```bash
node --input-type=module <<'JS'
import { writeFileSync } from 'node:fs';
import { buckyos, VerifyHubClient, createAppInstanceId } from 'buckyos/node';
const client = new VerifyHubClient(new buckyos.kRPCClient('http://test.buckyos.io/kapi/verify-hub'));
const nonce = Date.now();
client.setSeq(nonce);
const response = await client.loginByPassword({
  username: 'devtest', password: buckyos.hashPassword('devtest', 'bucky2025', nonce),
  login_nonce: nonce, target: { kind: 'app', app_instance_id: createAppInstanceId('buckycli', 'devtest') }
});
const { session_token } = VerifyHubClient.normalizeLoginResponse(response);
if (!session_token) throw new Error('DV login failed');
writeFileSync('.dv-session-token', session_token, { mode: 0o600 });
JS
npx buckyos --profile dv --session-token-file ./.dv-session-token auth whoami
npx buckyos --profile dv --session-token-file ./.dv-session-token --non-interactive app fetch ./dapp_dist/app1-0.1.0.pikg --plan ./app1.install-plan.json --policy local-developer
npx buckyos --profile dv --session-token-file ./.dv-session-token --non-interactive --yes app install ./dapp_dist/app1-0.1.0.pikg --plan ./app1.install-plan.json --policy local-developer
npx buckyos --profile dv --session-token-file ./.dv-session-token app status app1
```

首次安装需要 `fetch` 生成的计划；若返回 `PLAN_INPUT_REQUIRED`，按命令 schema 用 `--input` 补齐目标和配置后重新生成。未签名开发包必须显式使用 `local-developer`，且目标 Installer 允许此权限；它不代表正式发布。

安装命令默认等待任务完成。正常情况下，Zone Owner 安装的 `app1` 可通过 **`http://app1.test.buckyos.io`** 访问；先在浏览器登录 DV。实际域名以安装结果的分配为准：命名冲突或其他用户安装可能带后缀，同一应用实例升级时保留分配。验证页面/API 确实来自当前版本。

## 5. 日常循环：改代码 → 打包新版本 → 升级

将 `dapp_meta/app.json.version` 改为 `0.1.1`；将 `pikg.json` 中镜像 source 改为 `local/app1:0.1.1`，若有显式 `pikg_file` 也同步修改。保持 App DID 不变。

```bash
npm run build
docker build -t local/app1:0.1.1 .
npx buckyos pikg build ./dapp_meta
npx buckyos pikg pack ./dapp_dist
npx buckyos pikg info ./dapp_dist/app1-0.1.1.pikg
npx buckyos --profile dv --session-token-file ./.dv-session-token --non-interactive --yes app install ./dapp_dist/app1-0.1.1.pikg --policy local-developer
npx buckyos --profile dv --session-token-file ./.dv-session-token app status app1
```

**本地新包升级使用 `app install <新 PIKG>`，不带首次安装的 `--plan`。** `app upgrade` 检查 Catalog 中的已发布版本，不接收本地 PIKG。相同版本不会重装，每次测试新包都要递增版本号。升级保留当前安装设置，可能短暂停机；完成后仍在同一子域名测试新版本和数据保留情况。

只改应用无需重新构建或重置 BuckyOS。任务等待超时可用 `npx buckyos --profile dv --session-token-file ./.dv-session-token task wait <task-id>` 续等；系统异常先跑 `uv run check.py`，日志在 `$BUCKYOS_ROOT/logs`。

## 给 Coding Agent 的参考入口

以当前源码和 `npx buckyos command describe <module> <verb>` 为准；不要使用旧 AppDoc 示例替代 `dapp_meta` schema。

- [PIKG Docker 样例](../../test/app_installer_test/pikg_samples/docker/)：复用目录和端点声明，业务 Server/Dockerfile 留在应用项目。
- [PIKG 样例说明](../../test/app_installer_test/pikg_samples/README.md) 与 [安装测试](../../test/app_installer_test/README.md)：打包、安装和验收参考。
- [SDK/CLI 源码](https://github.com/buckyos/buckyos-websdk)：`cli/modules/pikg.ts`、`app.ts`、`cli/core/auth.ts`；Server 接入系统能力参考 `examples/app_service_local_debug.ts`。
- [start.py](../../src/start.py)、[DV 配置](../../src/devenv_config.ts)、[安装目录规则](../../src/bucky_project.yaml)、[CI 构建](../../.github/workflows/build-macos.yml)：系统启动、重建和构建的事实来源。
