# buckyos 与app安装相关的页面

> Migration note:
> - Canonical control panel docs now live under `doc/control_panel/`.
> - Install-related product intent will be progressively merged into `doc/control_panel/README.context.md` and `doc/control_panel/SPEC.context.md` as `Planned` or `Implemented` sections.
> - This file is retained as historical PRD input during migration.

本文件现已降级为 historical stub，详细 `Planned` 规格已迁移到 `doc/control_panel/SPEC.context.md`。

## Canonical Destination

- 产品目标与体验方向：`doc/control_panel/README.context.md`
- 安装/分享安装/发布相关 planned spec：`doc/control_panel/SPEC.context.md`
- 约束与边界：`doc/control_panel/CONTEXT.context.md`

## Historical Summary

- control panel 曾被规划为统一承载外部网页拉起安装、桌面内安装、分享安装、扫码安装、内置商店、信任解释、支付、发布等完整 app 分发生命周期。
- 当前这些 surface 大多仍属于 `Planned`，不应与已实现的 Files 分享、主控制面板路由混淆。

## Preserved Historical Buckets

### 1) 第三方网页侧（Web / JS SDK）

> Canonical split:
> - 产品入口与目标迁移到 `doc/control_panel/README.context.md`
> - route/spec-level behavior迁移到 `doc/control_panel/SPEC.context.md`
> - 本节暂保留为 planned historical source

### A. “安装 $APP_NAME”按钮（任意第三方网页嵌入）

* 能检测本机是否安装并可唤起 `buckyos-service / buckyos-desktop`
* 检测失败时：引导去官方安装页（或开发者自定义引导页）
* 检测成功时：跳转/唤起 `cyfs://sys.$my_zonehostname/install.html?method=install_app&url=...&params...`
  cyfs://的打开依赖buckyos app的安装
  如果没装buckyos app,如何根据当前zone,跳转到 https://`sys.$my_zonehostname/install.html?method=install_app&url=...&params...`?
* 需要有**失败兜底 UI**：唤起失败、URL 拉取失败、版本不兼容等（给出可复制的 meta url / 文本）

### B. BuckyOS App安装引导页

> Planned: to be normalized into `doc/control_panel/SPEC.context.md` when install surface becomes canonical.

* 清晰判断用户平台（mac/windows/linux/mobile）并引导安装
* 安装完成后，支持“返回继续安装 app”（最好能继续带回原 app 的 meta url）

---

### 2) App Install.html（核心安装流程 UI）

> Migrated-to: future `Planned` install flow sections in `doc/control_panel/SPEC.context.md`, with product rationale in `doc/control_panel/README.context.md`.

核心历史意图：

- 安装流程应包含确认、可选高级配置、进度、成功、失败五个阶段。
- 安装确认页要同时解释 app 信息、来源、信任、权限与技术影响。
- 安装进度应基于 `task_id` 追踪，而不是黑盒 loading。
- 失败页要给出人类可理解的错误分类与下一步动作。

---

### 3) 分享安装相关 UI

> Canonical split:
> - app-install related sharing remains planned in `doc/control_panel/SPEC.context.md`
> - generic files/share behavior lives in `doc/control_panel/SPEC.context.md` and `doc/control_panel/ARCHITECTURE.context.md`

> 注:分享APP是分享一个自己已经安装的App，不是发布App。

核心历史意图：

- 已安装 app 可生成多种分享载体：链接、二维码、文本、文本二维码。
- `share_app.html` 曾被设想为安装分享的统一入口页。
- desktop 粘贴安装与 mobile 扫码安装，本质上都是“把分发载体重新导回同一安装流程”。

---

### 4) 内置应用商店 UI（未来需求）

> Historical/planned only. Out of current control_panel implemented scope.

核心历史意图：

- 内置商店不只是 app 列表，还包括 source 管理、用户自管理 meta 记录、信任等级与去重逻辑。
- 同一 app 的多个来源应可见，而不是被简单覆盖。

---

### 5) 信任机制相关 UI（系统面板）

> Planned system capability. Security/trust UI should later be normalized with auth and policy sections in canonical docs.

核心历史意图：

- 信任至少分为作者/联系人、应用源、分享来源三类。
- 安装弹窗应能解释“为什么信任/为什么风险”，而不是只给一个结论分数。

---

### 6) 经济/付费相关 UI（未来需求）

> Historical/planned only. Not part of current canonical implemented surface.

核心历史意图：

- 付费安装可能包含链上支付、传统支付、作者自定义支付流程。
- 安装成功证明与奖励信息曾被规划为分发经济系统的一部分。

---

### 7) 发布 App（未来需求)

> Historical/planned only. Keep as intent archive until a canonical publish surface is defined.

核心历史意图：

- control panel 长期被设想为 app 分发闭环的一部分，不仅安装，也包括分享、信任、支付、发布。
- 真正进入 canonical spec 时，应拆成 install/share-app/store/trust/payment/publish 多个 planned 子章节，而不是继续保存在一个旧 PRD 长文里。
