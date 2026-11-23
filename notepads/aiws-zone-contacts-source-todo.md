# AIWorkspace 本 Zone 联系人数据源 TODO

2026-11-20 已实现。需求编写基线为 BuckyOS `51792a174`，实施基线为 `d3ab763d6`。验收结果见文末。

## 产品行为

在 AIWS「数据 → 新建 → 本 Zone 数据源 → 我的联系人」添加工作区所有者的通讯录。产物是一张标准的 `buckyos.table-source`，可以放在画布上、绑定表格控件、参与 SQL 查询和作为许愿格（Wish）的输入。本期目录只有联系人一项。

添加时显示通讯录所属账号、将要复制的字段和目标工作区。共享工作区要说明「有权读取这张表的成员可以看到同步进来的联系人」。来源属性保持可见，用户可以从那里切换同步方式。表内的对象链接打开 Message Center 的原记录，由原服务检查访问权限。

| 同步方式 | 何时取数 | 表的行为 |
| --- | --- | --- |
| **自动同步（默认）** | 添加后立即读取一次，此后由 AIWS 后台定期刷新，也可点「立即同步」。首版建议每 15 分钟，沿用现有 Source 允许的最短周期。 | 行、值与字段结构只读。没有打开工作区页面时也会同步。 |
| **手动同步** | 添加后立即读取一次，此后只在有权限的人点「立即同步」时读取。由自动切到手动时直接沿用最近成功版本。 | 仍然只读。属性面板显示最后成功同步时间。 |
| **不同步（保留可编辑副本）** | 添加时选此项，只取一次。已有有效版本时切到此项，直接保留屏幕上的那一版。 | 解除来源绑定，沿用原表、原有记录 ID、视图与控件。具备相应表权限的人可以编辑。编辑只写本工作区。 |

「手动」与「自动」可原位互切。已本地化的表也可在属性面板重新选择「本 Zone 联系人」并绑定为手动或自动。重新绑定会用通讯录替换本地行和字段，提交前应说明影响并允许取消。读取或校验失败要保住本地修改。上游始终单向供数，改联系人原记录仍走 Message Center。

## 接到哪一层

复用 **标准 Source 的本地副本**。联系人适配器只负责取得候选表，后续沿用首次 `pending`、`source.refresh@1`、`origin: source` 的原子提交、`data_version`、失败时保留旧版和工作区 ACL。现有 `kind: zone` 的协议是读取 owner home 下指定文件。`url_query` 按需查询远端行，当前无法作为本地 SQL 和 Wish 输入。[Source 设计](../doc/workspace/kernel/04-数据源发布与同步.md) · [已有来源校验](../src/frame/aiworkspace/core/src/types.rs) · [刷新落库](../src/frame/aiworkspace/store/src/source.rs)

**拟新增一个受限的来源描述**。在现有 `payload.source` 里支持 `kind: "builtin"`，本期唯一合法的 `provider` 是 `"contacts"`，仅用于本地表。校验器要求主键为 `["did"]`，按完整通讯录读取。添加流程默认写入 `refresh`，显式选手动才省略。以下是待实现的自动模式示例。

```json
{
  "kind": "builtin",
  "provider": "contacts",
  "refresh": { "interval_s": 900 },
  "def": {
    "meaning": "工作区所有者在本 Zone 可见的联系人",
    "primary_key": ["did"]
  }
}
```

