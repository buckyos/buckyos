# Agent＝特殊 App：统一授权契约与实施 TODO

日期：2026-10-29。源码基线：`buckyos@688df1577`；关联 [#689 全链路分析 §15](issue-689-cross-zone-attachments-protocol-todo.md)。本文是待实施方案；本轮只更新文档，不改变服务行为。

## 1. 已确定方向与执行边界

Agent 使用特殊 appid，Owner 的当前权限构成上限，scheduler 按 Agent 配置生成 App 授权。AgentDID 保留为消息作者、收件人、群成员、签名身份；本地资源执行统一为可信的 Owner＋AppInstance，并携带对应 Agent actor。

目标判定：

```text
Principal 当前有效
AND Agent 配置允许本操作
AND Owner 对资源具有有效访问依据
AND Agent App 对资源具有有效访问依据
AND 资源业务约束通过
```

Owner 的有效访问依据可能来自 RBAC、路径 ACL、用户 grant 或合法委托凭据，不能缩减为“Owner 的默认 RBAC 角色”。Agent App 的配置限制是所有授权来源必须遵守的上限。只读的自身运行写入例外应按资源明确授予；不能用服务身份全放行代替。

API 鉴权、容器挂载和 host／executor 是不同的执行边界：本方案先统一 API。挂载后直接访问本地文件仍依赖 mount／OS 限制，配置收紧还要撤销旧挂载／进程能力；不能只刷新 RBAC 就声称所有访问路径已收紧。

## 2. 统一可信主体，保留业务身份

建议由现有 buckyos-api 认证设施提供统一适配结果，不引入新认证服务或第二套权限引擎。下表为逻辑契约，具体类型名不在此冻结。

| 字段 | 来源与含义 |
|---|---|
| owner_user_id | AppInstance／AgentSpec 中的真实 Owner，不能直接信任请求参数 |
| app_id／authorization_key | 绑定的 runtime AppId 与 `app:<id>` RBAC subject |
| app_instance_id | 精确包含 Owner 的 AppInstance，防止同 appid 跨实例混用 |
| agent_did | 对应绑定的 Agent，作为消息／群业务 actor |
| authenticated_subject／method | 实际 token／签名／RTCP 验证来源，保留审计信息 |
| binding_generation／permission_mode | 当前可信配置，不能让旧 token 永久冻结旧权限 |
| sudo | 独立验证的短期能力；普通 Agent 不隐式继承 Owner 可申请的 sudo |

App runtime token 已经接近这一结构。AgentDID token 经验签后，也应解析 AgentSpec、Owner、AppInstance，核对账户可用、绑定有效、Agent 启用状态，再形成同一种执行 principal。不能仅把 AgentDID 查到的 Owner 填进 Caller 而保留其他硬编码放行分支。

统一执行 principal 时必须保留 delegated 标记和 `user_direct=false`。不能简单变成 Owner 的直接 User session：当前 bucket 的 `is_admin` 对 Zone owner 的 Session::User 有直接放行路径，可能把 quota／release 等管理入口一并开放。Owner 是管理员不意味着其 Agent 拥有管理员直接会话；HTTP、RPC、S3 key 的认证适配都需要识别 Agent 绑定。

同 AppId 多 Owner／多 AgentSpec 的场景必须保留实例维度。当前 scheduler 将角色授予 `app:<appid>`，共享 runtime 取最严格只读；首轮保留这一保护语义。以后增加细粒度 scope 时，要证明不会把一个实例的配置传播到另一个实例。

## 3. 权限组合：哪些是上限，哪些是访问依据

### 3.1 保留普通 RBAC 的双主体交集

现有 `rbac::enforce(owner, app, resource, action)` 对配置等普通 RBAC 资源仍可直接复用。读写 Agent 的 App 侧当前已有 `obj://* all`，Owner 侧继续限制最终结果。无需再加一条全局 App allow。

涉及路径 ACL／bucket grant／capability 的服务，需要复用同一个 enforcer 的按侧求值能力，分别计算 Owner 和 App；不能在一个合取 bool 失败后，不分原因地尝试 grant。

### 3.2 bucket 的组合必须能表达两个反例

```text
反例 A：Owner 只能通过 user grant 读到 X，App 配置也允许读。
正确结果：允许；不能因为 Owner 默认 RBAC 没有这条资源而提前拒绝。

反例 B：Owner 可以写 X，但 Agent 是只读；请求携有效 write grant/token。
正确结果：在已识别为此 Agent 的上下文中拒绝写；不能由 fallback 放行。
```

推荐顺序：

1. 验证 principal 当前有效，并解析受控的 resource/action。
2. 执行独立的 Agent 配置限制，包括只读／资源 scope／自身运行例外；拒绝后不继续尝试其他授权来源。
3. 求 Owner 对目标资源的有效访问依据；必要时组合 Owner RBAC、Owner grant 或文件 ACL。
4. 求 App 侧的授权范围；scheduler RBAC／显式 App grant 都不得突破第 2 步。
5. 校验被提交的 capability：目标、资源、操作、时效、身份绑定和委托；需要消费次数时只在最终授权成功后消费，避免分别计算 Owner/App 时重复消耗。
6. 执行数据策略／业务规则：documents 管理、已有对象覆盖／删除、消息收件关系、目录子树等。

组合结果还要保留真实授权来源和范围：Owner grant 不能包装成 `Via::Identity`，否则会改变再授权／签发 token 的限制。List 的结果范围应为 Owner 有效范围与 App 允许范围的交集，保留 `narrow_prefix`；首版可拒绝无法安全收窄的宽前缀请求。copy／batch 等逐目标求值，拒绝请求不能消费 token 次数或改变绑定。

具体资源的 capability 可以作为有效访问依据，但必须说明其授予的是 Owner、App 还是特定委托上下文。不能因为请求里出现一个 token，就跳过 principal／Agent 配置检查。

当前 `IdentityPolicy::allowed=false` 只表示此途径未放行，随后 grant/token 仍可放行。**将只读上限放进这个 bool 不足以实现硬限制。** 最小框架改动是在 bucket 所有授权组合之前加入与身份策略不同的强制限制入口；库只消费抽象限制结果，Owner／AgentSpec 解释由 nfs-server 提供。每个候选 action 分别受限制，`authorize_any` 不能因一个 action 被拒就错误拒绝其他合法 action。

### 3.3 匿名 bearer 的不可降格边界

现有无身份绑定的 `bkt_` 支持匿名持有者。请求若不携 session，源端无法辨别持有者是某个只读 Agent、普通用户还是外部程序。因此：

- 已认证的 Agent 请求必须受共同限制；这可以在现有协议内明确实施。
- 若要求“同一 Agent 进程即使删除身份也不能使用超限 token”，需使用身份绑定、不能降格成匿名的 delegated capability，或在受控执行／网络入口强制身份。
- 保留匿名 bearer 的现有用途时，不能同时宣称 RBAC 已识别并约束所有匿名持有者。裸 token 是独立的“持有即授权”模型。
- 首轮跨 Zone 附件只签发读取、限定 root／目标 identity 的传输授权；不向只读 Agent 分发超范围写凭据，也不把 Owner 会话／原始密钥交给 Agent 当通用认证。

匿名 bearer 本身的用途继续保留。Agent 执行上限的保证范围必须在协议中写清，不能仅靠客户端承诺“始终带 App token”。

## 4. 资源 URL 的具体收敛方案

### 4.1 三层名称各自有固定用途

| 层 | 例子 | 用途 |
|---|---|---|
| PermissionItem scope | `user/home`、`kapi/msg-center` | 用户批准和配置生成输入，不直接参与 Casbin URL 匹配 |
| Locator／传输 URL | `cyfs:///home/alice/a.txt`、`/bucket/v1/...` | 定位资源、协议传输，不直接假定等于权限资源 |
| Canonical authorization resource | `obj://config/...`、`obj://bucket/...` | policy 与 enforce 使用的同一个规范资源标识 |

入口先解析合法 locator，构造规范资源，再用这个解析结果执行访问。不要让 scheduler、SDK、服务各自手写不一致的转换；也不要让鉴权和实际读取分别解码／规范化。

### 4.2 推荐新增文件资源子空间（提案，尚未实施）

为保持 Agent App 的 `obj://` 授权模型，推荐将原生 NFSP 文件的授权资源映射为：

```text
定位：cyfs:///home/alice/Documents/a.txt
权限：obj://dfs/files/home/alice/Documents/a.txt

定位：cyfs://_/library/public/manual.pdf
权限：obj://dfs/files/library/public/manual.pdf
```

`obj://dfs/files/...` 表示文件资源的授权对象；客户端文件 locator 继续使用 cyfs。当前 `obj://dfs` 只有服务对象说明和少量历史策略引用，尚无这一子空间实现，所以这是显式新增契约，不能描述成已修正的现有地址。

选择这个方案的原因：现有 Agent App 的读写／只读 `obj://*` 规则可以覆盖文件资源，Owner 仍由 NFSP path ACL 判断上限；无需为每个 Agent 复制 Owner 的文件 ACL。若最终决定让 RBAC 直接接收 cyfs URI，则必须同步扩展所有 App 的 scheme 范围和 Owner 侧求值，不能只改一处字符串。

规范化要求：

- 当前 Zone 的 `cyfs:///` 与 `cyfs://_/` 等价；外 Zone locator 由跨 Zone 获取路径处理，不能在本地随意去掉 authority。
- root 与相对路径来自既有 namespace 解析；拒绝越根、二次解码、非法路径和已有 symlink 逃逸。
- root 本身与其 descendants 的授权明确，目录枚举按子项权限过滤。
- 路径大小写保持原文件系统语义，不能通过全量 lowercase 实现所谓归一化；现有 matcher 对资源字符串大小写敏感。
- 资源表示不重复包含 query／credentials；分隔符、URL 编码、点等 matcher 字符用共享 builder 和 scheduler 的策略字面量转义处理。
- NFSP 中显示为目录的 bucket projection，最终按其底层 bucket resource 授权，避免由展示路径绕过 bucket 数据策略；collection／view 同样要保留原资源语义，不能全部伪装原生文件。
- 审计历史 `obj://dfs/security/*` 等规则，新增 `files` 子空间后旧 deny 不会自动保护新路径。

### 4.3 首轮保持其他资源主线

config 复用 `get_full_res_path`；bucket 复用 `BucketId::resource`；NDM 复用 RES_* 常量；msg-center 复用 MailboxAddress 和对象 API resource。后续将这些 builder 列入共享契约及测试。不要因统一文件模型而全库改名。

Action 也要映射：list/read/create/write/delete/manage 等与服务动作建立确定对应；复制检查源 read 和目标 create/write，移动还检查源 delete，ACL 修改检查 manage。递归操作同时保留子树约束，不能只检查父目录。

当前尚无通用 PermissionItem scope→RBAC 编译器：安装阶段主要校验选择与声明一致，scheduler 主要生成角色，`user/home` 的具体执行点是读写模式和挂载。现有这些消费点未执行 `PermissionItem.exp`。新契约应同时明确 action、expiry 的单位／起算点、撤销与刷新如何落到 API／挂载等执行边界，不能只增加字段却没有执行者。

## 5. 各模块的实施落点与顺序

| 顺序 | 模块 | 具体改动 | 完成条件 |
|---|---|---|---|
| 1 | buckyos-api／身份适配 | 统一解析 App/Agent 到可信 Owner＋AppInstance＋Agent actor；读取当前权限／绑定 | 两类 token 对同一资源产生相同执行上下文 |
| 2 | bucket_store + nfs-server | 在 grant/token 组合外执行强制限制；去除硬编码 AgentPolicy 的权限语义；填齐 Caller app/user/agent | Owner grant 可用；Agent 上限不可被身份内的 fallback 绕过 |
| 3 | scheduler／RBAC | 从 AgentSpec／受控 scope 生成 App 策略；保留 Owner 动态上限和只读运行例外 | 配置收紧、共享实例行为可预测；无需复制 Owner 权限快照 |
| 4 | NFSP | Delegated actor 保留 Owner 与 App；增加异步 App 资源检查；用 Owner file actor 判断 ACL | App／Agent 可在授予范围内使用文件面，目录过滤和 symlink 约束仍有效 |
| 5 | node-daemon／executor | 按同一 permission mode 生成挂载／host 能力；收紧时更新运行能力 | ro/rw 与 API 一致，不依赖只刷新 RBAC |
| 6 | msg-center／NDN | 附件 ensure 使用统一执行 principal，业务 ACL 使用实际收件 Agent actor | Agent 收件／群身份保留，跨 Zone 源端仍验证可用的身份／委托 |

NFSP 当前权限检查多为同步函数。增加 App RBAC 后，需显式设计请求级异步授权／按资源缓存，不能只把 Delegated 转成 Owner file_actor 然后丢失 App 限制；路径检查应在访问文件前完成，ref/projection 解析后也检查实际底层资源。

`share.*` 也要纳入：当前它借 Owner 的 file_actor 并另查 Agent 只读模式，还没有通用的 App scope 检查。仅修改普通文件入口会继续留下分享操作的授权分叉。watch／已打开句柄的权限刷新同样需要确定边界。

优先把 bucket 的同一 Agent 两种 token 收敛，并验证强制限制接口，再开通 NFSP。不能先删除 `require_file_actor` 就把文件面开放，否则会把 Owner ACL 直接借给任意 App。

## 6. 授权判定实例

| Owner 有效访问 | Agent 配置／App 范围 | 其他依据／业务关系 | 结果 |
|---|---|---|---|
| 允许读 | 允许读 | 正常 | 允许 |
| 拒绝读 | 允许读 | 没有合法补充依据 | 拒绝 |
| 通过用户 grant 允许读 | 允许读 | grant 目标和时效正确 | 允许 |
| 允许写 | 只读 | write grant 或绑定 token | 在已认证 Agent 上下文拒绝 |
| Owner 当前停用／绑定失效 | 旧 token 曾允许 | 内容在本地命中 | 拒绝 |
| Owner 可通过自身 UI 管理 documents | App 配置最大权限 | 业务规则仅允许本人直接 UI 操作 | 依既有数据策略拒绝该操作；是否增加显式委托另作决定 |
| 允许调用 msg-center | 允许调用 | Agent 不是该消息收件人／无群访问权 | 拒绝附件读取 |
| Owner 没有直接群成员身份 | App 允许 | Agent 是成员，Owner 对自己 Agent 资源的管理／观察关系已验证 | 按 Agent 业务身份和既定观察模型判断，不能简单冒充 Owner 群成员 |
| 匿名持有效未绑定 bearer | 无可信 App 上下文 | 现有 token-only 模型 | 按 bearer 规则判断；不能声称已检查某 Agent 的上限 |

“最大等于 Owner”是可执行资源授权上限，服务仍保留有意的操作限制。尤其 `user_direct`、用户文档管理、消息作者／群成员关系不能因为 App RBAC 全放就自动消失；需要扩大委托能力时显式定义。

## 7. 实施 TODO 与回归

- [ ] 冻结 principal、强制配置限制、Owner 有效访问、App 访问、业务规则五个判定步骤；不得把它们折叠成可被 grant 替代的单个 Identity bool。
- [ ] 冻结 `obj://dfs/files/...` 文件授权资源方案及 scope 映射，更新 key URL 文档中过时的 users/home 示例。
- [ ] 统一 App/Agent token 的解析和 bucket Caller；验证 Caller.app 能正确匹配 App grant／bind_app_id。
- [ ] 修改授权组合，统一强制限制与 token 消费位置；检查 HEAD／Range／重试／authorize_any 多 action，避免重复计次。
- [ ] 保留授权来源与范围，验证 Owner grant 不被转成 Identity 再扩大委托；List、copy、batch 只作用于两侧交集。
- [ ] 明确身份绑定 capability 与匿名 bearer 的边界；需要不可降格保证的请求不接受裸 bearer 替代绑定身份。
- [ ] 为 Owner user-grant 可读、App 只读＋write grant、App 只读＋write token、Owner 降权／停用、不同 Owner 共用 AppId 建立最小真实路径测试。
- [ ] 验证正常用户、匿名 token-only、服务自有桶、S3 key、后台 reconcile 的既有行为；强制 Agent 限制不能误套成所有服务的用户模型。
- [ ] Zone owner 的 Agent 不获得直接 admin／user_direct／sudo 旁路；审计 is_admin、own_user、export、quota、release 等入口，以及 Agent actor 与实际创建 App 的审计／creator 语义。
- [ ] 验证自身运行写入例外：AgentRoot／Session／memory、自己的 mailbox、自己的 artifacts；只读不能修改他人的资料／正文或通过 raw key 重新登录。
- [ ] NFSP 测试原生路径、WireRef、projection、collection/view、share、watch、目录枚举、复制／移动／递归删除、symlink、越根和同一路径别名；修改前保留 App file-plane 拒绝的现有测试基线。
- [ ] 检查挂载与 host 能力撤销：旧进程、已打开文件／连接、任务续跑、配置缓存上限；明确哪些收紧要求重建 runtime。
- [ ] 用 Agent App token 与 AgentDID token 各跑相同资源矩阵，再执行 #689 双／三 Zone 附件流程；本地 RBAC 一致不代表外 Zone 已信任该凭据。
- [ ] 同步共享类型、SDK 错误分类、正式身份／RBAC／NFSP／bucket 文档及 `buckyos_admin`、`opendan_agent_dev` 手册；日志区分 principal、Agent 限制、Owner、App、业务 ACL 拒绝，禁止记录凭据。

实施时先执行命中的单测，再按影响范围验证 buckyos-api、scheduler、nfs-server、node-daemon 和跨仓库 bucket_store。正式验收必须覆盖 API、容器／host 两类执行边界和独立 Zone；本轮未运行这些测试。
