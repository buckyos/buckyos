# AI Workspace 真实 Zone E2E

入口是 Desktop 的 `playwright.aiworkspace.zone.config.ts`，用例在
`tests/dv/aiworkspace.spec.ts`、`aiworkspace.mobile.spec.ts`、
`aiworkspace.journeys.spec.ts`、`aiworkspace.logic.spec.ts` 和
`aiworkspace.integrity.spec.ts`，
身份与清理逻辑在 `aiworkspace.fixtures.ts`。
测试连接已经运行的 Zone，不构建或启动独立后台。

## 前置条件

- Desktop、verify-hub、control-panel、aiworkspace 已部署并可用。
- 基础 URL 使用 Zone 根 origin；登录不能用 `sys.<zone>` 替代。
- 已有 Playwright Chromium 与 Desktop 依赖，不在测试时安装或更新依赖。
- 提供可创建、删除本地用户的管理员账号。双用户用例在同一 Zone 创建随机临时用户。
- VM 必须处于 Running；执行节点命令前检查当前 VM 状态。
- 从能访问 VM 网络的宿主机执行；不要把浏览器无法路由到 VM 误判为服务失败。

## 配置与命令

以下命令在 Desktop 目录执行。凭据文件只包含密码正文，不加入 Git。
环境变量 `BUCKYOS_TEST_ADMIN_PASSWORD` 也受支持；设置密码文件时优先读取文件。

```bash
export BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io
export BUCKYOS_TEST_ADMIN_USER=devtest
export BUCKYOS_TEST_ADMIN_PASSWORD_FILE=/path/to/private/admin-password

export BUCKYOS_E2E_OUTPUT_DIR=/path/to/project/.work/aiworkspace-zone/run-unique
export TMPDIR=/path/to/project/.work/aiworkspace-zone/tmp
mkdir -p "$BUCKYOS_E2E_OUTPUT_DIR" "$TMPDIR"

pnpm exec playwright test --config=playwright.aiworkspace.zone.config.ts --list
pnpm exec playwright test --config=playwright.aiworkspace.zone.config.ts
```

当本机 DNS 不能解析测试域名时，可设置 `BUCKYOS_UI_DV_GATEWAY_IP` 为当前网关 IP。
配置通过 Chromium 的 host resolver 将根域名和子域名映射到该地址，仍保留 URL 和 Host；
不修改系统 hosts，也不把测试改成直打 aiworkspace 服务端口。
浏览器内的辅助 RPC 同样通过这个 Gateway。

若测试证书使 Chromium Service Worker 报 SSL 错误，可设置
`BUCKYOS_UI_DV_TLS_SPKI` 为当前服务端证书公钥的 SHA-256 base64 指纹，
仅为该公钥配置 Chromium 证书例外。先用当前 CA 和域名严格验证取得的证书，
再从证书计算指纹；不要直接信任未验证连接返回的证书。

```bash
openssl x509 -in verified-server.pem -pubkey -noout |
  openssl pkey -pubin -outform der |
  openssl dgst -sha256 -binary | base64
```

该配置继承 DV 的 `ignoreHTTPSErrors: true`，所以浏览器通过不代表证书验证通过。
另外使用当前 CA 做严格预检：

```bash
ZONE_HOST=test.buckyos.io
curl --fail --silent --show-error --max-time 10 \
  --cacert /path/to/current/ca.cert \
  --resolve "$ZONE_HOST:443:$BUCKYOS_UI_DV_GATEWAY_IP" \
  "https://$ZONE_HOST/kapi/aiworkspace/healthz"
```

长远程任务从项目 `.work` 下的脚本启动，所有环境变量在脚本中设置，再 detached 执行并轮询：

```bash
setsid nohup bash /path/to/project/.work/aiworkspace-zone/run.sh \
  > /path/to/project/.work/aiworkspace-zone/run.log 2>&1 </dev/null &
```

