# Desktop 对象展示统一 TODO

日期：2026-10-11

目标：在 `src/frame/desktop` 中收敛「DID → 对象 → 图标 + title + 描述」的展示规则。优先完成 Zone 名称/图标配置、用户头像上传及 MessageHub 展示、应用展示一致性。本文记录待开发工作，现状以本次源码核对为依据。

## 1. 当前实现与缺口

以下路径相对 `buckyos/`。

| 范围 | 已有实现 | 需要补齐 |
| --- | --- | --- |
| Zone 登录展示 | [LoginPage.tsx](../src/frame/desktop/src/auth/LoginPage.tsx) 的 `resolveZoneDisplayName` 取 Zone host/当前域名；页面使用固定锁形图标，`document.title` 固定为 `BuckyOS \| Login` | 接入 Zone 展示配置及公共读取，统一登录名称、图标和浏览器标题 |
| 系统设置 | [ClusterManagerPage.tsx](../src/frame/desktop/src/app/settings/pages/ClusterManagerPage.tsx) 只读展示 Zone 名称；[settings/mock/store.ts](../src/frame/desktop/src/app/settings/mock/store.ts) 的 cluster 仍来自 seed；[api/settings.ts](../src/frame/desktop/src/api/settings.ts) 只接入系统版本、开发模式 | 增加真实持久化的 Zone 展示设置，不能只更新 mock/localStorage |
| Zone 后端 | [zone_mgr.rs](../src/frame/control_panel/src/zone_mgr.rs) 的 `zone.overview` 提供运行概览；[sys_auth_backend.rs](../src/frame/control_panel/src/sys_auth_backend.rs) 公共 RPC 白名单不包含它 | 明确展示配置真相源；为登录页提供独立的最小公共展示数据 |
| 用户资料 | [SelfDetailPage.tsx](../src/frame/desktop/src/app/users-agents/components/detail/SelfDetailPage.tsx) 用 prompt 输入头像 URL；[users-agents/datamodel/store.ts](../src/frame/desktop/src/app/users-agents/datamodel/store.ts) 的 `updateSelfAvatar/updateSelfBio/updateSelfInfo` 只改内存 | 文件选择/上传、保存及回读、失败提示、跨页面刷新 |
| 用户资料 API | [api/user_mgr.ts](../src/frame/desktop/src/api/user_mgr.ts) 已有 `setUserProfile`；[control_panel/user_mgr.rs](../src/frame/control_panel/src/user_mgr.rs) 已支持本地 profile 写入，但 `merge_profile_values` 用 DID profile 覆盖同名本地字段 | 复用现有 RPC；定义本地显式配置与 DID 文档字段的优先级，避免刚保存的头像又被旧值覆盖 |
| MessageHub | [api/projection.ts](../src/frame/desktop/src/app/messagehub/api/projection.ts) 已映射联系人头像；[EntityList.tsx](../src/frame/desktop/src/app/messagehub/EntityList.tsx)、[EntityDetails.tsx](../src/frame/desktop/src/app/messagehub/EntityDetails.tsx) 仍使用类型图标；Zone 用户联系人同步未传递头像 | 同时修复资料传播和实际渲染，覆盖会话、群成员、消息发送者 |
| 应用 | [api/app_mgr.ts](../src/frame/desktop/src/api/app_mgr.ts) 的 `AppSummary` 已有 `app_did/show_name/app_icon_url/icon_res_url`；[backend-apps.ts](../src/frame/desktop/src/app/backend-apps.ts) 映射时未保留 DID 和两类图标，名称回退到 host/app_id，描述复用名称 | 贯通 AppDoc 展示字段与桌面对象，统一应用各入口的图标、标题、描述 |
| 可复用能力 | [AvatarPicker.tsx](../src/frame/desktop/src/app/agent-setup/AvatarPicker.tsx) 已有文件选择、裁剪压缩、预览和移除；[DesktopVisuals.tsx](../src/frame/desktop/src/components/DesktopVisuals.tsx) 的 `AppIcon` 支持图片失败回退；Users & Agents 已有 [EntityAvatar](../src/frame/desktop/src/app/users-agents/components/shared/EntityAvatar.tsx) | 从现有实现提取公共部分，避免每个业务页面重复解析和绘制 |

## 2. P0：确定统一展示契约

