# AICC Provider 实现冻结设计

状态：**Frozen / Beta 2.2**

冻结日期：2026-09-14

本文冻结 Provider 的实现分解、装配路径和扩展规则。当前完整执行绑定由
[provider_operation_bindings.md](provider_operation_bindings.md) 生成并由 golden test 校验，不在本文手抄第二份易漂移矩阵。

## 1. 冻结模型

一个可运行 Provider Instance 由四部分组合：

```text
Known Provider / Provider Profile
  + Provider Rules
  + Protocol Adapter + operation codecs
  + Discovery behavior
  + Provider Instance settings/credential
  = Runtime Provider Instance + Inventory
```

各部分职责：

| 部分 | 拥有的事实 | 不拥有的事实 |
| --- | --- | --- |
| Provider Profile / Known Provider | 展示、默认 adapter、连接字段、凭据 contract、discovery behavior | 模型固有能力、wire codec |
| Provider Rules | 渠道 model -> origin model、operation、request lowering、渠道限制/价格 | 原厂模型固有能力、凭据明文 |
| Protocol Adapter | 官方 wire、认证形态、错误、stream/native task 状态机 | Provider Instance 配置、逻辑模型目录 |
| Model Driver | 原厂模型 API type、能力、variant、逻辑挂点、版本语义 | 渠道 endpoint/operation |
| Discovery | 某实例当前实际模型、可用性、动态能力/价格 | 静态语义真相 |
| Provider Instance | base URL、credential refs、region/workspace/account、enabled、实例规则 | 共享 adapter/driver 定义 |

禁止按 Provider 品牌复制整套协议，也禁止在 runtime 中按模型名前缀猜测能力或 operation。

## 2. Runtime 装配

1. metadata source manager 解析四层来源并构建完整 `CatalogSnapshot`。
2. `BuiltinProviderRegistry` 从 catalog 的 Known Provider 构造 Profile、连接 contract 与 discovery behavior。
3. `CodecRegistry` 通过 `src/protocol/plugins/*.rs` 自动发现并注册 adapter plugin。
4. settings 中每个 `ProviderSettings` 选择 profile、adapter、认证和实例参数。
5. registry 校验 profile 是否允许该 adapter、credential contract 是否一致、连接字段是否合法。
6. discovery 首选机器接口；失败时按配置使用 catalog-only/static inventory fallback。
7. `InventoryBuilder` 将 discovery、Provider Rules、Model Driver 和 codec bindings 求交，产出可路由 inventory。
8. inventory 与 `metadata_applied_seq` 原子提交 LKGS；runtime 捕获不可变 Provider/Model snapshot 后再发布。

推理请求只能使用已发布 snapshot。refresh、reload 或 metadata converge 不能原地修改正在执行的调用。

## 3. 内置范围

Beta 2.2 内置 Provider Profile 基线：

| Profile | 默认 adapter | Discovery |
| --- | --- | --- |
| `openai` | `openai-responses` | OpenAI models |
| `claude` | `claude-messages` | Anthropic models |
| `gemini` | `gemini-interactions` | Gemini models |
| `fal` | `fal-queue` | catalog/static fallback |
| `openrouter` | `openrouter-responses` | OpenRouter models |
| `minimax` | `minimax-messages` | MiniMax models |
| `kimi` | `kimi-chat` | Kimi models |
| `glm` | `glm-chat` | GLM models |
| `deepseek` | `deepseek-responses` | DeepSeek models |
| `doubao` | `doubao-responses` | standard/OpenAI-compatible models |
| `qwen` | `qwen-responses` | standard/OpenAI-compatible models |
| `sn` | `sn-openai` | SN models；支持 API key 与动态登录 |
| `custom` | 用户选择已注册 adapter | standard 或 configured/static fallback |

“内置”表示行为实现和验收基线，不表示把 Provider Rules 硬编码进 Rust。每个渠道 vendor 必须有独立 `.provider.json`；每个原厂模型 vendor 必须有独立 `.model.json`。

## 4. Protocol 复用

基础协议族当前包括 OpenAI Responses、OpenAI Chat Completions、Claude Messages、Gemini Interactions 与 fal Queue。派生 adapter 只保留真实差异：

- descriptor 完全一致时继承基础 codec；
- 普通 endpoint、模型映射、参数默认值、字段删除/改名优先写 Provider Rules；
- 特殊 body、header、SSE、原生 task、错误 envelope 才写 codec/dialect；
- 基础 adapter 不得出现派生 Provider 品牌分支；
- 新历史接口只有在真实 Provider operation 需要时才加入。

