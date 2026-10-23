# OpenDAN 并发刷新令牌导致 Agent 无回复：修复 TODO

状态：**待实施**。2026-10-22（America/Los_Angeles）已完成 demo.buckyos.io 只读诊断；本记录不表示已修复、部署或恢复线上实例。

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

- [ ] 在共享 runtime 的认证续期路径增加互斥，覆盖“重新读取凭据 → 判断是否需要刷新 → 刷新 RPC → 提交新 token pair”。后台、bind 和工具执行统一走此入口，不在各调用方分别加锁。
- [ ] 等待者取得锁后必须重新检查有效期；若其他调用已经完成续期，直接复用结果，不能继续使用加锁前读取的旧 token。
- [ ] 明确现有 session / refresh 锁的获取顺序，避免死锁；新凭据成对提交，失败不得留下新旧混合状态。检查直接登录、初始化等其他写入点与刷新之间的竞争。
- [ ] 网络超时、调用取消和任务退出不会永久占锁；区分“服务端可能已轮换但客户端未收到响应”与明确拒绝，有限重试需复用服务端宽限语义，不能盲目生成新登录。
- [ ] 确认续期成功后，消息桥接、服务状态上报和新启动工具读到更新后的 token；核对是否有长生命周期 client 持有旧快照。

### P1：认证失效后退出无效重试，并提供实例恢复路径

- [ ] 区分暂时网络失败、凭据失效、明确撤销及权限拒绝，复用现有错误分类和退避机制；避免仅靠重复文本日志或一律按网络错误重试。
- [ ] 对无法继续刷新的 App 实例进入明确的认证不可用状态，保留诊断原因和有界恢复尝试；不得不断用同一失效 refresh 重试并把进程存活当作健康。
- [ ] 确定由现有 node-daemon / 实例生命周期机制重新核验当前 AppInstance 授权并签发新 LoginAssertion 的恢复流程。Agent 容器不持有设备私钥，不能假定它能自行生成可信启动断言。
- [ ] 恢复保持 owner、AppId、AppInstance 和 AuthTarget 不变。账号停用、实例删除、显式撤销或授权拒绝时，不得通过自动重新登录绕过拒绝；需要重新授权的情况明确暴露给宿主。
- [ ] 失败期间不提前 ACK 输入、不改写消息为已读；恢复后沿现有消费与 outbox 幂等机制处理积压消息，避免丢消息或重复回复。
- [ ] 将持续 `bind_failed` / 认证失效暴露到现有 Agent 或实例健康视图；认证恢复完成后才清除错误。核对日志中的 `RenewToken recovered` 是否只是“暂时未到续期时间”，不能因此宣称认证会话已恢复。

### P1：发布与恢复验收

- [ ] 确认交付包同时包含客户端修复和上述 verify-hub 服务端能力，记录二进制 / 镜像的实际版本与提交。
- [ ] 用 node-daemon 的实例重建路径生成新的启动凭据，保留 AgentRootFS、消息库和 Session 数据。不要把 `docker restart` 旧容器作为恢复方案：旧环境中的 LoginAssertion 有短有效期，重启不会重新签发。
- [ ] 在隔离测试环境验证恢复流程后，再按部署任务处理 demo 上已失效的小白实例；本 TODO 的建立不代表线上操作已执行。
- [ ] 恢复验收记录同一条积压消息的消费、模型运行、回复投递和后续续期结果，不能只检查容器为 `Up`。

### 文档和调用方联动

- [ ] 同步 [runtime-login.md](../doc/sdk/runtime-login.md) 的续期并发、失效分类及宿主恢复职责；如果修改对外错误、状态或协议，同步 buckyos-api 共享类型、服务实现、SDK 与实际消费方。
- [ ] 更新受影响的 `buckyos_admin`、`opendan_agent_dev` 命令手册，记录健康诊断与实例恢复方式；若交付 / 镜像更新流程变化，同时更新 `buckyos_dev_release`。实现时写明各手册修改或无需修改的依据。

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

verify-hub 现有测试 `concurrent_refresh_with_one_token_shares_one_pair`、`restart_keeps_families_refreshable_and_grace_idempotent` 应继续通过。按最终改动范围完成构建和真实 Zone 的消息闭环验证；本次仅写 TODO，未运行这些测试。

完成标准：客户端不再产生同一凭据的并发刷新；认证失效可明确诊断并按授权恢复；积压消息恢复后恰好回复一次；正常续期、服务端重启及明确撤销的行为均有验证记录。不得以“升级后进程运行”或“某次模型调用成功”替代完整验收。