- [x] 定义轻量 `ObjectPresentation`：稳定对象标识、可用时的真实 `did`、对象类型、`title`、可选 `description`、图标候选；区分加载中、已解析、解析失败。它是 UI 投影，不新增一套身份协议。
- [x] 以真实 DID 查文档/资料，明确支持 Zone、User、Agent、App；已有对象数据可直接投影，避免为每个组件重复解析 DID。
- [x] 身份与显示文字分开：标题变化不能改变路由、鉴权、消息收发或缓存的身份键。App DID、AppId、AppInstanceId、Agent DID 分别保留；内置应用没有 DID 时沿用现有稳定 ID，不伪造 DID。
- [x] 固定优先级：**有权限设置的显式展示配置 → DID 文档及其类型对应资料中的展示字段 → DID 的可读标识部分 → 类型默认文案**。配置缺失与显式清除须有明确语义。
- [x] 按对象类型定义候选字段，禁止递归扫描任意 `name/title` 字段。建议规则如下；字段不存在、非字符串或 trim 后为空时跳过。

| 对象 | 标题候选（从左至右，显式配置已在最前） | 描述候选 | 图标候选 |
| --- | --- | --- | --- |
| Zone | 设置中的展示名称 → 文档约定的展示名称字段 → DID 可读标识 | 设置/文档约定的描述 → 空 | 设置图标 → 文档约定图标 → Zone 默认图标 |
| User / Owner | 生效 profile 的 `display_name` → OwnerDocument 的 `display_name` → 对应 `name` → DID 可读标识 | `bio` → `headline` → 空 | 生效 profile 的 `avatar` → 文档 `avatar` → 首字/用户默认图标 |
| Agent | AgentProfile 的 `display_name` → 已注册 Agent 名称 → DID 可读标识 | profile `bio` / 文档 `public_description` → 空 | profile `avatar` → 模板图标 → Agent 默认图标 |
| App | `presentation.title` 的语言匹配值 → `show_name` → 文档 `name` → DID 可读标识 | `presentation.summary` → `presentation.description` → 空 | 文档 `presentation.icons/links.app_icon_url` 的适用资源 → `icon_res_url` → 内置/应用默认图标 |

- [x] 明确上述是目标映射。当前 `ZoneDocument` 没有标准 title/icon 字段，须先定设置字段和文档投影约定；不能假定这些字段已存在。App 的字段以 [AppPresentation / AppDoc](../src/kernel/buckyos-api/src/app_doc.rs) 为准。
- [x] 同一来源中的 `display_name/displayName/full_name` 等已有别名在适配层归一；不为此次展示改造增加旧协议兼容层。用户 profile 的 `title` 可能表示职务，不能不分类型地当用户名。
- [x] 多语言值统一使用「当前语言 → 基础语言 → 约定默认语言 → 固定顺序的首个非空值」；切换语言重新计算，保证所有页面一致。
- [x] 提供共享 DID 展示回退函数：`did:bns:myapp` 必须得到 `myapp`。复用 DID 解析器，按 method 处理标识及 DID URL 的 path/query/fragment；不用散落的 `split(':').pop()`。`did:web` 保留必要的域名/路径区分，`did:key` 可在 UI 缩写并保留完整 DID 的查看/复制入口。
- [x] 未配置、文档获取失败、字段全空时仍有稳定标题及默认图标；描述缺失可省略，不把 DID、标题或错误信息拼成假描述。

## 3. P0：提取最小公共实现

