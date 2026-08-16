# BuckyOS Tool PRD

> 状态：Draft v0.2  
> 目标版本：Beta 2.2 以后  
> 命令名称：`buckyos`  
> 实现语言与运行时：TypeScript + Deno

## 1. 背景

旧 `buckycli` 是 Rust 工程，混合了开发工具、线上运维、直接读写本地文件、直接修改
`system-config` 和部分已经失效的历史协议。它的命令结构、配置目录和输出格式都不适合作为
Agent 稳定调用的生产接口。

新一代 BuckyOS Tool 面向线上系统管理和 Agent First 场景重新设计。它不是旧 `buckycli`
的兼容升级，而是一个新的 TS/Deno 工具：

- 使用新命令名 `buckyos`；
- 使用新配置根目录 `~/.buckyos_tool`；
- 不自动读取、合并或迁移 `~/.buckycli`、`~/buckycli` 中的任何配置；
- 通过正式 BuckyOS SDK、kRPC 服务和受控的本机控制桥执行操作；
- 业务模块是薄客户端，不在 CLI 内复制 scheduler、installer、MessageHub 等服务逻辑；
- 默认输出稳定的机器可读 JSON，适合 Jarvis 和其它 Agent 调用。

Beta 2.2 是 breaking change，本工具不承担旧命令、旧参数、旧输出和旧配置兼容。

## 2. 产品目标

### 2.1 核心目标

1. 建立统一命令模型：

   ```text
   buckyos [通用参数] <模块> <动词> [动作参数]
   ```

2. 建立可扩展的 TS 命令框架，使新增模块只需要声明命令元数据、参数 schema 和 handler。
3. 统一配置、身份、认证、sudo、输出、错误、审计、任务等待和交互确认语义。
4. 让 Agent 无需解析自然语言或易变表格即可发现和执行命令。
5. 让线上操作经过正式服务边界，避免 CLI 直接修改系统真相源或重做业务编排。
6. 最终取代旧 Rust `buckycli` 工程中的线上运维能力。

### 2.2 非目标

- 不承载构建、打包、DV 环境安装等开发阶段工具。
- 不管理 cyfs-gateway 的专属配置和生命周期。
- 不包含 AI/AICC/OpenDAN 的业务工具；Jarvis 只是本工具的一个调用方。
- 不提供任意 `system-config get/set` 作为正式用户功能。
- 不把旧 `buckycli` 的命令逐条机械翻译成 TS。
- 第一阶段不支持从远程下载并执行第三方 CLI 模块。
- Windows Desktop 不要求为普通用户安装原生 Deno；开发者可以使用本机已有的 Deno/pnpm
  工具链直接运行源码。

## 3. 运行平台

### 3.1 正式支持

- Linux：安装器创建可直接执行的 `buckyos` 命令，底层依赖系统 Deno runtime。
- macOS：安装器创建可直接执行的 `buckyos` 命令，底层依赖系统 Deno runtime，支持企业版
  Linux + macOS 混合环境。
- Windows Desktop 开发环境：开发者使用标准 Deno/pnpm 工具链直接运行 `buckyos-tool`。
- Windows Desktop 普通用户环境：优先通过 `docker exec` 在已经运行的 Jarvis 容器中执行；
  Jarvis 容器不存在时，基于 paios 镜像创建临时容器执行。Windows Host 不要求安装 Deno。

安装器提供的 `buckyos` 是正式用户入口。源码环境同时保留 Deno 原始入口，便于开发、调试
和容器执行；两种入口必须进入同一个 `main.ts` 和 Command Registry，不允许形成两套行为。

### 3.2 宿主机操作边界

TS 工具不得假设自己可以直接执行 `systemd`、`launchd`、Windows 计划任务、Docker Host
命令或宿主机 mount。需要在系统故障时仍可执行的操作，例如：

- `node check/start/stop/restart`；
- 外部目录 mount/unmount；
- 离线诊断和恢复；