- **同步策略只存一处。** 自动模式带 `source.refresh.interval_s`，手动模式省略 `refresh`。两者的 `data.mode` 都为 `synced`，`data.write` 都为 `refresh`。不同步时由 owner 通过标准 `entity.unset_keys` 移除 `source`，带上该键的当前修订。内核随之清除 `data_version`，返回 `local/edit`。标题、说明等展示属性继续遵循普通 Source 的规则。[读模型](../src/frame/aiworkspace/core/src/meta.rs) · [来源所有权与只读校验](../src/frame/aiworkspace/core/src/types.rs) · [条件更新](../src/frame/aiworkspace/core/src/plan.rs)
- **首读沿用同一条路径。** 自动和手动都在建表后立即触发标准刷新。首次选择「不同步」时，由服务完成一次取数、校验和本地化。只有完整结果才成为可编辑表。若先创建占位表，服务端记录这次请求的一次性意图，首读成功即解除绑定，关掉浏览器也能完成。首读失败显示待重试或取消，不能显示成成功导入了 0 人。已有本地编辑的表重新接入时先暂存候选，在当前表版本仍匹配时完成替换和绑定。[已有建表后立即刷新流程](../src/frame/desktop/src/app/aiworkspace/ui/sources/sourceLinks.ts)
- **增加固定的本 Zone 读取分支。** AIWS 服务端在普通 Source fetch 阶段，通过 BuckyOS 客户端读取 Message Center，返回现有的 `Fetched::Table`。这一分支只调用编译期确定的只读接口。描述中只接受本方案列出的来源字段，拒绝自填 owner、路径、RPC 方法、URL、脚本或票据。它与现有公开 URL 读取、Zone 文件读取并列，使用同一刷新器、限额和调度器。安装联系人源无需准备 Agent、同步脚本或中间文件。实施时将这个固定服务来源补入内核文档 04。[刷新执行器和计时器](../src/frame/aiworkspace/server/src/source.rs)

## 哪一本通讯录，哪些字段

数据取自 `contact.list_contacts(query, Some(owner_did))`。显式 owner 得到用户自己的条目与该用户可见的 Zone 联系人投影。省略 owner 会选中系统通讯录。这里的「本 Zone」指提供通讯录的 Message Center，联系人本人的 DID 可以属于其他 Zone。首版同步这个 owner 视图中的所有可见记录。[读取鉴权](../src/frame/msg_center/src/owner_session.rs) · [通讯录合并与筛选](../src/frame/msg_center/src/contact_mgr.rs)

| 字段 ID | AIWS 类型 | 内容 |
| --- | --- | --- |
| `did` | `text` | 联系人 DID，业务主键。通过标准 `record_id_for_key([did])` 生成 AIWS 行 ID。 |
| `name` | `text` | 联系人显示名。重名保留多行，改名更新原行。 |
| `tags` | `json` | 可展示标签字符串数组，沿用 HomeStation 对用户标签及可见投影标签的筛选，去重并稳定排序。 |
| `friend_state` | `text` | Message Center 的 `FriendState` 序列化值，展示层可本地化。 |
| `contact_ref` | `text` | 用 `obj_service_url` 生成的原记录地址，含通讯录 owner 及联系人 DID。 |

这些字段在合法空通讯录中也存在。候选行按 DID 确定身份。昵称、顺序和标签均可变化。只将这五列送入 AIWS 提交、历史和导出，私人备注、第三方账号绑定和通信端点不进入工作区。[联系人对象行与对象链接](../src/frame/msg_center/src/obj_provider.rs) · [HomeStation 展示字段与分页范例](../src/frame/homestation/src/contacts.rs) · [标准主键算法](../src/frame/aiworkspace/core/src/wish.rs)

## 无用户会话时的读取权限

工作区 owner 的「添加」或「重新绑定」动作同时授权将这本通讯录复制到该表。通过 AIWS 现有 owner 校验（含获准代 owner 操作的 Agent），从已验证的本 Zone 工作区所有者解析账号及 DID。协作者沿用表的 `may_run` 权限触发刷新，数据身份始终取绑定的 owner。只读成员可看上一次成功的表。