- [x] 提取纯函数与类型适配器：字段归一、语言选择、标题/描述候选、图标候选、DID 回退。协议解析复用 WebSDK，React 展示规则先放 Desktop 公共目录。
- [x] 核实普通网页中的 DID 解析入口：当前 WebSDK 的 `resolve_did` 依赖宿主 `window.BuckyApi.resolve_did`，没有 bridge 时返回空；需要适配可用的服务端文档/资料读取入口，不能仅调用 SDK 就视为网页解析已打通。
- [x] 提供共享读取 hook/store：复用已有请求层，按 Zone、访问者可见性和真实对象标识隔离缓存；去重请求，支持批量已有资料注入、失败回退及配置保存后的失效通知。切换账号/Zone 清理不适用的缓存。
- [x] 组合现有 `AppIcon`、`EntityAvatar` 提供公共对象图标组件：支持圆形头像/应用方形图标、尺寸、加载占位、坏图回退和 `alt`；URL 变化后允许重新加载。在线状态、类型徽标由业务层组合。
- [x] 提供简单的对象标题/摘要行组件，统一省略、tooltip、可选 DID 副文本及复制入口；列表、卡片、详情页共享规则，不强制所有页面使用同一布局。
- [x] 从 `AvatarPicker` 提取图片选择与处理能力，供用户头像和 Zone 图标复用；支持预览、更换、移除、取消、格式/尺寸/大小校验及保存错误。现实现固定转为 192×192 JPEG，Zone/应用图标需要保留透明背景并允许不同尺寸。
- [x] 明确图片保存契约：复用现有存储能力，确定使用受限 data URL 还是持久资源引用，并统一前后端校验；本地临时 `blob:` URL 只用于预览。图标引用需能由对应读者加载，尤其是未登录的 Zone 访客。
- [x] 建立统一网页标题/favicon 管理入口：页面提供当前标题，Zone 提供品牌名称/图标。收敛 `LoginPage` 和 [AIWorkspaceRoute.tsx](../src/frame/desktop/src/app/aiworkspace/AIWorkspaceRoute.tsx) 中直接写 `document.title` 的逻辑，避免路由切换或卸载恢复旧标题。

## 4. P1：Zone 展示设置 → 登录页 → 网页标题栏

- [x] 在系统设置增加「Zone 名称与图标」：展示名称、图标选择/上传、预览、保存、恢复默认；描述作为同一契约中的可选字段。原 Zone DID、域名作为身份信息独立显示。
- [x] 将展示配置持久化到 system-config 的明确 Settings 键，补读取/更新接口与共享类型；明确允许修改的管理员权限。这里只修改显示元数据，不更改 Zone DID、域名或 owner。
- [x] 定义设置与 Zone DID 文档的关系：设置是显式覆盖，文档提供默认候选；如需要向文档发布，走正式签发/更新流程，不把前端改名等同于修改已签名文档。
- [x] 提供未登录可读的最小 Zone 展示接口/文档投影，接入认证白名单和网关路由。只返回展示所需的 DID、名称、描述和图标引用，不直接开放完整 `zone.overview`。
- [x] 登录页面优先显示配置后的 Zone 图标和名称，加载失败回退到文档/DID/默认值；慢请求或图片失败不阻塞登录。
- [x] 浏览器标签页采用一致规则，例如登录页「登录 · Zone 名称」、桌面「Zone 名称」、独立应用「应用标题 · Zone 名称」；favicon 优先使用 Zone 图标。应用窗口自身标题使用应用标题，避免把两个层级混在一起。
- [x] 保存后当前页面及时刷新名称、图标和 favicon；刷新、退出登录、新浏览器会话仍能取得同一设置。检查 favicon 缓存，确保换图后可见。
- [x] 核对 [index.html](../src/frame/desktop/index.html) 与 [manifest.json](../src/frame/desktop/public/manifest.json) 的静态默认值及各独立路由；PWA 已安装名称/图标的更新能力另列边界，不把网页刷新误当成安装信息已更新。

验收：管理员设置「我的家庭云」和新图标，保存后无痕窗口打开登录页也显示这组信息；登录、退出、切换应用后标题/favicon 正确；未配置或公共读取失败仍可登录；普通用户无法修改 Zone 展示配置。

## 5. P1：用户头像上传与 MessageHub 全链路

