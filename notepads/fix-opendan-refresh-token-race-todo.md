# OpenDAN 并发刷新令牌导致 Agent 无回复：修复 TODO

状态：**代码修复已实施，验证结果见第 5 节；补丁发布与发布后验收待执行**。2026-10-22（America/Los_Angeles）完成本地实现；10-23 已重建 demo 上的小白实例并验证回信，见第 6 节。线上仍为旧版本，不能将服务恢复视为补丁已上线。

## 1. 故障与证据

用户向小白发送“SF 今天天气如何”后没有收到回复。按消息正文确认，目标是 `xiaobai2.demo.buckyos.io@waterflier`，不是另一实例 `xiaobai.demo.buckyos.io`。

- 消息 ID：`cymsg:2b0337c5a09955c3eeb004acb16ce4340cd5939f54b182b5fafbeabc83077ef9`。
- 消息会话：`5069ad01-76a0-4a60-80e0-f900d9d0473c`。
- 发送时间：2026-10-22 18:30:19.147 UTC，即洛杉矶 11:30:19.147。
- msg-center 投递状态为 `SENT`，收件箱状态仍为 `UNREAD`；消息未进入 Agent worklog。
- Agent 最后一次模型运行仍是 `20261019-132041-76c6cb`。本次故障发生在会话绑定和消息消费阶段，尚未调用模型或天气工具。
- 线上构建：`0.7.0+build261019.main.def4c9592ea0`，对应提交 `def4c9592ea02f88b860e0ec150dad638759327e`。

以下日志时间均为 UTC：

| 时间 | 记录 | 含义 |
| --- | --- | --- |
| 10-19 14:44:54.307 / .345 | 同一 OpenDAN 进程两次 `use refresh-token to renew session-token from verify-hub...` | 两次刷新重叠执行 |
| 10-19 14:44:54.327 | verify-hub：`Refresh successful` | 第一次刷新完成轮换 |
| 10-19 14:44:54.361 | verify-hub：`Refresh reuse detected (jti-mismatch), revoking session` | 34 ms 后第二次使用旧令牌，整条认证会话被撤销 |
| 10-19 14:59:24 起 | `Refresh token not found or already invalidated`，随后 `bind_failed` | 已取得的短期 session token 临近过期，无法再次续期 |
| 10-22 18:30:03 / 18:30:35 | `refresh appclient session token before exec failed` | 天气消息发送前后，Agent 都处于同一认证故障中 |

证据位置：

- 服务端：`/opt/buckyos/logs/verify_hub/verify_hub.3497635.log`，行 1394–1401。
- Agent：`/opt/buckyos/logs/apps/6ba878bf6965f7c544ad94464f915fcb2aee3cb3074dea513a9d4d370a9ab345/opendan/opendan.1.log`，行 27293–27298、27333–27336、110564、110574。
- Agent session：`ui-e60c7587f7b3e73c1879e4ccb604ad89212f63bba7d37e74d99dc5d92a7b98a5`；其 `.opendan_agent_session/state.json` 中 `last_error.kind=bind_failed`、`inputs.q.acked_index=5`、`turn_seq=6`、`live_run=null`。
- 本地脱敏日志摘录：[evidence.json](../../diagnostics/xiaobai-weather-20261022/evidence.json)。该文件位于仓库外，仅作为本地排查附件；关键证据已摘录于本文。

## 2. 根因与当前实现

部署版本的 `runtime.rs::renew_token_from_verify_hub_once` 在读取 session token 后释放锁，刷新 RPC 期间只持有 refresh token 的共享读锁。后台 `keep_alive` 与会话 bind / 工具执行都能进入刷新，因而可能同时出示同一个旧 refresh token。

旧 verify-hub 对第二次请求的 jti mismatch 撤销整个 session，包括第一次刷新刚生成的凭据。客户端仅在 refresh token **为空**时尝试重新登录；对“非空但已失效”的 refresh 直接返回错误，后台与 session supervisor 持续重试，未恢复消息处理。进程存活不能证明 Agent 可工作。

日志已经证明两次刷新重叠；具体是哪两个调用入口相撞，现有日志未唯一标识，不应写成已证实的调用栈。候选入口如下：

