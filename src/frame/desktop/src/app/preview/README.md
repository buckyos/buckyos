# BuckyOS Preview — Component & App

Implementation of `product/bucky_file/BuckyOS Preview App-Component PRD.md` (Draft v1.0, P0 scope).

| Piece | Location | PRD |
|---|---|---|
| Preview Component | `src/components/ContentPreview.tsx` | §10, §12 |
| Protocol types / Source Resolver / Session / Runtime Adapter | `src/components/preview/*.ts` | §11, §23.3, §23.7 |
| Providers (P0 Pipeline Provider = nfs_server) | `src/components/preview/nfspProvider.ts`, `mockProvider.ts` | §9, §23.1, §23.6 |
| PDF by pdf.js, sheet renderer | `src/components/preview/PdfJsRenderer.tsx`, `pdfjs.ts`, `SheetRenderer.tsx`, `sheet.worker.ts`, `pinchZoom.ts` | §10.7, §23.3.1 |
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

- Sources: `cyfs-path`, `object-id`, plus the `blob` extension (host-provided bytes) — unknown kinds resolve to Unsupported unless a host registered a Source Resolver for them (see below).
- Session contexts: `single`, `container` (enumerated through the provider), `list` (explicit, stable), `provider` (P1 — one-shot `listItems()` today).
- Hosting: `variant="inline"` for a small surface inside another UI (an AI Workspace media Block) — no toolbar, edge navigation or info panel; media controls and the PDF viewer stay, a sound gets a compact stage (title + controls). `autoPlay` starts audio / video once ready (the host acts on an explicit "play").
- Events: `onReady`, `onProgress`, `onItemChanged`, `onCapabilitiesChanged`, `onUiVisibilityChanged`, `onRequestExit`, `onRequestOpenWith`, `onActionInvoked`, `onError`.
- Imperative handle (`ref`): `next/previous/goTo`, `zoomIn/zoomOut/fitToView/actualSize/rotate`, `toggleInfo`, `retry`, `getCapabilities/getStatus/getItem`. `zoomIn/zoomOut/fitToView` work for images, text, sheets and PDFs (pdf.js: pages are drawn again at the new size; the built-in viewer is reloaded with `#zoom=`).
- Root data attributes for hosts/tests: `data-status`, `data-renderer`, `data-item-index`, `data-item-count`, `data-ui-visible`, `data-degraded`.

Shortcuts: `Esc` exit · `←/→` previous/next (media: seek ±5 s; use `PageUp/PageDown` or `[`/`]`) · `Home/End` · `Space` play/pause · `+ − 0 1` zoom/fit/100 % · `R` rotate · `I` info · `F` fullscreen · `Ctrl/⌘+F` find (text) · `Ctrl/⌘ + wheel` zoom · drag (left or right button) pan · double-click fit ⇄ 100 %.

## Host-resolved sources (§10.2, §11.5)

A host that owns content the system provider cannot address registers a resolver for its own source kind (`components/preview/hostSources.ts` → `registerPreviewSourceResolver(kind, resolver)`). `ensurePreviewProvider()` routes resolution of that kind to the host; Pipeline work stays with the system provider, keyed by `inputObjectId`. An extension ref's `value.name` / `value.reference` (strings) are used as display name and copyable reference. MessageHub registers `messagehub-object` (msg-center attachments, trusted external images); AI Workspace registers `aiworkspace-asset` (workspace assets read with the workspace session into a blob; the source value carries its reader function, which is not part of the source identity).

## Thumbnails (§9.3 `purpose: thumbnail`, §17.2)

`components/preview/thumbnail.ts` → `ensurePreviewThumbnail(source, { width, height })` / `usePreviewThumbnail()`:
resolve → bounded probe → classify; a Runtime-decodable image is its own thumbnail; otherwise the Pipeline is asked with `purpose: 'thumbnail'` and an image-only target; when no Pipeline can produce one, a video the Runtime plays gets a frame captured in the browser (at most 2 captures at a time). Results are shared per source and size bucket; shared work stops only when its last consumer leaves. The mock catalog plans posters for `mp4/mov/mkv/avi` and raster thumbnails for `heic/tiff/psd`; WebM is left to the frame capture.

## Resolution flow (§23.2)

`setSource` → session (cached per session key) → `provider.resolvePreviewSource` → bounded probe (Content-Type + 512 magic bytes) → `classifyMedia` → `decideDirect` against the Runtime profile (`mediaTypes.ts`).
Direct → built-in renderer; otherwise `provider.ensurePreviewWork` (idempotent) and `getPreviewWork` polling until `completed | failed`. Unsupported (no Direct, no Pipeline) and `failed` are distinct states. A decode failure of a Direct render falls back to the Pipeline once; if nothing can handle it the item is reported as corrupted (bytes confirmed the type) or unsupported.

Every async result is matched against the current load key — switching items invalidates late results and aborts the request; shared Pipeline work is never cancelled by one consumer.

