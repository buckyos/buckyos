# AGENTS

## Scope

- Rust workspace root: `src/` (Cargo workspace; most crates live here)
- Control panel service (Rust + kRPC): `src/frame/control_panel/`
- Control panel UI (React/Vite/Tailwind): `src/frame/control_panel/web/`
- Control panel PRD + RPC spec: `doc/PRD/control_panel/control_panel.md`
- UI/UX skill rules: `src/frame/control_panel/SKILL.md` (treat as required)
- Dev proxy: `/kapi/control-panel` -> `http://127.0.0.1:3180` (see `src/frame/control_panel/web/vite.config.ts`)

## Control panel map
### Rust service
- Package manifest: `src/frame/control_panel/Cargo.toml` (package name `control_panel`)
- Entry point + RPC router: `src/frame/control_panel/src/main.rs`
- RPC handler type: `ControlPanelServer` implements `RPCHandler`
- Implemented endpoints: `ui.main`, `ui.layout`, `ui.dashboard`, `system.overview`, `system.metrics`, `sys_config.*`, `apps.list`
- Unimplemented endpoints return `Not implemented` via `handle_unimplemented`

### Web UI
- App entry: `src/frame/control_panel/web/src/main.tsx` and `src/frame/control_panel/web/src/App.tsx`
- Router: `src/frame/control_panel/web/src/routes/router.tsx`
- Layout: `src/frame/control_panel/web/src/ui/RootLayout.tsx`
- Pages: `src/frame/control_panel/web/src/ui/pages/*.tsx`
- API client + mocks: `src/frame/control_panel/web/src/api/index.ts`
- Global types: `src/frame/control_panel/web/src/interface.d.ts`
- Styling tokens + shared classes: `src/frame/control_panel/web/src/index.css`

## Build / lint / test
### Workspace / system
- Install devkit (required for buckyos-build/install): `python3 -m pip install -U "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"`
- Build all apps (CI uses this from `src/`): `cd src && buckyos-build`
- Build Rust only (skip web apps): `cd src && python3 build.py --no-build-web-apps`
- Install rootfs (full): `cd src && buckyos-install --all`
- Install rootfs (incremental): `cd src && buckyos-install`
- Start local rootfs: `cd src && python3 start.py`
- Cargo tests (CI when enabled): `cd src && cargo test -- --test-threads=1`

### Control panel service (Rust)
- Run: `cd src && cargo run -p control_panel`
- Build: `cd src && cargo build -p control_panel`
- Test all: `cd src && cargo test -p control_panel`
- Test single (name filter): `cd src && cargo test -p control_panel <test_name_substring>`
- Test single integration file: `cd src && cargo test -p control_panel --test <test_file_name>`
- Legacy docs mention `control_panel_service`; current crate name is `control_panel`

### Control panel UI (web)
- Install deps (pnpm lockfile present): `cd src/frame/control_panel/web && pnpm install`
- Dev server: `cd src/frame/control_panel/web && pnpm dev`
- Build: `cd src/frame/control_panel/web && pnpm build`
- Lint: `cd src/frame/control_panel/web && pnpm lint`
- Preview build: `cd src/frame/control_panel/web && pnpm preview`
- Tests: no test runner configured in `src/frame/control_panel/web/package.json`

## Rust style (control_panel service)
- Main entry is `src/frame/control_panel/src/main.rs`; keep related RPC logic together.
- RPC routing lives in `handle_rpc_call`; add new methods there and group by module.
- Prefer `ui.*` names for UI endpoints and keep legacy aliases (`main`, `layout`, `dashboard`) when touching routes.
- Response shape: `RPCResponse::new(RPCResult::Success(json!(...)), req.id)`.
- Errors: return `Result<RPCResponse, RPCErrors>`; use `RPCErrors::ReasonError` for user-visible errors.
- Params: `param_str` for optional values, `require_param_str` for required values.
- Auth: accept token from `req.token`, `params.session_token`, or env (see `get_session_token_env_key`).
- Sys config: use `SystemConfigClient::{get,set,list}`; map errors to `RPCErrors`.
- JSON: use `serde_json::json!` and `serde_json::Value` for dynamic payloads.
- Async: use `tokio` for sleeps/IO; avoid blocking calls inside handlers.
- Logging: use `log::info/warn/error`; avoid `println!`.
- Errors in core logic: prefer `anyhow::Result` for helpers and `thiserror::Error` for typed enums.
- Imports: follow the local file style; keep `std`/external/internal groups readable.
- Naming: snake_case functions/vars, PascalCase types, SCREAMING_SNAKE_CASE consts.
- Keep RPC responses aligned with `doc/PRD/control_panel/control_panel.md` field names.