使用独立输出目录运行两轮，以验证可重复执行。不要自动 retry 掩盖第一次失败。
可通过 `-g 'Chinese rich text'`、`-g 'reader grant'`、`-g 'two real users'`
或 `-g 'replays a commit'` 单独选择场景。

## 当前覆盖

| 场景 | 独立验证 |
| --- | --- |
| 登录、Desktop 打开、模板创建、表格和中文 IME 编辑、刷新 | 真实 session 的主体、Zone transport、服务端字段和富文本 AST、图片下载渲染 |
| 私有工作区、UI 授予 reader、拒绝写入、撤回 | 第二用户不可枚举私有工作区；拒绝写入后值与版本未变；撤权后 API 与 UI 均不可读 |
| 双用户协作 | 富文本收敛；不同字段都保留；同字段冲突保留双方输入；确认覆盖后两端刷新结果一致 |
| 手机视口 | Chromium Pixel 7、375px 宽度；文档和工具栏可见、无横向溢出，刷新不增加文档提交 |
| 会话与幂等 | 无会话、无效会话拒绝；相同提交键返回同一 commit，head 只增加一次 |

UI 使用真实 SDK 的 Zone transport，不设置 `aiworkspace.dev`，不拦截或替换服务响应。
辅助 RPC 经浏览器同源 Gateway 发出，用于独立读取服务端结果和构造明确的异常请求。
现有 `playwright.aiworkspace.config.ts` 继续负责独立后台的更大回归集合。
本入口尚未覆盖离线冷启动、展示、真实 AICC/Provider 或模型质量。

## 两条入门业务场景

仅运行本轮的两条业务用例：

```bash
pnpm exec playwright test --config=playwright.aiworkspace.zone.config.ts \
  aiworkspace.journeys.spec.ts --project=chromium-real-zone
```

| 选择器 | 用户路径 | 独立验收 |
| --- | --- | --- |
| `-g journey-sales` | 创建季度经营分析模板，分析输入、预览候选、应用；修改一条销售额，检查过期提示，再执行并应用，刷新页面 | 12 行销售总额先为 770000；s-1 从 128000 改为 138000 后变为 780000；候选预览不改变文档 head，应用后才更新；四个结果 ID 稳定，服务端 KPI 与摘要一致，刷新后 freshness 为 current |
| `-g journey-handoff` | 作者编辑短报告和表格，将图片连同内容导出为自包含分享包；无源工作区权限的第二用户明确选择导入为新工作区，编辑、刷新并再次导出 | 初次导入的 content_root、富文本 AST、表格字段及图片 SHA-256 与原件一致；接收者修改持久化，作者原件 AST 与 content_root 不变；再次导出成功，副本与临时用户清理 |

经营分析使用产品内明确标识的“模拟执行器”，断言结果带有
`executor=mock`、`simulated=true`，不替换网络响应或 Zone 后台。
它验证确定性的计算、候选应用和来源更新链路，不调用真实模型，
不代表 AICC、Provider 或模型质量通过。
短报告交接使用用户编写的简短内容，覆盖文档、表格和资源的交付，
不代表完成了论文或带引用的研究报告生成。

### 已知失败：返回许愿格后仍显示旧的新鲜度

真实 Zone 上的 `journey-sales` 发现：首次应用结果后切到销售表修改金额，
再返回许愿格，后台 `doc.freshness` 已返回 `stale`，UI 仍显示 `current`。
源码中的 `FreshnessService.invalidate()` 仅重新读取当时正在被观察的实体；
许愿格离开详情后不再被观察，其缓存仍保留。
重新挂载时 `watch()` 只对没有缓存的实体发起读取，因而显示旧状态。

用例对该 UI 状态使用软断言以继续核对刷新计算，但整个测试仍判为失败，
不跳过、不把预期改成 `current`。后续执行、应用得到 780000，
刷新页面后结果仍存在；这不抵消过期提示失败。
本轮仅新增测试与文档，未修复或部署产品代码。

## 两条逻辑用例