PDF: the original PDF and the PDF a Pipeline converts a document to are shown the same way. On a desktop with a built-in viewer (`navigator.pdfViewerEnabled`) that is `PDFIframeRenderer` (no `sandbox`, because Chromium disables its PDF plugin in sandboxed frames). Where there is no viewer (Android Chrome) or the pointer is coarse (`matchMedia('(pointer: coarse)')`: phones and tablets), `PdfJsRenderer` draws it: `mediaTypes.ts` → `pdfUsesPdfjs()`. The iframe hands over to pdf.js as well when its pre-check or its load timeout fails.

- pdf.js is loaded on demand through `components/preview/pdfjs.ts` (shared with the AI Workspace importer); the worker is a build asset, CMaps / standard fonts / decoders are served under `aiworkspace/pdfjs/` of the Zone root, so `/embed/preview` gets them too.
- A URL read reference is opened by HTTP Range (`rangeChunkSize` 256 KiB, `disableAutoFetch`): `/nfs/v1/read/` and the bucket data plane answer single ranges with 206. Blob sources (object ids, host memory) are handed over whole.
- Pages scroll in one column; only the pages in view and one before / after hold a canvas, capped at 16 M pixels (iOS). Two fingers zoom (a CSS transform while they move, a redraw on release); the chip at the bottom shows the page and jumps to a typed one. A password-protected PDF shows a notice with Download / Open with.

Sheets: xlsx / xlsm / csv / tsv are read from the original bytes by `SheetRenderer` — never through the Pipeline. The workbook is parsed in a Worker with the table importer's own readers (`@table/xlsx.ts`, `@table/csv.ts`: shared strings, number formats, dates, merged ranges), with a cap on what the zip may inflate to (512 MiB). xls / et / ods / numbers / xltx go to the Pipeline (`preview-sheet`), whose result is an xlsx shown by the same renderer. Limits: 30 MiB per file (`DIRECT_SIZE_BUDGET.sheet`), 5 000 rows per sheet (`SHEET_ROW_LIMIT`) and 256 columns, with a notice when something is left out. Rows and columns are virtualized, column letters and row numbers stay in view, hidden sheets get no tab, csv / tsv can be switched to plain text, an encrypted workbook shows a notice.

HTML runs in `sandbox=""` with an injected CSP (`default-src 'none'`), so scripts, network and navigation are blocked.

## Providers

- **NFSP (`nfspProvider.ts`)** — Source Resolver = `resolve` (`base/ident/access`), containers = `list`, read refs = `/nfs/v1/read/{node_id}` (plus `?download=1` for exports). The Pipeline maps onto `repr` / `get_repr` (wire frozen with nfs_server `src/preview/rpc.rs` and websdk `NfspClient.repr`), gated on the hello feature list and on `hello.repr.profiles`: a format whose profile the node does not advertise plans as **Unsupported** with the node's reason. Result families (`result_type` → renderer): `image`, `video`, `audio`, `document` (a PDF → `pdf`), `sheet` (an xlsx → `sheet`), `html` (a self-contained page → `html`). Which profile a document asks for is in `mediaTypes.ts` (`PIPELINE_DOCUMENTS`): Office / ODF / WPS / iWork / Visio / Publisher text and slides → `preview-document`; xls / et / ods / numbers → `preview-sheet`; epub / ipynb / Markdown → `preview-html` (pandoc; rst, org, DocBook, FictionBook, LaTeX and Typst stay out because pandoc does not render them whole, see `notepads/preview-more-formats-todo.md` §7). The text ones of the last group (ipynb, md) are shown by the text renderer when the node has no Pipeline for them. Sources go as `at` (NFSP ref), a bucket binding (`bucket-object`, or a host resolver's `providerHandle`) or an object id; results are read from the `preview-cache` bucket URL the node signs.
- **Object IDs (`ndmSource.ts`)** — `cyfile`, chunk and chunk-list IDs use the authenticated NDM store/read endpoints; they do not use NFSP's unsupported object-ref resolution. File metadata supplies the name and MIME hint, and the server resolves single-chunk or chunk-list content. Object previews load the complete content into a Blob before rendering, with cancellation and a 256 MiB limit checked against metadata, response headers and received bytes. These previews currently have no HTTP Range streaming; CYFS path previews retain their existing streaming URLs. The SDK's structured store/proxy helpers reject Browser runtime, so this adapter calls the existing gateway endpoints with the active session token.
- **Mock (`mockProvider.ts`)** — the File Browser mock library plus `cyfs:///samples` with generated content for every result family, `/home/Private` → permission denied, and a simulated built-in Pipeline catalog (`office-html`, `sheet-xlsx` with a failing first attempt for xls, `media-transmux` via MediaRecorder, `raster-decode`) with idempotent `ensure`, attempts, retry CAS and negative caching.

The provider is chosen like the File Browser backend (`VITE_CP_USE_MOCK`, `?fbData=mock|nfsp`).

Completed AICC tasks expose their named-object artifacts in Task Center's Results section. Each Preview action opens this component inside the task detail page, including the standalone `/taskcenter?taskid=…` route; content loads only after the action, and closing the preview releases it.

