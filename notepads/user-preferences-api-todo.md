# 用户偏好 SDK 接口 TODO（`user.preferences.get`）

2026-11-27 立项。

## 1 来源

AI Workspace 在新标签页打开（`aiws-<id>.<zone>`，独立的源）时不跟随桌面的明暗主题：那个源的 localStorage 是空的，默认亮色。第三方 App 都在自己的域名里，同样拿不到当前用户的系统设置。owner：SDK 里应该有这个接口，实现方按 App 的权限决定返回哪些值。

## 2 现状

- 应用在桌面窗口或客户端里：app-frame 的 `frame.init` / `frame.themeChanged` / `frame.localeChanged` 已经推送主题、强调色、语言。单独在浏览器标签页里：什么都拿不到。
- 账号外观存在 system-config `users/{user}/desktop/default/appearance`（Desktop 经 `/api/desktop` 读写，`shared/appearance/model.ts`），字段：theme、locale、iconSize、window、wallpaper、accent、mobileFeed。后四项只对桌面有意义。
- 资料：`users/{user}/profile` 是 private profile，逐字段设公开，`to_public_profile()` 是公开视图；`users/{user}/doc` 里是 DID 文档的公开资料。`user.profile.get` 把两者合并。
- `getCurrentIdentity` 已经给每个 App：user_id、did、user_type、show_name、zone_host。
- 网关：任何主机上的 `/kapi/control-panel` 都转给 Control Panel，App 用自己的会话就能调用；Control Panel 从会话的 target 知道调用方（`app:crm@bob`、`system:aiworkspace`）。
- system-config 的 RBAC 对 App 会话是 App 侧与用户侧的合取，App 侧只开放自己的 app 数据，所以 App 的会话读不到 `users/{user}/desktop/*`、`users/{user}/profile`（`user.profile.get` 用调用方的令牌读，App 调它会被 system-config 拒）。新方法用 Control Panel 自己的身份读、按调用方过滤：Control Panel 是唯一的闸。

## 3 裁决（11-27 owner）

- Q1：按现在的最佳实践分档；用户的全部 profile 只开放给系统服务（如 aiws）。
- Q2：应用在宿主 frame 里时用宿主给的值，单独标签页才问 Zone。
- Q3：不留 `?theme=`（上一轮为新标签页加的 URL 参数撤掉）。
- Q4：命名 `getUserPreferences` / `onUserPreferencesChanged`，kRPC `user.preferences.get`。

## 4 档位（我按最佳实践替 owner 定的，待过目）

参照：浏览器和手机系统把明暗、语言、强调色给所有应用，不要权限；OIDC 的 profile / email / phone 要用户同意；账号安全设置只有系统自己能看。

| 档 | 谁能读 | 字段 |
|---|---|---|
| basic | 本账号的任何用户会话：第三方 App、系统页面，Limited 用户也可以 | `theme`、`accent`、`locale`；`profile`：用户设为公开的资料（本来就在 DID 文档里，谁都能看） |
| declared | App 在 AppDoc 声明某个 scope，且安装时批准（做法同 `msg-center/notify`） | 第一期没有字段，不实现；出现敏感设置（如时区、位置）时加这一档 |
| system | 会话目标是系统服务（`system:*`：AI Workspace、Desktop 本身） | 以上全部，加 `private_profile`：完整资料，含未公开字段 |
| 不开放 | — | 桌面专用：iconSize、window、wallpaper、mobileFeed；账号安全：`users/{user}/settings`、登录绑定 |

外部 DID 访客、匿名访客：没有本 Zone 的账号，SDK 直接返回 `{}`，不调用。

## 5 接口

kRPC `control-panel` 的 `user.preferences.get {}`：

```
{ theme?: 'light' | 'dark', accent?: string | null, locale?: string,
  profile?: UserProfile, private_profile?: UserPrivateProfile }
```

没设置过或无权读的字段不出现；`accent: null` 表示用 Zone 默认色。只读，修改仍在 Settings。

SDK：

```ts
getUserPreferences(): Promise<UserPreferences>
onUserPreferencesChanged(cb: (prefs: UserPreferences) => void): () => void
```