| 模块 | 代码入口与职责 |
| --- | --- |
| [buckyos-api/runtime.rs](../src/kernel/buckyos-api/src/runtime.rs) | `keep_alive`、`renew_token_from_verify_hub`、`renew_token_from_verify_hub_once`、认证状态及后台重试 |
| [lib_opendan/runtime/mod.rs](../src/frame/lib_opendan/src/runtime/mod.rs) | `session_env_vars`，绑定 Session 时刷新工具环境 |
| [agent_tool/runtime_context.rs](../src/frame/agent_tool/src/runtime_context.rs) | `refresh_appclient_session_env`，工具执行前更新 session token |
| [verify_hub/main.rs](../src/kernel/verify_hub/src/main.rs)、[family_store.rs](../src/kernel/verify_hub/src/family_store.rs) | refresh 校验、轮换、重用撤销及持久化 |
| [node_daemon/app_loader.rs](../src/kernel/node_daemon/src/app_loader.rs)、[verify_hub_client.rs](../src/kernel/buckyos-api/src/verify_hub_client.rs) | 由宿主签发 App LoginAssertion，装配实例启动环境 |

截至本次核对的 main `de42ed261`，客户端尚无覆盖整个刷新流程的互斥，失效 refresh 的恢复缺口仍在；服务端已有以下实现，应复用，不重复开发：

| 提交 | 已有能力 | 本次任务中的处理 |
| --- | --- | --- |
| `959624aa7`（10-21） | 原子轮换提交检查、上一枚 refresh 的 10 秒宽限期；并发请求返回同一对 token | 作为修复基线，运行现有并发刷新回归 |
| `0b9faa18a`（10-22） | SQLite `sessions.db` 持久化及重启后刷新恢复 | 保留并回归；此次最初故障由刷新竞态触发，不能归因为 verify-hub 重启丢缓存 |

## 3. 修复任务

### P0：客户端刷新只允许一个请求执行

- [x] 在共享 runtime 的认证续期路径增加互斥，覆盖“重新读取凭据 → 判断是否需要刷新 → 刷新 RPC → 提交新 token pair”。后台、bind 和工具执行统一走此入口，不在各调用方分别加锁。
- [x] 等待者取得锁后必须重新检查有效期；若其他调用已经完成续期，直接复用结果，不能继续使用加锁前读取的旧 token。
- [x] 明确现有 session / refresh 锁的获取顺序，避免死锁；新凭据成对提交，失败不得留下新旧混合状态。检查直接登录、初始化等其他写入点与刷新之间的竞争。
- [x] 网络超时、调用取消和任务退出不会永久占锁；区分“服务端可能已轮换但客户端未收到响应”与明确拒绝，有限重试需复用服务端宽限语义，不能盲目生成新登录。
- [x] 确认续期成功后，消息桥接、服务状态上报和新启动工具读到更新后的 token；核对是否有长生命周期 client 持有旧快照。

### P1：认证失效后退出无效重试，并提供实例恢复路径

- [x] 区分暂时网络失败、凭据失效、明确撤销及权限拒绝，复用现有错误分类和退避机制；避免仅靠重复文本日志或一律按网络错误重试。
- [x] 对无法继续刷新的 App 实例进入明确的认证不可用状态，保留诊断原因和有界恢复尝试；不得不断用同一失效 refresh 重试并把进程存活当作健康。
- [x] 确定由现有 node-daemon / 实例生命周期机制重新核验当前 AppInstance 授权并签发新 LoginAssertion 的恢复流程。Agent 容器不持有设备私钥，不能假定它能自行生成可信启动断言。
- [x] 恢复保持 owner、AppId、AppInstance 和 AuthTarget 不变。账号停用、实例删除、显式撤销或授权拒绝时，不得通过自动重新登录绕过拒绝；需要重新授权的情况明确暴露给宿主。
- [x] 失败期间不提前 ACK 输入、不改写消息为已读；恢复后沿现有消费与 outbox 幂等机制处理积压消息，避免丢消息或重复回复。
- [x] 将持续 `bind_failed` / 认证失效暴露到现有 Agent 或实例健康视图；认证恢复完成后才清除错误。核对日志中的 `RenewToken recovered` 是否只是“暂时未到续期时间”，不能因此宣称认证会话已恢复。

### P1：发布与恢复验收

- [ ] 确认交付包同时包含客户端修复和上述 verify-hub 服务端能力，记录二进制 / 镜像的实际版本与提交。
- [ ] 用 node-daemon 的实例重建路径生成新的启动凭据，保留 AgentRootFS、消息库和 Session 数据。不要把 `docker restart` 旧容器作为恢复方案：旧环境中的 LoginAssertion 有短有效期，重启不会重新签发。
- [ ] 在隔离测试环境验证恢复流程后，再按部署任务处理 demo 上已失效的小白实例；本 TODO 的建立不代表线上操作已执行。
- [ ] 恢复验收记录同一条积压消息的消费、模型运行、回复投递和后续续期结果，不能只检查容器为 `Up`。

### 文档和调用方联动