必须通过 `node-control`、native helper 或受控的 host bridge 完成。Windows 下不设计第二套
命令协议和认证协议：本机源码、Jarvis 容器和 paios 临时容器都使用相同的 BuckyOS
session/identity 认证及 HostControlClient 抽象，不在业务模块中散落平台特例。

## 4. 命令模型

### 4.1 标准语法

```text
buckyos [global-options] <module> <verb> [primary-selector] [action-options]
```

示例：

```bash
buckyos --profile production user list
buckyos --profile production user get alice
buckyos --zone corp.example.com --identity ops app restart --app-instance notes@alice
buckyos --output json system status
buckyos --non-interactive --yes --idempotency-key upgrade-2026-08 app upgrade --app-instance notes@alice
```

约束：

- module、verb 和 option 使用小写 kebab-case。
- 通用参数的规范位置在 module 之前。
- 一个命令最多允许一个含义明确的 primary selector 位置参数；其余参数使用具名 option。
- 复杂对象使用 `--input <file>` 或 `--input -` 从 stdin 读取 JSON。
- 同一字段同时出现在 `--input` 和 option 中时直接报参数冲突，不做隐式覆盖。
- 不提供行为不明确的缩写和隐式别名。
- `--help`、`--version` 和 shell completion 是命令树的框架级例外。

### 4.2 标准动词

各模块优先复用以下动词，不自行发明同义词：

| 语义 | 动词 |
| --- | --- |
| 枚举 | `list` |
| 获取单项 | `get` |
| 创建 | `create` |
| 修改 | `update` |
| 删除或卸载 | `delete` / 领域明确时使用 `uninstall` |
| 状态 | `status` |
| 生命周期 | `start` / `stop` / `restart` |
| 无副作用预演 | `dry-run` |
| 执行预演结果 | `apply` |
| 等待异步任务 | `wait` |
| 检查 | `check` |
| 验证内容 | `verify` |
| 导入/导出 | `import` / `export` |

涉及软删除、归档、封禁、忘记联系人的领域必须在模块资源模型中明确真实语义；当 `delete`
会产生误解时，应使用 `archive`、`close`、`forget` 等领域动词。

### 4.3 通用参数

| 参数 | 含义 |
| --- | --- |
| `--config-dir <path>` | 本次命令使用的配置根目录 |
| `--profile <name>` | 选择连接与身份 profile |
| `--zone <host-or-did>` | 覆盖 profile 中的目标 Zone |
| `--endpoint <url>` | 覆盖自动解析出的 API 入口，主要用于诊断 |
| `--identity <did-or-name>` | 选择本地身份；优先按 DID 解析 |
| `--identity-root <path>` | 显式指定本次命令的 public identity root |
| `--security-root <path>` | 显式指定本次命令的 security root |
| `--session-token <token>` | 直接使用外部 session token |
| `--session-token-file <path>` | 从文件读取临时 session token |
| `--output <format>` | 输出模式，可选 json/jsonl/table/text/raw，默认 `json` |
| `--input <path-or-stdin>` | 从文件读取动作 JSON；值为 `-` 时读取 stdin |
| `--timeout <duration>` | 本地请求或等待超时，例如 `30s`、`5m` |
| `--trace-id <id>` | 覆盖自动生成的 trace id |
| `--idempotency-key <key>` | 写操作幂等键 |
| `--wait` | 对返回 Task 的操作等待结束 |
| `--non-interactive` | 禁止提示、密码输入和确认交互 |
| `--yes` | 接受本地确认，不绕过服务端权限和 sudo |
| `--no-color` | 禁止 stderr 和人类输出着色 |
| `--verbose` | 增加 stderr 诊断信息，不泄露 secret |

安全约束：不提供 `--password` 明文 argv 参数。允许按运维和 Agent 场景使用
`--session-token`，但调用方必须理解它可能进入 shell history 和进程列表；自动化优先使用
环境变量或 `--session-token-file`。框架必须在 help、日志、错误和 effective config 中统一
脱敏 token。

### 4.4 框架内置模块

以下模块由 core 提供，不属于业务模块：

- `config list|get|set|use|check`
- `auth whoami|session-status`
- `command list|describe`
- `completion generate`

