# AI Workspace 短编号（子域名缩短）TODO

日期：2026-11-01　状态：§2 已裁决、§3 已实施（未提交）、三 crate 测试过；§4 真实 Zone DV 待做；`?k=` 保密链接另立项（§5）

## 0. 诉求

现在 `https://aiws-pxmyr26jfiyascwlkxwfy5nfn2.demo.buckyos.io/buckyos-demo/fogworks-weekly-update` 的子域标签是 workspace id 去掉 `ws_` 后的 26 位 base32（130 bit 随机数），太长不好记。改为固定 8 位：`YYMM`（创建年月）+ 4 位 base36（`[0-9a-z]`，36⁴ = 1,679,616 个/月），本地已存在则 +1。目标形态：

`https://aiws-2611a0b3.demo.buckyos.io/buckyos-demo/fogworks-weekly-update`

## 1. 现状（依赖点盘点）

- id 生成：`store/src/workspace.rs::random_id("ws_")` → `core/src/id.rs::prefixed_id`（26 位 base32）。调用方：`service.rs::create_workspace`（可带入 id）、`service.rs::import` 的 `"new"` 分支、`show.rs::clone_for_show`（放映临时克隆）。
- 格式校验（都按 `ws_`+26 位硬编码）：`service.rs:238-239`（create）、`service.rs:265`（目录扫描 `workspace_ids`）、`service.rs:346`（restore）、`service.rs:67`（restore_ 备份目录）、`workspace.rs:432`（`Workspace::create`）、`entries.rs:53`（entry id 拆分）、`show.rs:198`；JSON schema `schemas/commit-request.schema.json` `^ws_[a-z2-7]{26}$`。
- 主机 ↔ id：服务 `server/src/source.rs::host_label`（去 `ws_`）/`host_workspace`（加 `ws_`，含 `ws_ws_` 去重 hack）；Desktop `app/aiworkspace/links.ts` `HOST_LABEL = /^aiws-([a-z0-9]+)\./`、`workspaceIdOfHost`、`workspaceOrigin`。网关 `boot_gateway.yaml` 与 control-panel `parse_sso_redirect_url`、`buckyos-api::is_aiworkspace_host_label` 都只要求 `aiws-[a-z0-9]+`，**短编号天然满足，不用动**。
- 安全含义：公开链接设计（aiworkspace-public-link）把 130 bit id 当"密钥"：public grant = "有链接的人可看"。短编号后一个 Zone 每月至多 168 万个编号，可被枚举，public 从"不列出"变成"真公开"；私有工作区被枚举时只得到 denied（`workspace.rs::access`），不泄露标题。

## 2. 裁决（2026-11-01 用户）

- Q1 (a) 改 id 本身；Q2 全 36 字符（“不是车牌”，不去易混）；Q3 随机起点 +1；Q4 公开可枚举接受，保密语义日后用链接后的长参数而不是域名（域名在 HTTPS 里也是明码）；Q5 **不兼容旧 URL/旧格式**，校验只认短格式，旧 26 位目录不再被列出；Q6 不比对 epoch（包清单无 epoch，restore 每次生成新 epoch 无法比对），沿用 replace+Manage 门槛且旧目录进 trash 可找回；Q7 同一分配器。

原待拍板项：

- Q1 改 id 本身 vs 另加别名：**建议 (a) 改 id 本身** `ws_<YYMM><4 base36>`，主机标签 = 去 `ws_`。(b) 保留 130 bit id、另存 code→id 映射表：两套标识会同时出现在 URL/`obj://`/包清单里，`host_workspace` 变成查库，长期负担，不推荐。
- Q2 字母表：按诉求用全部 36 个 `[0-9a-z]`（DNS 小写，无大小写歧义）。可选去掉 `0/o`、`1/l` 等易混字符（34⁴ ≈ 134 万），**建议不去**，保持与诉求一致。
- Q3 分配：在当月前缀下随机取一个 4 位起点，`ws_dir` 已存在则 +1（36⁴ 环绕），服务级 mutex 串行化 create/import/clone；年月取服务时钟 UTC（测试可注入）。满 36⁴ 返回 `ID_CONFLICT`（现实不会发生）。**建议随机起点而不是从 `0000` 顺序**：顺序号会暴露 Zone 的工作区数量并让第一个总是 `26110000`。
- Q4 公开链接语义：接受"public = 可被枚举发现"（符合先能力后安全）。分享面板文案把"有链接的人可看"改为"任何人可看"。需要"不列出"链接时再立项（URL 带 `?k=` 密钥），本期不做。
- Q5 旧格式：按"DV 全新安装、不写兼容"原则新建只出短编号；**建议校验函数同时接受旧 26 位格式**（`is_workspace_id` = 短 | 旧），一处改动、零迁移，demo.buckyos.io 上现有工作区继续能开、旧链接不断。若裁决不保留，demo 需重建。
- Q6 restore 碰撞：`import(semantics="restore")` 沿用包内 id。短编号后另一个 Zone 的备份包可能与本地无关工作区同号，`replace=true` 会覆盖错对象。**建议** restore 只在目标目录不存在或 `epoch` 相同（同一血统）时接受，否则报 `ID_CONFLICT` 让用户改 `"new"`。
- Q7 放映临时克隆（`clone_for_show`）：同一分配器拿编号（它们也在 `workspaces/` 目录里、也可能上主机），不另设前缀。