- [x] 将 Users & Agents 的个人资料头像入口接入公共图片选择器；支持从文件选择、预览、更换、移除和保存，不再只有输入 URL。
- [x] `updateSelfAvatar/updateSelfBio/updateSelfInfo` 接入已有 `setUserProfile`，以服务端回读结果更新 UI；补保存中、成功、失败与重试状态，失败不能显示为已持久化成功。
- [x] 明确「不修改 / 设置新值 / 清除并恢复文档默认」的请求语义，特别是头像清除，不能靠省略参数表达删除。检查 `merge_profile_values` 的覆盖规则与资料公开范围，使显式本地设置能生效。
- [x] 统一当前用户、其他 Zone 用户和联系人资料的 title/avatar/description 投影；保持真实 DID，不从用户名推导新的身份。公开展示只使用调用者可见字段。
- [x] 跨用户读取采用公开资料投影/联系人接口；现有 `user.get/user.profile.get` 要求本人或管理员，不能为了显示头像而放开完整 `local_profile`。同步时遵守 `to_public_profile()` 的字段可见性，头像改为私有后也须撤下旧公开投影。
- [x] 补齐 `msg_center` 的 Zone 用户联系人同步：[main.rs](../src/frame/msg_center/src/main.rs) 的 `build_zone_user_seed/zone_contact_seed_signature`、[contact_mgr.rs](../src/frame/msg_center/src/contact_mgr.rs) 的 `ZoneUserContactSeed/upsert_zone_contacts` 传递头像及所需展示字段；更新同步指纹/变更检测，确保仅换头像也会传播。保留联系人备注、屏蔽状态和可见性规则。
- [x] 明确 profile 保存后如何触发联系人刷新/重新投影，以及其他登录客户端何时刷新；覆盖 MessageHub store 与 DID/profile 缓存失效，避免必须重启服务才能换头像。
- [x] MessageHub 使用公共对象展示：联系人列表、详情、会话头、群成员列表、消息发送者名称及已有头像位。检查 [ConversationView.tsx](../src/frame/desktop/src/app/messagehub/ConversationView.tsx)、[SessionDetails.tsx](../src/frame/desktop/src/app/messagehub/SessionDetails.tsx)、[history/renderers.tsx](../src/frame/desktop/src/app/messagehub/conversation/history/renderers.tsx)。
- [x] 自己与对方、User 与 Agent、群组使用各自真实身份的头像；联系人备注仅作为当前用户的显示覆盖，不能写回对方全局资料；群头像/群名称不能被单个成员头像/姓名替代。
- [x] 图片缺失、无权限或加载失败时采用统一回退；类型图标保留为默认图标或徽标，不能挡住已经取得的有效头像。

验收：用户上传头像后刷新/重新登录仍保留；另一位有权查看该资料的用户在 MessageHub 看见同一头像；只更换头像也能同步；移除头像正确回退；坏图和私有资料不产生破图或跨账号串图。

## 6. P1：应用统一为图标 + title + 描述

- [x] 贯通 `AppDoc → apps.list/apps.details → AppSummary → AppDefinition → UI`。当前 `AppPresentation` 已有多语言 title/summary/description 及 icons，但 [build_app_summary](../src/frame/control_panel/src/app_servcie_mgr.rs) 未传递它们；选择在摘要中返回必要展示信息或集中读取详情，避免应用列表逐项请求详情。
- [x] 修复 `createBackendAppDefinitionMapper` 丢失 `app_did/app_icon_url/icon_res_url` 的问题；应用标题采用统一候选链，不用启动 host 替代文档标题或 DID 回退，描述不再复制名称。
- [x] 区分内置 i18n 资源键和外部文档的字面标题/描述；当前 `labelKey/summaryKey` 混装二者，不能把外部 title 无条件交给 `t()`，避免名称恰好命中字典键时被错误翻译。
- [x] 明确 `presentation.icons` 的 ObjId 如何解析为可加载资源，以及与 `links.app_icon_url` 的选择次序；统一相对资源地址的基准，独立路由与应用子域名下都能显示图标。
- [x] 合并内置 catalog 的默认文案/图标与后端 AppDoc 数据：catalog 保留内置默认与运行行为，不能整体遮蔽有效的文档展示字段；已有内置应用无完整文档时继续显示合理默认值。
- [x] 桌面图标、全部应用、任务栏、窗口标题、独立应用标题栏、应用列表/详情统一消费同一展示投影；详情/tooltip 展示描述，窄列表可只显示图标和标题。
- [x] 核对应用管理页的数据接入：当前 [use-app-service-store.ts](../src/frame/desktop/src/app/app-service/hooks/use-app-service-store.ts) 仍绑定 mock store；按本任务所需范围接入真实展示数据，不能用演示数据宣称应用管理页联动已完成。
- [x] 应用升级、重新加载元数据、语言切换后统一刷新；同一 App 的不同实例保持实例身份和操作目标，Agent 入口仍使用 Agent 身份与头像。

验收：带自定义图标及中文 title/description 的应用在桌面与应用管理页一致；展示候选缺失/不可用的 UI 输入中，`did:bns:myapp` 显示 `myapp`（此为展示层容错，现行 AppDoc 校验仍要求 `show_name` 非空）；图标资源失败有回退；改名、换图、多实例都不影响启动和权限判断。

