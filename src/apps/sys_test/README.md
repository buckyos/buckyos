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

发布验收应在 OOD 上的 Docker 应用容器内运行，保留 node-daemon 注入的 `BUCKYOS_THIS_DEVICE`、`BUCKYOS_ZONE_CONFIG` 和 `BUCKYOS_HOST_GATEWAY`。不要清空设备文档或使用宿主网络绕过 #640。容器需要 bash 和待验证的 `agent_tool`。

后台使用 `Deno.Command` 启动 `/bin/bash`，再执行 `agent_tool xllm`，将 AppService session 通过 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 传入。测试禁用模型工具，只要求一个固定短回答，会产生一次真实模型请求及少量用量。

通过条件包括：xllm 正常完成，AICC 用量事件的 `trace_id` 对应此次 `run_id`，且事件中的 `caller_app_id`、`user_id`、`tenant_id` 与当前应用实例和所属用户一致。返回结果只包含身份、运行 ID、任务 ID 等证据，不包含 token。运行目录在结束后删除；AICC 用量记录保留供核对。

可选环境变量：

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `BUCKYOS_SYSTEST_AGENT_TOOL` | `$BUCKYOS_ROOT/bin/opendan/agent_tool` | 指定候选版可执行文件，`BUCKYOS_ROOT` 默认 `/opt/buckyos` |
| `BUCKYOS_SYSTEST_XLLM_MODEL` | `llm.chat` | 使用该 Zone 可用的逻辑模型或精确模型 |

模型请求超时为 60 秒，任务超时为 90 秒，后台在 120 秒时强制结束未退出的子进程。`deno task start` 已包含运行 bash 和写入临时目录所需权限。

## 构建检查

```bash
pnpm install
pnpm update -r buckyos
pnpm check
pnpm build
```

SDK 依赖跟随 `buckyos-websdk#main`。`pnpm build` 包含 Deno 后台和浏览器 TypeScript 类型检查，再生成可打包的 `dist/`。

在已由 node-daemon 启动的应用页面，分别点击各组的页面/后台检测按钮。也可向应用地址发送 `POST /sdk/appservice/selftest` 运行全部后台用例；HTTP 200 且 `ok: true` 才算通过。全量检测包含上述 xllm 真实模型请求。

## 验证记录（2026-10-03）

使用 WebSDK `main` 的 `0.7.126`（`3cfc984`）在 `test.buckyos.io` 验证：后台 17/17、浏览器 15/15；真实 SSO 登录、NDM 小文件上传及存储状态回查通过。无 token/无效 token 的 AICC 请求被拒绝，未知自测分组（含 `toString`、`__proto__`）返回 404。

`pnpm build` 和在 `src/` 执行的 `uv run buckyos-build.py -s buckyos_systest` 均通过，生成 `dapp_dist/buckyos-systest.buckyos.bns.did-0.5.1.pikg`。后台使用现有 OOD 应用容器内的临时候选进程，保留容器网关和设备环境；浏览器加载候选静态文件并连接该进程，SSO 与系统服务请求走真实 ZoneGateway。未替换已安装应用；大文件分块上传、断线重试及长期运行未在本轮验证。

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