`command describe <module> <verb>` 必须输出完整参数 schema、权限要求、是否写操作、是否可能
返回 Task、结果 schema 版本和示例，使 Agent 不依赖人类 help 文本推断参数。

## 5. 配置与身份

### 5.1 新配置根目录

默认配置根目录固定为：

```text
~/.buckyos_tool/
├── config.json
├── profiles/
│   └── <profile>.json
├── local/
│   └── identity/
│       └── <encoded-did>/
├── security/
│   └── <encoded-did>/
├── cache/
└── state/
```

- Tool 自己拥有的 config/state JSON 必须包含 `schema_version`；DID Document 和 keyref 等身份
  材料遵循 IdentityRoots 协议自身的 schema。
- `config.json` 只保存默认 profile 和非敏感全局偏好。
- profile 保存 Zone、endpoint、identity DID/名称和默认输出，不嵌入私钥、密码或 token。
- `local/identity` 和 `security` 遵循 BuckyOS IdentityRoots 路径协议，分别存放公开身份材料和
  私钥/keyref；不得在 profile JSON 中内嵌私钥。
- 第一阶段不持久化 session token 或 refresh token，也不实现 refresh 流程。
- 写配置使用临时文件 + rename，避免中途退出产生半文件。
- 日志和错误不得打印 private key、refresh token、session token 或数据库完整连接串。

`~/.buckycli` 和 `~/buckycli` 不在任何默认搜索路径中。若未来提供人工导入工具，也必须要求
用户显式指定源路径并生成全新的配置，不能在运行时继续引用旧目录。

### 5.2 配置示例

`config.json`：

```json
{
  "schema_version": 1,
  "default_profile": "production",
  "output": "json"
}
```

`profiles/production.json`：

```json
{
  "schema_version": 1,
  "zone": "corp.example.com",
  "identity": "did:bns:ops-admin",
  "default_protocol": "https://"
}
```

Tool 私有身份材料使用 IdentityRoots 布局，例如：

```text
~/.buckyos_tool/local/identity/<encoded-did>/did.json
~/.buckyos_tool/security/<encoded-did>/authentication.private.pem
~/.buckyos_tool/security/<encoded-did>/authentication.keyref.json
```

