# AICC Driver Metadata 云更新上线 TODO

日期：2026-09-25。状态：待决策；代码链路已存在但默认关闭，尚未上线。

目标：让 Model Driver、Provider Rules（含静态价格 `model_pricing`）和 Known Provider 三类 catalog 能通过官方云端发布、自动下发到各 Zone 的 AICC，且下载链路以 **cyfs:// 协议**为主，而不只是传统 HTTPS。

相关文档：[云更新协议](../doc/aicc/driver_metadata_update_protocol.md)、[持久化边界](../doc/aicc/driver_metadata_update_storage.md)、[价格事实源](../doc/aicc/provider_pricing_sources.md)。代码入口：[cloud_update.rs](../src/frame/aicc/src/service/cloud_update.rs)、[runtime/mod.rs](../src/frame/aicc/src/runtime/mod.rs)、[provider/runtime.rs](../src/frame/aicc/src/provider/runtime.rs)。

## 0. 现状（2026-09-25 Review 结论）

| 数据 | 当前更新方式 | 是否真的自动 |
| --- | --- | --- |
| Model Driver / Provider Rules / Known Provider | builtin 随二进制；cloud/local/system-config 逐身份整文件覆盖 | cloud 层代码可用，但默认 `enabled=false`、无 `source_url`、仓库没有发布工具 → 实际只随二进制更新 |
| discovery 动态价格（OpenRouter、FAL） | Provider 定时 discovery（15 分钟，需 `auto_sync_models=true`） | 是，价格变化会触发 ModelRegistry 重建 |
| 逻辑树骨架（任务节点、功能路径→规格引用及权重、媒体家族权重） | 硬编码于 `service/model_defaults.rs` | 否，只能随二进制升级（**这是最需要自动更新的部分**，见 D10 / §5） |
| 逻辑树挂载内容 | 由 catalog + inventory 每次收敛重建，并重放 `routing_commands` | 随上两项变化 |

现有下载链路：`CyfsNdnClient`（cyfs-over-HTTP 传输），index 无 `expected_obj_id`、无签名校验；manifest 和文件由 index/manifest 给出的 ObjId + 可选 sha256 校验。即**信任根是 index，而 index 目前只靠传输层保护**。

## 1. 需要决策的问题（需与运维讨论）

每项给出选项与建议；最终结论填在「决定」一栏。

### D1 默认源地址与发布者身份

- 问题：默认 `source_url` 填什么？发布方用哪个 Zone/DID？
- 选项：
  - A. 官方发布 Zone 的语义路径，如 `cyfs://<官方发布 zone DID 或域名>/aicc/provider-catalog/`（建议）
  - B. 普通 HTTPS 域名 + CDN
  - C. 不设默认，由安装器/激活流程写入 system-config
- 需运维确认：发布 Zone 的名字/DID、是否与现有 repo/app 分发共用同一 Zone、中国区是否需要独立源或镜像。
- 决定：

### D2 协议与信任链（cyfs 优先）

- 建议方案：
  - index 放在发布 Zone 的**可变语义路径**上，响应带 `cyfs-path-obj` JWT，由发布 Zone 的 `ZONE_PUBLISH` 作用域密钥签名；客户端用 `NameClientPathVerifier`（`resolve_did` → ZoneDocument → kid → 验签 + path/host 绑定 + exp）验证。
  - manifest、catalog 文件全部按 **ObjId 内容寻址**获取（已实现），不再依赖可选 sha256。
- 待决：
  - 信任根是「固定 pin 官方发布 DID」还是「按 host 走 BNS 解析」？pin 能防 DNS/BNS 劫持但换密钥需发版。
  - 是否允许 `https://` 源作为回退？建议仅允许在 local/system-config 显式配置的私有部署或开发环境，且同样要求 path-obj 签名；官方默认不回退到无签名 HTTPS。
  - path-obj JWT 的 `exp` 设多长？决定了发布端必须多久重签一次 index（即使内容没变）。
- 决定：

### D3 分发拓扑与流量

- 问题：各 Zone 直连官方发布 Zone，还是经 SN / cyfs-gateway / 镜像节点缓存？内容寻址的 manifest 与文件是否允许从任意节点（含邻近 Zone）获取？
- 需运维估算：默认每节点每 15 分钟拉一次 index（`interval_secs=900`）的总 QPS；是否需要加随机抖动；是否需要 CDN / 国内镜像。
- 决定：

### D4 默认开关与频率

- 问题：云更新默认开还是关？默认间隔多少？用户能否关闭？
- 建议：正式版默认开启，间隔 1–6 小时并加抖动；UI 可关闭；关闭后仍使用 builtin + local/system-config。
- 决定：

