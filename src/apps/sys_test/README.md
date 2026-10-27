# System test

在 node-daemon 启动的 AppService 中运行。后台先将 `BUCKYOS_APP_TOKEN` 中的设备登录断言交给 Verify Hub，换取绑定当前应用实例的 session；传给子进程的是 SDK 当前有效的 session。

页面从 `/sdk/appservice/runtime` 读取应用实例和 Zone 域名，再初始化 Browser SDK。后台优先使用 `BUCKYOS_ZONE_HOST`，否则解析 node-daemon 注入的 `BUCKYOS_ZONE_CONFIG.zone_document`（JSON 或 JWT），避免将应用子域名误当成 Zone 导致 SSO 跳转失败。

## 自测约定

- AppService 验证实例身份与 NodeGateway 服务地址，不要求 SDK 从容器本地文件推断 Zone 域名。
- KEvent 订阅和发布通过网关 HTTP 接口并携带当前 session；后台显式使用 `KEventClient` HTTP transport，避免默认 native transport 连接容器自己的 localhost。
- 页面使用 `SystemConfigClient` 读写 `users/<user>/apps/<app>/settings`，并恢复原值。当前 SDK Browser 的 `getAppSetting/setAppSetting` 使用系统服务命名空间，仅在后台 AppService 用例验证应用设置便利方法。
- 本应用的 `dapp_meta/app.json` 未申请 MsgCenter 收件箱权限；自测要求 `peekBox({ mailbox })` 返回对应资源的权限拒绝。它不代表已验证获授权的收件箱读取。
- NDM Proxy 后台同时验证随机对象查询返回 `not_exist` 和受限 `outboxCount` 返回明确的 403；页面验证 Browser runtime 不允许访问 Proxy。网络错误、协议错误和非预期权限错误仍算失败。
- TaskManager 生命周期以归档结束，不调用已经过时的删除接口。

## xllm 身份回归（#640）

在页面的 **xllm 子进程身份（#640）** 卡片点击“在后台服务中运行检测”，或向应用发送：

```text
POST /sdk/appservice/selftest/xllm
```

该组也包含在 `POST /sdk/appservice/selftest` 全量检测中。

测试可在 OOD 的 node-daemon 启动的应用容器或 Desktop Lite 原生应用进程中运行，保留注入的 `BUCKYOS_THIS_DEVICE`、`BUCKYOS_ZONE_CONFIG` 和 `BUCKYOS_HOST_GATEWAY`。#640 的容器发布验收仍应在 Docker 应用容器内执行，不要清空设备文档或使用宿主网络绕过。运行环境需要 bash 和待验证的 `agent_tool`。

后台使用 `Deno.Command` 启动 `/bin/bash`，再执行 `agent_tool xllm`，将 AppService session 通过 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 传入。测试禁用模型工具，只要求一个固定短回答，会产生一次真实模型请求及少量用量。

通过条件包括：xllm 正常完成，AICC 用量事件的 `trace_id` 对应此次 `run_id`，`caller_app_id` 等于业务 AppId，`user_id`、`tenant_id` 等于安装账号。完整实例身份检查使用 `attribution.caller=app:<AppId>@<owner>`，同时要求 `attribution.app_id`、`user_id`、`operator_id` 与本应用和安装账号一致，`mode=direct`；启动前仍严格核对 AppService session 的实例 claims。返回结果只包含身份、运行 ID、任务 ID 等证据，不包含 token。运行目录在结束后删除；AICC 用量记录保留供核对。

可选环境变量：

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `BUCKYOS_SYSTEST_AGENT_TOOL` | `$BUCKYOS_ROOT/bin/opendan/agent_tool` | 指定候选版可执行文件，`BUCKYOS_ROOT` 默认 `/opt/buckyos` |
| `BUCKYOS_SYSTEST_XLLM_MODEL` | `llm.chat` | 使用该 Zone 可用的逻辑模型或精确模型 |

模型请求超时为 60 秒，任务超时为 90 秒，后台在 120 秒时强制结束未退出的子进程。`deno task start` 已包含运行 bash 和写入临时目录所需权限。

`pnpm test` 运行不调用模型的身份断言回归，覆盖正确业务 AppId、完整实例归属，以及错误或缺失身份字段、错误实例 token 的拒绝行为。2026-10-26 使用 `service_debug.tsx` 的环境装配逻辑建立 Desktop Lite 原生 AppService 会话，候选代码的真实 xllm 请求 `20261026-132819-0c00e9` 完成，AICC 的应用、实例、用户与租户归属检查通过；该次验证未覆盖 Docker 容器链路。

## 客服群与客户链接（Self-host Group 扩展官方例子）

对应 [应用扩展实施 TODO](../../../notepads/self-host-group-extension-todo.md) §11（EXT-10 / EXT-11）：一个真正装在 Zone 里的 App 用“L1 配置 + L2 面板 + 客户链接”服务外部客户。页面上的 `support_group` 卡片有三个按钮，源码在 `web/src/support_group.ts`（页面）与 `support_group.ts`（后台）。