## Preview App

`openPreview({ source, session, origin, newWindow })` schedules a desktop window:

- `newWindow: true` → always a fresh manual window, never auto-reused, not counted against the cap.
- Smart mode → relevance (same session > same container > parent/child > same host > same media type > recency); strong matches reuse (jump / append / replace), otherwise create until the automatic cap (default 8), then reuse the most relevant unpinned automatic window.
- Single mode → the oldest automatic window is the main window.
- Pinned windows are protected from unrelated requests.

Windows carry a `WindowLaunch` (`requestId` + payload) through the desktop store (`openAppWindow` / `updateWindow`); the panel reacts to new request ids and updates the window title on item change.

Settings (`settings.ts`, localStorage): window mode, cap, default UI mode, default image layout, wrap in folders / selections, reopen last session, prefetch, prefer Full App.

File Browser routes file open actions through the shared Content Open Router; when Preview is selected it receives a Container Context for folder listings and an explicit list for views / collections / search / multi-selection (`FileBrowserView.tsx` → `handleOpenFile`, menu commands `open`, `preview-new-window`, `preview-selection`). On mobile, tapping a file follows the same default Handler resolution. Explicit multi-selection preview continues to open Preview. `origin.windowId` identifies the Files window; Preview exit and the mobile shell Back action return focus to it. Files retains tab/location/search/scroll snapshots in memory across that transition. Metadata remains a separate Files details action. The built-in Preview manifest uses the standard mobile shell bar, reserving its height so the preview toolbar/content stay below it; the shell title follows the current file.

## Embedding in an App (`/embed/preview`)

Apps on their own origin show this component inside their UI through `<zone root>/embed/preview` (`embed/PreviewEmbedApp.tsx`; `main.tsx` renders only the component there — no shell, router or wallpaper). The wire protocol is `buckyos.preview-embed` v1 in websdk `buckyos/app-frame`: the App uses `PreviewEmbed` (it owns the iframe and sends `embed.init { source | blob, session?, theme, locale }`), the page uses `PreviewEmbedHost` and accepts only an embedder inside the Zone (`originInZone`; any `*.localhost` / `127.0.0.1` in the mock runtime). The page reads content with its own Zone-root session: no token crosses origins. Theme and language are fixed by the App and not written back to the desktop's stored preferences (`PrototypeThemeProvider mode`, `I18nProvider locale`). The page reports `embed.state` (`loading` / `ready` / `error` with the Preview error kind) and forwards Open with… as `embed.openWith`, which the App hands to the shell (`content.open` target `chooser`). A draft (blob) source offers no Open with.

## Open with… (system chooser)

`app/content/OpenWithSheet.tsx` is the one chooser: the Preview App renders it for its own content; `OpenWithHost` (mounted by `DesktopRoute`) renders it for App requests (`content.open` target `chooser`, from `WebAppFramePanel`) and for the system entry `/?open=…&intent=chooser[&name&mime&size&from=<AppId>]` (`parseSystemOpen`). Handlers of the requesting App are left out (`handlerOfApp`, by AppId — no App-specific code); Preview is offered for requests from outside Preview. `openContent(…, { exclude })` keeps that exclusion when a chosen handler turns out unavailable. Cancelling leaves the requesting App untouched.

## Verification

```bash
pnpm run check && pnpm run lint
node --experimental-strip-types tests/datamodel/preview-window-policy.test.ts
node --experimental-strip-types tests/datamodel/preview-media-types.test.ts
deno test --no-check --allow-env --allow-read --allow-sys --node-modules-dir=manual --unstable-sloppy-imports tests/datamodel/preview-ndm-source.test.ts
npx playwright test tests/e2e/pages/preview.spec.ts tests/e2e/pages/preview-formats.spec.ts tests/e2e/pages/preview-embed.spec.ts tests/e2e/pages/content-open.spec.ts
```

## Known gaps (tracked against the PRD)

- "Open with…" resolves installed App Handlers through `buckyos/content`, filtered by `apps.list` visibility and user defaults. Download / Copy reference remain available. Manifest projection, App Frame hosting, the system chooser for Apps and default app selection are implemented; third-party preview handlers, sandbox policy and the system default-app settings page remain later work (§23.8 item 10).
- Provider-based sessions are one-shot; no Session Navigation Stack (P1).
- Without ffmpeg on the node there is no `repr`: real-mode thumbnails come only from Runtime frame capture (videos the browser plays) and natively decodable images.
- Fullscreen uses the component's own element (`allowFullscreen`).
- pdf.js renderer: no text layer (no selection or find), no links, no rotation. The phone tests emulate Pixel / iPhone in Chromium; Android Chrome and iOS Safari on real devices are not verified yet (preview-more-formats TODO A4).
- Sheet renderer: values and merged ranges only — no cell styles, column widths of the file, charts or images; csv encodings follow the importer (UTF-8, GB 18030, UTF-16).
- Markdown is still shown as plain text (TODO B0 decides the renderer).
