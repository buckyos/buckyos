# 跨 Zone SSO 邀请设计评审与实施 TODO

2026-11-20 · 待评审，开发项尚未完成。A 是资源所在的 Zone，X 是安装在 A 的 App 实例，B 是使用自己归属 Zone 的收件人，C 是收到转发链接的另一人。[产品场景和 AC-25～34](<../doc/arch/verify-hub/BuckyOS 混合SSO PRD.md>) · [技术契约 §8.5](../doc/arch/verify-hub/混合SSO技术契约.md) · [SDK 当前流程](../doc/sdk/SSO.md)

## 1. 设计动机

A 想请 B 编辑 X 中的一份文档。A 应能在 X 里选文档、选收件人、复制链接。B 本来就有 DID，应能看清用途，用自己的身份打开原文档。整个过程里，A 可以始终不允许 B 登录自己的系统。反过来，即使 A 允许 B 登录系统，私有文档也应继续按自己的 ACL 判权。

这要求三方各回答一个问题。**A 的系统决定 X 能申请哪类邀请，以及这次能签发什么会话。B 的归属 Zone 证明来的是哪个 DID。X 的资源服务决定这个人对这份文档能做什么。** 为了让三个答案在浏览器跨主机跳转后仍对应同一件事，从申请邀请、返回 URL 到兑现访问资格，都要能关联到同一条系统记录。

## 2. 原有缺陷如何暴露

