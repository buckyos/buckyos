# BuckyOS Preview — Component & App

Implementation of `product/bucky_file/BuckyOS Preview App-Component PRD.md` (Draft v1.0, P0 scope).

| Piece | Location | PRD |
|---|---|---|
| Preview Component | `src/components/ContentPreview.tsx` | §10, §12 |
| Protocol types / Source Resolver / Session / Runtime Adapter | `src/components/preview/*.ts` | §11, §23.3, §23.7 |
| Providers (P0 Pipeline Provider = nfs_server) | `src/components/preview/nfspProvider.ts`, `mockProvider.ts` | §9, §23.1, §23.6 |
| Preview App (window, landing, settings) | `src/app/preview/` | §13 |
| Window policy (Smart / Single, cap, relevance) | `src/app/preview/windowPolicy.ts` (+ `tests/datamodel/preview-window-policy.test.ts`) | §13.4–§13.7 |
| Host entry point | `src/app/preview/launch.ts` → `openPreview()` | §13.2 |

## Embedding the component

```tsx
<ContentPreview
  source={{ kind: 'cyfs-path', path: 'cyfs:///home/photos/a.jpg' }}
  session={{ kind: 'container', container: { kind: 'cyfs-path', path: 'cyfs:///home/photos' }, current: source }}
  uiMode="auto"            // 'auto' | 'visible' | 'silent'
  onRequestExit={closeOverlay}
  onRequestOpenWith={(req) => …}
/>
```

- Sources: `cyfs-path`, `object-id`, plus the `blob` extension (host-provided bytes) — unknown kinds resolve to Unsupported.
- Session contexts: `single`, `container` (enumerated through the provider), `list` (explicit, stable), `provider` (P1 — one-shot `listItems()` today).
- Events: `onReady`, `onProgress`, `onItemChanged`, `onCapabilitiesChanged`, `onUiVisibilityChanged`, `onRequestExit`, `onRequestOpenWith`, `onActionInvoked`, `onError`.
- Imperative handle (`ref`): `next/previous/goTo`, `zoomIn/zoomOut/fitToView/actualSize/rotate`, `toggleInfo`, `retry`, `getCapabilities/getStatus/getItem`.
- Root data attributes for hosts/tests: `data-status`, `data-renderer`, `data-item-index`, `data-item-count`, `data-ui-visible`, `data-degraded`.

Shortcuts: `Esc` exit · `←/→` previous/next (media: seek ±5 s; use `PageUp/PageDown` or `[`/`]`) · `Home/End` · `Space` play/pause · `+ − 0 1` zoom/fit/100 % · `R` rotate · `I` info · `F` fullscreen · `Ctrl/⌘+F` find (text) · `Ctrl/⌘ + wheel` zoom · drag (left or right button) pan · double-click fit ⇄ 100 %.

## Resolution flow (§23.2)

`setSource` → session (cached per session key) → `provider.resolvePreviewSource` → bounded probe (Content-Type + 512 magic bytes) → `classifyMedia` → `decideDirect` against the Runtime profile (`mediaTypes.ts`).
Direct → built-in renderer; otherwise `provider.ensurePreviewWork` (idempotent) and `getPreviewWork` polling until `completed | failed`. Unsupported (no Direct, no Pipeline) and `failed` are distinct states. A decode failure of a Direct render falls back to the Pipeline once; if nothing can handle it the item is reported as corrupted (bytes confirmed the type) or unsupported.

Every async result is matched against the current load key — switching items invalidates late results and aborts the request; shared Pipeline work is never cancelled by one consumer.

PDF (P0): the original PDF always takes the Direct path through `PDFIframeRenderer` (Runtime built-in viewer, no `sandbox` because Chromium disables its PDF plugin in sandboxed frames). If the Runtime cannot embed PDF, the pre-check or the load timeout fails, the renderer shows the degraded state with Download / Open with.

HTML runs in `sandbox=""` with an injected CSP (`default-src 'none'`), so scripts, network and navigation are blocked.

## Providers

- **NFSP (`nfspProvider.ts`)** — Source Resolver = `resolve` (`base/ident/access`), containers = `list`, read refs = `/nfs/v1/read/{node_id}` (plus `?download=1` for exports). The Pipeline maps onto `repr` / `get_repr` and is gated on the hello feature list: nfs_server does not advertise `repr` yet, so every non-Direct format is planned as **Unsupported** until the server ships it. The wire shape in that file is provisional (§23.8 item 2) and isolated there.
- **Mock (`mockProvider.ts`)** — the File Browser mock library plus `cyfs:///samples` with generated content for every result family, `/home/Private` → permission denied, and a simulated built-in Pipeline catalog (`office-html`, `sheet-html` with a failing first attempt, `media-transmux` via MediaRecorder, `raster-decode`) with idempotent `ensure`, attempts, retry CAS and negative caching.

The provider is chosen like the File Browser backend (`VITE_CP_USE_MOCK`, `?fbData=mock|nfsp`).

## Preview App

`openPreview({ source, session, origin, newWindow })` schedules a desktop window:

- `newWindow: true` → always a fresh manual window, never auto-reused, not counted against the cap.
- Smart mode → relevance (same session > same container > parent/child > same host > same media type > recency); strong matches reuse (jump / append / replace), otherwise create until the automatic cap (default 8), then reuse the most relevant unpinned automatic window.
- Single mode → the oldest automatic window is the main window.
- Pinned windows are protected from unrelated requests.

Windows carry a `WindowLaunch` (`requestId` + payload) through the desktop store (`openAppWindow` / `updateWindow`); the panel reacts to new request ids and updates the window title on item change.

Settings (`settings.ts`, localStorage): window mode, cap, default UI mode, default image layout, wrap in folders / selections, reopen last session, prefetch, prefer Full App.

File Browser hands over a Container Context for folder listings and an explicit list for views / collections / search / multi-selection (`FileBrowserView.tsx` → `handleOpenFile`, menu commands `open`, `preview-new-window`, `preview-selection`).

## Verification

```bash
pnpm run check && pnpm run lint
node --experimental-strip-types tests/datamodel/preview-window-policy.test.ts
npx playwright test tests/e2e/pages/preview.spec.ts
```

## Known gaps (tracked against the PRD)

- `repr` Pipeline on nfs_server is not implemented server-side; the client contract is provisional.
- "Open with…" offers Download / Copy reference only — the Full App association protocol is pending (§23.8 item 10).
- Provider-based sessions are one-shot; no Session Navigation Stack (P1).
- Fullscreen uses the component's own element (`allowFullscreen`); mobile sheets are unchanged (File Browser keeps its metadata sheet on mobile).