默认 RBAC 为 AIWS 授予的是 `contact.check_membership` 所用的分组判定权限。方案为 `system:aiworkspace` 增加仅匹配 `obj://msg-center/owners/{owner}/contacts` 的 `read` 权限。每次取数从 runtime 获取当前服务客户端，显式传入绑定的 DID。**这条规则在 Message Center 层能读不同 owner 的通讯录，因此逐工作区授权必须由 AIWS 强制执行。** 普通 Source 文档和导入包本身不能授权一次后台读取。[默认 RBAC 和已有隔离测试](../src/kernel/buckyos-api/src/rbac_config.rs) · [联系人资源路径](../src/kernel/buckyos-api/src/msg_center_client.rs) · [AIWS 身份解析](../src/frame/aiworkspace/server/src/auth.rs)

在本部署的工作区控制状态保存绑定的 `workspace_id / epoch / entity_id / owner_did` 和绑定代次。绑定只随已鉴权的 owner 添加或重新绑定生效。读取前和提交候选前都要核对绑定，并确认账号仍有效、DID 仍属于当前 owner。解绑、删除、账号停用或 owner 变化使旧绑定失效。工作区重开、服务重启继续使用有效绑定。导出、Clone、模板、历史重放只带按原有规则可复制的内容。新的工作区若已有快照，可按其 ACL 阅读，属性面板显示「需要重新绑定」，后台不会自动读取原 owner 或新 owner 的通讯录。导入和恢复路径同样要按工作区身份及 epoch 校验。[工作区本地控制状态](../src/frame/aiworkspace/store/src/schema.rs) · [Clone 和恢复入口](../src/frame/aiworkspace/store/src/service.rs)

## 刷新的正确性

- **完整分页再提交。** 沿用分页客户端，例如每页 200 条，按实际返回数推进 `offset`，到末页再生成整表。现有联系人对象摘要只返回 `rows`，默认查询只返回前 100 条。每次候选遵循 Source 的大小、超时与配额限制。任一页失败、出现重复 DID 或超限，整次刷新失败。成功读完且确实无人时才提交空表。[上游列表语义](../src/frame/msg_center/src/contact_mgr.rs) · [现成分页调用](../src/frame/homestation/src/contacts.rs)
- **考虑读取期间的变化。** 上游按更新时间排序，offset 分页没有快照令牌。对上次有效版本存在、本轮却缺失的 DID，用同一 owner 的 `contact.get_contact` 复核后再删行。仍可见则采用其当前投影。遇到翻页重叠或复核失败时丢弃候选，按刷新策略重试。按轮次收敛。本次检查时间记作同步时间，不伪造全通讯录的业务版本。删除后重新出现的 DID 应恢复同一行 ID 并更新内容，使用已有的记录恢复操作，避免与已删除行的 ID 冲突。[列表与单条读取](../src/frame/msg_center/src/msg_center.rs) · [Source 行差异](../src/frame/aiworkspace/store/src/source.rs) · [表记录恢复](../src/frame/aiworkspace/core/src/plan_table.rs)
- **阻止旧结果覆盖新状态。** Run 启动时捕获 Source 键修订与绑定代次，提交前在工作区写入边界重新核对。合并或串行化同一绑定的并发刷新。自动切手动、解绑、重新绑定或删除后，旧 run 都不能写回。配置与本地绑定的持久化必须可安全重试，半完成状态先视为无有效绑定。复用现有 run 幂等键、取消、失败保留上一版及内容无变化时跳过提交的行为。[刷新 run 与应用逻辑](../src/frame/aiworkspace/store/src/source.rs)

## 实施清单