### D5 发布流程、签名密钥与权限

- 问题：
  - 谁有权发布？是否需要审批（价格改动尤其敏感）？
  - `ZONE_PUBLISH` 密钥如何保管（离线/HSM/CI secret）？轮换流程？
  - `revision_seq` 由谁分配（全局唯一、严格递增、不可复用）？
  - 发布工具放在哪里（仓库 `harness/` 或 `src/tools/`）？CI 是否强制跑 catalog 校验 + ModelRegistry 试建 + golden 测试后才能发布？
- 决定：

### D6 投放维度（track 选择）

- 问题：`client_version`、`update_channel`、`rollout_group` 的来源。当前 channel/group 来自环境变量 `BUCKYOS_UPDATE_CHANNEL` / `BUCKYOS_ROLLOUT_GROUP`，默认 `stable`/`default`，`supported_features` 写死为空。
- 待决：是否复用系统升级的通道/灰度配置（建议复用，而非 AICC 自己一套）；灰度分组如何分配；`supported_features` 的命名与声明方式。
- 决定：

### D7 cloud 与 builtin 的关系

- 问题：
  - cloud 每次发**完整集合**还是只发**有改动的身份**？发完整集合会让 cloud 永久遮蔽新二进制里的 builtin 改进。
  - builtin 版本号常量 `BUILTIN_CATALOG_REVISION_SEQ = 2`，二进制升级带来的 builtin 变化不推进版本，LKGS 兜底时会把旧 builtin 算出的库存当作最新。
- 建议：cloud 只发需要热修的身份；builtin 版本以内容哈希参与收敛判断（或 LKGS 记录 builtin 指纹）；新二进制发布后，云端撤掉（tombstone）已被 builtin 吸收的身份。
- 决定：

### D8 应急与回滚

- 问题：发布了错误价格/错误映射时怎么止血？
- 协议现状：只能以更高 `revision_seq` 重新发布旧内容；本机可用 local/system-config 覆盖。
- 待决：是否需要全局 kill switch（例如 index 发布空 track 集合或特殊标记让客户端停用 cloud 层回落 builtin）；运维应急 SOP 与响应时限。
- 决定：

### D9 收敛失败时 Provider 是否继续可路由（产品决策）

- 现状：云版本推进后，某 Provider 刷新失败即 `routable=false`，其全部模型从路由中消失，即使旧库存可用。
- 建议：保留旧库存继续可路由、标记 Degraded 并退避重试；只有新 metadata 明确下架的模型才移除。
- 决定：

### D10 逻辑树纳入云更新（优先级最高）

- 已定方向（2026-09-25）：**要做**，且比 Model Driver 云更新意义更大——逻辑树承载了「特定用途下的默认模型优先级」（如 `llm.chat` / `llm.plan` 下各规格的权重、媒体家族权重），这是最需要随市场变化频繁调整的部分。实现思路见 §5。
- 仍需决策：
  - D10.1 新增 catalog kind `logical_tree`（建议），还是塞进现有某类 catalog？新增 kind 会改变 manifest 的 `catalog_kind` 枚举，旧客户端需以 `required_features` 隔离。
  - D10.2 云端能改到什么程度：只调权重/引用（建议首版），还是也允许新增/删除任务节点、修改 min_line、fallback、scheduler_profile？
  - D10.3 用户（WebUI）调整与云端默认冲突时谁赢：建议用户调整永远覆盖云端默认，云端更新不重置用户调整；另提供「恢复默认」。
  - D10.4 发布节奏与审核：权重调整是否允许比 Model Driver 更轻量的发布流程（例如运营同学发布、需双人确认）。
- 决定：

### D11 NDN 与 AICC 的职责边界

- 现状矛盾：协议文档写「下载、验签、staging、防回退由 NDN 负责，AICC 不重复实现」，但 `cloud_update.rs` 自己实现了下载、sha256、staging、revisions 目录和高水位。
- 待决：交给通用 NDN 更新链路（需要 NDN 提供对应能力与接口），还是改文档承认 AICC 自行实现（并补上 path-obj 验签）。
- 决定：

### D12 可观测性与上报

- 问题：运维是否需要看到全网更新成功率、版本分布？是否允许客户端上报 `metadata_target_seq` / 失败原因（涉及隐私与流量）？本地 UI 现有 `driver_metadata_update.get` 状态是否足够？
- 决定：

## 2. 已确认的缺陷（不依赖决策，可先修）