## 3. 实施项（按 §2 建议值）

### 3.1 core / store
- [x] `core/src/id.rs`：`workspace_id(yymm: &str, n: u32) -> String`（`ws_` + yymm + 4 位 base36）、`is_workspace_id(s)`（短格式，Q5 取 (保留) 则 `|| is_prefixed_id("ws_", s)`）；单测。
- [x] `store/src/service.rs`：`allocate_workspace_id(&self) -> WsResult<String>`（mutex + 随机起点 + 线性探测 + 当月前缀）；`create_workspace`/`import "new"`/`show.rs::clone_for_show` 改用；`create_workspace` 带入 id 改为校验 `is_workspace_id`；`workspace_ids` 目录扫描、`entries.rs:53`、`show.rs:198`、`service.rs:67` 改校验函数；`workspace.rs::create` 同。
- [x] Q6：按裁决不加（见 §2）。
- [x] `schemas/commit-request.schema.json` `workspace_id.pattern` 改为短格式（Q5 保留则用并集）；`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace`。

### 3.2 server
- [x] `source.rs::host_workspace` 删 `ws_ws_` hack（新 id 不含下划线，直接 `ws_{label}`）；`host_label` 不变；`server/tests/pages.rs`/`sso.rs` 用短 id 的 fixture 补一例。

### 3.3 Desktop
- [x] `links.ts` 已兼容 `[a-z0-9]+`，只改注释；ShareDialog/PermissionsPanel 公开文案（Q4）；mock 数据里的 ws id 换成短格式示例（可读性）。
- [x] e2e fixtures 不依赖 26 位长度（检查 `ws_test` 之类硬编码）。

### 3.4 文档
- [x] 《第一期详细设计》§2.3 行、UI改进 §8.3 注记已改；README/cmd_skills 未见 26 位假设§2.3 标识符语法、aiworkspace-public-link 设计（§8.3 "有链接的人"）、`frame/aiworkspace/README.md`、`cmd_skills/aiworkspace_*.md` 里出现的 id 示例。

## 4. 验收
- [x]（单测 `core::id::tests::workspace_ids`、store/server 全套）新建工作区 id 形如 `ws_2611xxxx`，`ws.get_info.public_base` = `https://aiws-2611xxxx.<zone>`；同月连建两个不撞号；人为占位后 +1 探测正确。
- [x] 旧 26 位工作区按裁决不再识别（目录保留但不列出、主机 404）；demo.buckyos.io 需重建工作区。
- [x] restore 同号包：按裁决走 replace+Manage，不另加门。
- [ ] 真实 Zone：子域短链接免登录公开访问、已登录成员无感（与 aiworkspace-subdomain-todo §4 合并 DV）。

## 5. 实施备注与后续

- 落点：`core/src/id.rs`（`workspace_id(created_ms, code)`、`is_workspace_id`、`workspace_host_label`、`WORKSPACE_CODE_SPACE`）；`store/src/service.rs`（`Service.reserved` + `IdLease` + `allocate_workspace_id`，create/import new/fork/create_from_template 改用）、`show.rs::clone_for_show`、`session/mod.rs::clone_for_session`；校验点 service.rs（重启恢复 restore-intent、`workspace()`、目录扫描、restore）、`workspace.rs::create`、`entries.rs::split_entry_id`、`show.rs::live_clones`；`server/src/source.rs`（`host_label` 用 core 的、`host_workspace` 校验格式、Source URL 解析去掉 `trim_start_matches("ws_")`）；schema `^ws_[0-9]{2}(0[1-9]|1[0-2])[0-9a-z]{4}$`；`store/tests/sso.rs` fixture 改短 id。
- 分配器：`reserved` 集合在 tmp 构建期间占号，`IdLease` drop 释放；月份取服务时钟 UTC（测试时钟可注入）；trash 目录名是 `<id>-<ms>`，不阻挡编号复用。
- 后续另立项：保密链接 `?k=<长随机>`（per-workspace link key，public 授权只对带正确 key 的请求生效；standalone 页读 query 存 sessionStorage、transport 随 kRPC/WS 带上；分享面板生成/轮换）。
