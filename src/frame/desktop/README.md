# BuckyOS Web Desktop

`src/frame/desktop` is the Web Desktop shell for BuckyOS. It is a React + TypeScript + Vite frontend that hosts the system panel, windowing experience, mobile app sheets, built-in app routes, and SDK-backed system apps.

This module is the desktop runtime and app container. It should stay focused on shell behavior and app integration boundaries, not on the business logic of each default app.

## Scope

In scope:

- Desktop home screen and launcher.
- Desktop window layer, window geometry, mobile window sheet, and standalone app title bar.
- Status bar, system sidebar, background, widgets, and shell-level state.
- Built-in app registration and app container integration.
- Mock/runtime switching for local development and automated UI tests.
- SDK bridge usage for system apps.

Out of scope:

- Backend service logic.
- TaskManager, Workflow, AICC, or file service business rules.
- Default app-specific domain logic except where it affects shell/app integration.

## Key Paths

- `src/App.tsx`
  Top-level providers and browser routes.

- `src/desktop/DesktopRoute.tsx`
  Main desktop shell route.

- `src/desktop/windows/*`
  Desktop and mobile window primitives.

- `src/desktop/shell.ts`
  Shell-level status bar sizing, connection labels, and shared shell helpers.

- `src/app/registry.tsx`
  Built-in app definitions and app panel mapping.

- `src/models/DesktopUIDataModel.ts`
  Unified desktop UI data model. It separates syncable settings/layout data from local runtime state.

- `src/runtime.ts`
  Runtime mode helpers, including `VITE_CP_USE_MOCK`.

## Built-In App Integration

Desktop supports two main app presentation modes:

- Window/panel mode through the app registry and window layer.
- Standalone route mode for apps that need a full-page route, such as `/taskcenter`.

App-specific code should live under `src/app/<app-name>`. The shell should only know:

- app id
- display metadata
- icon metadata
- route or panel loader
- window/mobile presentation preferences

App-specific state machines and backend data adaptation should remain inside the app module or its API adapter.

## TaskCenter Relationship

TaskCenter lives at:

- `src/app/task-center`

It is a built-in app hosted by Desktop. It is not a Desktop framework module and should not define shell behavior. Its backend data adapter lives at:

- `src/api/task_mgr.ts`

TaskCenter uses `buckyos.getTaskManagerClient()` from `buckyos-websdk` to read and update task state.

## Runtime Modes

Mock mode is controlled by:

```text
VITE_CP_USE_MOCK=1
```

`src/runtime.ts` treats `1`, `true`, `yes`, and `mock` as enabled mock mode values.

Mock mode is intended for:

- frontend development
- Playwright tests
- UI layout and navigation checks

Real runtime mode should use SDK-backed APIs and should not silently depend on mock data.

## Development

From this directory:

```bash
pnpm install
pnpm dev
```

Default dev server port:

```text
5174
```

Useful scripts:

```bash
pnpm build
pnpm check
pnpm lint
pnpm test:e2e
```

## Verification Expectations

Desktop verification should be automated. Do not rely on manual clicking as an acceptance method.

Preferred test layers:

1. TypeScript checks and build for compile-time integration.
2. Playwright for desktop shell, mobile sheet, standalone app routes, and app panel behavior.
3. SDK/DV tests only when validating cross-service behavior beyond the Desktop shell.

Important UI scenarios:

- Desktop home loads with no console errors.
- Built-in app can open as a desktop window.
- Built-in app can open in mobile sheet mode.
- Standalone app route renders without the desktop shell when expected.
- TaskCenter can render as both app panel and `/taskcenter` route.
- Mobile title bar back/minimize behavior is deterministic.

## Boundary Rules

- Keep shell state separate from app state.
- Keep mock data isolated from SDK-backed runtime code.
- Keep app-specific business semantics out of Desktop shell files.
- Prefer app-local adapters for backend-specific data normalization.
- Add Playwright coverage when changing shell navigation, windowing, mobile presentation, or app registry behavior.