`aiworkspace.logic.spec.ts` 复用同一套 Playwright、Zone 配置、真实登录和清理夹具。
用 UI 准备样例后，核心操作和验收经浏览器同源 Gateway 的真实 kRPC 完成，
读取服务端业务状态和变化流，最后少量核对刷新后的渲染。
这组是服务逻辑 E2E，不评判布局、鼠标键盘体验或移动端交互。

```bash
pnpm exec playwright test --config=playwright.aiworkspace.zone.config.ts \
  aiworkspace.logic.spec.ts --project=chromium-real-zone
```

| 选择器 | 逻辑路径 | 独立验收 |
| --- | --- | --- |
| `-g logic-views` | 完成任务并提前另一任务的截止日期，再重新打开该任务 | 原始五条记录身份不变；已保存的未完成视图按状态排除、按日期排序且投影不变；额外负责人条件与已保存条件取交集；正文内嵌与独立视图同序，刷新后仍一致；查询条件没有写回视图定义 |
| `-g logic-atomic` | 一次提交包含树排序、富文本插入、表格改值，最后一项使用不存在的状态选项；随后改为合法选项重新提交 | 非法项准确定位到第 3 个操作；拒绝后 ContentRoot、树、富文本 AST、CRDT 快照、表格值和版本、head 和变化流均不变；修正后所有修改一起持久化，只新增一条提交和一个变化事件 |

对应设计中的同源多视图 / 稳定寻址（V08）和混合原子批次（V10）。
这里没有注入存储故障、杀进程或重启服务，因此不把验证范围扩展为崩溃恢复。
两条均不调用模型。

## Fork 与分页一致性

本轮仅运行 `aiworkspace.integrity.spec.ts` 的两条逻辑用例：

```bash
pnpm exec playwright test --config=playwright.aiworkspace.zone.config.ts \
  aiworkspace.integrity.spec.ts --project=chromium-real-zone
```

| 选择器 | 逻辑路径 | 独立验收 |
| --- | --- | --- |
| `-g logic-fork` | 给第二个真实用户授予原件读取权限，Fork，随后分别修改副本和原件 | 初始 ContentRoot 和正文一致；协作 lineage 不同，原件授权不复制；本地引用在副本中解析为副本工作区，预算批注仍有正确锚点；副本改表格和正文不改变原件，原件随后改表格不影响副本 |
| `-g logic-pagination` | 按数值预算每页读取两行，第一页后把另一行预算调低，使其排序位置跨过游标；用旧游标继续，再从第一页重读 | 旧游标返回可重试的 `SNAPSHOT_EXPIRED`；重读各页同一 source revision，页大小为 2、2、1，五条记录无重复或遗漏，按数值而非字符串排序；失败请求与查询不产生额外提交或改变内容根 |

Fork 对应 V05，分页对应内置对象设计 §2.5.4 的一致性裁决。
本轮验证内置 TableSource 的分页；URL 查询源使用自己的快照协议，需要单独验收。

## 证据与清理

每个用例保存关键截图、浏览器错误清单和清理记录；失败保留 trace。
输出目录中的 `report.json` 记录实际 passed、failed、skipped 数量。
查看截图与错误记录后才能给出通过结论，`--list` 不能作为执行证据。

工作区标题使用随机 `dv-aiws-` 前缀。清理前核对精确 ID 与本次标题，
通过 `ws.delete` 删除，再查询活动列表确认消失。
临时用户通过 `user.delete` 软删除；工作区按服务约定进入回收目录。
这些操作不物理清除数据库对象、回收目录或审计记录。
保留已有用户、工作区、身份、应用和服务进程。

记录源码 HEAD、未提交差异、被测 URL、当前 VM 状态、运行二进制 hash 与执行时间。
源码工作树和 VM 已部署版本分别记载；没有构建 provenance 时不声称它们一致。
trace 与截图可能包含登录会话及界面信息，保留在受限的 `.work` 下，
对外分享前只导出经过检查的证据。
