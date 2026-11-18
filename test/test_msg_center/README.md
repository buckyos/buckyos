# Message Center DV

`test_native_routes_dv.mjs` 验证 `src/dev_configs/ssotest.json` 的三 Zone 环境：Bob 查询自托管 Alice 与绑定 Charlie 的 Zone，再发送一条原生消息到 Alice，等待其进入 Alice 的会话。会写入一条带 `multi-zone F-4 native-route acceptance` 标识的验收消息，保留在两端历史中；不修改联系人和用户设置。

在仓库根目录运行，要求三个 Zone 已激活、msg-center 可用，宿主机安装 Node.js：

```bash
BUCKYOS_TEST_DNS=<sn ip> node test/test_msg_center/test_native_routes_dv.mjs
```

环境参数与混合 SSO DV 一致：`BUCKYOS_TEST_DNS` 为必填 SN DNS 地址；`BUCKYOS_TEST_CA` 默认为 `~/buckycli/ca/buckyos_test_ca_ca_cert.pem`；`BUCKYOS_TEST_PASSWORD` 默认为开发口令 `bucky2025`。`BUCKYOS_TEST_ZONE_A/B/C` 可覆盖 Alice、Bob、Charlie 的主机名，默认分别为 `alice.web3.devtests.org`、`bob.web3.devtests.org`、`charlie.me`。`BUCKYOS_TEST_HOST_MAP="host=ip,..."` 可覆盖个别主机解析。账号及 DID 使用种子中的 alice、bob、charlie。

跨 Zone 附件（Bucket 设计 §21 N18，#689）的 DV 在 `test/nfs_server_test/`，同样对 `ssotest` 运行，运行方式见各文件开头：`remote_attachment_dv.mjs` 是私聊，`group_attachment_dv.mjs` 是作者、host、成员分属三个 Zone 的群，`attachment_ui_dv.mjs` 是 Desktop 经开发代理连真实 Zone。它们会在三个 Zone 的会话里留下带 `N18 remote attachment`、`#689` 标识的消息和附件，在 bob 的 Zone 留下账号 `mal689`，并改写三个用户之间的联系人访问级别。

现有 `test_messagehub_groups.ts`、`test_messagehub_sessions.ts`、`test_my_network.ts` 的单 Zone 用例及运行参数见各文件开头。

### 跨 Zone 群推送实机时延（#737）

两台已部署当前 msg-center、时钟同步的 ssotest OOD 默认由 Bob 托管、Alice 加入。设置 `BUCKYOS_TEST_GROUP_HOST=alice` 可交换两端角色，脚本会让 Bob 加入。运行脚本会创建一个带时间标记的群、加入成员并留下 20 条验收消息。也可用 `BUCKYOS_TEST_JOINED_GROUP` 指定所选宿主托管、另一端已同步的测试群。依赖使用 `test/nfs_server_test` 的现有安装。

```bash
BUCKYOS_TEST_DNS=<SN-IP> node test/test_msg_center/group_push_dv.mjs
```

脚本通过普通发送入口逐条发送，在成员 OOD 的本地 INBOX 看到上一条后再发下一条。P50/P95 使用记录上的宿主 `sort_key`（accepted_at_ms）和成员 `created_at_ms`，不包含客户端轮询时间。如将两个节点当前的 debug 日志映射到运行脚本的机器，可分别以 `BUCKYOS_TEST_HOST_LOG` 和 `BUCKYOS_TEST_MEMBER_LOG` 指定。日志路径分别对应本次选择的宿主、成员。脚本在入群后等待 1.5 秒，从首条消息的宿主接受时间起，统计到末条成员投影后 1 秒，按日志中的毫秒时间过滤异步送达的日志行。默认以 UTC 解析两端服务日志；其他时区可指定 `BUCKYOS_TEST_LOG_OFFSET`（例如 `+08:00`）。输出包含两个节点、采样时间界限、两种传输各自的成功请求数、快路径命中数、changes/sessions/objects 回查次数，包含零计数。未提供日志时相应指标输出 null。设置 `BUCKYOS_TEST_SAMPLES` 可以改变样本数。浏览器展示耗时应另外从浏览器埋点测量。

### 群消息在浏览器中的展示耗时（#737）

先保持聊天窗口关闭完成上面的后端采样，再对已同步的同一测试群运行浏览器 DV。运行机需要 Desktop 的现有 `@playwright/test` 与 Chromium 安装，以及与两端 OOD 同步的系统时钟。脚本打开成员 Zone 已部署的 MessageHub，宿主通过普通发送入口逐条发送 24 条独立标记的文本，并为每条保留原始记录。

```bash
BUCKYOS_TEST_DNS=<SN-IP> BUCKYOS_TEST_JOINED_GROUP=<已同步的群DID> \
  node test/test_msg_center/group_push_ui_dv.mjs
```

沿用 `BUCKYOS_TEST_GROUP_HOST`、`BUCKYOS_TEST_ZONE_A/B`、`BUCKYOS_TEST_HOST_MAP`、测试账号和 CA 配置。可通过 `BUCKYOS_TEST_UI_SAMPLES` 调整样本数，`BUCKYOS_TEST_UI_BASE` 指向指定的成员 Desktop（默认是成员 Zone 的 HTTPS 根地址）。`BUCKYOS_TEST_UI_OUT` 指定输出父目录，默认使用系统临时目录，每次生成独立的样本 JSON 和最终画面。测试 Chromium 允许测试域的证书。

事件时间取自页面 WebSDK 解析到本会话指定 msg_id 的 INBOX upsert。展示时间取正文进入聊天区域且可命中该文本的首个浏览器动画帧。相减得到同一浏览器单调时钟下的事件到可见时延；成员 INBOX 的 `created_at_ms`、宿主 `sort_key` 对应入库和接受时间，和该帧的绝对时间相减得到另外两列。每列分别给出有效样本数、nearest-rank P50/P95 和最大值；未捕获到对应事件的样本该列记 null。