路径和文件名规则以 buckyos-base 的
[DID Identity/Certificate Manager](https://github.com/buckyos/buckyos-base/blob/main/doc/did-identity-certificate-manager.md)
为准。第一阶段不接入 macOS Keychain、Linux Secret Service 或 Windows Credential Manager。

### 5.3 身份扫描顺序

当没有使用外部 session token 时，按以下顺序寻找所选 DID 对应的认证材料：

1. `--identity-root` + `--security-root` 显式指定的 roots；
2. `~/.buckyos_tool/local/identity` + `~/.buckyos_tool/security`；
3. `BUCKYOS_IDENTITY_ROOT` + `BUCKYOS_SECURITY_ROOT`；
4. `$BUCKYOS_ROOT/local/identity` + `$BUCKYOS_ROOT/security`。

每一层都使用 IdentityRoots 的 encoded DID + usage 文件协议。找到完整、可用且 DID 匹配的
认证材料后停止；不扫描 `~/.buckycli`。旧 buckycli 只作为实现行为参考，不是身份来源。
`--identity-root` 和 `--security-root` 必须成对出现，禁止把两个不同优先级的 root 隐式拼在一起。

### 5.4 配置覆盖优先级

最终 `CommandContext` 按以下优先级构造，前者覆盖后者：

1. 当前命令显式传入的通用参数；
2. `BUCKYOS_TOOL_*` 环境变量，以及 Jarvis 注入的标准
   `BUCKYOS_APPCLIENT_SESSION_TOKEN`；
3. 选中 profile 中的字段；profile 的选择顺序是 `--profile`、`BUCKYOS_TOOL_PROFILE`、
   `config.json.default_profile`；
4. `config.json` 中的其它全局默认值；
5. 框架内安全默认值。

至少支持以下环境变量：

- `BUCKYOS_TOOL_CONFIG_DIR`
- `BUCKYOS_TOOL_PROFILE`
- `BUCKYOS_TOOL_ZONE`
- `BUCKYOS_TOOL_ENDPOINT`
- `BUCKYOS_TOOL_IDENTITY`
- `BUCKYOS_TOOL_OUTPUT`
- `BUCKYOS_IDENTITY_ROOT`
- `BUCKYOS_SECURITY_ROOT`
- `BUCKYOS_APPCLIENT_SESSION_TOKEN`

解析结果应能通过 `buckyos config check --effective` 查看，但所有 secret 只显示来源和脱敏摘要。

### 5.5 认证与 sudo

认证按以下顺序选择可用凭证：

1. `--session-token`；
2. `--session-token-file`；
3. 注入的 `BUCKYOS_APPCLIENT_SESSION_TOKEN`；
4. 按 §5.3 找到的 UserDocument + authentication private key/keyref；
5. 交互式密码登录。

命令生命周期通常很短，第一阶段不申请、不保存也不刷新 refresh token。使用身份私钥时，
Tool 在当前命令进程内构造登录 JWT，通过 verify-hub 换取可用 session token，并只在内存中
使用到命令结束。

Tool 默认使用稳定 appid `buckycli` 登录。使用外部 session token 时，以 token claims 中的
appid/app instance 为有效调用方，禁止强行改写成 `buckycli`；CommandContext 和审计输出必须
记录最终生效的 appid。

Tool 用身份私钥登录时，由 verify-hub 按 `buckycli` 的 App availability 策略决定是否签发
token。外部 session token 不再经过 `buckycli` availability 替换或二次签发，下游服务直接按
token 自带的 appid/app instance、principal 和 RBAC 做授权。

不需要连接系统的命令，例如 `--help`、`--version`、`command describe` 和本地 `config`
操作，不得触发登录。

需要 sudo 的命令由框架统一处理：

- 命令元数据声明所需 scope/action/resource；
- 交互模式可以安全读取密码并向 verify-hub 请求短期 sudo token；
- 非交互模式缺少 sudo token 时返回稳定的 `SUDO_REQUIRED`，不得偷偷降级或无限重试；
- `--yes` 只表示接受本地确认，不能替代 sudo；
- sudo token 不得作为普通身份长期写入 profile。

## 6. TS/Deno 框架架构

### 6.1 建议源码布局

```text
src/tools/buckyos-tool/
├── main.ts
├── deno.json
├── core/
│   ├── argv.ts
│   ├── registry.ts
│   ├── command.ts
│   ├── config.ts
│   ├── identity.ts
│   ├── auth.ts
│   ├── runtime.ts
│   ├── output.ts
│   ├── errors.ts
│   ├── confirm.ts
│   ├── task.ts
│   └── context.ts
├── modules/
│   └── <module>/
│       ├── mod.ts
│       ├── commands.ts
│       └── types.ts
└── tests/
```

实际落地可以调整文件粒度，但必须保持 core 与业务模块单向依赖：core 不依赖具体业务模块，
业务模块不得绕开 core 自行解析全局配置、登录或输出。

### 6.2 命令注册模型

每个模块静态注册，第一阶段不动态执行远程代码。建议接口：

```ts
interface CommandModule {
  name: string;
  summary: string;
  commands: CommandDefinition[];
}

type AccessLevel = "read" | "write" | "destructive" | "privileged";
type AccessPolicy =
  | { mode: "fixed"; level: AccessLevel }
  | {
    mode: "operation";
    operationIdField: string;
    possibleLevels: AccessLevel[];
  };

interface CommandDefinition<TInput, TOutput> {
  verb: string;
  summary: string;
  inputSchema: JsonSchema;
  outputSchema: JsonSchema;
  access: AccessPolicy;
  asyncMode: "sync" | "task" | "either" | "stream";
  sudo?: SudoRequirement;
  handler(ctx: CommandContext, input: TInput): Promise<CommandResult<TOutput>>;
}
```

框架通过同一份定义生成：

- argv 校验；
- `--help`；
- `command describe` JSON；
- completion；
- 确认策略；
- 输出 envelope；
- parser 和 schema 测试样例。

### 6.3 CommandContext

handler 只能从 `CommandContext` 获取运行能力：

```ts
interface CommandContext {
  command: { module: string; verb: string };
  connection: ResolvedConnection;
  principal: ResolvedPrincipal;
  clients: ServiceClientRegistry;
  output: OutputPolicy;
  traceId: string;
  idempotencyKey?: string;
  deadline?: number;
  interactive: boolean;
  confirmed: boolean;
}
```

业务模块不得：

- 自行读取 `~/.buckyos_tool`；
- 自行读取环境变量；
- 自行创建第二套登录流程；
- 直接 `console.log` 业务结果；
- 直接写 `system-config` 绕过领域服务；
- 自己实现通用重试、Task 轮询或 sudo 对话框。

### 6.4 服务访问

- 复用 BuckyOS TS SDK 和已有 service client，不手写重复 HTTP/kRPC 协议。
- 用户请求必须透传调用者身份，不能用 Tool 自身 service token 覆盖源身份。
- core 负责 endpoint 解析、token 注入、trace、deadline 和 transport error 归一化。
- 写操作只对明确声明为幂等的错误进行自动重试；其余交给用户或 TaskManager。
- 本机控制使用独立 `HostControlClient` 接口，禁止模块根据 OS 执行 shell 分支。

### 6.5 Deno 权限

- 正式启动脚本不得使用无边界的 `-A`。
- 只授予配置目录、显式输入输出路径、必要环境变量和目标网络地址所需权限。
- 业务模块新增文件、进程或网络权限时，必须在模块 PRD 和命令元数据中声明。
- 普通在线模块不得申请 `--allow-run`。

## 7. 输入、输出和错误协议

### 7.1 默认 JSON 输出

成功结果：

```json
{
  "schema_version": 1,
  "ok": true,
  "data": {},
  "meta": {
    "command": "user.list",
    "trace_id": "...",
    "duration_ms": 42
  }
}
```

失败结果：

```json
{
  "schema_version": 1,
  "ok": false,
  "error": {
    "code": "PERMISSION_DENIED",
    "message": "admin permission is required",
    "retryable": false,
    "details": {}
  },
  "meta": {
    "command": "user.create",
    "trace_id": "..."
  }
}
```

- stdout 只写最终数据协议。
- 进度、确认、警告、诊断和日志写 stderr。
- `jsonl` 用于 watch、tail 和持续任务事件，每行都是完整、可独立解析的 envelope。
- `table` 和 `text` 只用于人类显示，不作为自动化稳定接口。
- `raw` 仅用于明确声明支持原始字节的命令；使用 raw 时不得混入 JSON 或进度。
- 二进制导出默认要求 `--output-file`，避免误写终端。
- 所有时间使用 RFC 3339 UTC 或明确标注单位的 Unix 毫秒字段。
- 所有 ID 保持字符串，不根据显示需要截断机器输出。

### 7.2 稳定退出码

| 退出码 | 类别 |
| --- | --- |
| `0` | 成功 |
| `2` | 参数、schema 或配置错误 |
| `3` | 未登录、token 失效、需要重新认证 |
| `4` | 权限不足或需要 sudo |
| `5` | 网络、endpoint 或目标服务不可用 |
| `6` | 领域操作失败、冲突或资源状态不允许 |
| `7` | 部分成功，结果中必须列出逐项状态 |
| `8` | 超时或用户取消 |
| `9` | Tool 内部错误 |

具体领域错误通过 `error.code` 表达，不能不断扩展进程退出码。

### 7.3 错误要求

- 每个错误必须有稳定 code、可读 message、retryable 和 trace id。
- 认证失败、授权失败、资源不存在、CAS 冲突和服务不可用必须区分。
- 服务端错误详情先脱敏，再进入 JSON。
- `--verbose` 可以增加诊断链，但不得输出凭证和敏感业务内容。
- 不允许 panic stack、Deno stack trace 直接成为默认 stdout 协议。

## 8. 写操作、任务与审计

### 8.1 写操作分级

命令元数据必须声明：

- `read`：只读；
- `write`：可恢复或低风险修改；
- `destructive`：删除、卸载、覆盖、恢复等高影响操作；
- `privileged`：需要 sudo 或 Host 权限。

普通命令使用固定访问级别。`apply <operation-id>` 可以声明 `operation-defined` 策略，由 core
从可信 dry-run result 中取得实际级别和 sudo/确认要求。

`destructive` 默认要求交互确认，或者在非交互模式同时提供 `--yes` 和满足领域要求的
幂等键/operation revision。模块不得通过换动词规避分级。

### 8.2 Dry-run/Apply

可能触发调度、批量变更、数据删除、升级、恢复或 mount 的操作应优先支持：

```text
<module> dry-run -> operation_id / revision / diff / warnings
<module> apply   -> 引用 operation_id 或携带 expected_revision
```

`dry-run` 必须无业务副作用；允许读取远程状态、解析依赖和生成临时预检结果，但不得修改目标
资源。Dry-run result 必须声明 operation 的访问级别、sudo scope、确认摘要、revision 和过期时间。
Apply 先从可信服务读取这些元数据，再执行确认和授权；不能信任调用方自行提交的风险级别。
Apply 还必须验证 operation 未过期、revision 和目标当前状态，避免 Agent 基于过期结果执行。

### 8.3 异步任务

- 长操作必须返回 TaskManager `task_id`，不能由 CLI 进程持有唯一状态。
- 默认返回 task summary；调用者使用 `--wait` 或 `task wait` 等待。
- `--wait` 的超时只停止本地等待，除非用户明确 `task cancel`，不得隐式取消远程任务。
- task 进度通过 `jsonl` 输出，最终仍输出一次终态 envelope。
- 重试、取消、恢复能力以 TaskManager 真实能力为准，CLI 不伪造。

### 8.4 审计

每次写操作至少携带：

- principal / selected identity；
- source=`buckyos-tool`，Jarvis 调用时追加 agent/session 信息；
- module + verb；
- trace id；
- idempotency key；
- target resource；
- operation/revision；
- 最终 task id 或操作结果。

## 9. Agent First 要求

1. 默认 JSON，不依赖 TTY 检测改变协议。
2. `command list/describe` 提供机器可读的自描述能力。
3. 复杂输入支持 stdin JSON，避免 shell escaping 和 argv 长度限制。
4. 不在非交互模式询问问题；缺参数、确认或 sudo 时立即返回稳定错误。
5. 命令必须支持 trace id、idempotency key、timeout 和任务等待。
6. 输出中区分 desired state、observed state、task state，不能压成一个模糊的 `status`。
7. 所有 list 支持服务端分页；持续输出使用 jsonl。
8. 所有变更支持明确 selector，禁止“当前默认 App”“最近联系人”等隐式目标。
9. 对 secret、外部账号、文件路径等敏感字段实施统一脱敏策略。
10. help 示例不得成为唯一协议，schema 才是 Agent 依赖的稳定接口。

## 10. 模块边界

业务模块的详细需求独立维护，主 PRD 不重复具体字段和服务协议：

| 模块 | 文档 | 主要范围 |
| --- | --- | --- |
| User | [user.md](modules/user.md) | 用户、状态、类型、Profile、密码与 Message Tunnel 绑定 |
| App | [app.md](modules/app.md) | Catalog、安装事务、AppSpec、运行实例与可用性 |
| Contact | [contact.md](modules/contact.md) | 外部联系人、binding、关系和消息准入 |
| Message | [message.md](modules/message.md) | Zone/外部消息、会话、投递与回执 |
| Group | [group.md](modules/group.md) | Self-Host-Group、成员、角色、proof 和 subgroup |
| Object | [object.md](modules/object.md) | NamedData、NamedObject、Repo 持有与导入导出 |
| Content | [content.md](modules/content.md) | 分享、发布、版本、ACL 和可用性 |
| Files | [files.md](modules/files.md) | 可变层级文件系统操作 |
| Storage | [storage.md](modules/storage.md) | 目录发现、存储根和外部 mount |
| Backup | [backup.md](modules/backup.md) | 备份、校验、恢复和保留策略 |
| System | [system.md](modules/system.md) | Node/System/Service 生命周期、健康和更新 |
| Task | [task.md](modules/task.md) | Task、审计、日志和诊断 |
| DID Object | [did-object.md](modules/did-object.md) | DID resolve、对象验证和 DID Object Protocol |

新模块文档必须使用 [module-template.md](modules/module-template.md) 中的结构，并在本表注册。

## 11. 实施阶段

### Phase 0：Core Skeleton

- Deno 工程、launcher 和 `buckyos --version`；
- 两阶段 argv parser 与静态 registry；
- `command list/describe`；
- 新配置目录、profile 和覆盖优先级；
- JSON envelope、stderr、退出码；
- mock `CommandContext` 和单元测试。

### Phase 1：认证和最小在线闭环

- identity/session resolver；
- BuckyOS TS runtime 和 service client registry；
- `auth whoami/session-status`；
- `system status`；
- trace、timeout 和错误归一化。

### Phase 2：写操作框架

- confirm / `--yes` / non-interactive；
- sudo；
- idempotency；
- TaskManager create/wait/cancel；
- dry-run/apply 公共结构；
- audit metadata。

### Phase 3：业务模块迁移

按独立模块 PRD 实现。迁移的是正式服务能力，不是旧 Rust 函数。每个模块必须同时提交：

- 命令定义和 schema；
- service client 映射；
- 权限与 sudo 要求；
- mock 单测和至少一个真实 DV 用例；
- 模块文档状态更新。

### Phase 4：淘汰 buckycli

- 建立旧线上功能到新命令的覆盖矩阵；
- 包安装入口切换到 `buckyos`；
- Jarvis skill/提示词只使用新命令；
- 删除 Rust buckycli 的生产构建和发布入口；
- 开发工具若仍有价值，迁移到明确的 dev tool，而不是保留旧命令兼容层。

## 12. Core 验收标准

1. `buckyos --help`、`--version`、`command list/describe` 在无网络、无身份时工作。
2. 工具不会访问 `~/.buckycli` 或 `~/buckycli`。
3. `--config-dir`、环境变量、profile 和默认值的优先级有完整单元测试。
4. 同一个 handler 在 CLI flags 和 stdin JSON 下得到相同的类型化 input。
5. JSON 输出可以被逐次 `JSON.parse`，stdout 不混入日志和进度。
6. 每个失败路径产生稳定 error code 和退出码。
7. `--non-interactive` 下不会读取 stdin 密码或等待确认。
8. secret 不出现在默认输出、verbose 输出、错误和测试 snapshot 中。
9. `--wait` 超时不会隐式取消远程 task。
10. Deno 正式 launcher 不使用 `-A`。
11. Windows Desktop 可从本机 Deno 开发入口、Jarvis 容器和 paios 临时容器执行同一只读命令。
12. Linux 与 macOS 可使用同一命令 schema 和 JSON 输出执行同一在线操作。

## 13. 已确认设计决策

1. 安装器创建正式 `buckyos` 命令；开发和容器环境可以通过 Deno 原始入口运行
   `buckyos-tool`，两者共享实现。
2. 身份材料遵循 BuckyOS IdentityRoots 路径协议。Tool 私有 roots 优先于 BuckyOS 系统 roots；
   第一阶段不接入各操作系统的 Secret Service。
3. 第一阶段不保存或刷新 refresh token。支持命令行、文件和环境变量注入 session token；默认
   路径是用身份私钥在命令进程内换取短期 session token。
4. Windows 开发者可以直接使用 Deno/pnpm；普通用户优先在运行中的 Jarvis 容器执行，没有
   Jarvis 时使用 paios 临时容器。
5. 无副作用预演的标准动词使用 kebab-case 的 `dry-run`，执行使用 `apply`。
6. Tool 自行登录时使用稳定 appid `buckycli`；外部 session token 的 appid/app instance 以
   token claims 为准。