- [x] **内核与协议**　扩展 `check_source` 和标准表的来源校验，把 `provider` 暴露给 `doc.read` 的来源摘要、Desktop/SDK 的 Source 类型及协议说明。已有的自动、手动、解绑统一从 `source` 推导。更新 Rust/WASM 和消费端的契约用例。[内核](../src/frame/aiworkspace/core/src/types.rs) · [Desktop 类型](../src/frame/desktop/src/app/aiworkspace/api/types.ts)
- [x] **授权与适配器**　接通精确 RBAC 资源、owner 解析、本地绑定生命周期、五列投影和完整分页。为后台取数提供可注入的联系人客户端，使失败、慢响应和两用户数据可以独立测试。
- [x] **刷新与本地化**　接入标准刷新、定时器与版本记录，处理一次性取数、重连时保留本地修改、过期 run、恢复同一 DID、重启与重试。同步期间让普通编辑继续由后端返回 `SOURCE_READ_ONLY`。
- [x] **Desktop**　在数据模式的现有新建菜单增加「我的联系人」。属性面板显示同步方式、来源账号、最后成功时间、记录数、最近失败和可用操作。添加后打开普通表格。菜单与按钮使用后端同样的 owner、目标目录和 `may_run` 权限。[新建菜单](../src/frame/desktop/src/app/aiworkspace/ui/datamode/actions.ts) · [来源属性面板](../src/frame/desktop/src/app/aiworkspace/ui/sources/SourcePanel.tsx)
- [x] **文档与回归**　更新内核数据源文档和已有 AIWS Agent 使用手册的联系人示例。通过下列场景后回填测试命令、结果和实机 Zone 验证记录。[内核文档](../doc/workspace/kernel/04-数据源发布与同步.md) · [同步手册](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_sync/SKILL.md)

## 验收用例

| 场景 | 可观察的结果 |
| --- | --- |
| 首次插入、页面关闭、服务重启 | 默认选中自动。添加后立即发起首读，成功后出现首个版本。关闭页面后在 Message Center 改名，再次打开可见后台刷新的新值。 |
| 自动 ⇄ 手动 | 切换保持同一实体和行 ID。手动模式跨过原周期仍保留旧值，点「立即同步」后更新。两种模式下 UI 和直接 Commit 改行均被拒绝。SQL、控件和 Wish 读到同一成功版本。 |
| 不同步与重新绑定 | 从自动或手动断开后可按 ACL 编辑，原控件继续引用同一表。后续上游更新和旧 run 都不覆盖本地修改。首次选「不同步」只导入一次。重连取消或失败保住本地值，成功后只读。Message Center 原记录全程不受本地编辑影响。 |
| 大通讯录与合法空集 | 超过 1000 人仍完整导入。同名不同 DID、改名、删除再加入均能正确对账。所有联系人移除后，刷新成功且为 0 行，仍有五列表头。 |
| 失败与竞争 | 首读失败显示 pending 和原因。已有版本后发生拒绝、超时、中途页失败时仍可读旧版。分页中改名、连续点击刷新、在途解绑后编辑不会导致误删或回滚。 |
| 两个账号与共享工作区 | Alice 添加自己的通讯录，Bob 按表 ACL 阅读。有运行权的 Bob 刷新时仍取 Alice 的视图。用客户端指定另一个 owner、伪造 source、复制文档或恢复旧绑定均不能取得额外上游权限。 |

回归入口为 [core 的 Source 权限测试](../src/frame/aiworkspace/core/tests/sources.rs)、[store 的 Source 测试](../src/frame/aiworkspace/store/tests/sources.rs)、[server 的来源集成测试](../src/frame/aiworkspace/server/tests/zone_source.rs) 和 [Desktop 的来源用例](../src/frame/desktop/tests/aiworkspace/sources.spec.ts)。最终用真实 Zone 的两个账号验证服务身份、owner 投影和无人打开页面时的自动刷新。

## 实施与验收记录（2026-11-20）

