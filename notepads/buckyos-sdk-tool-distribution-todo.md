# BuckyOS SDK / Tool 双分发与 PIKG 开发闭环 TODO

> 状态：待实施  
> 基线日期：2026-08-29  
> 涉及仓库：`buckyos-websdk`、`buckyos`  
> 目标用户：BuckyOS 系统管理员、App 开发者  
> 相关需求：[`buckyos-websdk/doc/buckyos tool PRD.md`](https://github.com/buckyos/buckyos-websdk/blob/main/doc/buckyos%20tool%20PRD.md)、
> [`buckyos-websdk/doc/modules/pikg.md`](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md)

## 0. 已确认设计决策

1. `buckyos-tool` 成为 `buckyos-websdk`/npm `buckyos` 包的一部分，不再发布独立的
   `@buckyos/cli` 或 `buckyos-tool` npm 包。
2. Tool 只有一套源码、一个 Command Registry 和一套命令协议，但有两种正式分发渠道：
   - **system distribution**：随 BuckyOS 系统安装，在 `$BUCKYOS_ROOT` 固定目录中提供与该
     系统版本配套的 Tool、SDK 和 Deno runtime；
   - **developer distribution**：App 项目通过 `npm install buckyos` 获得与项目 SDK 同版本的
     Tool，并通过 `npx buckyos`、`npm exec buckyos` 或 npm scripts 使用，直接运行在项目
     已有的 Node 上，不额外交付 runtime。
3. CLI 源码从一开始就按 "Deno 兼容子集" 编写，目的就是保证同一套代码在 Node 环境下同样
   可运行。该前提显式化为 `cli/runtime/` host 抽象层：业务代码只依赖 typed host 接口，
   `Deno.*` 与 `node:*` 只允许出现在各自的 host 实现里。developer distribution 使用 Node
   host，system distribution 使用 Deno host；同一命令在两个 host 下必须产生相同的行为、
   输出、错误码和退出码。
4. 系统 Tool 只随 BuckyOS 安装器/updater 更新；项目 Tool 只随 npm dependency 和 lockfile
   更新。两者不得互相覆盖、自更新或隐式替换。
5. 不新增 `buckyos dev` 模块。App 开发者的核心心智模型固定为 PIKG；本地开发辅助、环境检查
   和未来可能增加的测试环境辅助统一放在 `buckyos pikg ...` 下。
6. `pikg` 不接管 App 自己的 Vite/Webpack/Cargo/Docker build script。它消费已经存在的构建
   输出，负责元数据、PIKG 构造、验证、测试发行物和与测试环境衔接。
7. `app install/status`、`task wait`、`log tail` 等线上领域能力继续保留在各自模块；PIKG
   开发闭环可以复用同一内部 client/context 编排这些能力，但不得复制 App、Task 或 Log 协议，
   也不得通过递归执行另一条 CLI 命令实现编排。
8. SDK Tool 可以包含系统运维 Tool 的完整命令超集，但实际可用能力由调用身份、目标服务
   capabilities、runtime/host permission 和 distribution policy 决定，不维护两套裁剪后的
   业务实现。
9. Beta 2.2 是 breaking change；迁移不保留旧 `buckycli` 命令、旧 Tool 路径或旧输出兼容层。

---

## 1. 目标用户体验

### 1.1 系统管理员

安装 BuckyOS 后，无网络、无 npm、无源码 checkout 时始终可以执行配套 Tool：

```bash
$BUCKYOS_ROOT/bin/buckyos --version
$BUCKYOS_ROOT/bin/buckyos system status
```

安装器可以额外创建 PATH 入口：

```bash
buckyos system status
```

裸 `buckyos` 的系统入口必须最终解析到 `$BUCKYOS_ROOT/bin/buckyos`，并显示它所配套的
BuckyOS、SDK、Tool 和协议版本。

### 1.2 App 开发者

App 项目不需要下载 BuckyOS 或 buckyos-websdk 源码：

```bash
npm install buckyos
npx buckyos --version
npx buckyos pikg init . --owner did:bns:alice --kind static-web --source ./dist
npm run build
npx buckyos pikg build ./dapp_meta
npx buckyos pikg pack ./dapp_dist
npx buckyos pikg info ./dapp_dist/example-0.1.0.pikg
npx buckyos app install ./dapp_dist/example-0.1.0.pikg --policy local-developer
npx buckyos log tail --app example
```

在 `package.json` scripts 中，npm 自动把 `node_modules/.bin` 放入 PATH：

```json
{
  "scripts": {
    "build:pikg": "buckyos pikg build ./dapp_meta && buckyos pikg pack ./dapp_dist"
  }
}
```

`npx buckyos` 直接运行在项目已有的 Node 上；`npm install buckyos` 不会为 CLI 额外下载
第二个 runtime。

文档不推荐 App 开发者执行 `npm install -g buckyos`，避免覆盖 BuckyOS 系统安装器提供的
`buckyos`。项目外需要临时执行时使用 `npx buckyos` 或 `npm exec buckyos -- ...`。

### 1.3 同机存在两个 Tool 时的确定性规则

| 调用方式 | 选择的 Tool | 版本来源 |
| --- | --- | --- |
| `$BUCKYOS_ROOT/bin/buckyos` | 系统配套 Tool | BuckyOS 安装清单 |
| PATH 中裸 `buckyos` | 系统安装器入口 | BuckyOS updater |
| `npx buckyos` | 当前项目本地 Tool | `package-lock`/`pnpm-lock` |
| npm scripts 中 `buckyos` | 当前项目本地 Tool | `package-lock`/`pnpm-lock` |

- [ ] `buckyos --version --verbose` 和 `buckyos pikg doctor` 输出实际 executable、distribution、
  Tool/SDK 版本、协议版本、目标 Zone 及兼容性，避免用户猜测当前运行的是哪一份。
- [ ] 禁止根据 cwd、是否存在 `node_modules` 或是否存在 `/opt/buckyos` 自动跳转到另一份 Tool。
- [ ] 禁止 npm Tool 自动调用系统 Tool，反之亦然。

---

## 2. 目标产物与唯一真相源

### 2.1 buckyos-websdk 仓库布局

`buckyos-websdk` 成为 SDK 和 Tool 的源码、构建和 npm 发布真相源：

```text
buckyos-websdk/
├── src/                         # browser/node/provision SDK
├── cli/
│   ├── main.ts
│   ├── core/
│   ├── modules/
│   ├── runtime/
│   │   ├── host.ts              # typed host 接口 + distribution policy 类型
│   │   ├── host_node.ts         # developer distribution 实现
│   │   └── host_deno.ts         # system distribution 实现
│   ├── launcher.mjs             # npm/developer launcher（Node host）
│   ├── system_launcher.ts       # system distribution launcher/权限策略（Deno host）
│   └── deno.json
├── dist/
├── package.json
└── README.md
```

- [ ] 将 `buckyos/src/tools/buckyos-tool` 迁入 `buckyos-websdk/cli`，保留同一个 `main.ts` 和
  Command Registry。
- [ ] 迁移期间先保持行为和测试不变，不同时重写 CLI runtime、模块协议或输出格式。
- [ ] `cli/runtime/` host 抽象（P0.2.1）在目录迁移合入之后单独一批提交，不与迁移、命令协议
  或输出格式改动混在同一个 PR。
- [ ] npm `buckyos` 包通过 `package.json.bin.buckyos` 暴露 `cli/launcher.mjs`。
- [ ] npm `files` 白名单只包含 `dist`、CLI 运行文件、README、LICENSE 和必要 schema/fixture，
  不发布测试缓存、源码仓库相对引用或 BuckyOS rootfs 内容。
- [ ] CLI 通过 npm 包自身的正式 Node entry 使用 SDK；优先验证 package self-reference
  `buckyos/node`，不得再从 `sys_test/node_modules`、raw GitHub URL 或其它仓库相对路径导入。
- [ ] CLI 内部源码不作为新的公共 JavaScript API 导出；首版公共契约只有 `bin` 和已有
  `buckyos` SDK exports，避免外部代码依赖 CLI 内部类。

### 2.2 系统分发目录

目标布局：

```text
$BUCKYOS_ROOT/
├── bin/
│   └── buckyos
└── libexec/
    └── buckyos-tool/
        ├── cli/
        ├── dist/
        ├── package.json
        ├── distribution.json
        └── runtime/
            └── deno
```

- [ ] Linux/macOS/Windows 的 `$BUCKYOS_ROOT` 规则与 `doc/path_usage.md` 对齐；Windows 固定目录
  不依赖用户自行安装 npm/Deno。
- [ ] 系统安装器从一个带 integrity 的 SDK/Tool release artifact 安装，不在打包过程中临时
  checkout `buckyos-websdk#main`。
- [ ] `distribution.json` 至少记录 BuckyOS version/build id、Tool/SDK version、npm tarball
  integrity、Deno version、协议版本和支持的 capability range。
- [ ] 系统 Tool 完全离线可运行 `--help`、`--version`、`command list/describe`、`pikg` 本地命令
  和本机故障诊断所需命令。
- [ ] 系统 Tool 的更新、回滚和删除纳入 BuckyOS installer/updater 事务；不得由 CLI 自更新。

### 2.3 同源产物要求

- [ ] npm 发布和 BuckyOS 系统打包消费同一次 CI 产生的 SDK/Tool 内容，不允许从两个 checkout
  分别构建后只凭版本字符串认为它们相同。
- [ ] CI 生成 `npm pack` tarball、文件清单和 SHA-256；系统 packaging 固定该 tarball 或同源
  artifact 的 digest。
- [ ] 系统分发自带 pin 版本的 Deno runtime 和 system launcher，不要求目标机器预装 npm 或
  全局 Deno；developer 分发不携带任何 runtime，直接使用项目已有的 Node。
- [ ] 两种分发的 `cli/`、`dist/`、schema 和版本文件必须与对应 npm tarball 一致；`runtime/`
  是系统 artifact 独有的附加目录，从同源比对的文件清单中显式排除，不作为差异容忍项。

---

## 3. P0：npm `buckyos` 包集成 Tool

### P0.1 package.json 与 bin

- [ ] 保留现有 `.`, `./browser`, `./node`, `./provision`, `./package.json` exports。
- [ ] 增加 `"bin": { "buckyos": "./cli/launcher.mjs" }`。
- [ ] `launcher.mjs` 使用 `#!/usr/bin/env node`，由 npm 生成 POSIX/Windows `.bin` shim；不发布
  手写的 `buckyos.cmd`。
- [ ] 补齐 `license`、`repository`、`homepage`、`bugs`、`engines`、`packageManager`、keywords。
- [ ] LICENSE 文件进入 npm tarball 和系统 artifact。
- [ ] package version 是 SDK、Tool 和 PIKG `tool_version` 的唯一产品版本来源。
- [ ] 构建时生成只读 `cli/version.ts` 或等价文件；删除 CLI 和 PIKG 中各自硬编码的版本。

### P0.2 runtime host 抽象与 runtime 交付

目标是：developer 渠道用项目已有的 Node 直接运行 `npx buckyos`，不下载第二个 runtime；
system 渠道在断网、无 npm、无全局 Deno 的机器上自带 runtime 可用。两个渠道跑同一套业务代码。

CLI 本来就是按 Deno 兼容子集写的，`Deno.*` 的实际用量集中在 fs / env / spawn / stdio 这类
两个 runtime 都有对应实现的能力上（非测试文件 13 个、去重后约 30 个 API），所以这里做的是
把已有的隐含前提落成显式接口，不是一次 runtime 迁移。

选择依据（2026-08-29 实测）：

| 项 | 值 |
| --- | --- |
| `buckyos@0.7.5-145` unpacked | 11.3 MB |
| `@deno/linux-x64-glibc@2.9.6` unpacked | 95.6 MB |
| 打包 Deno 后的合计包体 | 约 107 MB（约 9.5×） |
| `deno@2.9.6` 平台覆盖 | win32-x64/arm64、darwin-x64/arm64、linux-x64/arm64-glibc；**无 musl** |

- `npx` 场景下 Node 必然存在，为它再交付一个 runtime 是给唯一不缺 runtime 的渠道加成本；
- 缺 musl 构建意味着 Alpine 基础镜像拿不到 Deno binary，与 §9.1 "暂不支持的平台在 install/run
  时明确失败" 冲突，且失败发生在运行期而不是安装期；
- browser-only SDK consumer 不使用 CLI，却要承担全部体积；
- Deno permission 是纵深防御而不是授权边界（见 §12），不足以支撑该成本。

#### P0.2.1 host 接口

- [ ] 新增 `cli/runtime/host.ts`：文件读写、目录枚举、stat/lstat/realPath、临时目录与临时文件、
  rename/remove/mkdir/chmod/symlink/copyFile、env 读取、stdin/stdout/stderr、子进程 spawn、
  cwd/platform/arch/pid/exit，以及 distribution policy 类型。接口按当前 CLI 实际用到的 API
  收敛，不顺带设计通用 VFS 或插件机制。
- [ ] host 实例由 launcher 构造并注入 `main.ts`；`core/` 和 `modules/` 只通过 host 访问外部世界。
- [ ] lint 规则禁止 `cli/main.ts`、`cli/core/`、`cli/modules/` 出现 `Deno.*`、`node:*` import 或
  `process.*`，违反即发布阻断。
- [ ] 错误语义统一：host 抛出 CLI 自有的 typed 错误（NotFound / PermissionDenied / AlreadyExists
  等），不让 `Deno.errors` 或 Node `errno` 直接泄漏到业务层和用户输出。
- [ ] host 接口本身不进入公共 JavaScript API 导出，规则与 §2.1 的 CLI 内部源码一致。

#### P0.2.2 Node host（developer distribution）

- [ ] `cli/runtime/host_node.ts` 基于 `node:fs/promises`、`node:child_process`、`node:process`
  实现 host 接口；`engines.node` 固定最低版本并在 CI 矩阵中验证。
- [ ] CLI TypeScript 在构建期编译/打包为 Node 可直接执行的产物；`launcher.mjs` 不在用户机器上
  做 TypeScript 转译，也不引入运行期编译依赖。
- [ ] developer policy 的可读写路径和 env 白名单由 host 层在每次调用前强制校验，不依赖外部
  runtime flag；越界访问返回稳定错误码，而不是裸 `EACCES`/`ENOENT`。
- [ ] 评估 Node `--permission --allow-fs-read=` 作为额外纵深防御；在其脱离 experimental 之前
  只作可选加固，不作为 developer policy 的唯一执行点。
- [ ] `npx buckyos` 在只有 Node 的空目录、Alpine/musl 镜像和 Windows 上都能跑完整
  `pikg init/build/pack/info/clean`。

#### P0.2.3 Deno host（system distribution）

- [ ] `cli/runtime/host_deno.ts` 保留现有 `Deno.*` 实现，逐条与 Node host 对齐语义。
- [ ] 系统 artifact 自带 pin 版本的 Deno binary（`libexec/buckyos-tool/runtime/deno`），
  不使用无上限的 `latest` 漂移。
- [ ] Deno 版本进入 `distribution.json`、lockfile、release manifest、`--version --verbose`
  和系统 SBOM。
- [ ] `system_launcher` 继续用 `--allow-read=`/`--allow-write=`/`--allow-env=` 收敛权限，并把
  同一份 policy 同时传给 core，使 host 层校验与 runtime 沙箱双重成立。
- [ ] 现在 `buckyos` shell launcher 里那段 argv 路径扫描迁入 policy 构造代码，两个 launcher
  共用同一实现，不再各维护一份 shell 与 TS 解析。

#### P0.2.4 过渡期（Node host 落地前）

- [ ] 若 npm 渠道必须早于 Node host 先跑起来，`deno` 只能进 `optionalDependencies`，不进
  `dependencies`，使 browser-only consumer 可以 `--omit=optional` 跳过。
- [ ] 过渡期 launcher 的 runtime 解析顺序固定且可观察：`BUCKYOS_TOOL_DENO` 显式覆盖 → 包内
  dependency 副本 → PATH 中**版本满足 pin 范围**的 `deno` → 否则明确失败并给出安装指引；
  版本不满足即拒绝，不静默使用。
- [ ] 首版不得通过 postinstall 从未知 URL 下载可执行文件。
- [ ] 过渡状态在 README 和 `pikg doctor` 输出中显式标注，并在 Gate C 之前移除。

#### P0.2.5 双 host 一致性

- [ ] 建立 command 级 conformance 用例集：同一命令的 stdin/stdout、JSON envelope、错误码、
  退出码和文件副作用，在 Node host 与 Deno host 下逐字节比对。
- [ ] 现有 60 处 `Deno.test` 迁移为与 runtime 无关的用例形式（`node:test` 或双 runner），
  host 实现各自另有针对性用例；该迁移工作量单列排期，不塞进目录迁移批次。
- [ ] `--version --verbose` 和 `pikg doctor` 输出当前 host 类型、runtime 名称与版本、
  distribution 和 policy 名称。

### P0.3 CLI 使用包内 SDK

- [ ] 所有 CLI `buckyos` import 切换到同一 npm package 的 Node SDK entry。
- [ ] npm tarball 解压到任意目录后可以解析 SDK，不依赖 monorepo hoist、pnpm symlink 形态或
  `src/apps/sys_test`。
- [ ] launcher/host policy 的可读范围包含 Tool/SDK package root，但不因此放开用户 home 或
  整个 filesystem；Deno host 下同时体现为 `--allow-read` 集合。
- [ ] CLI typecheck/test 必须消费本次构建的 `dist/node.mjs`，不能只对 SDK 源码 mock。
- [ ] SDK public exports 缺少 CLI 所需能力时，先补正式 typed API；不得让 CLI 导入
  `dist/internal/*` 或未导出的源文件。

### P0.4 发布质量门

- [ ] 修复现有 `deno lint` 问题并将 lint 设为发布阻断项，含 P0.2.1 的 host 边界规则。
- [ ] `npm pack --dry-run` 检查不包含绝对路径、仓库外 fixture、token、私钥、rootfs 或 test data。
- [ ] 在空临时目录安装真实 tarball，运行 `npx buckyos --version`、`command list/describe` 和完整
  `pikg init/build/pack/info/clean` fixture；至少覆盖一个只有 Node 的 musl（Alpine）镜像。
- [ ] 分别验证 npm、pnpm，项目路径包含空格和非 ASCII 字符。
- [ ] 首次 npm 发布使用 `next`/`beta` dist-tag；真实 App 开发闭环和系统打包闭环通过后才晋升
  `latest`。

---

## 4. P0：system / developer distribution policy

distribution policy 是 launcher 创建并传给 core 的受控 capability，只描述**本地**可访问的
文件路径、环境变量和子进程范围。

明确非目标：distribution policy 不决定身份。Tool 装在 `$BUCKYOS_ROOT` 还是 `node_modules`
与“哪些身份可用”无关 —— 目标系统接受哪类身份（例如 developer 身份需要目标 BuckyOS 显式开启
开发模式）是**目标系统自己的显式配置**，由目标系统在 handshake 中声明、由服务端强制，见 §5。
本 TODO 定义 Tool 侧的身份**解析顺序**（P0.8），不定义身份**类别**、开发模式语义或目标系统
的接受规则。

### P0.5 Developer policy

- [ ] npm launcher 固定创建 developer policy。
- [ ] developer policy 默认不把 `$BUCKYOS_ROOT` 及系统私钥/凭据目录加入 host 可读路径；该限制
  由 host 层校验强制，不依赖 runtime flag 是否存在。这是本地文件访问范围的收敛，不是身份规则。
- [ ] 在线命令的连接目标只来自显式 profile、Zone、endpoint 或外部凭据；不扫描本机、不因为
  机器上恰好装了 BuckyOS 就改变默认目标。
- [ ] 无配置时 `pikg` 本地命令、`command`、非敏感 `config` 和 `pikg doctor` 正常工作，不触发网络、
  Zone 解析或对本机 BuckyOS 安装状态的探测。

### P0.6 System policy

- [ ] system launcher 固定创建 system policy，并只授予配套 `$BUCKYOS_ROOT`、config 和显式
  输入输出路径所需的 host 可读写范围与 Deno permission。
- [ ] system policy 本身不绕过 VerifyHub、RBAC、sudo 或服务端授权；它扩大的只是本地可读写
  范围，不授予任何服务端权限。
- [ ] 复制 system launcher、设置同名环境变量或传隐藏参数不改变实际权限；本地文件权限、
  Deno permission 和服务端认证仍是强制边界。
- [ ] 系统渠道的 policy 同时经 host 层校验和 Deno permission 两道；任一道缺失都视为回归。
- [ ] `command describe` 对两种 distribution 返回同一命令 schema，并额外显示该命令所需的
  目标系统 capability；capability 是否满足由目标系统回答，不由本地 distribution 推断。

### P0.7 共同行为

- [ ] handler 不得通过 executable path、cwd、npm 环境变量或 OS 类型猜 distribution。
- [ ] handler 不得通过 host 类型、runtime 名称或 runtime 版本改变业务行为；runtime 差异只允许
  存在于 `cli/runtime/` 内部。
- [ ] 同一显式 profile/凭据调用在 system 与 developer distribution 下产生相同 RPC、输入、
  输出、错误和退出码；distribution 不参与身份选择，也不改变服务端对该请求的判定。
- [ ] 只有默认可读写路径、host/runtime 实现和更新渠道可以因 distribution 不同。
- [ ] REPL、completion、JSON schema 和审计字段不分叉。

### P0.8 身份解析顺序（两种 distribution 共用）

原则：**显式指定优先；未显式指定时按确定顺序从候选目录搜索；被目标系统拒绝则尝试下一个。**

解析算法与 distribution 无关。distribution 只决定候选目录集合（见 P0.5/P0.6），不决定顺序、
不决定是否轮换、也不决定哪类身份可用（那是目标系统的配置，见 §5）。

- [ ] 命令行参数或 profile 显式指定身份时只使用该身份：被拒绝即以拒绝原因失败，不回退到目录
  搜索，也不静默改用其它身份。
- [ ] 未显式指定时按**固定且可打印**的顺序枚举候选身份；顺序在代码中显式定义，不依赖目录遍历
  返回顺序、文件名排序偶然性、mtime 或环境变量注入。
- [ ] 只有目标系统返回“该身份不被接受/认证失败”这类可区分的稳定错误才尝试下一个候选。
  网络错误、超时、5xx、capability 不匹配，以及“身份已被接受但 RBAC 拒绝该操作”都**不**触发
  轮换，直接返回原始错误。轮换条件写死在一处，不由各 handler 自行判断。
- [ ] 轮换只发生在建立 session 阶段。一旦某个身份被接受并开始执行有副作用的 RPC，失败不得改用
  另一个身份重试。
- [ ] 候选数量有上限；每次尝试可观测：`--version --verbose` 和 `pikg doctor` 打印候选顺序，
  最终失败时列出**所有**试过的身份标识与各自被拒原因，而不是只报最后一个。
- [ ] 候选目录必须落在当前 policy 的 host 可读范围内；范围外的候选直接跳过并在 verbose 输出中
  标注跳过原因，不触发越权读取，也不因此提升权限。
- [ ] 日志、错误和审计字段只打印身份标识（DID/路径），不打印私钥内容，复用 SDK 的 secret
  redaction。
- [ ] `--non-interactive` 下轮换行为不变，不因轮换弹出交互确认。
- [ ] 候选顺序本身是稳定契约：新增候选位置、目录布局变化视为 breaking change，需要同步文档。

---

## 5. P0：版本、协议与目标兼容性

- [ ] SDK/Tool 使用同一个 npm semver；BuckyOS 系统版本独立，通过 compatibility manifest 建立
  关系，不强行要求版本号相等。
- [ ] BuckyOS 在线入口提供稳定的 protocol/capabilities 信息；Tool 建立 session 后先做兼容性
  判定，再执行依赖缺失 capability 的 RPC。
- [ ] 冻结稳定错误：`INCOMPATIBLE_BUCKYOS_VERSION`、`UNSUPPORTED_SERVER_CAPABILITY`、
  `PIKG_TARGET_VERSION_MISMATCH`。
- [ ] 兼容性错误返回本地 Tool/SDK version、目标 BuckyOS version、required/observed capability，
  不只返回“RPC method not found”。
- [ ] PIKG/AppDoc 是否需要声明 `min_buckyos_version`、`sdk_api_version` 或 capability requirements
  由 PIKG/App 安装协议评审冻结；不得由 CLI 单方面新增未被 Installer 校验的字段。
- [ ] npm Tool 可以连接较旧测试 Zone，但必须按 capability 降级或明确拒绝，不自动改用系统 Tool。
- [ ] 目标系统在 handshake 中声明它接受哪些身份类别（例如 developer 身份是否可用，通常取决于
  该 BuckyOS 是否显式开启开发模式）。Tool 只按声明提示和报错，不在本地推断、不因 distribution
  不同而改变结论。身份类别的定义、开发模式语义和开关方式由 BuckyOS 侧协议评审冻结，不在本
  TODO 范围内。
- [ ] 目标系统拒绝某类身份时返回可区分的稳定错误（如 `IDENTITY_KIND_NOT_ACCEPTED`），并说明
  目标系统需要开启什么配置；不得退化为通用 401/403 或“RPC method not found”。该错误是 P0.8
  中唯一允许触发身份轮换的信号之一，因此它必须与“身份已接受但授权不足”明确可区分。

---

## 6. P1：围绕 PIKG 的 App 开发测试体验

### 6.1 命令边界

明确禁止：

```text
buckyos dev ...
```

现有 PIKG 主路径保持：

```text
buckyos pikg init
buckyos pikg build
buckyos pikg pack
buckyos pikg info
buckyos pikg clean
```

- [ ] App 开发新增的本地准备和测试环境辅助只能注册到 `pikg` 模块，并继续使用统一的
  `<module> <verb>` Command Registry。
- [ ] 优先评估补充 `pikg doctor`：检查 Tool/SDK/host runtime、Docker（仅需要时）、dapp_meta、构建输出、
  profile/目标 Zone 和兼容性；默认只读，不自动安装依赖或修改系统。
- [ ] 若 Tool 负责版本化本地测试环境，命令命名限定在 `pikg` 下，例如候选
  `pikg env-up|env-status|env-down`；开工前必须先在 PIKG PRD 中冻结，不直接采用
  `dev env ...`。
- [ ] 若增加“一次构建后投放测试 Zone”的便捷入口，优先评估 `pikg test`，但它必须复用
  Installer/App/Task typed clients，不复制 `app install`、Task wait 或日志协议。
- [ ] `pikg init` 保持 PIKG metadata-first；是否增加官方 App template 由独立产品评审决定，
  不把 `pikg` 扩成执行任意项目脚本的通用脚手架。

### 6.2 无源码测试环境

App 开发至少支持两条不依赖 BuckyOS 源码的路径：

1. **远程/共享 DV Zone**：配置 profile 后安装本地 PIKG（该 Zone 需已按 BuckyOS 侧配置接受
   开发者身份）；
2. **本地正式开发环境 artifact**：下载版本化 BuckyOS image/installer 后启动测试 Zone，不从
   git checkout 构建 BuckyOS。

- [ ] 提供稳定的 DV profile 配置和凭据过期处理说明。目标 Zone 需要开启什么才接受开发者
  身份，由 BuckyOS 侧文档定义，Tool 文档只做引用和错误指引。
- [ ] 提供与 SDK release compatibility manifest 对齐的 BuckyOS 开发镜像或安装包。
- [ ] 测试环境的下载源、digest、缓存、更新和清理语义在 PIKG PRD 中定义；禁止隐式 pull
  `latest`。
- [ ] Docker 仍遵守限定进程权限；不得通过 shell 字符串执行任意命令，也不得隐式 login/push。
- [ ] 环境启动失败时输出可诊断的阶段、日志位置和恢复动作；不得要求用户回到 BuckyOS 源码
  执行 `start.py` 才能定位。
- [ ] 远程 DV 与本地环境执行同一 PIKG、App、Task 和 Log 协议，不维护专用测试后门。

### 6.3 文档主路径

- [ ] SDK README 首页把 App 开发主路径改为“安装 SDK → PIKG init/build/pack/info → 测试 Zone
  install → task/log 调试”。
- [ ] 分别提供 static-web、Docker、Script/AppService 最小示例；示例只依赖 npm `buckyos` 和
  正式发布的测试环境。
- [ ] 文档明确项目内 `npx buckyos` 与系统裸 `buckyos` 的选择规则。
- [ ] 文档不再要求复制 `src/tools/buckyos-tool`、安装 `sys_test/node_modules` 或 checkout
  `buckyos-websdk#main`。
- [ ] 错误排查从 `buckyos pikg doctor`、`pikg info`、`app status`、`task get/wait`、`log tail` 开始，
  不以阅读 BuckyOS 内部 system-config/rootfs 为默认步骤。

---

## 7. P1：SDK 与 Tool 的正式 API 边界

Tool 是 SDK 的官方命令行前端。线上 handler 应是 typed SDK client 的薄封装，而不是另一套协议
实现。

- [ ] 盘点 CLI 当前直接字符串调用的 kRPC method，建立 Tool command → SDK client → service
  protocol 映射表。
- [ ] WebSDK 补齐 App Installer/Control Panel、Log、Diagnostic 等 Tool 所需 typed clients。
- [ ] CLI 的 Task 跟踪复用正式 TaskManager client 和 KEvent reader；正确性仍以 TaskManager
  snapshot/durable cursor 为准。
- [ ] PIKG parser、canonicalization、Object ID、strict verifier 形成单一共享实现；CLI、Installer
  和其它 SDK consumer 不各自维护易漂移的格式规则。
- [ ] SDK 统一 timeout、abort、trace、idempotency、RPC error 和 secret redaction 类型。
- [ ] 禁止 CLI 通过读取 system-config 私有 key、Task 内部 JSON 或服务端本地路径补协议缺口。

---

## 8. P1：buckyos 主仓库迁移

### 8.1 BuckyOS 内部始终消费最新 SDK

- [ ] `src/apps/sys_test`、`src/kernel/node_active`、`src/frame/desktop`、tests 和 Deno import maps
  统一跟随 GitHub `main`，不再引用固定 SDK commit 或本地 `sys_test/node_modules`。
- [ ] BuckyOS build 的 PIKG 构造通过安装依赖后的 `node_modules/.bin/buckyos` 或明确 package
  launcher 执行，不再 import `../../tools/buckyos-tool/pikg_launcher.mjs`。
- [ ] BuckyOS 内部 package 和 test 不提交 lockfile；构建时主动更新 `buckyos`。只有第三方 App
  项目通过 lockfile 固定 SDK/Tool 版本。
- [ ] harness/Jarvis skill、测试文档和示例统一使用新入口。

### 8.2 删除重复源码与旧入口

- [ ] 在 npm 和系统双分发闭环通过前，原 `src/tools/buckyos-tool` 只作为迁移源，不并行开发
  新功能。
- [ ] 切换后删除原目录和相对 launcher import，`buckyos-websdk/cli` 成为唯一源码真相源。
- [ ] 建立旧 Rust `buckycli` 到新 `buckyos` 的覆盖矩阵。
- [ ] 安装器切到 `$BUCKYOS_ROOT/bin/buckyos` 后，删除 Rust `buckycli` 的生产 build、rootfs module
  和 macOS/Windows package entry；仍有价值的内部开发能力迁到明确的工具，不保留同名兼容层。

---

## 9. CI、发布与供应链

### 9.1 SDK/Tool CI

- [ ] Node + Deno 最低支持版本和当前稳定版本矩阵；Node 版本同时是 developer distribution 的
  运行时下限，写入 `engines.node`。
- [ ] Linux/macOS/Windows，x64/arm64 矩阵，Linux 侧覆盖 glibc 与 musl；暂不支持的平台在
  install/run 时明确失败。
- [ ] SDK unit/browser/node/provision tests 与 CLI check/lint/unit tests 全绿。
- [ ] P0.2.5 的 command 级 conformance 套件在 Node host 与 Deno host 下都执行并比对输出；
  任一 host 缺失或结果不一致即阻断发布。涉及文件、进程或 stdio 的改动在该套件就位前不合入。
- [ ] npm tarball 空目录 smoke test，且测试进程不能访问 buckyos/buckyos-websdk checkout；
  smoke 环境中不得存在全局 `deno`，以证明 developer 渠道确实只依赖 Node。
- [ ] system artifact 解包 smoke test，断网且 PATH 中没有 npm/deno 时使用内置 runtime 运行。
- [ ] 至少一个真实 DV：PIKG build/pack/info → local-developer install → task terminal → app status →
  log query。

### 9.2 发布控制

- [ ] npm 使用 Trusted Publishing/provenance、2FA 和最小发布权限。
- [ ] 发布前生成 SBOM、tarball digest、文件清单和 release manifest。
- [ ] `next` 通过 App 开发和系统配套验收后再 promote `latest`，不重新构建 tarball。
- [ ] BuckyOS 系统 release 只引用已发布且验证过的 SDK/Tool digest。
- [ ] 回滚 npm dist-tag 不改变已安装 BuckyOS 的系统 Tool；BuckyOS rollback 恢复对应配套 Tool。

---

## 10. 分阶段实施顺序

### Gate A：可以发布 npm beta

- [ ] CLI 已迁入 buckyos-websdk 并注册 package `bin`。
- [ ] `cli/runtime/` host 抽象落地，`main.ts`/`core/`/`modules/` 无直接 `Deno.*`、`node:*` 引用，
  lint 规则生效。
- [ ] npm package 不携带额外 runtime；只有 Node 的空目录（含 Alpine/musl 镜像）`npx buckyos`
  闭环通过。若仍处于 P0.2.4 过渡期，则 `deno` 必须是 optionalDependency 并在文档中标注。
- [ ] command 级 conformance 套件在 Node host 与 Deno host 下结果一致。
- [ ] CLI 使用包内 `buckyos/node`，无源码树路径。
- [ ] SDK/Tool 版本统一，check/lint/unit/pack smoke 全绿。
- [ ] developer policy 的本地路径收敛生效：默认不可读 `$BUCKYOS_ROOT` 及系统私钥/凭据目录。
- [ ] 身份解析按 P0.8 实现：显式指定优先、候选顺序固定可打印、只在可区分的拒绝错误上轮换。

### Gate B：可以随 BuckyOS 系统安装

- [ ] 固定 `$BUCKYOS_ROOT/bin`/`libexec` 布局和 system launcher。
- [ ] 系统 artifact 与 npm tarball 内容同源、版本和 digest 可追溯。
- [ ] 断网、无 npm、无全局 Deno 时，系统 Tool 用 artifact 自带的 runtime 可用。
- [ ] system policy 的 host 层校验、Deno permission 和 RBAC 验收通过。
- [ ] installer/updater/rollback 覆盖 Tool。

### Gate C：可以宣称 App 开发不需要 BuckyOS 源码

- [ ] README 中完整 PIKG-first quickstart 在全新机器跑通。
- [ ] 远程 DV 或本地版本化测试环境至少有一条正式可用路径。
- [ ] static-web、Docker、Script/AppService 示例都不引用源码相对路径或 GitHub `main`。
- [ ] PIKG 安装、Task、App status、日志调试闭环通过。
- [ ] 所有新增开发辅助命令位于 `pikg`，不存在 `buckyos dev`。
- [ ] P0.2.4 过渡期的 `deno` optionalDependency 已移除，developer 渠道只依赖 Node。

### Gate D：可以淘汰 buckycli

- [ ] 旧线上运维能力覆盖矩阵完成或明确删除。
- [ ] BuckyOS/Jarvis/CI/文档不再调用 Rust `buckycli`。
- [ ] 三平台安装器不再发布旧 CLI。
- [ ] 连续一个正式版本验证新 Tool 的升级和回滚后删除旧生产入口。

---

## 11. 全局验收清单

1. App 开发者从空目录执行 `npm install buckyos` 后，可以用 `npx buckyos` 完成 PIKG
   init/build/pack/info，并向明确配置的测试 Zone 安装，不需要 BuckyOS 源码。
2. BuckyOS 系统安装完成后，固定目录始终存在配套 Tool；即使 npm registry、网络和用户 PATH
   不可用，也能执行本机状态检查与故障诊断。
3. 同机的系统 Tool 和项目 Tool 可以同时存在；调用路径、版本、权限和更新渠道确定且可观察。
4. npm Tool 不会因为机器上存在 BuckyOS 系统而隐式读取系统私钥/凭据目录或改变默认连接目标。
5. system Tool 和 npm Tool 对同一显式凭据/Zone/命令使用同一 handler、SDK client、schema、
   JSON envelope 和稳定错误码；目标系统对该请求的判定与 Tool 从哪个渠道安装无关。
6. 身份解析在两种 distribution 下使用同一顺序与同一轮换条件；差异只体现在候选目录集合，
   且候选顺序和每次尝试的结果可从 verbose 输出中完整看到。
7. SDK/Tool、系统 build 和 PIKG target 的版本不兼容在执行副作用前被识别。
8. 发布产物可由 digest、SBOM、provenance 和 lockfile 追溯，不依赖移动的 Git branch。
9. App 开发文档的核心名词和操作顺序围绕 PIKG；没有第二套 `dev` 命令心智模型。
10. developer distribution 只依赖项目已有的 Node，system distribution 自带 runtime；两者跑同一份
    业务代码，runtime 差异全部收敛在 `cli/runtime/` 内，并可由 conformance 套件证明输出一致。

## 12. 已知风险与待冻结项

- npm `buckyos` 同时面向 browser SDK 和 CLI。实测携带 Deno platform package 会把包体从
  11.3 MB 推到约 107 MB，且官方 `deno` npm 包没有 musl 构建，Alpine 镜像在运行期才失败。
  这是选择 Node host 的直接原因，也是 P0.2.4 过渡期必须尽快结束的原因。
- 双 host 存在行为漂移风险：只在一个 host 下验证的改动可能悄悄破坏验收 #5 和 #9。
  conformance 套件是这条风险的唯一控制手段，不能降级为"抽查"。
- 60 处 `Deno.test` 的迁移是本轮最大的一次性工作量，容易被低估；它不应与 §2.1 的目录迁移
  或命令协议改动混在同一批提交里。
- host 接口容易被扩成通用抽象层。它只覆盖 CLI 现有调用点，新增方法必须先有真实调用方，
  不为"将来可能换 runtime"预留能力。
- npm 本地 bin 与系统 PATH 的解析由 npm/shell 规则决定；文档必须坚持 `npx buckyos`，不能承诺
  普通 `npm install` 后任意 shell 都能直接找到裸命令。
- distribution policy 不是授权边界，也不是身份机制；本地私钥文件权限、host 层校验、
  Deno permission（system 渠道）、VerifyHub 和 RBAC 必须同时成立。Node host 没有等价于 Deno 的进程级沙箱，developer policy
  的实际强度就是 host 层校验加上 OS 文件权限，文档和 PRD 不得对外宣称更强的隔离。
- P0.8 的候选目录清单和顺序尚未冻结，需要与 BuckyOS 侧的身份存放约定一并评审。是否允许按
  目标 Zone 缓存“上次成功的身份”以跳过前面的候选，也在同一次评审中决定：它能避免重复被拒，
  但会让顺序不再只由代码决定；在冻结前不实现缓存。
- 身份轮换有被误用成对目标系统试探凭据的风险。轮换上限、拒绝错误的可区分性和服务端侧的
  失败计数必须一起成立，缺一不可实现轮换。
- SDK Tool 可能比测试 Zone 新；capability handshake 未落地前不能把底层 RPC 404 当作正式
  兼容策略。
- PIKG 开发辅助不能重新演化成通用构建系统。任何 `doctor`、`env-*` 或 `test` 新命令都要先
  更新
  [`buckyos-websdk/doc/modules/pikg.md`](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md)，
  冻结副作用、进程、网络和文件权限。
- BuckyOS 内部构建与 App 开发使用同一 npm package 后，release graph 必须避免循环：SDK/Tool
  先产生不可变 artifact，BuckyOS 再 pin artifact；不得让 SDK release 反向依赖未发布的
  BuckyOS build。