- [x] 同步 [runtime-login.md](../doc/sdk/runtime-login.md) 的续期并发、失效分类及宿主恢复职责；如果修改对外错误、状态或协议，同步 buckyos-api 共享类型、服务实现、SDK 与实际消费方。
- [x] 更新受影响的 `buckyos_admin`、`opendan_agent_dev` 命令手册，记录健康诊断与实例恢复方式；若交付 / 镜像更新流程变化，同时更新 `buckyos_dev_release`。实现时写明各手册修改或无需修改的依据。

## 4. 验证与完成条件

测试应通过同步屏障、可控时间或已有 mock 稳定制造竞争，不能依靠随机延时偶然复现。

| 场景 | 必须验证的结果 |
| --- | --- |
| 后台续期与 bind / exec 同时发现过期 | 同一 runtime 只发一次刷新 RPC；所有调用获得一致的新凭据，认证会话不被撤销 |
| 多个等待者随后取得刷新锁 | 在锁内重查并复用结果，不再次出示旧 refresh |
| RPC 失败、超时或调用取消 | 锁可再次获取，凭据不发生部分提交；后续恢复行为有界且可观察 |
| 服务端已轮换、响应丢失 | 宽限期内重试获得同一 token pair；不会错误撤销有效会话 |
| refresh 非空但已失效，初始 LoginAssertion 也过期 | 不进入无限无效刷新；宿主能通过重新核验授权、签发新凭据和实例重建恢复 |
| 用户停用、实例删除或明确撤销 | 保留拒绝语义，不自动越权重新登录或复活已撤销会话 |
| Agent 失效期间收到新消息 | 消息保留可恢复；恢复后只消费一次、只回复一次，无输入或 outbox 丢失 |
| 连续两轮续期与 verify-hub 重启 | Agent 持续可用；服务端持久化与宽限期回归通过 |

实施后在 `src/` 下运行相关检查并回填实际结果：

```bash
cargo test -p buckyos-api -- --test-threads=1
cargo test -p verify_hub -- --test-threads=1
cargo test -p agent_tool -p libopendan -p opendan -- --test-threads=1
# 修改 node-daemon 恢复流程时执行
cargo test -p node_daemon -- --test-threads=1
```

verify-hub 现有测试 `concurrent_refresh_with_one_token_shares_one_pair`、`restart_keeps_families_refreshable_and_grace_idempotent` 应继续通过。实际检查结果记录在第 5 节；真实 Zone 的消息闭环与镜像发布单独记录，未执行的检查不计为通过。

完成标准：客户端不再产生同一凭据的并发刷新；认证失效可明确诊断并按授权恢复；积压消息恢复后恰好回复一次；正常续期、服务端重启及明确撤销的行为均有验证记录。不得以“升级后进程运行”或“某次模型调用成功”替代完整验收。


## 5. 实施记录（2026-10-22）

- `buckyos-api/runtime.rs`：共享续期锁覆盖重读、过期检查、RPC 和凭据提交；有限重试及共享退避；结构化认证状态保留拒绝原因，失效后不自动重登。pair 与认证状态一起提交，处理取消与结果未知的刷新。
- `verify_hub/main.rs`：沿用 `AccessDenial`，明确过期、非法、重用与 family 已失效；复用已有 10 秒宽限期和 SQLite family 存储；App 登录/续期重新核验 owner、实例与 Agent binding。
- `buckyos-api/agent_permissions.rs`、`node_daemon/app_loader.rs`：签发前读取当前授权事实，拒绝停用账号、删除/禁用实例及陈旧部署配置。继续使用获授权的 `apps.restart` / `restart_strategy=recreate`，不会因认证失败自动创建新 family。现有容器重建保留卷和 AgentRootFS。
- OpenDAN/libopendan/agent_tool：`loader.status.authentication` 与 Loader 页面暴露认证状态；消息调用读最新 runtime token；绑定认证错误保持 `bind_failed`，不当作工作区永久失败，不提前 ACK 输入；直接登录 pair 提交也共用 runtime 入口。
- 文档：更新 `runtime-login.md`；`buckyos_admin`、`opendan_agent_dev` 增加健康诊断和显式重建流程；`buckyos_dev_release` 增加宿主二进制与 aios 镜像配套交付检查。没有新增依赖、RPC 方法或存储结构。SDK 已支持现有 `access_denied` / `credential_state`，无需新增 TS 错误类型；OpenDAN WebUI 类型同步新增的状态字段。

### 验证记录

Rust 回归结果（`src/` 下，统一 `--test-threads=1`）：

| Package | 通过 | 失败 | 既有忽略项 |
| --- | ---: | ---: | ---: |
| `buckyos-api` | 305 | 0 | 0 |
| `agent_tool` | 271 | 0 | 6 |
| `libopendan` | 348 | 0 | 1 |
| `node_daemon` | 130 | 0 | 3 |
| `opendan` | 40 | 0 | 1 |
| `verify_hub` | 36 | 0 | 0 |