- 在宿主 frame 里（`AppFrameClient` 收到 `frame.init` 之后）：theme / accent / locale 取宿主给的值，宿主推送变化时回调；资料仍问 Zone。
- 单独页面：问 Zone。有回调时，页面回到前台（`visibilitychange` / `focus`）重读，变了才回调。不加服务端推送（机制越少越好），代价是在别的标签页切了主题，回到这个页面才生效。
- 失败（网络、旧 Zone 没有这个方法）：返回 `{}`，调用方用自己的默认值。

## 6 实施

- P1 control_panel：`user_preferences.rs`、dispatch、`ACCOUNT_SELF_METHODS`；单测覆盖分档过滤。
- P2 websdk：`user_preferences.ts`，sdk_core / index / browser 导出，app-frame 接入；单测；build dist。
- P3 desktop：撤掉 `?theme=`（links / launch / PrivacyPage / aiws.tsx / e2e）；aiws.tsx 启动时读偏好（主题；没有 `?lang=` 时也用账号的语言），之后跟随变化。
- P4 DV：本机 Zone 部署 control-panel 与 desktop：从桌面在新标签页打开工作区、在桌面切主题后回到标签页、直接打开工作区链接。

## 7 实施记录

11-27 P1–P4 全部实施，未提交。

**落点**
- control_panel：`src/user_preferences.rs`（分档与过滤 `preferences_view`、`PreferenceTier::of`、handler）；`main.rs` dispatch；`sys_auth_backend.rs` 的 `ACCOUNT_SELF_METHODS`；`user_mgr.rs` 三个资料函数改为 `pub(crate)` 复用。
- websdk：`src/user_preferences.ts`（`readUserPreferences`、`UserPreferencesStore`、`setFrameAppearance`）；`sdk_core.ts` 的 `getUserPreferences` / `onUserPreferencesChanged`；`index.ts` / `browser.ts` / `node.ts` 导出；`app-frame.ts` 的 `AppFrameClient` 在 `frame.init` / `frame.themeChanged` / `frame.localeChanged` 时登记宿主的外观，`dispose` 时撤销。
- desktop：`aiws.tsx` 启动时读偏好（等最多 1.5 秒）作为首帧主题；`app/aiworkspace/standalone/FollowUserPreferences.tsx` 之后跟随变化；`theme/provider.tsx` 的 `initialMode`。`?theme=` 已撤（links / launch / PrivacyPage 还原，e2e 删掉对应用例）。
- 手册：`cmd_skills/buckyos_app_dev/references/sdk.md` 加一条。`buckyos_admin` 不涉及（只多一个只读方法，没有新的设置或诊断路径）。

**实施中我定的取舍（待过目）**
- 没有 `?lang=` 时，工作区标签页也用账号的语言；有 `?lang=`（从桌面打开）时仍以它为准。`?lang=` 保留。
- 只有账号的用户会话有答案；设备、服务、Agent 自身、外部 DID 访客得到 `{}`。
- 在 frame 里宿主没给 accent 时按 `null`（默认色）算，不回落到 Zone 的值。
- declared 档没有字段，没写代码。

**验证**
- `cargo test -p control_panel --bin control_panel -- user_preferences restricted_sessions_reach`：5 过。
- websdk：`npx jest tests/user_preferences.test.ts tests/app_frame_native.test.ts tests/app_frame_transfer.test.ts` 34 过；全量 1433 过、1 失败（`file_meta.test.ts` 的 `dk10000.pdf`：样本目录里多了一个 PDF，用例把目录里的文件都当图片，与本改动无关）。
- desktop：`tsc -b`、eslint 通过；`tests/e2e/pages/desktop-i18n.spec.ts` 14 过。
- 真实 Zone（test.buckyos.io，11-27 10:28 部署 control_panel 与 Desktop web，`start.py --skip-update`）：`tests/dv/user-preferences.spec.ts` 2 过。内容：系统会话拿到 `private_profile`（等于 `user.profile.get` 的 `local_profile`），Text Editor 的 App 会话拿到的是去掉 `private_profile` 的同一份，且它调 `user.profile.get` 被拒；工作区标签页首次打开即账号主题，切换后回到前台跟随，重新打开仍是账号主题。

**没做 / 没验**
- 第三方 App 还没有用上它（Text Editor 等）；只验了 kRPC 层的分档。
- BuckyOS 客户端容器、Windows / macOS 没测。
- 提交顺序：websdk 先推（Desktop 经 git main 依赖它），再提交 buckyos。