- [ ] **安全**：`NdnCloudObjectFetcher` 把 AICC 的 service session token 作为 `Authorization: Bearer` 发给任意 `source_url`（[cloud_update.rs:196-247](../src/frame/aicc/src/service/cloud_update.rs#L196-L247)）。外部源不应收到本 Zone 凭据；官方公开源应匿名获取。
- [ ] index 获取无任何完整性/来源校验（`fetch(&index_url, None)`），接入 D2 的 path-obj 验证前属于信任链缺口。
- [ ] 收敛失败无自动重试：推理路径不调用 `before_inference`（仅测试使用），`check_once` 无新版本时也不再发事件；若没有 `auto_sync_models` 的 Provider，失败会一直持续到手动刷新或重启。
- [ ] `reconcile_inventory` 对**所有** Provider 串行做完整网络 discovery，没有「沿用已保存模型列表 + 新 metadata 重建」的路径（与协议 §10 不符），放大了单个 Provider 故障的影响。
- [ ] 下载阶段只校验 CatalogSnapshot，不试建 ModelRegistry；与硬编码逻辑树冲突的 catalog 会被接受，之后每次收敛都在 registry 构建处失败，而 Provider 库存/LKGS 已写入新版本。
- [ ] `revisions/<seq>/` 从不清理。
- [ ] `driver_metadata_update.set` 无法清空 `source_url`（`request.source_url.or(current)`）。
- [ ] local/system-config 覆盖文件变更没有监听，只在 settings reload 或重启时生效；需在文档中写明或补触发。

## 3. 决策后的实现 TODO

- [ ] 按 D1/D2 实现 cyfs 源：`CyfsNdnClient` 配置 `path_verifier(NameClientPathVerifier)`（或 pin DID 的自定义 verifier），index 必须带有效 `cyfs-path-obj`；manifest/文件按 ObjId 获取；按 D2 结论处理 https 回退。
- [ ] 默认 `CloudUpdateConfig`（source、enabled、interval、抖动）按 D1/D4 落地；安装/激活流程按需写入 `services/aicc/driver_metadata_update`。
- [ ] 按 D6 接入系统升级通道/灰度配置与 `supported_features` 声明。
- [ ] 按 D7 调整 builtin 版本/指纹语义与 LKGS 兜底判断。
- [ ] 按 D9 修改 `routable` 判定与重试退避。
- [ ] 按 §5 实现 logical tree catalog（可先于云源上线，只走 builtin/local/system-config）。
- [ ] 发布工具（D5）：从 `driver_metadata/` 生成 catalog 文件、manifest、index，计算 ObjId，签 path-obj JWT，发布到发布 Zone；CI 校验门禁。
- [ ] 应急 SOP 与 kill switch（D8）；运维文档。
- [ ] 同步协议/存储文档（D11 结论），删除与实现不符的描述。

## 4. 验收

- [ ] 离线：伪造/过期/错 host 的 path-obj 被拒绝；ObjId 不符被拒绝；不向外部源发送本地 token；registry 冲突的 catalog 在下载阶段被拒绝；单 Provider 故障不导致其模型被摘除（按 D9）。
- [ ] 测试环境：搭一个测试发布 Zone，走完整「发布 → 客户端拉取 → 收敛 → UI 显示新 revision」；再发更高 seq 回滚一次；验证 kill switch。
- [ ] DV：开启云更新后观察一个完整周期，确认 `metadata_target_seq` 与各 Provider `metadata_applied_seq` 一致、无 unmatched 回归、价格来源正确。

## 5. Logical Tree 自动更新：实现思路（草案）

### 5.1 现状拆解

`ModelRegistry::build`（[model/mod.rs:419](../src/frame/aicc/src/model/mod.rs#L419)）每次收敛完整重建逻辑树，输入分四部分：

| 部分 | 当前来源 | 可否外置为数据 |
| --- | --- | --- |
| 任务节点定义（path、api_type、min_line、mount_mode、scheduler_profile、fallback、tier） | `builtin_logical_model_definitions()`，Rust 构造 `LogicalModelDefinition`（非 serde 类型） | 需要新 schema |
| 功能路径 → 规格引用及权重、媒体家族权重 | `builtin_logical_tree_overlay()` + `MEDIA_FAMILIES`，产出的是公共类型 `AiccRouteOverlay`（已可 serde） | 几乎直接可用 |
| 规格 / 家族 / driver mount / auto mount | Model Driver catalog + inventory | 已是数据 |
| 用户调整 | settings `session_config`（`AiccRouteOverlay` + `routing_commands`），每次重建后重放 | 已是数据 |

结论：「默认优先级」这部分本来就是 `AiccRouteOverlay` 形态，只是用 Rust 代码拼出来；外置的主要工作在任务节点定义的 schema。

### 5.2 目标形态

- 新增 catalog kind `logical_tree`，文件放 `src/frame/aicc/driver_metadata/logical-tree/<catalog_id>.logical-tree.json`，与另外三类一样走 `system-config > local > cloud > builtin` 逐身份整文件选择。
- 建议拆两个 catalog_id，便于分别更新（D10.2 决定云端首版开放范围）：
  - `tasks`：任务节点定义（变动少、风险高）。
  - `defaults`：功能路径 → 规格/家族引用及权重、全局权重（变动多、风险低，首版云端主要发它）。
- 文件结构草案：

```json
{
  "format": "buckyos.aicc.logical-tree-catalog",
  "schema_version": 1,
  "schema_revision": 1,
  "revision_seq": 1,
  "catalog_id": "defaults",
  "tasks": [ /* 仅 tasks 文件：path/api_type/min_line/mount_mode/scheduler/fallback/tier */ ],
  "overlay": { /* 仅 defaults 文件：AiccRouteOverlay 子集：logical_tree、global_exact_model_weights、provider_weights */ }
}
```

- builtin 版本由现有 Rust 常量**一次性导出**成 JSON（写一个测试/工具把 `builtin_logical_model_definitions()` 和 `builtin_logical_tree_overlay()` 序列化并与 JSON 做 golden 对比），之后删除 Rust 常量，JSON 成为唯一事实源。不保留双轨。

### 5.3 运行时改动

1. `catalog`：`CatalogKind` / `CloudCatalogKind` 增加 `LogicalTree`；`CatalogSnapshot` 暴露 `logical_tasks()` 与 `logical_defaults_overlay()`；schema 校验（路径合法、api_type 与路径命名空间一致、权重有限且非负、引用目标形如 `llm.<spec>` / 家族路径）。
2. `ServiceModelAssembler::build`（[model_defaults.rs:475](../src/frame/aicc/src/service/model_defaults.rs#L475)）：定义与 factory overlay 改为从 `catalog` 读取，其余不变——因为逻辑树本来就随每次收敛重建，catalog 一变树就跟着变，**不需要新的触发机制**。
3. 引用悬空处理：`defaults` 引用了当前 Model Driver 中不存在的规格（例如云端先发了树、规格还没到，或旧客户端没有该规格），应忽略该引用并记诊断事件（复用 `routing_command_stale` 类事件），而不是让 registry 构建失败。
4. 与规格命名冲突（`register_specs` 目前直接报错）：在 catalog 校验阶段就把「任务节点 path 与规格/家族名冲突」判为非法，拒绝该发布；运行时不应走到 registry 报错。
5. 用户调整优先（D10.3）：用户 overlay / `routing_commands` 继续作为更高层在 factory 之后应用和重放，云端更新默认值不改用户层；已有的 stale 检测覆盖「用户调整的目标被云端删除」。
6. WebUI：Routing/Models 页显示每个权重的来源（builtin / cloud revision / 用户），提供「恢复默认」= 删除对应用户命令。

### 5.4 云更新链路改动

- manifest `catalog_kind` 增加 `logical_tree`，track 用 `required_features: ["logical_tree_v1"]` 隔离，不支持的旧客户端不会选到含该 kind 的 track；客户端 `supported_features` 声明 `logical_tree_v1`（目前写死为空，需一起改，见 D6）。
- 下载阶段的校验必须**试建一次 ModelRegistry**（空 inventory 或当前 inventory），树结构非法就拒绝整个发布，不推进 `metadata_target_seq`（同时修复 §2 的「只校验 CatalogSnapshot」缺陷）。
- 只改 `defaults` 的发布不应导致全部 Provider 重新 discovery：逻辑树变化不影响 inventory，应只重建 ModelRegistry。可在收敛时比较「影响 inventory 的 catalog（Model Driver / Provider Rules / Known Provider）」是否变化，未变则跳过 `reconcile_inventory`，只推进各 Provider 的 `metadata_applied_seq` 并重建 registry。

### 5.5 分步落地

1. [ ] 定义 `logical_tree` schema 与校验；Rust 常量导出为 builtin JSON，golden 测试保证导出前后 `ModelRegistry` 完全一致（节点、引用、权重、fallback）。
2. [ ] Assembler 改为读 catalog；删除 Rust 常量；悬空引用降级为诊断。
3. [ ] local / system-config 覆盖可用（不依赖云源，可立即用于 DV 调整默认优先级）。
4. [ ] 云更新接入：新 kind、required feature、下载期 registry 试建、tree-only 发布跳过 reconcile。
5. [ ] WebUI 来源标注与恢复默认。
6. [ ] 文档：`aicc 逻辑模型目录.md`、`frozen_model_driver_and_logical_model_fs.md`、云更新协议 §4/§5 增加 `logical_tree`。

第 1–3 步不依赖运维决策，可先做。