共 1130 项通过，11 项既有测试（含 1 项文档示例）按原设置忽略。首次六包联合回归发现旧 `tool_session_token` 用例假设原始 token 回写能清除终止状态；已调整为先验证正常轮换，再验证终止状态拒绝手工回写恢复，定向测试通过，并完整重跑 `libopendan`、`node_daemon`、`opendan`、`verify_hub`（`--no-fail-fast`）。`buckyos-api`、`agent_tool` 的单测及集成测试已在首次联合回归中全部通过，文档测试单独补跑通过。

实际命令：

```bash
cargo test -p buckyos-api -p verify_hub -p node_daemon -p agent_tool -p libopendan -p opendan -- --test-threads=1
cargo test -p libopendan --test tool_session_token -- --test-threads=1
cargo test -p libopendan -p node_daemon -p opendan -p verify_hub --no-fail-fast -- --test-threads=1
cargo test -p buckyos-api -p agent_tool --doc -- --test-threads=1
cargo build -p buckyos-api -p verify_hub -p node_daemon -p agent_tool -p libopendan -p opendan
```

- `cargo build`：通过，生成本机开发构建产物；未执行完整 `buckyos-build.py`、跨平台构建或镜像构建。
- OpenDAN WebUI：`pnpm check`、`pnpm lint`、`pnpm build` 全部通过。
- `git diff --check`：通过。
- 并发回归通过屏障同步 16 个等待者，验证一次 RPC 和第二轮续期；响应丢失、取消、部分提交、缓存同步取消、有界超时、越过宽限窗口及永久拒绝均有用例。
- `concurrent_refresh_with_one_token_shares_one_pair`、`restart_keeps_families_refreshable_and_grace_idempotent` 及 logout/reuse 拒绝用例通过。
- `authentication_failure_keeps_inputs_and_outbox_until_new_runtime_recovers` 使用独立进程制造真实 runtime 失效，确认输入/待发回复保留、工作区 Session 不被终止、恢复后回复 ID 保持且只回复一次；`authentication_failure_keeps_the_inbox_unread_until_recovery` 验证失败时未读消息保留。这些是隔离回归，不代替真实 Zone / 模型服务验收。

### 尚未执行的发布验收

本机 `uv run check.py` 显示已激活、核心进程与端口可用，但运行的已安装服务不是本次改动产物；因此不能据此声称修复已在 Zone 生效。本次不替换宿主服务、不构建/发布 aios 镜像、不重建 demo 实例。部署时记录 verify-hub/node-daemon 二进制和运行镜像的实际版本、提交与摘要，按第 3 节逐项验证同一积压消息及后续两轮续期。模型/天气工具的真实调用、跨服务消息投递和生产恢复尚未验收。

## 6. 线上服务恢复记录（2026-10-23）

用户授权线上修复后，确认宿主服务在 18:27 UTC 重启，但小白容器仍是 10-19 启动的进程。宿主二进制内嵌版本和容器镜像仍为 `def4c959`，认证失效继续阻塞消息处理。

- 先保存实例、挂载、App 状态和 Session 快照到服务器 `/opt/buckyos/local/diagnostics/xiaobai2-recovery-20261023/`。
- 执行 `/opt/buckyos/bin/buckyos --yes --timeout 30s app restart xiaobai2.demo.buckyos.io@waterflier --strategy recreate --no-wait`；任务 `t-f200e402b40c47818428d5bf5d9a5b35` 最终 `Succeeded`。
- 新容器 `d3c633acaa69` 于 18:43:19 UTC（洛杉矶 11:43:19）登录成功；重建前后持久挂载完全一致，保留 AgentRootFS、instance volume 和 storage。
- 昨日及当日 18:40:29 UTC 发送的两条“SF 今天天气如何”均从 `UNREAD` 变为 `READ`，合并进入 turn 7。AICC 任务 `t-9b635b03f6ea4574928dfeeb7e2c9b34` 成功；最终回复 `cymsg:6aa1ccdde77949d512415e6e920c1a997b2763c8e6b1a9254e6c362af24f6548` 于 18:43:26.832 UTC 投递成功。
- 验证后 `turns_completed=7`、`last_error=null`、`live_run=null`，最后运行 `20261023-184319-305727`。本地恢复记录：[recovery-result.json](../../diagnostics/xiaobai-weather-20261023/recovery-result.json)。

本次仅恢复了实例认证及真实消息回复；未升级共享宿主服务或 aios 镜像，未验证后续两轮自然续期，也未独立核验模型所答天气内容。旧版无受支持的单 Agent worker image 配置，镜像选择来自全局 `devenv.json.aios`；没有为本次恢复改变其他实例的运行版本。第 3 节补丁发布验收仍待执行，旧版刷新竞态仍有复发风险。