| 步骤 | 谁的身份 | 做了什么 |
| --- | --- | --- |
| 创建客服群 | 页面上登录用户在本 App 子域的会话 | `user.list` 取两位成员的 DID → msg-center `group.create`：模板 `support`（`allow_guests`、`post=all_participants`）、具名 Session `support`、群级 `display.announcement`，并把本 App 的 `GET /support/panel` 声明为面板 `tickets`（D-2：面板 host 必须是群主 Zone 的 App 子域） |
| 生成客户链接 | 页面把自己的会话以 `Authorization: Bearer` 交给后台；后台先问 CP `auth.current_identity` 确认是本 App 实例的安装账号，再用 **App 服务会话**调 `user.app_login.issue {kind:"link", resource:{group_session}, return_path:"/messagehub/guest?group=&session="}` | 得到 `https://<zone>/sso/invite/<id>#s=<secret>`，有效 48 小时；CP 同时以 App 身份预置该账号的 Session 邀请。同一外部标识再次生成复用同一 Limited 账号 |
| 清理 | 页面用户会话 | `group.delete`。客户账号与链接记录保留，链接到期自动失效 |

客户点链接：未设密码时直接以自己的受限账号进入 `/messagehub/guest`，顶部出现非阻塞的“设置密码”提示（不点即跳过），点“加入”后聊天，侧栏显示“客服工单”面板。设置密码后再点同一链接会要求先输密码，密码登录后回到同一会话。面板内容来自后台 `GET /sdk/appservice/support/summary`（App 服务会话读 `group.list_messages`，只给条数、参与者数和时间，不给正文）。

前置：App 实例 owner 在 App Service 的应用详情页打开“本应用为外部客户代登录”开关（`apps.app_login.set`），成员账号（缺省 alice、bob）存在。限制：只有 App 实例的安装账号能生成链接；客户是 Session Only Member，不进群成员表；面板摘要接口对持有群 DID 的任何人可读（只有计数）；`/support/panel` 由 `www` 端点的 `allow_guest: true` 放行，所以 iframe 内无 App 会话也能加载。

该开关默认关闭，创建客服群不会自动打开。关闭时 `user.app_login.issue` 返回 `access_denied`，结构化 `data.reason=zone_method_disabled`、`data.detail=app_login_disabled`；页面据此提示安装账号开启开关。这是应用实例的授权前置条件，Desktop Lite 同样适用。

验收脚本：后端 `test/test_sys_test/support_group_dv.ts`（见文件头）；浏览器 `src/frame/desktop/tests/dv/support-group-link.spec.ts`。

## 构建检查

```bash
pnpm install
pnpm build
```

SDK 依赖跟随 `buckyos-websdk#main`。每次 `pnpm build` 先更新工作区根目录和 `web/` 的 SDK 依赖，再执行 Deno 后台和浏览器 TypeScript 类型检查，生成可打包的 `dist/`。`pnpm build:web` 和 `web/` 内的 `pnpm build` 也会先更新 SDK，无需手动执行 `pnpm update`。

静态 HTML（含根路径、`index.html` 与 SPA 回退）使用 `Cache-Control: no-store`，不发布基于文件时间和长度的 ETag / Last-Modified；携带旧缓存校验头也返回当前页面。PIKG 解包文件的时间可能固定为 1970 年，同长度的新旧 HTML 不能据此判断未修改。带 hash 的 JS / CSS 保留静态服务器缓存行为。`pnpm test` 同时覆盖同长度、零时间戳页面升级、旧条件请求、GET / HEAD / SPA 回退及资产、404、405 行为；无需更新 SDK 或运行真实模型。

在已由 node-daemon 启动的应用页面，分别点击各组的页面/后台检测按钮。也可向应用地址发送 `POST /sdk/appservice/selftest` 运行全部后台用例；HTTP 200 且 `ok: true` 才算通过。全量检测包含上述 xllm 真实模型请求。

## 验证记录（2026-10-03）

使用 WebSDK `main` 的 `0.7.126`（`3cfc984`）在 `test.buckyos.io` 验证：后台 17/17、浏览器 15/15；真实 SSO 登录、NDM 小文件上传及存储状态回查通过。无 token/无效 token 的 AICC 请求被拒绝，未知自测分组（含 `toString`、`__proto__`）返回 404。

`pnpm build` 和在 `src/` 执行的 `uv run buckyos-build.py -s buckyos_systest` 均通过，生成 `dapp_dist/buckyos-systest.buckyos.bns.did-0.5.1.pikg`。后台使用现有 OOD 应用容器内的临时候选进程，保留容器网关和设备环境；浏览器加载候选静态文件并连接该进程，SSO 与系统服务请求走真实 ZoneGateway。未替换已安装应用；大文件分块上传、断线重试及长期运行未在本轮验证。

## 应用对象桶冒烟（DV）