| 代码演进或现状 | 能解释的行为 | 对改动的要求 |
| --- | --- | --- |
| 10 月 22 日引入混合 SSO。归属证明成功后，资源 Zone 可向未映射本地账号的 DID 签发根域受限 External 会话。委派逻辑拒绝所有 External。 | B 能在 A 根域以自己的 DID 加入获准资源，跨主机交付身份却没有路径。 | 保留已有 DID 证明和后端 code 交换，在 A 明确本次邀请允许交付的目标。[历史委派判定](https://github.com/buckyos/buckyos/blob/0b9faa18a2784235423d378fd919e83ac2c9d964/src/kernel/verify_hub/src/main.rs#L1625) |
| 10 月 29 日 AI Workspace 迁到独立 `aiws-*` 主机。委派失败后前端直接打开目标 URL，根域的 host-only Cookie 不随请求送达。 | **B 已成功加入编辑组，到了工作区页面还是 Guest。** 这是此次 bug 的直接原因。当前工作树的窄修复为受限 External 提供工作区交接，并让工作区识别该身份。 | 保留这条回归并独立验收。普通 App 邀请需要自己的实例约束。[历史子域交接](https://github.com/buckyos/buckyos/blob/127a1eaad0ad1f7bac3785ab69402dd459b11355/src/frame/desktop/src/app/aiworkspace/launch.ts#L19) · [工作区回归用例](../src/frame/desktop/tests/dv/hybrid-sso-federation.spec.ts#L124) |
| AI Workspace 由资源服务检查 `manage`、写分享入口，前端用 ID 拼 `/sso/entry`。Message Hub 也会按群 DID 拼 SSO 链接。Control Panel（CP）在读取入口时才叠加 Zone 登录方式开关。 | 系统能关闭 DID 登录，却没有一个统一的创建入口来决定「X 可以定向邀请，但不能开放邀请」。 | 让会触发 SSO 的入口统一申请系统 URL，保留工作区组 ACL、群准入和匿名访问规则。[工作区分享](../src/frame/desktop/src/app/aiworkspace/ui/shell/ShareEntries.tsx#L102) · [群入口](../src/frame/desktop/src/app/messagehub/guest/model.ts#L22) · [CP 入口描述](../src/frame/control_panel/src/sso_entry.rs#L113) |
| 客服 App 已可用自己的服务身份调 `user.app_login.issue`。CP 查实例开关、登记本地客户邀请并返回 URL，客户兑换时才调用 verify-hub。`user.invite.create` 则是面向入驻的管理员接口。 | 旧实现已有「App 申请、系统返回链接」的先例，但客服的 `enabled` 只管本地客户代登录，未定义通用 DID 邀请策略。 | 复用对调用身份、安装实例和系统 URL 的检查。客服使用 Zone 内 Limited 身份，单独验证其业务连续性。[客服签发](../src/frame/control_panel/src/app_login.rs#L588) · [现有实例开关](../src/kernel/buckyos-api/src/hybrid_sso.rs#L864) · [入驻邀请](../src/frame/control_panel/src/user_mgr.rs#L1401) |

## 3. 新契约与用户流程

**系统是 SSO 邀请链接的签发方，App 可以是申请方。** X 用自己的实例身份向 A 的系统服务提交用途、明确的受众方式、资源引用、允许操作及返回位置。系统校验实例策略及分享者的资源授权，通过内部接口让 verify-hub 登记可核验的邀请，再由系统返回正确的 URL。URL 的域名与路径由已注册目标和路由确定。只拼出相似的 URL 不会获得可兑现的邀请。这里描述服务职责，具体 RPC 名称和记录存储位置在协议实现时确定。前版文档中的「App 签发」在本稿中明确为「App 向系统申请」，评审通过后同步产品和技术契约。

| 用途 | 谁能向系统申请 | 谁可以接受 | 系统最终交付什么 |
| --- | --- | --- | --- |
| 访问 X 的指定资源 | X 代表对该资源有分享权的主体申请，始终绑定 X 的安装实例。系统按实例区分「禁止邀请」「仅指定 DID」「允许指定 DID 或开放邀请」。 | 定向时须匹配记录里的 DID。开放时，持链接且满足入口条件的人可各自用真实 DID 接受。 | 限于 X 的会话，业务操作继续查 ACL。AI Workspace 是 `System(aiworkspace)`，其多个工作区共用服务目标，服务仍须检查工作区和资源。 |
| 登录 A 的系统 | 有授予 Zone 登录资格权限的 service 通过系统入口申请，页面明确写出「允许登录 A」。 | 本轮按指定 DID 设计。App 的开放邀请策略仅适用于该实例。 | 依据系统登录资格签发根域会话。进入其他 App 或具体文档仍查各自权限。 |
| 客服本地身份 | A 的客服业务可以请求系统为客户创建或复用本地 Limited 身份及临时凭据。 | 客户证明持有 A 提供的本地凭据，无需外部归属 Zone。 | 保留同一客户和指定工单。今天 `kind=code` 交付本 App，`kind=link` 可先登录根域再去 Message Hub，后者需迁移。[现有工单入口](../src/apps/sys_test/support_group.ts#L178) |

**B 打开一份 App 邀请的完整流程**

1. A 在 X 中邀请 B 编辑文档 D。获准后系统先登记邀请再返回 URL，记录区分实际申请实例、代表的分享者、授予目的、资源和受众。创建者只公开说明用途所需的信息，文档正文与成员名单继续受资源授权保护。开放邀请也要先有记录。`did:*:*` 只可作为“任意符合条件的 DID”的说明写法，不能当用户、解析目标或会话主体。
2. B 打开 A 的用途页，看见谁请他在哪个 App 做什么。定向链接直接用记录中的 DID，开放链接在同一页填 DID。预览或取消不会加入。B 在 A 已有适用的匹配会话时，可在确认用途后直接继续。发生身份冲突时允许显式切换或取消。
3. 需要身份证明时，A 把一次性登录尝试绑定到邀请、目的地址和目标，浏览器前往 B 的归属 Zone。B 已登录即可免输密码，新的身份披露按归属 Zone 的同意规则处理。A 的 CP 收到根域回调后，用 state 找回这次尝试。两边的 verify-hub 在后端通过 PKCE 兑换一次性 code，A 核对目标 Zone、nonce 和实际 DID。
4. A 按记录复查受众、实例和授权，向资源服务提交受约束的身份，接受成功后关联真实 DID。目标主机通过一次性回调建立自己的会话，回到文档 D 的原路径、查询参数和片段。只有 X 访问资格的 B 到此仍不能登录 A 系统，原有根域身份也不会被替换。开放邀请可以继续供 C 使用，是否限人数、何时过期按入口设置。

**分清记录的寿命。** 邀请记录决定谁还能新加入。接受记录把实际 DID、邀请来源和已获实例或资源资格关联起来。一次性登录尝试只负责本次往返，会话只在指定目标生效。管理员关闭新邀请、关闭一张入口、移除 B 的既有授权，是三个操作。B 以后直接重开深链接时，可根据仍有效的资格恢复访问。单张邀请关闭不应抹掉通过其他来源获得的独立权限。

## 4. 本次 Review 需要定的行为

下面四项影响用户能做什么以及测试应该观察到什么。建议值供本次评审决定。

| 要定的事 | 建议及可观察结果 |
| --- | --- |
| 谁配置 X 的邀请策略，默认值是什么 | 新安装普通 App 默认「禁止邀请」，由 Zone 管理者配置到具体安装实例。若安装者也能配置，明确其授权上限。策略降级先限制新签发。对已经发出且仍可接受的链接是否同步关闭，需要明确选择。已接受授权通过单独的撤销操作管理。 |
| 首次向 A 披露 DID 是否要在 B 再点一次 | 沿用现行 G-04 / Q-05。首次在 B 明示目标和资料范围并确认。已记住对 A 的同范围授权时，只在 A 的用途页点一次，B 自动返回。若首次也要只在 A 确认，需要定义 B 自己认可的放行策略。 |
| 在用户管理点击「允许登录本 Zone」后能进入哪里 | 定向邀请一发出，管理员就能在「不可登录用户」中找到 B 并开通。建议资格按已有 DID 独立保存，不因邀请创建密码、授予默认角色或扩大 App ACL。评审时定下无本地映射者首次系统登录后的页面、允许的系统操作及是否必须建立账号映射，作为系统登录正例。已有账号的封禁、停用和 Root 规则独立生效。 |
| 已发链接、旧会话和客服工单怎样切换 | 建议先接通新的受限交接，再收紧无系统资格的根域签发。对在用的存量入口决定按可信资源记录映射或提示重发。旧根域 External 会话须有明确的续期截止规则，不能因切换获得长期系统资格。客服选定由 App 承载工单页或由 Message Hub 提供仅限该工单的交接，再切换现有 `kind=link`。 |

## 5. 实施顺序与完成条件

- [ ] **0. 保住工作区修复。** 在当前工作树复验 External 工作区交接和私有工作区深链接。记录根域与工作区主机的身份、授权结果。
  - 对应 T1、T6 的旧路径基线。B 能在工作区主机写入，A 能读回，其他资源仍按 ACL 拒绝。该修复可独立交付。

- [ ] **1. 系统签发与实例策略。** 定义申请、登记和返回 URL 的接口，落地实例三档策略。鉴权从真实服务身份推导安装实例，另查操作者分享权。定向必须提供有效 DID，开放须显式选择。记录含系统签发方、申请来源、目标、受众、资源引用、期限和状态。写入成功才返回 URL，重复申请按约定幂等。
  - T2、T10。直接调用接口与 UI 得到相同判定，拒绝后没有可兑现记录。登记失败可安全重试。

- [ ] **2. 工作区接入系统签发。** 把 AI Workspace 分享按钮接到系统申请入口。保留 `group_id` 及资源端 `manage`、加入和 ACL 检查。以相同契约接入 Message Hub 中触发 SSO 的入口。纯业务群邀请和匿名 Guest 继续用各自的准入规则。已有入口按评审决定处理，旧客户端拼 URL 或直调资源接口也须服从新授权。
  - T1～T6、T8、T10。开放受众在成功接受后才关联实际 DID。拒绝、取消、并发和中断均不误授、不重复占名额。

- [ ] **3. 区分两种会话。** CP 恢复 A 根域 pending，复用既有 code 交换。verify-hub 核对邀请或已接受资格及准确目标。只有 App 资格时，以限于该次交付的证明向目标服务建立可撤销的会话，不依赖一个可登录系统的 CP 父会话。已有适用系统会话的委派则继续继承其权限上限和期限。经网关核对主机/安装者，在目标主机写 host-only Cookie。
  - T4～T7、T9、T12。从 `sys.<A>/login` 和根域两条起点都能恢复原资源。身份证明失败、入口失效、资源无权或暂时交付失败分别给出对应处理，重试时恢复原分享。创建链接时或浏览器离开前保存 `#fragment`，HTTP 请求本身不携带片段。

- [ ] **4. 系统登录和用户管理。** 有权限的 service 能申请系统链接，未在 A 登录的 B 能先证明 DID 再接受，避免循环依赖。管理员可按 DID 汇总邀请、查看具体来源、开通登录并审计。不向普通用户开放完整邀请名单。明确映射账号的 DID 取自受信 OwnerDocument，账号 `sub` 可能是本地用户名。接好现有 `user.invite.accept` 所需的身份证明，再切换旧签发策略。
  - T4、T8～T12。在 B 首访之前和之后均能开通。新登录使用新资格，旧 App 会话维持原范围。[现有入驻路径](../src/frame/control_panel/src/user_mgr.rs#L1556) · [用户管理设计](../product/users_and_agents/Users_Agents_PRD.md)

- [ ] **5. 普通 App 与客服。** 接一个真实普通 App 和第二个安装实例。通过已注册实例定位应用，CP 调用它时只使用面向该实例的受限凭据。App 能识别 External，在每次读写时检查资源 ACL。另行迁移和回归客服的本地身份、同一工单、设密码和凭据到期流程。
  - T6～T9、T13。验证同 App 不同安装者、同实例不同资源、权限撤销及实例停用。

- [ ] **6. SDK 与文档收口。** 同步 Rust/TypeScript 类型、SDK、CP 服务说明、产品文档和权限设置界面。统一写明「App 申请，系统签发」，以真实 API 和示例替换拟议文字。记录旧链接、在用授权与会话的升级结果。
  - 示例能从实际 App 分享按钮走到真实业务操作。分别列出工作区、普通 App、系统登录和客服的覆盖范围。[CP 接口说明](../doc/control_panel/Control_Panel_Service.md) · [用户类型](../product/users_and_agents/UserType.md)

## 6. 测试矩阵

| 用例 | 要验证的结果 |
| --- | --- |
| T1 定向首访（AC-25） | B 在 A 无账号。A 从真实 App 申请给 B 编辑 D 的邀请。B 无需填 DID，已登录归属 Zone 不重复输密码。回到 X 后核对实际 DID，B 能编辑，A 能读回。 |
| T2 开放邀请（AC-26） | X 获准开放后，B、C 可分别输入自己的 DID 并接受同一条仍有效的邀请。两条接受记录指向同一邀请，保留各自的业务主体和授权。 |
| T3 复用（AC-27） | 分别预置 A 有匹配会话、仅 B 有会话并已同意 A、B 首次披露。记录需要输入什么、在哪个 Zone 确认几次。撤回同意、资料范围或归属 Zone 改变时重新确认。 |
| T4 错人与取消（AC-28） | C 打开给 B 的链接、B 的归属 Zone 登录成 C、B 主动取消、A 已有另一个系统身份。错误 DID 均不能加入，不消耗 B 的机会，不静默切换身份。另测已映射本地账号的 B 可用同一 DID 接受。 |
| T5 生命周期与失败（AC-28/29） | 分别测试到期、关闭新加入、撤销已接受授权、资源删除、code/state 重用、双标签并发和交付中断。一个 DID 至多新增一次相同资格。已加入后交付失败可重试，其他来源的授权不受影响。 |
| T6 深链接（AC-29） | 最终实际 URL 的主机、实例、path、query、fragment 及页面资源标识都匹配。路由换绑、外部目的地址或实例停用时拒绝交付。直接重开获准的深链接仍能使用有效资格。 |
| T7 权限与续期 | 只读者不能写，可编辑者不能读相邻私有资源。普通 App 的会话不能进入同 App 另一安装实例。分别验证实例停用、资源授权撤销、相关会话退出或到期后的下次操作及续期。 |
| T8 旧入口与身份路径 | 存量 SSO 链接按选定策略恢复或重发。覆盖历史多登录方式、匿名资源、入驻邀请、客服本地凭据。已发链接及旧 External 会话不能绕过切换后的授权。 |
| T9 两项资格四种组合（AC-30/31） | 在没有额外权限的前提下，分别准备只有 X、只有系统、两者都有、两者都无的 DID。对每个 DID 实测打开系统入口和访问 X 中的私有 D。仅有 X 时可完成 D 的操作，且无通用根域系统会话。 |
| T10 申请方与登记（AC-30/32） | 对实例三档策略逐一请求定向、开放邀请。另测 X 请求其他实例或系统登录、操作者无分享权、无权 service、绕过 UI 直调旧入口、登记部分失败或重试。只有通过全部检查时能得到有效 URL。verify-hub 与管理员看到同一记录。策略降级、旧邀请关闭、移除既有权限按评审决定分别断言。 |
| T11 用户管理（AC-32/33） | 两个 App 邀请 B。B 首访前即可在「不可登录用户」里看到他和两个来源。开放邀请尚未兑换时没有虚构用户。管理员开通后 B 在新会话中能使用获准的系统功能，仍不能打开未授权资源。重复开通不重复建号或授予资源权限。停用账号不被意外恢复。 |
| T12 首次系统登录（AC-31） | 获准 service 申请的系统邀请可从 A 无会话的浏览器开始并取得系统会话。另测转发、过期、拒绝及撤销，同一失败不妨碍 B 使用仍有效的邀请。 |
| T13 客服本地身份（AC-34） | 无外部 DID 的客户使用临时链接进入指定工单、留言并再次访问。能区分本地 Limited、App `kind=code` 与外部 DID。全程无归属 Zone code 交换。新 App 客户链接不能用于他人的工单或通用系统登录。 |

### 现有测试入口及交付证据

**Rust。** 在 `src/` 中运行以下测试。在对应包补新策略、记录和权限变更用例，核对过滤后实际执行的测试数。

```bash
cargo test -p buckyos-api --test hybrid_sso_fixtures
cargo test -p control_panel federation::tests
cargo test -p control_panel app_login::tests
cargo test -p verify_hub federation::tests
cargo test -p verify_hub delegate_external_workspace
cargo test -p aiworkspace-store --test sso --test acl
cargo test -p aiworkspace --test sso
```

**浏览器。** 在 `src/frame/desktop/` 中运行。第一条是 mock 交互回归，第二条使用两个允许 DID 登录、可相互解析并进行 HTTPS 交换的真实 Zone。按脚本默认值，Bob 是归属侧，Charlie 是资源侧。替换 IP，并准备 A 的资源管理账号、B 的归属账号、负例身份 C 及普通 App 实例主机。缺少任一 Zone 映射会跳过用例。

```bash
VITE_CP_USE_MOCK=true pnpm exec playwright test tests/e2e/pages/sso-entry.spec.ts tests/e2e/pages/sso-federate.spec.ts tests/e2e/pages/sso-login.spec.ts tests/e2e/pages/user-invite.spec.ts
BUCKYOS_TEST_HOST_RULES='MAP bob.web3.devtests.org <B_IP>, MAP charlie.me <A_IP>' pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/hybrid-sso-federation.spec.ts
```

**这条双 Zone 测试要改起点和终点。** 当前用例直接调 `ws.create_entry`、手拼 URL，最后还断言 A 根域存在 External 会话并能打开受限首页。保留子域身份、实际写入、owner 读回的断言。新链路从 X 正常申请并复制系统返回的 URL 开始。分别以「仅受邀 App」和「具有系统登录资格」验证根域结果，覆盖 T9。管理流程增加首访前可见和开通前后的真实浏览器断言。[当前 DV](../src/frame/desktop/tests/dv/hybrid-sso-federation.spec.ts#L124)

系统入驻另跑 Node 24 的协议 DV，需三个开发 Zone、测试 SN、CA 和 Charlie 的 OwnerDocument。客服使用安装了 `sys-test` 的开发 Zone，运行现有浏览器与业务 DV。[协议 DV 环境及命令](../test/test_hybrid_sso/README.md) · [客服浏览器 DV](../src/frame/desktop/tests/dv/support-group-link.spec.ts) · [客服业务 DV](../test/test_sys_test/support_group_dv.ts)

完成时按工作区、普通 App、系统登录、客服四条路径记录版本、实际命令、通过/失败/跳过数，以及脱敏的目标主机、主体、会话用途和业务操作结果。报告保留失败时的资格状态，不记录 token、code 或邀请 secret。
