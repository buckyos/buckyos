# beta2.2 主要版本改动

`beta2.2` 是一次面向 BuckyOS 下一阶段能力的大版本升级，不是小范围 bugfix。这个版本把系统从早期的本机服务、旧 Control Panel 和基础网关能力，推进到以 Web Desktop、AI-Native 开发循环、Workflow、统一数据存储、CYFS Named Data、可观测网络路由、WebSDK 能力扩展和更清晰内核模块边界为核心的新架构。

整体版本号从 `0.6.0` 升级到 `0.6.1`。主仓、`buckyos-base`、`cyfs-gateway`、`cyfs-ndn`、`buckyos-websdk` 需要使用匹配的 `beta2.2` 分支。

## 1. 新 Web Desktop 取代旧 Control Panel Web

- 新增完整的 React/Vite Web Desktop，作为新的用户入口。
- 删除旧 `control_panel/web` 前端，构建模块切换为 `desktop`，运行时仍安装到 `bin/control-panel/web/`，保持路径兼容。
- Desktop 内置应用扩展到 AI Center、App Service、Files、Message Hub、Home Station、Task Center、Users & Agents、Workflow、Settings、Diagnostics、Market、Studio、Code Assistant 等。
- Control Panel 后端从单体入口拆分为面向 Desktop 的多个管理 API 模块，包括系统设置、系统日志、用户管理、Zone 管理、Message Hub、AICC 设置、UI session 等。

## 2. AI-Native 开发循环：AICC、Workflow 和 Task Manager 联动

- AICC 从简单 provider adapter 升级为模型注册、模型路由、模型调度、会话管理、用量日志和 SN AI provider 的组合服务。
- 新增 FAL、SN AI provider，并扩展 OpenAI、Gemini、Claude、Minimax 等 provider 实现。
- 新增 `workflow` kernel service，提供 DSL、编译、分析、编排、执行器注册、AICC adapter、run subscription 和 task manager tracker。
- Workflow 支持把 run、step、map shard、thunk 等执行状态同步到 Task Manager，为 Agent-human-loop 和可视化任务编排打基础。
- 新增 `workflow` 文档和 DV 测试；当前 Workflow 仍偏一期 MVP，部分 definition/run store 使用进程内实现，后续需要继续补齐可靠持久化。

## 3. 内核基础设施增强

- `klog` 大幅增强为带 Raft 语义、状态机、持久化、强读、leader forwarding、request-id 幂等、快照和成员管理能力的内核日志组件。
- 新增 `klog_daemon`，提供 cluster bootstrap、auto-join、admin API、inter-node forwarding、本地 client RPC、graceful shutdown 和 benchmark 工具。
- `node_daemon` 增强 boot 流程、finder、gateway tunnel probe、name provider 注入、kevent server、run plist 和 node executor。
- `scheduler` 新增 `zone_route_builder` 和 `thunk_runner`，开始根据 device doc、network observation、probe 结果、SN、ZoneGateway 等信息构造服务路由和 forward plan。
- `buckyos-api` 扩展为更多服务的共享协议和客户端边界，新增 AICC usage log、group manager、msg center client、network observation、node control、RDB manager、repo client、workflow runtime/service/types、thunk object 等类型。

## 4. Gateway、Tunnel 和 RTCP 能力升级

- 新增 `cyfs-gateway-api` crate，把 Gateway control client、control handler、token factory/verifier、SN client 等公共 API 从应用内部下沉为可复用接口。
- Gateway 新增动态 `add-name-provider` 能力，支持运行时添加 HTTP/HTTPS name resolver provider 并设置 trust level。
- 新增 tunnel URL 状态查询和探测 API，支持批量查询、force probe、max age、timeout、排序策略和 caller priority。
- `TunnelManager` 增加 URL 级状态缓存、可达性探测、RTT 记录、TTL、并发限制、in-flight probe 合并、历史持久化和脱敏展示。
- `forward` 机制从简单 URL 选择升级为带 group、primary/backup、retry policy、失败摘除、weighted round robin、hash、consistent hash、ip hash、least_time 和 provider-first route 的 upstream 选择模型。
- HTTP forward 支持 connect-only fallback、HTTP 状态码 retry、request body buffer 重放和 provider failover scope。
- RTCP 增强握手、token/nonce 校验、source device info 解析、HKDF 会话密钥派生、AES-GCM record 加密、URL 级 probe、multi-IP 竞速、远端 bootstrap stream URL 和 reconnect 语义。

## 5. CYFS Named Data / NDN 架构升级

- `cyfs-ndn` 从早期本地 NDM / named-store 实现，演进为分层 CYFS Named Data stack。
- `NamedStore` 从本地文件系统绑定模型扩展为可插拔 backend，支持 local backend、HTTP backend 和 gateway-based remote bucket 访问。
- `NamedDataMgr` 聚焦 store layout、bucket 选择、remote access、GC 和 named-data 操作；`NamedFileMgr` 承担更高层的文件系统 namespace 编排。
- `ndn-lib` 提供 object、chunk、CYFS HTTP 和验证原语。
- `named_store` 提供 named-data 存储、layout 管理、远端 backend、gateway 和 GC。
- `cyfs` / `cyfs-lib` 提供上层 named filesystem 能力；`ndn-toolkit` 提供 client、directory server 和端到端测试工具。
- 新增 CYFS Protocol、NDM Protocol、Named Store HTTP Protocol、GC 等协议和架构文档。
- Gateway 侧新增 `cyfs-dir` server 类型，接入 `ndn-toolkit`、`named_store` 等能力，推动目录服务从旧 NDN server 形态转向更明确的 CYFS/NDN named data 服务。

## 6. DID、Name Client 和可信链升级

