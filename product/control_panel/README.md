# BuckyOS Control Panel

> Migration note:
> - Canonical docs now live under `doc/control_panel/`.
> - Read `doc/control_panel/README.context.md` first.
> - This file is retained as historical PRD input during migration.

这是 control panel 早期入口说明，现已降级为 historical stub。

## Canonical Entry

- 入口文档：`doc/control_panel/README.context.md`
- 运行结构：`doc/control_panel/ARCHITECTURE.context.md`
- 规格定义：`doc/control_panel/SPEC.context.md`
- 约定与迁移：`doc/control_panel/CONTEXT.context.md`

## Legacy Doc Status

| File | Current role | Notes |
| --- | --- | --- |
| `product/control_panel/README.md` | historical stub | old entry and route-name memo |
| `product/control_panel/app安装UI.md` | historical stub | old install/share/store/payment/publish intent buckets |
| `product/control_panel/control_panel.md` | main historical source | still retains the largest RPC/interface planning archive |

## How To Read Legacy Docs

- Start from `doc/control_panel/README.context.md` for canonical meaning.
- Use `product/control_panel/control_panel.md` only when you need historical RPC/planning context that has not yet been fully normalized.
- Treat all route names and flows in this directory as historical unless they are also present in `doc/control_panel/SPEC.context.md`.

## Historical Notes Kept Here

- `control_panel` 是系统控制面板服务，历史上使用 `sys` 短域名。
- 控制面板长期被设想为统一承载 dashboard、SSO、安装、发布等系统入口。
- app store 从一开始就倾向于独立成单独系统服务，而不是完全塞进 control panel。

## Historical Route Names

- `index.html`
- `/sso/login`
- `/login_index.html`
- `/install.html`
- `/share_app.html`
- `/ndn/publish.html`
- `/my_content.html?content_id=...`

这些名字只保留为历史入口名或规划名，当前是否已实现请以 `doc/control_panel/SPEC.context.md` 为准。