## 7. 收敛与验证

- [ ] 按「契约与公共能力 → Zone → 用户/MessageHub → 应用」拆成可独立验收的改动；后续将 Users & Agents、My Network、HomeStation 中已有同类展示逐步迁入，避免一次改完整个 Desktop。（本次改动尚未提交/拆分；Users & Agents 头像已迁入，My Network、HomeStation 未迁）
- [x] 搜索并收敛散落的 `split(':')`、首字头像、固定类型图标、名称/描述拼接及 `document.title` 写入；保留各业务确有意义的显示覆盖。（剩余 `split(':')` 多为非 DID 字符串；HomeStation mock 与 MessageHub `parseTunnelDid`/`ownerToken` 的协议解析保留）
- [x] 单测重点覆盖行为：候选优先级、空白/类型错误字段、多语言回退、DID method 差异、失败后重试、头像清除，以及同 DID 在不同可见性上下文中的缓存隔离。
- [x] 接口验证覆盖：Zone 配置持久化与匿名最小读取、写权限、用户 profile 设置/清除/回读、头像单独变化的联系人同步、AppDoc 展示字段传递。（Rust 单测 + DV `tests/dv/object-presentation.spec.ts`，见 §8）
- [x] 浏览器回归覆盖上述三条验收链路，并包含刷新/新会话、移动端、慢请求、坏图、路由切换、切换账号/Zone；测试接入真实接口，mock 仅用于可控失败场景。（DV 真实登录覆盖 Zone 品牌、头像上传/刷新保留、他人头像在 MessageHub 显示；mock 覆盖坏图、非管理员、新会话登录页；移动端、慢请求、切换 Zone 未单独覆盖）
- [x] 实现时同步检查 Rust 类型、control_panel/msg_center、WebSDK 消费和前端类型；涉及 DID 文档/profile 类型时核对同级 `buckyos-base` 的定义，不在 Desktop 自造协议镜像。
- [x] 实现后的检查按影响范围执行：Desktop `pnpm check`、相关 lint/浏览器用例与 `pnpm build`，以及变更服务的定向 Rust 测试。实施后的执行情况见 §8。

## 8. 补充：各应用的身份头像显示位置盘点（2026-10-11）

原 TODO 只列了 Zone、Users & Agents、MessageHub、应用四条链路，遗漏了 HomeStation 等同样显示人/Agent/Zone 的应用。以下按应用盘点所有「身份图标」位置（含首字圆、类型图标占位），并给出处理决定。「已统一」指已使用 `presentation/` 公共层（`ObjectAvatar/ObjectImage`、`presentUser/presentAgent`、`didDisplayName`）。