- `name-client` 新增地址 RTT 数据库，用于记录“本机出口 IP -> 远端地址”的连接质量，并根据历史 RTT、成功率、连续失败等数据对多 IP 结果排序。
- `resolve_ip` / `resolve_ips` 语义增强：优先从可信 Device DID 文档解析 IP，再合并 NameInfo、DeviceInfo 动态 IP，并结合 RTT 数据排序。
- DID 文档缓存策略改为优先比较 `version_seq`，避免旧文档仅因更晚 `iat` 覆盖新文档。
- OwnerConfig 新增 `mini_version_seq` 和 `valid_iat`，支持对旧 JWT DID 文档进行吊销和缓存清理。
- Owner、Zone、Device、Agent DID 文档的 `@context` 支持 BuckyOS 类型化 context。
- JWT DID 文档要求携带 `version_seq`；新生成文档默认 `version_seq = 0`。
- DID 文档新增 `capabilityInvocation` 和 `keyScope`，支持按 scope 限定 key 的用途，为内容发布、消息创建、Agent 支付、Agent 收款、Agent 内容创建等能力授权打基础。
- OwnerConfig 新增 wallets，可在 Owner DID 文档中声明多钱包地址。

## 7. 统一 RDB Instance 和 Message Center 扩展

- 新增 `RDB instance` 抽象，开始把系统服务的数据存储从服务内部本地实现迁移到统一数据库实例模型。
- `repo_service`、`task_manager`、`msg_center`、AICC usage log 等服务接入 RDB instance，支持 sqlite/postgres 等后端。
- Message Center 新增 Self-host Group 能力，扩展 group DID collection、group doc、member proof、subgroup、expansion snapshot 和多租户隔离模型。
- 新增 `buckyos-api` 中的 group manager、msg center client 和 RDB manager 类型，前后端及服务间协议边界更清晰。

## 8. WebSDK 开发者能力扩展

- WebSDK 从 `1.4.2` 升级到 `1.5.17`，从基础 runtime/client SDK 扩展为覆盖 AICC、KEvent、消息队列、消息中心、Repo、NDN/NDM 的开发者 SDK。
- 删除旧 `OpenDanClient` 入口，新增 `getAiccClient()`、`getKEventClient()`、`getMsgQueueClient()`、`getMsgCenterClient()`、`getRepoClient()` 等服务 client。
- 登录 API 拆分为 `loginByPassword()`、`loginByBrowserSSO()`、`loginByRuntimeSession()`；旧 `doLogin(username, password)` 不再作为入口导出。
- Runtime 重构为 profile 模型，分别处理 Browser、AppRuntime、AppClient、AppService 的服务 URL、SystemConfig URL、settings 路径、session 初始化和 token 续期差异。
- 浏览器登录从弹窗/postMessage 模式改为当前窗口 SSO 跳转，回跳后通过 `getAccountInfo()` 读取登录态。
- kRPC client 和 Runtime 增强 session token provider、token refresh 和 token changed 同步能力；`SystemConfigClient` 缓存从静态共享改为实例级，降低不同登录态之间的缓存污染。
- 新增 WebSDK NDN 类型系统，包括 `ObjId`、`ChunkId`、`FileObject`、`DirObject`、`PathObject`、`InclusionProof`、`RelationObject`、canonical JSON/JCS、chunk/object build/verify/load 等能力。
- 新增浏览器侧 NDM client，支持文件、目录、多文件、混合导入、QCID lookup、chunk/object 状态查询、TUS 上传、进度回调、thumbnail 生成和可替换 ImportProvider。
- 新增 NDM proxy client，封装 `/ndm/proxy/v1` 的 object/chunk 查询、reader/writer、pin/unpin、anchor/materialization、forced GC 等 API。
- 测试体系从 jsdom 为主重组为 node Jest、真实浏览器 Playwright、AppClient 集成测试和 AppService systest 分层，更贴近真实登录态和 runtime 行为。

## 9. 构建、发布和 CLI 改进

- 构建链路支持更多 beta2.2 依赖，包括 `buckyos-base`、`cyfs-ndn`、`cyfs-gateway`、`buckyos-http-server`、`cyfs-gateway-api`、`openssl` vendored、`sqlx` 等。
- 新增 `buckyos-http-server` crate，提供 Hyper server、kRPC over HTTP、router、DirServer、ETag、Range request、fallback file、autoindex、MIME 推断和路径穿越防护。
- 构建脚本增强 macOS 到 Linux target 的交叉编译处理，并更新 PAIOS / AIOS 镜像、exttool 镜像和 rootfs 安装流程。
- `buckycli` 新增本机 node runtime 控制命令：`check`、`start`、`ensure-running`、`stop`、`restart`、`detect-host-control`，为后续替代部分开发期 Python runtime 控制脚本打基础。

## 升级注意事项

- `beta2.2` 依赖主仓、`buckyos-base`、`cyfs-gateway`、`cyfs-ndn`、`buckyos-websdk` 的配套分支，不能只单独升级一个仓库。
- 旧的无 `version_seq` JWT DID 文档可能无法解析，需要迁移或重新签发。
- `keyScope` 会收紧 key 的授权语义，不能再假设认证 key 可以用于所有能力。
- Gateway forward retry、least_time、tunnel 状态缓存和 RTCP reconnect 会改变部分请求选路与失败恢复行为，发布前需要覆盖真实网络场景。
- Workflow 当前部分状态仍是进程内实现，不能把一期能力视为完整生产级持久化。
- 新 Desktop 里仍存在 prototype/mock 数据页面，产品化状态需要按具体应用逐项确认。
- WebSDK 调用方需要迁移 `doLogin`、`getOpenDanClient`、浏览器 SSO token 返回、同步 `getAccountInfo()` 等旧接口和旧流程。