## Web style (control_panel UI)
- Stack: React + TypeScript + Vite + Tailwind.
- Component naming: PascalCase files/components (e.g., `DashboardPage.tsx`).
- Pages live in `src/frame/control_panel/web/src/ui/pages/`.
- Imports: type-only imports first, then React/third-party, then `@/` absolute, then relative.
- Types: global UI types live in `src/frame/control_panel/web/src/interface.d.ts`.
- Aliases: `@` resolves to `src` (see `tsconfig.app.json` and `vite.config.ts`).
- Formatting: follow existing style (2-space indent, no semicolons, trailing commas).
- API layer: use `buckyos.kRPCClient` in `src/frame/control_panel/web/src/api/index.ts`.
- API return shape: `{ data, error }` and guard against invalid payloads.
- Error handling: warn and fall back to mock data when backend is unavailable.
- Mock data lives in `src/frame/control_panel/web/src/api/index.ts`; keep it in sync with types.
- Routing: add pages in `src/frame/control_panel/web/src/routes/router.tsx`.
- Nav items come from layout data; update `mockLayoutData` when adding nav.
- Styling: prefer Tailwind utilities + CSS variables from `src/frame/control_panel/web/src/index.css`.
- Reuse shared classes: `cp-panel`, `cp-card`, `cp-pill`, `cp-nav-link`, `cp-shell`.
- Colors: use `var(--cp-*)` tokens instead of hard-coded hex unless adding tokens.
- Icons: use `Icon` + `IconName` from `src/frame/control_panel/web/src/ui/icons.tsx`.
- Add new icons by updating `IconName` union and `icons` map; no emoji icons.
- Accessibility: keep focus states visible and respect `prefers-reduced-motion`.
- State: include loading/empty/error states on data-driven pages.

## UI/UX rules (from `src/frame/control_panel/SKILL.md`)
- Priority: accessibility, interaction, layout, typography/color, motion.
- Primary color: `#0f766e`.
- Accent color: `#f59e0b`.
- Fonts: Space Grotesk (headings), Work Sans (body).
- Neutrals: `#0f172a` ink, `#52606d` muted, `#d7e1df` border, `#f4f8f7` surface-muted, `#ffffff` surface.
- Radius scale: 8 / 12 / 18 / 24.
- Shadow scale: soft / strong (avoid heavy blur).
- Spacing scale: 4 / 8 / 12 / 16 / 24 / 32.
- Typography: sizes 12 / 14 / 16 / 20 / 24 / 32; body line-height 1.5, heading 1.2.
- Icon system: one SVG icon set only; sizes 16 / 20 / 24; no emoji icons.
- Layout rules: max width 1280, sidebar 260, card gap 16-24, page padding 24 desktop / 16 mobile.
- Interaction: touch targets >= 44x44px; no layout shift on hover.
- Data density: default medium; keep line length <= 75 chars where possible.
- States: loading skeleton/shimmer; empty state with explanation + next action; error with clear message + retry.
- Charts: primary then accent; avoid low-contrast pairs; light gridlines and muted labels.
- Motion: 150-300ms transitions; respect `prefers-reduced-motion`.
- Workflow: define a design system before building components.
- Quick checks: verify 375/768/1024/1440 widths and visible focus states.

## RPC / docs references
- RPC contract and endpoint list: `doc/PRD/control_panel/control_panel.md`.
- HTTP entry: POST `/kapi/control-panel` with kRPC body (`method`, `params`, `id`).
- Naming: `module.action` (UI uses `ui.*`); legacy aliases for `main/layout/dashboard`.
- Versioning: add fields backwards-compatibly; breaking changes use new method name.
- When adding RPC data: update backend handler, frontend types, and mock data.

## Cursor / Copilot rules
- None found in `.cursor/rules/`, `.cursorrules`, or `.github/copilot-instructions.md`.