`dapp_meta/app.json` 声明一个小桶 `smoke`（`data` / `temp`，默认 1 天过期，`quota_mib` 16，App 池 `storage_quota_mib` 64），实例为 `user/buckyos-systest.buckyos.bns.did/<user>/smoke`。页面的“应用对象桶（DV 冒烟）”组：

| 用例 | 写入 | 读取 | 删除 |
| --- | --- | --- | --- |
| `putBytes → objectUrl → bucket.delete` | `BucketClient.putBytes`（`PUT /bucket/v1/...`，`if_none_match: *`） | `objectUrl` 只带 Cookie 的 `fetch`（同 `<img src>`） | `bucket.delete` 后 `fetchObject` 必须得到错误码 `no_such_key` |
| `ndm.importFiles + ensureUploaded({ bind })` | NDM 上传，finalize 时 `auto_bind` 直达桶，结果带 `binding` | 同上 | 同上 |

后台 `POST /sdk/appservice/selftest/bucket` 用 AppService 会话对 `user/<appid>/<安装账号>/smoke` 做 `putBytes → fetchObject → delete`。失败时用例会尽力删除自己写的 key；漏删的 key 一天后由桶的 TTL 解绑。DV 验收：两组都通过，且 `buckyos bucket stat user/buckyos-systest.buckyos.bns.did/<user>/smoke` 的 key 数回到 0。依赖 websdk 的 `bucket.userBucketId`、`ndm.ensureUploaded({ bind })`（本仓库构建时取 `buckyos-websdk#main`，需先发布）。

## 文件 / 目录 SDK 导入演示

NDM 卡片提供单文件、多文件和目录选择，统一调用 `ndm.pickupAndImport` → `ndm.ensureUploaded`。页面只展示 SDK 给出的稳定 ObjId、对象就绪状态、逻辑进度、实际发送 payload 字节与查重警告；不重算 ChunkList，不读取 SDK 私有字段，不向后台发送 canonical 对象登记请求。旧 `/sdk/appservice/ndm_query` 登记接口已删除，浏览器使用自己的当前用户 / AppInstance session 完成 NDM finalize。

默认只取就绪 ObjId，不产生实体文件。可填写有权限的目标父目录，点击“按 ObjId 本地提取”，由 `NfspClient.commitObject` 请求服务端通过 NamedStore 本地提取并提交路径；不会再从浏览器传文件字节。目标已存在时不覆盖，多项提取不宣称目录原子提交；页面保留已确认成功项，失败后点击提取只处理未确认项。网络响应丢失后若路径已存在，应检查目标内容再决定新路径，不能盲目覆盖。

取消覆盖选择后哈希、查重、上传和 finalize；SDK session 内可重试失败任务，已上传 chunk 不重传，只有就绪才允许目标提取。释放任务或刷新后需重新选择，服务器仍可去重。目录 picker 保留根和嵌套路径，但 FileList 无法表达空目录，页面展示此限制；能枚举目录节点的应用应调用 `ndm.importDirectory` 保留空目录。不支持目录 picker 的浏览器仅启用文件按钮。

重复验收建议：

1. 首次导入单文件、多文件、目录及 >32 MiB 文件，确认状态最终为 `ready`，记录实际发送字节。
2. 重新选择相同内容、改名文件及只修改一个叶文件的目录，确认授权 QCID 命中没有完整哈希，完全命中发送 0 字节，部分命中只传缺失内容。
3. 在上传、登记、目标提交阶段分别取消或断网，检查 SDK 失败原因和重试；登记失败绝不能展示就绪。
4. 同一个 ready ObjId 分别只取 ID 和提交目标目录，检查最终根目录、叶文件内容与空目录策略。普通用户和不同 AppInstance 分别验证；查重存在性不自动授予复用权限。
5. 页面显示临时上传 holding 就绪不代表业务持久 retention；此 demo 的只取 ID 分支不会创建长期业务引用。真实应用应在业务提交中接管 retention。

这些步骤要求已部署支持 `/ndm/v1/import/finalize`、授权 `reusable` lookup 和 NFSP `commit_object` 的服务。源码构建 / mock 测试通过不代表已在真实 Zone 完成上述验收；账号角色、SDK/服务版本、浏览器及实际发送字节应随验收记录保存。

### 目录选择回归（2026-10-26）

在 macOS Desktop Lite、Chrome 156、`test.buckyos.io` 上，用本地 WebSDK `0.7.148` 加 picker 修复构建 systest 候选页，复用 devtest 的应用会话，经真实 ZoneGateway 验证原生目录选择及上传：2 个文件、2 个目录、63 字节最终为 `ready`；重复选择相同目录得到相同 ObjId，实际发送 0 字节。SDK 回归覆盖焦点恢复后延迟返回文件列表、原生取消、AbortSignal、空目录及多文件。修复不再以窗口恢复焦点后的固定超时推断取消。

同轮确认：未开启实例代登录时，客服链接显示具体的开关提示；SDK 初始化后才渲染客服卡片，已有群的 Message Hub 链接正确。未验证 Safari/Firefox、大目录、开启代登录后的客户登录流程；未替换已安装应用。