- BuckyOS 内的添加与重绑入口为 `source.bind_contacts`。Desktop、服务端与同级 `buckyos-websdk` 均已更新。WASM 和 SDK 类型声明已从源码重新生成。
- `cargo test --offline -p aiworkspace-core --test sources --test vectors` 验证来源描述、读模型与标签数组的跨语言值契约。`cargo test --offline -p aiworkspace-store --test sources --test packages` 验证本地授权、配置变更的崩溃重试、SQL 和 Wish 读取 1,201 条记录、一次性本地化、恢复原行 ID，以及导入、Clone 和恢复后的授权隔离。最终 store 来源回归共 6 项，也验证了完整及部分撤销遵守同步表的只读约束。
- `cargo test --offline -p aiworkspace --lib contacts::tests` 验证分页、缺失 DID 复核、两用户视图、账号变更与错误处理。`cargo test --offline -p aiworkspace --test contract --test zone_source --test source --test sync_source` 验证 HTTP 请求、owner 与其 Agent 的授权、Bob 在 Clone 后重新绑定自己的视图、来源只读、过期刷新、原子重绑和已有来源兼容性。
- `cargo test --offline -p buckyos-api --lib homestation_reads_owner_contacts_only` 验证 AIWS 的权限仅扩至 owner 联系人集合的精确读取路径。
- 同级 WebSDK 运行 `node_modules/.bin/tsc -p tsconfig.build.json` 与 `node_modules/.bin/jest --runInBand tests/aiws_write.test.ts`。13 项测试通过，含 `readMeta` 的内建联系人来源类型。
- Desktop 运行 `pnpm run build`、`node_modules/.bin/tsc -b` 及改动文件 ESLint。用新构建的 AIWS 和 Chromium 运行 `node_modules/.bin/playwright test --config playwright.aiworkspace.config.ts tests/aiworkspace/sources.spec.ts tests/aiworkspace/datasource.spec.ts`，8 项通过。浏览器检查了添加前的共享提示、来源只读、取消重绑、本地编辑标签数组和对象链接。i18n 表结构与语言区域检查 55 项通过，全量 i18n 中 3 项既有失败与实施基线相同。
- `bash src/frame/aiworkspace/wasm/build.sh` 生成 Desktop 的 WASM。实际载入 WASM 后运行共用向量，15 组值规范化检查通过。
- 实机原记录链接已在 Chromium 验证。两个用户分别通过本 Zone SSO 登录，打开同一条 `contact_ref`。该通讯录的 owner 收到 Message Center 的 HTTP 200，协作者 Bob 收到 HTTP 403。
- 扩展回归时，store 既有用例 `session::a_registry_from_before_gets_the_speaking_columns` 失败。它用版本 0 的非空会话数据库测试升级，当前版本 1 的会话注册库拒绝打开该数据库。用 `cargo test --offline -p aiworkspace-core -p aiworkspace-store -- --skip a_registry_from_before_gets_the_speaking_columns` 复跑，其余 351 项通过、7 项按配置忽略。
- server 全包在用户运行目录的独立临时目录下串行执行 `cargo test --offline -p aiworkspace --no-fail-fast -- --test-threads=1`，91 项通过、2 项按配置忽略。

### 真实 Zone 验证

使用独立启动的 dev Zone，运行完整的 node_daemon、Verify Hub、Message Center 和 AIWorkspace。命令为 `deno run --allow-net --allow-env src/frame/aiworkspace/server/tests/contacts_zone_dv.mjs`，退出码为 0。

owner 添加通讯录后，Bob 在共享工作区通过 SQL 读到 owner 的五列投影；Bob 点击刷新后仍读取 owner 的通讯录。2026-11-21 08:09:59 UTC 在上游修改临时联系人的名字。期间工作区页面关闭，并重启 AIWorkspace。08:27:24 UTC 已能从表中读到新名字，记录 ID 与改名前一致。共用时约 17 分 25 秒，符合 900 秒间隔与调度器的巡检周期。完成后已清理两条临时联系人、共享工作区和测试 Zone。

现役宿主机已备份并替换这两个服务的可执行文件。截至上述验收结束，旧进程仍在运行。宿主机账号的免密 sudo 仅允许整站停启，单独重启两个服务仍需操作员从有 sudo 权限的终端发送 TERM。