operation 是否可执行必须同时满足：typed `ApiType` 存在、codec 已注册、Model Driver 声明能力、Provider Rules 绑定 operation、实例 discovery 确认可用、合同测试通过。

## 5. Discovery 与 Inventory

`ProviderDiscovery::discover` 接收 `DiscoveryContext`，输出 `ProviderDiscoverySnapshot`。动态事实优先级为：

```text
Provider 实时 discovery
  > Provider Instance override / configured inventory
  > Provider Rules 渠道静态事实
  > Model Driver 保守估值
```

机器 discovery 失败不能让已经存在的 LKGS 消失。refresh 失败保留旧 inventory 和 applied seq，并更新 health；不完整结果不能部分提交。未知模型进入保守 fallback，不自动宣称 tool、JSON、vision 等能力。

Provider 停止、禁用、删除、reload 替换或服务退出时，必须先阻止新刷新，再停止并等待后台循环；旧 generation 不得在退出后提交 inventory 或 health。

## 6. 凭据与安全

- settings 持久化的是 locked credential/reference，不是可日志输出的明文。
- `CredentialResolver` 在调用边界解析；`ResolvedCredential` 只传入 codec context。
- adapter descriptor 冻结 Bearer、Named Header 等 contract；profile 可声明受控 credential variants。
- `base_url` 必须是无 userinfo/query/fragment 的绝对 HTTP(S) URL。
- Debug、管理 API、golden request、trace、task data 与错误正文必须脱敏。
- 协议 probe 的认证、限流、服务端错误不能误判为“不支持 adapter”并触发代际 fallback。

## 7. Provider 新增/修改流程

### 7.1 仅 metadata 接入

适用于 wire、认证和 discovery 都可复用的 Provider：

1. 添加/修改 `driver_metadata/known-providers/<id>.known-provider.json`；
2. 添加 `driver_metadata/providers/<id>.provider.json`；
3. 仅在引入新原厂模型语义时修改对应 `models/<vendor>.model.json`；
4. 复用已注册 adapter 与标准 discovery behavior；
5. 更新 binding golden、catalog validation 和 Provider 装配测试。

不得为了注册一个品牌创建空 Rust provider 模块。

### 7.2 需要新 wire 行为

1. 在 `protocol/` 增加最小 codec/dialect；
2. 在 `protocol/plugins/` 增加 `ProtocolAdapterPlugin`，由 build script 自动注册；
3. 为每个 `(operation, api_type)` 精确声明 execution modes 和 limits；
4. 如需机器 discovery，实现独立 `ProviderDiscovery` 并在行为 registry 注册稳定 behavior id；
5. Provider builtin 模块只负责无法由 catalog 表达的连接或登录行为；
6. 增加 request/response/error/SSE/native task golden tests 和装配测试。

### 7.3 修改已有 Provider

先判断变更属于 Model Driver、Provider Rules、Protocol Adapter、Discovery 还是 Instance。跨两层以上的修改必须逐层提供合同测试；不得把临时修复塞到 service/routing 层。

更详细操作步骤见 [How to add a Provider](maintenance/how_to_add_provider.md)。

## 8. Custom Provider

`custom` 允许用户选择任一已注册 adapter，并使用 `{}` Provider Rules 接入兼容 endpoint。其边界冻结为：

- 原始模型名直接参与所有 Model Driver 匹配；
- AICC 不自动去除厂商前缀、生成别名或猜 origin；
- 用户不能上传任意可执行 codec；
- 动态登录目前只属于明确注册的 SN 行为；
- “OpenAI-compatible”不等于所有 operation/stream/error 都兼容，必须以 probe 与 contract 为准。

## 9. 验收门槛

每个内置 Provider 至少覆盖：profile、连接字段与 credential 校验；主 operation 非流式调用；官方支持时的 stream；discovery 或 catalog-only fallback；rules/driver/codec 能力交集；错误归一化；health 与 inventory LKGS；metadata seq；生命周期停止；脱敏；合同及装配测试。

原生异步 operation 还必须覆盖 submit/status/result/cancel、retry-after、终态、超时、恢复和 TaskMgr bridge。

建议验证：

```bash
cd src
cargo test -p aicc protocol::
cargo test -p aicc provider::
cargo test -p aicc catalog::
cargo test -p aicc
```

## 10. 冻结后的变更规则

Profile/adapter/operation/behavior ID、credential contract、来源优先级、inventory 原子提交和 lifecycle 语义均为冻结边界。修改它们必须同步：本文、metadata schema、operation bindings、用户管理 API、UI/backend mapping、合同 fixtures 与验收矩阵。Beta 2.2 不增加旧 ID alias。