| 应用 | 位置 | 显示的身份 | 数据来源 | 改造前 | 决定 |
| --- | --- | --- | --- | --- | --- |
| 桌面外壳 | 任务栏/状态栏/资料卡 `UserAvatar`、Jarvis 引导图标 | 本人、本人的 Agent | `user.get` profile、`agent.list` | — | 已统一 |
| Users & Agents / My Network | `EntityAvatar`（列表、卡片、详情头） | 本人、本地用户、Agent、联系人 | `user.get`、`user.list`、`agent.list`、msg-center 联系人 | — | 已统一；- [x] My Network 联系人头像补 `usableImageUrl`、名称缺失回退用 `didDisplayName` 而非原始 DID、不再拼 `did:bns:<user>` |
| Agent 创建 | `AgentAvatar`（确认页、创建状态） | 新 Agent | 草稿/模板图标 | — | 已统一；- [x] 权限页「主人」行改用本人真实头像（原为无图的 Agent 组件） |
| MessageHub | 列表、详情、会话头、群成员、建群选择 | 联系人、Agent、群、本人 | msg-center 联系人 `avatar` | — | 已统一；- [x] 转发对话框目标图标改用 `HubAvatar`；- [x] 消息发送者名称缺省时用 `didDisplayName` 而非原始 DID |
| HomeStation | 顶栏/侧栏/「我」页本人头像、动态卡片作者、引用卡片作者、评论作者、个人主页头像 | 本人、关注的人、评论者 | `ui.bootstrap.identities`、卡片 `publisher`、`profile.get`/`portal.profile`（后端 `identity_view`/`PublicProfile`） | 只有首字+色相，后端不给头像 | - [x] 后端：msg-center 联系人头像进 `identity_view`/bootstrap，`PublicProfile` 与 `profile.get` 带 `avatar`（跨站读取随公开资料传递）；- [x] 前端 `Avatar` 有图显示图片、坏图回退原首字样式；- [x] `peekIdentity` 名称回退用 `didDisplayName`，后端回退用 DID 解析 |
| HomeStation 门户 / Zone feed | 门户顶栏、`~zone` 标题与资料 | Zone | `portal.home.zoneName`、`zone_profile` | Globe 图标；名称只看 HomeStation 自己的 `zone_name` | - [x] 名称改为「HomeStation `zone_name` → Zone 展示设置 → Zone 主机名」，Zone 图标进 `zone_profile.avatar` 与门户顶栏，服务每 60 秒刷新 |
| HomeStation 来源管理 | 关注来源列表 `SourceIcon` | 关注的人/RSS/站点 | `SourceView.kind` | 类型图标 | - [x] 人物来源有 DID 时显示其头像（`peekIdentity`），其余仍用类型图标 |
| AI Workspace | 工具栏身份按钮与身份面板 | 本人 | 会话 principal（用户名） | 首字 | - [x] 显示本人头像（与任务栏同源），批注/锁/版本作者仍为文字（无头像位） |
| OpenDAN WebUI（独立包 `src/frame/opendan/web`） | 首页 Agent 头像、编辑预览、卡片头 | Agent | `agent.profile` | `<img>` 无坏图回退；会话标题用 `split(':').pop()` | - [x] 坏图回退到首字；- [x] DID 名称回退按 DID 解析（去掉 DID URL 部分、`did:web` 路径） |
| 公开资料页 `/userprofile` | 头像圆 | 任意用户 | 无数据源（占位页） | 首字 | 不改：占位页，接入 HomeStation 公开资料另列 |
| 应用管理 / 安装器 | 所有者 | 用户 | `owner_user_id` | 纯文字 | 不改：无头像位 |
| Task Center、文件、预览、AI 中心、Studio、市场、诊断、Workflow | — | — | — | 无身份头像 | 不涉及 |

未纳入本次：MessageHub 群邀请卡片（群无图片源）、@ 提及候选与消息详情（无头像位）、Users & Agents 实体组成员（真实运行时为空）、My Network「添加联系人」的写死占位、CodeAssistant 的假 DID。

## 9. 实施记录（2026-10-11）

### 落点

