
# Control panel

## Quick start

Mock-first local UI loop:

```bash
cd src/frame/control_panel/web
pnpm install
pnpm dev:mock
```

This starts the control panel on `http://127.0.0.1:4020` with:

- mock auth enabled
- monitor data served from in-memory fixtures
- files `/api/*` served by an in-browser mock server
- no dependency on `127.0.0.1:3180`

Live backend mode:

```bash
cd src/frame/control_panel/web
pnpm install
pnpm dev:live
```

This keeps the existing Vite proxy to `127.0.0.1:3180`.

Mock smoke tests:

```bash
cd src/frame/control_panel/web
pnpm install
pnpm test:e2e:mock
```

Current smoke coverage:

- desktop monitor window
- standalone files route
- public share route

原型参考：
https://spray-jargon-85834573.figma.site/

https://hug-reach-51789548.figma.site







# Service
cargo run -p control_panel

## Files integration

- Files 页面属于 Control Panel Web 的内嵌模块（不是独立前端工程）。
- Files API 由 `control_panel` 服务统一提供（`/api/*`），并在后端转发到内嵌 file manager。
- 本地前端开发目录：`src/frame/control_panel/web`。
- Mock-first 开发模式下，`/api/*` 会被浏览器内 mock handler 接管，用于验证 `monitor + files` 主流程。