- 共享展示层 `src/frame/desktop/src/presentation/`：`model.ts`（`ObjectPresentation`、按类型的候选链 `presentZone/presentUser/presentAgent/presentApp/presentDidDocument`、`pickLocalized`、`didDisplayName`（复用 WebSDK `namelib.DID`）、`usableImageUrl/resolveImageReference`）；`store.ts`（按 `zoneHost|viewer` 隔离的缓存、请求去重、失败 30s 后重试、`put` 注入、`fetchDidDocument` 走 Zone 的 `/1.0/identifiers/<did>`，有 bridge 时先用 `buckyos.resolve_did`）；`ObjectIcon.tsx`（`ObjectImage`/`ObjectAvatar`：候选逐个加载、坏图回退、加载占位）；`ObjectTitle.tsx`；`imageFile.ts` + `ImagePicker.tsx`（从 AvatarPicker 提取，头像 192px JPEG 方形裁剪、Zone 图标 256px PNG 保留透明）；`documentHead.ts`（标签页标题栈 + Zone 品牌名/图标 favicon）；`zone.ts`；`appText.ts`；`useDidPresentation.ts`。`AppIcon`、Users & Agents `EntityAvatar`、`AgentAvatar`、任务栏 `UserAvatar`、MessageHub `HubAvatar` 都建在 `ObjectImage/ObjectAvatar` 上。
- Zone：共享类型 `buckyos-api` `ZonePresentationConfig` / `ZonePresentationFields`（键 `system/zone_presentation`；ZoneDocument 投影约定为其扩展字段中的 `display_name/description/icon`）；control_panel `zone.presentation.get`（公开，进白名单，走已有 `/kapi/control-panel` 网关路由）与 `zone.presentation.set`（Admin/Root + Control Panel 用户 sudo session，调用者 token 经 RBAC `su_admin` + revision CAS）。前端：设置「Zone 名称与图标」页、登录页、`ZoneBrand`（`App.tsx`）。标签页规则：登录「Sign in · Zone」、桌面「Zone」、独立路由（MessageHub / TaskCenter / HomeStation / AI Workspace）「应用 · Zone」。
- 图片保存契约：受限的内联 base64 data URL（PNG/JPEG/WebP，≤512KB；头像 ≤192px，Zone 图标 ≤512px），control_panel `image_data_url.rs` 统一校验（Agent 头像同用），任何能读到记录的读者都能直接加载，匿名访客也能看到 Zone 图标。
- 用户资料：`user.profile.set` 文本字段三态（省略=不改、`null`/空串=清除、字符串=设置），头像校验；`merge_profile_values` 改为本地显式值优先、DID 文档只补空；`user.list` 附带公开投影的 `did/avatar`；保存后 control_panel 调 msg-center `reload_settings`（`agent.profile.set` 同）。前端 Users & Agents 头像对话框、`updateSelf*` 走 `setUserProfile` 并以 `user.get` 回读、保存中/成功/失败+重试提示；本地用户不再拼 `did:bns:<user>`。「管理账户」直接打开本人资料页。
- msg-center：`ZoneUserContactSeed.avatar`，用户取 `to_public_profile()`、Agent 取 `AgentProfile.avatar`；同步指纹含头像 SHA-256；Zone 同步对头像以 Zone 公布值为准（公开值消失即撤下），备注/屏蔽不受影响。前端 MessageHub：投影 `avatars`（含本人），列表、详情、会话头、群成员、建群成员选择器用 `HubAvatar`/`MemberAvatar`（不在联系人里的成员读其 DID 文档）；联系人在本人资料/Agent 变更事件与每 60s 重读。
- 应用：`apps.list` 摘要增加 `name` 与 `presentation{title,summary,description}`，删除从未被任何路由提供的 `icon_res_url`；`AppDefinition.text`（文档字面文本，按语言）与 `did`，`labelKey/summaryKey` 只作内置 i18n 键；`useAppText()/appTitleNow()` 供桌面图标、全部应用、任务栏、窗口标题、移动状态栏、iframe 标题、内容处理器标签统一使用；`app_icon_url` 相对地址以应用自己的 web host 为基准；应用管理页在真实运行时把「应用」层换成 `apps.list` 的真实应用（只读展示与状态，不挂演示操作）。
- 文档：`doc/control_panel/Control_Panel_Service.md`（§4.1/§4.4/§4.6/§6.1/§7/§8）、`doc/message_hub/Contact Mgr.md`、MessageHub `UI_DATAMODEL.md`。

### 取舍与边界

- `presentation.icons` 的 ObjId 目前没有浏览器可加载的 NDN 下载路由，前端只用 `links.app_icon_url`；待有对象读取路由后再接入并排在 `app_icon_url` 之前。
- 不向 ZoneDocument 发布展示字段；需要时走正式签发流程。PWA 已安装的名称/图标（`manifest.json`）不随设置变化，网页刷新只更新标签页标题与 favicon。
- msg-center 的头像只取本地 profile 的公开投影，不读 `users/<id>/doc` 的 `profile/meta`；用户清除本地头像后，Users & Agents 会回落到 DID 文档里的头像（若有），MessageHub 显示默认图标。
- 应用管理页的「系统/内核」层与安装器流程仍是原型数据（其 UI_DATAMODEL 已说明）；本次只接入「应用」层的真实展示。
- 设置页「集群管理」仍是 mock seed（含其中的 Zone 名称），未接入真实数据。
- 消息气泡没有头像位，发送者只显示名称（来自联系人投影），未新增头像位。

### 验证

- `cargo test -p buckyos-api -p msg_center -- --test-threads=1`（232 + 137 通过）、`cargo test -p control_panel`（104 通过）；新增：Zone 配置校验/文档投影、补丁三态、图片校验、合并优先级、公开头像投影、只换头像触发同步、头像设置与撤下。
- Desktop：`pnpm check`、`pnpm exec eslint src tests`（0 error，8 条既有 warning）、`pnpm build`；`deno test --config tests/datamodel/messagehub.deno.json --no-check -A tests/datamodel/*.test.ts` 全过（新增 `object-presentation.test.ts`，扩充 `app-launch-mapping`、`messagehub-projection`）。
- mock e2e：新增 `tests/e2e/pages/object-presentation.spec.ts`（4 例）；全量 `pnpm exec playwright test` 235 过，失败只剩已知基线（desktop「SN Router」、mobile「App Service redundant back button」、messagehub「codeassistant wheel scrolling」并行偶发）；`app-installer-cancel` 已随 `apps.list` 调整；AI Workspace UI-P15 标题用例通过。浅色/深色截图已核对设置页、头像对话框、登录页、MessageHub。
- 真实 Zone（2026-10-11，用户手动部署新的 control_panel / msg_center / web 后）：`BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… pnpm exec playwright test --config=playwright.dv.config.ts object-presentation` 4 例全过——匿名读取、无 sudo 拒写、URL 图标/头像被拒、字段级清除；未登录登录页显示 Zone 名称/描述/标题/favicon；真实登录后桌面标签页与 favicon；Users & Agents 上传头像→任务栏→刷新保留；管理员（sudo）给 alice 设头像后 devtest 的 MessageHub 显示该头像；`user.list` 公开头像、自身联系人同步与清除后撤下；`apps.list` 摘要字段。测试结束后 Zone 设置与头像均已清空。注意 baseURL 必须是 Zone 根域，`sys.` 子域登录会被拒（control-panel 只向根域签发系统 token）；修改他人资料需要管理员的 sudo 会话。

### 第二轮：补齐盘点出的位置（2026-10-11）

- 起因：DV 上桌面图标「小白」有头像、MessageHub 仍是机器人图标。核实为浏览器仍在用 service worker 缓存的旧前端（新浏览器里三处都显示头像）；Desktop 没有「有新版本」提示（只有 AI Workspace 有），新前端要等所有标签页关闭重开才生效。另查出 HomeStation 等位置原 TODO 未覆盖，见 §8。
- HomeStation 后端：`ContactInfo.avatar`（取 msg-center `Contact.avatar`）→ `ViewCtx.avatars` → `identity_view` / `ui.bootstrap.identities`；`PublicProfile.avatar` 与 `profile.get` 带头像（跨站随公开资料传递）；`identity_view` 名称回退改为 `did_display_name`（DID 解析）；`HostConfig.zone_name` 改为可选，`Host::zone_name()` 为「HomeStation 设置 → Zone 展示设置 → 主机名」，`ZoneBrand` 由服务每 60 秒从 `system/zone_presentation`（及 ZoneDocument 约定字段）刷新，`~zone` 资料与 `portal.home.zoneIcon` 带 Zone 图标。新测试 `tests/presentation.rs` 与 `projection.rs` 单测。
- HomeStation 前端：`IdentityView/ProfileView.avatar`、`PortalHome.zoneIcon`；`ui/primitives` 的 `Avatar` 有图显示图片、坏图回退原首字样式（所有作者/评论者/本人/个人主页头像随之生效）；门户顶栏与 `~zone` 标题显示 Zone 图标；来源管理人物来源显示头像；`peekIdentity` 名称回退用 `didDisplayName`。
- 其他：MessageHub 转发对话框用 `HubAvatar`，发送者名称缺省用 `didDisplayName`；Agent 创建权限页主人行用本人真实头像；My Network 联系人头像过滤、名称回退、不再拼 `did:bns:<user>`；`useCurrentUser` 移到 `presentation/currentUser.ts`，AI Workspace 身份按钮/面板显示本人头像与名称（开发直连身份不显示）；OpenDAN WebUI `Avatar` 坏图回退、`didName` 按 DID 解析。
- 文档：`doc/homestation/HomeStation 协议与实现.md`（`portal.home`、`/home/<user>/profile`、`~zone` 资料、`zone_name` 回退、单机 `contacts[].avatar`）、HomeStation `UI_DATAMODEL.md`。
- 验证：`cargo test -p homestation`（含新增 3 例）；Desktop `pnpm check`、eslint（0 error）、datamodel 单测全过；HomeStation mock e2e 16 例通过；OpenDAN WebUI `tsc`、eslint、mock e2e 14 例通过。DV 用例新增「HomeStation 显示本人头像、`~zone` 采用 Zone 展示设置」，需部署新的 homestation 与 web 后运行。
