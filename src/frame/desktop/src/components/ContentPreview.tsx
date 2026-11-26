/**
 * ContentPreview — the BuckyOS Preview Component (PRD §10, §12).
 *
 * A host gives it a display area, a Source, an optional Session Context and a
 * UI Mode; the component owns everything inside that area: source
 * resolution, the Direct / Pipeline decision, the standard renderers, the
 * content-first toolbar, gestures, shortcuts, the info panel and every error
 * state. It never creates windows or decides where it is shown (§7.1).
 *
 * Data flow (§23.2):
 *   setSource → Session Context → Source Resolver → classify (bounded probe)
 *   → Direct?  yes → Built-in Renderer
 *              no  → ensurePreviewWork (nfs_server Pipeline) → poll → Renderer
 *   Any late result is dropped unless it matches the current request key.
 */

import { Menu, MenuItem } from '@mui/material'
import './preview/files.css'
import clsx from 'clsx'
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  ExternalLink,
  FileQuestion,
  Files,
  FileX,
  Fullscreen,
  GripVertical,
  Info,
  Loader2,
  Lock,
  Maximize2,
  Minimize2,
  MoreHorizontal,
  Music,
  Pin,
  PinOff,
  RefreshCw,
  RotateCw,
  Search,
  Share2,
  X,
  ZoomIn,
  ZoomOut,
} from 'lucide-react'
import {
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type Ref,
} from 'react'
import { contentRefsRepresentations } from 'buckyos/transfer'
import { useI18n } from '../i18n/provider'
import { copyWithCommand, writeTransferEvent } from '../transfer'
import { readAll, readProbe, readText } from './preview/io'
import { zoneContentOfPreviewSource } from './preview/transferSource'
import {
  classifyMedia,
  contentLabelOf,
  decideDirect,
  detectRuntimeProfile,
  ensureRuntimeProfile,
  pdfUsesPdfjs,
  TEXT_READ_BUDGET,
  type MediaClassification,
} from './preview/mediaTypes'
import { PdfJsRenderer } from './preview/PdfJsRenderer'
import type { ZoomHandle } from './preview/pinchZoom'
import { ensurePreviewProvider } from './preview/provider'
import { archiveTargetOf, type ArchiveTarget } from './preview/archive'
import { ArchivePreview } from './preview/ArchiveBrowser'
import { archiveSplit, isArchiveName } from '../app/filebrowser/data/urls'
import { SheetRenderer } from './preview/SheetRenderer'
import { DiagramRenderer } from './preview/DiagramRenderer'
import {
  locateIndex,
  neighborIndex,
  refDisplayName,
  refIdentity,
  resolveSessionContext,
  sessionKeyOf,
  type ResolvedSession,
  type SessionItem,
} from './preview/session'
import {
  extensionRefField,
  isAbortError,
  isBlobRef,
  isBucketObjectRef,
  isCyfsPathRef,
  isObjectIdRef,
  PreviewError,
  type ContentRef,
  type PreviewCapabilities,
  type PreviewErrorState,
  type PreviewFitMode,
  type PreviewHostAction,
  type PreviewItemInfo,
  type PreviewOpenWithRequest,
  type PreviewProgress,
  type PreviewReadRef,
  type PreviewReadyInfo,
  type PreviewResult,
  type PreviewRuntimeProfile,
  type PreviewSessionContext,
  type PreviewShareRequest,
  type PreviewStatus,
  type PreviewUIMode,
  type PreviewUnsupportedReason,
  type PreviewWorkState,
  type ResolvedPreviewSource,
} from './preview/types'

export type {
  ContentRef,
  PreviewCapabilities,
  PreviewErrorState,
  PreviewFitMode,
  PreviewHostAction,
  PreviewItemInfo,
  PreviewOpenWithRequest,
  PreviewProgress,
  PreviewReadyInfo,
  PreviewSessionContext,
  PreviewShareRequest,
  PreviewStatus,
  PreviewUIMode,
} from './preview/types'

// ─── Public surface ───

export interface ContentPreviewProps {
  source: ContentRef
  session?: PreviewSessionContext
  /** Auto (default): content only until the user shows intent. The user can pin the toolbar (`visible`) or let it
   * hide again (`auto`); `silent` is the host's and offers no toolbar. */
  uiMode?: PreviewUIMode
  /** The user pinned the toolbar or let it hide again: the host may keep the choice. */
  onUiModeChange?: (mode: 'auto' | 'visible') => void
  /** The host already names the item and has its own close (a window title bar): the toolbar shows neither. */
  titleInHost?: boolean
  fitMode?: PreviewFitMode
  hostActions?: PreviewHostAction[]
  /** Passed through to the Source Resolver; never widens permissions (§16). */
  permissionsContext?: unknown
  /** Host capability: fullscreen on the component's own element. */
  allowFullscreen?: boolean
  /** Host policy: copy / download of the current content (§10.7). */
  allowExport?: boolean
  /** Prefetch ±1 neighbours once the current item is ready (§11.7). */
  prefetchAdjacent?: boolean
  autoFocus?: boolean
  /** `inline`: hosted in a small surface of another UI (a canvas Block) — no toolbar, edge navigation or info panel;
   * the renderer's own controls (media controls, PDF viewer) stay and a sound gets a compact stage. */
  variant?: 'default' | 'inline'
  /** Audio and video start playing once ready (the host acts on an explicit "play"). */
  autoPlay?: boolean
  className?: string
  style?: CSSProperties
  'data-testid'?: string
  ref?: Ref<ContentPreviewHandle>
  onReady?: (info: PreviewReadyInfo) => void
  onProgress?: (progress: PreviewProgress) => void
  onItemChanged?: (item: PreviewItemInfo) => void
  onCapabilitiesChanged?: (capabilities: PreviewCapabilities) => void
  onUiVisibilityChanged?: (visible: boolean) => void
  onRequestExit?: () => void
  onRequestOpenWith?: (request: PreviewOpenWithRequest) => void
  /** Share the original (the system share dialog): a Share action is offered when the host gives this. */
  onRequestShare?: (request: PreviewShareRequest) => void
  onActionInvoked?: (action: { id: string; item: PreviewItemInfo | null }) => void
  onError?: (error: PreviewErrorState, item: PreviewItemInfo | null) => void
}

export interface ContentPreviewHandle {
  next(): boolean
  previous(): boolean
  goTo(index: number): boolean
  zoomIn(): void
  zoomOut(): void
  fitToView(): void
  actualSize(): void
  rotate(): void
  toggleInfo(): void
  retry(): void
  focus(): void
  getCapabilities(): PreviewCapabilities
  getStatus(): PreviewStatus
  getItem(): PreviewItemInfo | null
}

// ─── Small utilities ───

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
const MOD_KEY = IS_MAC ? '⌘' : 'Ctrl'
const AUTO_HIDE_MS = 2600
/** Below this width the toolbar keeps its main actions and moves the rest into More. */
const COMPACT_TOOLBAR_PX = 640
/** Below this width the zoom buttons go too (a phone pinches). */
const TINY_TOOLBAR_PX = 420
/** The band the toolbar takes at the top of a renderer with controls of its own up there (the browser's PDF viewer). */
const TOOLBAR_BAND_PX = 52
const LOADING_DELAY_MS = 260
const PIPELINE_TOTAL_TIMEOUT_MS = 90_000
const PDF_LOAD_TIMEOUT_MS = 15_000
const PROBE_BYTES = 512
const ZOOM_STEP = 1.25
const MIN_SCALE = 0.05
const MAX_SCALE = 32

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max)
}

function formatBytes(bytes: number | undefined): string {
  if (bytes === undefined || Number.isNaN(bytes)) return '—'
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`
}

function formatDuration(seconds: number | undefined): string {
  if (seconds === undefined || !Number.isFinite(seconds)) return '—'
  const total = Math.round(seconds)
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`
}

function sourceLabel(ref: ContentRef): string {
  if (isCyfsPathRef(ref)) return ref.path.startsWith('cyfs://') ? ref.path : `cyfs://${ref.path.startsWith('/') ? '' : '/'}${ref.path}`
  if (isObjectIdRef(ref)) return ref.objectId
  if (isBucketObjectRef(ref)) return ref.url
  if (isBlobRef(ref)) return ref.value.name ?? 'blob'
  return extensionRefField(ref, 'reference') ?? ref.kind
}

function errorStateFrom(err: unknown, contentLabel?: string): PreviewErrorState {
  if (err instanceof PreviewError) {
    const kind: PreviewErrorState['kind'] =
      err.code === 'PERMISSION_DENIED'
        ? 'permission-denied'
        : err.code === 'CORRUPTED'
          ? 'corrupted'
          : err.code === 'NOT_FOUND'
            ? 'not-found'
            : err.code === 'CANCELLED'
              ? 'cancelled'
              : err.code === 'TIMEOUT'
                ? 'timeout'
                : err.code === 'UNSUPPORTED' || err.code === 'TOO_LARGE'
                  ? 'unsupported'
                  : 'error'
    // A server code the host acts on (an archive password, a changed archive) stays visible to it.
    const code = err.detail === 'PASSWORD_REQUIRED' || err.detail === 'PASSWORD_INVALID' || err.detail === 'STALE' ? err.detail : err.code
    return { kind, code, message: err.message, retryable: err.retryable, contentLabel }
  }
  if (err instanceof DOMException && err.name === 'AbortError') {
    return { kind: 'cancelled', code: 'CANCELLED', message: 'Cancelled', retryable: false, contentLabel }
  }
  return { kind: 'error', code: 'INTERNAL', message: err instanceof Error ? err.message : String(err), retryable: true, contentLabel }
}

function unsupportedState(reason: PreviewUnsupportedReason, contentLabel: string, detail?: string): PreviewErrorState {
  return { kind: 'unsupported', code: `UNSUPPORTED_${reason.toUpperCase().replace(/-/g, '_')}`, message: detail ?? '', retryable: false, contentLabel }
}

/**
 * A renderer could not show the bytes and no Pipeline result replaced them.
 * `<img onError>` cannot tell an unknown format from damage, so an image only
 * counts as damaged when this Runtime decodes its (byte-confirmed) type
 * natively; media errors tell the two apart themselves. An image format the
 * Runtime lacks is never reported as corrupted (preview iPhone media TODO §3).
 */
function renderFailureState(
  failure: RenderFailure | undefined,
  classification: MediaClassification,
  runtime: PreviewRuntimeProfile,
  t: TranslateFn,
): PreviewErrorState | null {
  const contentLabel = contentLabelOf(classification)
  const renderer = classification.rendererType
  if (failure === 'network') {
    return { kind: 'error', code: 'NETWORK', message: t('preview.error.networkBody', 'The content could not be loaded'), retryable: true, contentLabel }
  }
  if (failure === 'runtime') {
    return { kind: 'unsupported', code: 'RUNTIME_UNSUPPORTED', message: t('preview.error.runtimeBody', 'This Runtime cannot display the content'), retryable: false, contentLabel }
  }
  const corrupted: PreviewErrorState = { kind: 'corrupted', code: 'CORRUPTED', message: t('preview.error.corruptedBody', 'The content could not be decoded and may be damaged'), retryable: false, contentLabel }
  if (renderer === 'image' || renderer === 'svg') {
    if (failure && classification.confirmed && runtime.imageMediaTypes.includes(classification.mediaType)) return corrupted
    return { kind: 'unsupported', code: 'UNSUPPORTED_ENCODING', message: t('preview.error.imageFormatBody', 'This browser does not support this image format'), retryable: false, contentLabel }
  }
  if (renderer === 'video' || renderer === 'audio') {
    // MEDIA_ERR_DECODE: the codec is supported (else SRC_NOT_SUPPORTED) yet the stream failed.
    return failure === 'corrupted'
      ? corrupted
      : { kind: 'unsupported', code: 'UNSUPPORTED_ENCODING', message: t('preview.error.encodingBody', 'This browser does not support this audio or video encoding'), retryable: false, contentLabel }
  }
  return failure ? corrupted : null
}

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() =>
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia('(prefers-reduced-motion: reduce)').matches
      : false,
  )
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const query = window.matchMedia('(prefers-reduced-motion: reduce)')
    const update = () => setReduced(query.matches)
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [])
  return reduced
}

/** Object URL for blob read refs; URLs pass through. Revoked on change. */
function useContentUrl(readRef: PreviewReadRef | null): string | null {
  const blob = readRef?.kind === 'blob' ? readRef.blob : null
  const objectUrl = useMemo(() => (blob ? URL.createObjectURL(blob) : null), [blob])
  useEffect(() => {
    if (!objectUrl) return
    return () => URL.revokeObjectURL(objectUrl)
  }, [objectUrl])
  if (!readRef) return null
  return readRef.kind === 'url' ? readRef.url : objectUrl
}

function useElementSize(ref: React.RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ width: 0, height: 0 })
  useLayoutEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new ResizeObserver(() => {
      const rect = element.getBoundingClientRect()
      setSize((prev) => (prev.width === rect.width && prev.height === rect.height ? prev : { width: rect.width, height: rect.height }))
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref])
  return size
}

// ─── Controller ───

interface LoadState {
  key: string
  phase: 'converting' | 'rendering' | 'ready' | 'error'
  resolved?: ResolvedPreviewSource
  classification?: MediaClassification
  result?: PreviewResult
  via?: 'direct' | 'pipeline'
  progress?: PreviewProgress
  work?: PreviewWorkState
  error?: PreviewErrorState
  media?: { width?: number; height?: number; durationSeconds?: number; pageCount?: number }
  /** The renderer reached its own fallback UI (e.g. PDF without an inline viewer). */
  degraded?: boolean
}

interface SessionState {
  key: string
  session?: ResolvedSession
  error?: PreviewErrorState
}

type RenderFailure = 'unsupported-encoding' | 'corrupted' | 'network' | 'runtime'

const resolvedCache = new Map<string, ResolvedPreviewSource>()
const RESOLVED_CACHE_MAX = 12

function rememberResolved(identity: string, resolved: ResolvedPreviewSource) {
  resolvedCache.delete(identity)
  resolvedCache.set(identity, resolved)
  while (resolvedCache.size > RESOLVED_CACHE_MAX) {
    const first = resolvedCache.keys().next().value
    if (first === undefined) break
    resolvedCache.delete(first)
  }
}

interface ImageViewInfo {
  scale: number
  mode: 'fit' | 'cover' | 'custom'
  rotation: number
}

interface TransientState {
  key: string | null
  findOpen: boolean
  findQuery: string
  findActive: number
  imageView: ImageViewInfo | null
}

const EMPTY_TRANSIENT: Omit<TransientState, 'key'> = { findOpen: false, findQuery: '', findActive: 0, imageView: null }

const sameImageView = (a: ImageViewInfo | null, b: ImageViewInfo | null): boolean =>
  a === b || (!!a && !!b && a.scale === b.scale && a.mode === b.mode && a.rotation === b.rotation)

// ─── Component ───

/**
 * The Preview Component. A ZIP (or a folder or member inside one) opens as the archive's folder page with its own
 * back stack (`preview/ArchiveBrowser.tsx`); everything else goes straight to the viewer.
 */
export function ContentPreview(props: ContentPreviewProps) {
  // Browsing onto an archive inside a session enters it; leaving its root returns to the item before it.
  const [returned, setReturned] = useState<{ key: number; source: ContentRef } | null>(null)
  const [entered, setEntered] = useState<{ target: ArchiveTarget; back: ContentRef } | null>(null)
  const lastItem = useRef<ContentRef | null>(null)
  const source = returned?.source ?? props.source
  const identity = refIdentity(source)
  const quick = props.variant === 'inline' || returned ? 'viewer' : archiveVerdict(source)
  const [decided, setDecided] = useState<{ identity: string; target: ArchiveTarget | null; fallback?: boolean } | null>(null)
  useEffect(() => {
    if (quick === 'viewer') return
    const abort = new AbortController()
    archiveTargetOf(source, abort.signal).then(
      (target) => { if (!abort.signal.aborted) setDecided({ identity, target }) },
      () => { if (!abort.signal.aborted) setDecided({ identity, target: null }) },
    )
    return () => abort.abort()
    // The source's identity decides; a new object with the same identity is the same source.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity, quick])
  const hostItemChanged = props.onItemChanged
  const onItemChanged = useCallback((item: PreviewItemInfo) => {
    hostItemChanged?.(item)
    if (props.variant === 'inline' || archiveVerdict(item.source) === 'viewer' || !isArchiveName(item.title)) {
      lastItem.current = item.source
      return
    }
    const back = lastItem.current
    void archiveTargetOf(item.source).then((target) => {
      if (target && back) setEntered({ target, back })
    }, () => undefined)
  }, [hostItemChanged, props.variant])
  if (entered) {
    const leave = () => {
      setReturned((r) => ({ key: (r?.key ?? 0) + 1, source: entered.back }))
      setEntered(null)
    }
    return <ArchivePreview target={entered.target} viewerProps={{ ...props, onRequestExit: leave }} Viewer={ContentViewer} onFallback={leave} />
  }
  if (quick === 'viewer') return <ContentViewer key={returned?.key} {...props} source={source} onItemChanged={onItemChanged} />
  const current = decided?.identity === identity ? decided : null
  if (!current) {
    return (
      <div className={clsx('flex h-full w-full items-center justify-center', props.className)} style={props.style} data-testid={props['data-testid']} data-status="resolving" role="status">
        <Loader2 className="animate-spin text-[color:var(--cp-muted)]" size={22} />
      </div>
    )
  }
  if (current.target && !current.fallback) {
    return <ArchivePreview target={current.target} viewerProps={props} Viewer={ContentViewer} onFallback={() => setDecided({ ...current, fallback: true })} />
  }
  return <ContentViewer {...props} onItemChanged={onItemChanged} />
}

/** Whether a source might be an archive (then it is checked before anything is read). */
function archiveVerdict(source: ContentRef): 'viewer' | 'check' {
  if (isCyfsPathRef(source)) {
    const local = source.path.replace(/^(cyfs|dfs):\/\//, '')
    return archiveSplit(local) || isArchiveName(local.split('/').pop() ?? '') ? 'check' : 'viewer'
  }
  if (isObjectIdRef(source)) return 'check'
  if (isBlobRef(source) || isBucketObjectRef(source)) return 'viewer'
  const name = extensionRefField(source, 'name')
  return name && isArchiveName(name) ? 'check' : 'viewer'
}

function ContentViewer(props: ContentPreviewProps) {
  const {
    source,
    session: sessionContext,
    uiMode = 'auto',
    fitMode = 'contain',
    hostActions,
    onRequestExit,
    onRequestOpenWith,
    onRequestShare,
    titleInHost = false,
    allowFullscreen = true,
    allowExport = true,
    prefetchAdjacent = true,
    autoFocus = false,
    variant = 'default',
    autoPlay = false,
    className,
    style,
    ref,
  } = props
  const inline = variant === 'inline'
  const { t } = useI18n()
  const reducedMotion = useReducedMotion()
  // Latest props for async work and event emitters (never read during render).
  const latest = useRef(props)
  useEffect(() => {
    latest.current = props
  })
  const rootRef = useRef<HTMLDivElement | null>(null)
  const toolbarRef = useRef<HTMLDivElement | null>(null)
  const rootWidth = useElementSize(rootRef).width
  const toolbarLayout: 'full' | 'compact' | 'tiny' = rootWidth > 0 && rootWidth < TINY_TOOLBAR_PX ? 'tiny' : rootWidth > 0 && rootWidth < COMPACT_TOOLBAR_PX ? 'compact' : 'full'

  // ── session ──
  const sourceIdentity = refIdentity(source)
  const sessionKey = sessionKeyOf(sessionContext, source)
  const [sessionState, setSessionState] = useState<SessionState | null>(null)
  const currentSession = sessionState?.key === sessionKey ? sessionState.session ?? null : null
  const sessionError = sessionState?.key === sessionKey ? sessionState.error ?? null : null
  const [nav, setNav] = useState<{ sessionKey: string; sourceIdentity: string; index: number } | null>(null)

  const index = useMemo(() => {
    if (!currentSession) return 0
    if (nav && nav.sessionKey === sessionKey && nav.sourceIdentity === sourceIdentity) {
      return clamp(nav.index, 0, currentSession.items.length - 1)
    }
    const located = locateIndex(currentSession.items, source)
    return located >= 0 ? located : clamp(currentSession.index, 0, currentSession.items.length - 1)
  }, [currentSession, nav, sessionKey, sourceIdentity, source])

  const item: SessionItem | null = currentSession?.items[index] ?? null
  const [retryNonce, setRetryNonce] = useState(0)
  const [fallbackNonce, setFallbackNonce] = useState(0)
  const skipDirectRef = useRef<Map<string, RenderFailure>>(new Map())
  const retryContextRef = useRef<{ key: string; expectedAttemptId?: string } | null>(null)
  const loadKey = item ? `${sessionKey}|${item.id}|${retryNonce}|${fallbackNonce}` : null
  const [load, setLoad] = useState<LoadState | null>(null)
  const current = load && load.key === loadKey ? load : null
  // Renderer effects depend on onFailure; reading `load` through a ref keeps
  // that callback stable across phase changes (else onRendered → new load →
  // new onFailure → the renderer re-reads the content a second time).
  const loadRef = useRef(load)
  useEffect(() => {
    loadRef.current = load
  })

  const status: PreviewStatus = !currentSession
    ? sessionError
      ? 'error'
      : 'resolving'
    : !current
      ? 'resolving'
      : current.phase

  const error = sessionError ?? (current?.phase === 'error' ? current.error ?? null : null)
  const rendererType = current?.result?.resultType ?? null

  useEffect(() => {
    if (sessionState?.key === sessionKey) return
    const controller = new AbortController()
    void (async () => {
      try {
        const provider = await ensurePreviewProvider()
        const resolved = await resolveSessionContext(latest.current.source, latest.current.session, provider, controller.signal)
        if (controller.signal.aborted) return
        setSessionState({ key: sessionKey, session: resolved })
      } catch (err) {
        if (controller.signal.aborted || isAbortError(err)) return
        setSessionState({ key: sessionKey, error: errorStateFrom(err) })
      }
    })()
    return () => controller.abort()
  }, [sessionKey, sessionState?.key, latest])

  // ── load the current item ──
  useEffect(() => {
    if (!loadKey || !item) return
    const key = loadKey
    const controller = new AbortController()
    const { signal } = controller
    const itemRef = item
    const update = (patch: Partial<LoadState>) => {
      if (signal.aborted) return
      setLoad((prev) => ({ ...(prev && prev.key === key ? prev : { key, phase: 'converting' }), ...patch, key }))
    }
    let pollTimer: number | null = null
    const wait = (ms: number) =>
      new Promise<void>((resolve, reject) => {
        pollTimer = window.setTimeout(resolve, ms)
        signal.addEventListener('abort', () => reject(new PreviewError('CANCELLED', 'Cancelled')), { once: true })
      })

    void (async () => {
      let classification: MediaClassification | undefined
      try {
        const provider = await ensurePreviewProvider()
        const identity = refIdentity(itemRef.source)
        let resolved = resolvedCache.get(identity)
        if (!resolved) {
          resolved = await provider.resolvePreviewSource(itemRef.source, { signal, permissionsContext: latest.current.permissionsContext })
          rememberResolved(identity, resolved)
        }
        if (signal.aborted) return
        if (resolved.objectType && resolved.objectType !== 'file' && resolved.objectType !== 'symlink' && resolved.objectType !== 'object') {
          update({ phase: 'error', resolved, error: unsupportedState('not-content', resolved.objectType, t('preview.error.notContent', 'Folders and containers cannot be previewed as content')) })
          return
        }

        // Bounded probe: Content-Type + magic bytes (§23.3 rule 5).
        let probeBytes: Uint8Array | null = null
        let contentType: string | null = null
        try {
          const probe = await readProbe(resolved.readRef, PROBE_BYTES, signal)
          probeBytes = probe.bytes
          contentType = probe.contentType
          if (resolved.size === undefined && probe.totalLength !== null) {
            resolved = { ...resolved, size: probe.totalLength }
          }
        } catch (probeErr) {
          // The server answering with an error is the answer; an unreachable probe falls back to hints.
          if (probeErr instanceof PreviewError && (probeErr.code === 'PERMISSION_DENIED' || probeErr.code === 'NOT_FOUND' || probeErr.status !== undefined)) throw probeErr
        }
        if (signal.aborted) return
        classification = classifyMedia({
          name: resolved.displayName,
          hints: resolved.mediaTypeHints,
          contentType,
          magic: probeBytes,
          objectType: resolved.objectType,
        })
        const runtime = await ensureRuntimeProfile()
        const skipDirect = skipDirectRef.current.get(itemRef.id)
        const direct = skipDirect ? ({ ok: false, reason: 'runtime-unsupported' } as const) : decideDirect(classification, resolved.size, runtime)

        if (direct.ok && classification.rendererType) {
          update({
            phase: 'rendering',
            resolved,
            classification,
            via: 'direct',
            result: { resultType: classification.rendererType, readRef: resolved.readRef, mediaType: classification.mediaType, sourceVersion: resolved.versionToken },
            progress: undefined,
          })
          return
        }

        if (direct.ok === false && direct.reason === 'too-large') {
          update({ phase: 'error', resolved, classification, error: unsupportedState('too-large', contentLabelOf(classification), t('preview.error.tooLarge', 'This content is too large to preview directly')) })
          return
        }

        // Pipeline path (§23.7): ensure → poll.
        update({ phase: 'converting', resolved, classification, via: 'pipeline', progress: { phase: 'converting', message: t('preview.status.preparing', 'Preparing preview') } })
        const root = rootRef.current
        const targetProfile = {
          purpose: 'preview' as const,
          viewport: {
            width: Math.max(1, Math.round(root?.clientWidth ?? window.innerWidth)),
            height: Math.max(1, Math.round(root?.clientHeight ?? window.innerHeight)),
            dpr: window.devicePixelRatio || 1,
          },
          quality: 'balanced' as const,
        }
        const retry = retryContextRef.current?.key === itemRef.id ? retryContextRef.current : null
        retryContextRef.current = null
        // A video that just failed to play here is not playable whatever canPlayType said.
        const pipelineRuntime = skipDirect && classification.rendererType === 'video' ? { ...runtime, videoCodecs: { ...runtime.videoCodecs, hevc: false } } : runtime
        let state = await provider.ensurePreviewWork({
          source: resolved,
          runtimeProfile: pipelineRuntime,
          targetProfile,
          mediaType: classification.mediaType,
          options: { signal, retry: !!retry, expectedAttemptId: retry?.expectedAttemptId },
        })
        if (signal.aborted) return
        if ('kind' in state) {
          // Markup no Pipeline converts here is still readable as the text it is.
          if (classification.textFallback && (state.reason === 'no-pipeline' || state.reason === 'no-renderer')) {
            update({ phase: 'rendering', resolved, classification, via: 'direct', result: { resultType: 'text', readRef: resolved.readRef, mediaType: classification.mediaType, sourceVersion: resolved.versionToken }, progress: undefined })
            return
          }
          const browserLimited = !!skipDirect || (direct.ok === false && direct.reason === 'runtime-unsupported')
          const failed = browserLimited && state.reason === 'no-pipeline' ? renderFailureState(skipDirect, classification, runtime, t) : null
          update({ phase: 'error', resolved, classification, error: failed ?? unsupportedState(state.reason, contentLabelOf(classification), state.detail) })
          return
        }
        const startedAt = Date.now()
        while (state.state === 'processing') {
          update({ phase: 'converting', work: state, progress: { phase: 'converting', completed: state.progress?.completed, total: state.progress?.total, message: state.progress?.message } })
          if (Date.now() - startedAt > PIPELINE_TOTAL_TIMEOUT_MS) {
            throw new PreviewError('TIMEOUT', t('preview.error.timeoutBody', 'Preparing the preview took too long'), { retryable: true })
          }
          await wait(clamp(state.retryAfterMs ?? 800, 300, 3000))
          state = await provider.getPreviewWork(state.workKey, { signal })
          if (signal.aborted) return
        }
        if (state.state === 'failed') {
          update({
            phase: 'error',
            work: state,
            error: {
              kind: state.error.code === 'PERMISSION_DENIED' ? 'permission-denied' : state.error.code === 'CORRUPTED' || state.error.code === 'CONTENT_CORRUPTED' ? 'corrupted' : state.error.code.includes('TIMEOUT') ? 'timeout' : 'error',
              code: state.error.code,
              message: state.error.message,
              retryable: state.error.retryable,
              contentLabel: contentLabelOf(classification),
            },
          })
          return
        }
        update({ phase: 'rendering', work: state, result: state.result, via: 'pipeline', progress: undefined })
      } catch (err) {
        if (signal.aborted || isAbortError(err)) return
        update({ phase: 'error', error: errorStateFrom(err, classification ? contentLabelOf(classification) : undefined) })
      }
    })()

    return () => {
      controller.abort()
      if (pollTimer !== null) window.clearTimeout(pollTimer)
    }
  }, [loadKey, item, latest, t])

  const handleRendered = useCallback(
    (media?: LoadState['media'], opts?: { degraded?: boolean }) => {
      setLoad((prev) =>
        prev && prev.key === loadKey && prev.phase === 'rendering'
          ? { ...prev, phase: 'ready', media: media ?? prev.media, degraded: opts?.degraded ?? false }
          : prev,
      )
    },
    [loadKey],
  )

  const handleRenderFailure = useCallback(
    (failure: RenderFailure) => {
      if (!item) return
      const snapshot = loadRef.current
      const state = snapshot && snapshot.key === loadKey ? snapshot : null
      if (state?.via === 'direct' && (failure === 'unsupported-encoding' || failure === 'corrupted') && !skipDirectRef.current.has(item.id)) {
        // Direct decode failed → back to the Pipeline Planner (§23.3).
        skipDirectRef.current.set(item.id, failure)
        setFallbackNonce((n) => n + 1)
        return
      }
      const classification = state?.classification
      const shown = classification && state?.result ? { ...classification, rendererType: state.result.resultType, mediaType: state.result.mediaType } : classification
      const err: PreviewErrorState = (shown ? renderFailureState(failure, shown, detectRuntimeProfile(), t) : null) ?? {
        kind: 'corrupted',
        code: 'CORRUPTED',
        message: t('preview.error.corruptedBody', 'The content could not be decoded and may be damaged'),
        retryable: false,
      }
      setLoad((prev) => (prev && prev.key === loadKey ? { ...prev, phase: 'error', error: err } : prev))
    },
    [item, loadKey, t],
  )

  // ── navigation ──
  const goTo = useCallback(
    (nextIndex: number) => {
      if (!currentSession) return false
      if (nextIndex < 0 || nextIndex >= currentSession.items.length) return false
      setNav({ sessionKey, sourceIdentity, index: nextIndex })
      return true
    },
    [currentSession, sessionKey, sourceIdentity],
  )
  const step = useCallback(
    (delta: number) => {
      if (!currentSession) return false
      const target = neighborIndex(currentSession, delta, index)
      return target === null ? false : goTo(target)
    },
    [currentSession, goTo, index],
  )
  const canPrev = currentSession ? neighborIndex(currentSession, -1, index) !== null : false
  const canNext = currentSession ? neighborIndex(currentSession, 1, index) !== null : false

  const retry = useCallback(() => {
    if (item) {
      skipDirectRef.current.delete(item.id)
      const work = current?.work
      retryContextRef.current = { key: item.id, expectedAttemptId: work?.state === 'failed' ? work.attemptId : undefined }
      resolvedCache.delete(refIdentity(item.source))
    }
    // A failed session context is keyed by sessionKey, which retryNonce is not
    // part of — drop it so the session effect resolves again.
    setSessionState((prev) => (prev && prev.key === sessionKey && prev.error ? null : prev))
    setRetryNonce((n) => n + 1)
  }, [current?.work, item, sessionKey])

  // ── prefetch neighbours (±1) ──
  useEffect(() => {
    if (!prefetchAdjacent || status !== 'ready' || !currentSession) return
    const neighbours = [neighborIndex(currentSession, -1, index), neighborIndex(currentSession, 1, index)]
      .filter((i): i is number => i !== null)
      .map((i) => currentSession.items[i])
    const controller = new AbortController()
    void ensurePreviewProvider().then(async (provider) => {
      for (const neighbour of neighbours) {
        if (controller.signal.aborted) return
        const identity = refIdentity(neighbour.source)
        if (resolvedCache.has(identity)) continue
        try {
          const resolved = await provider.resolvePreviewSource(neighbour.source, { signal: controller.signal, permissionsContext: latest.current.permissionsContext })
          rememberResolved(identity, resolved)
          const hint = resolved.mediaTypeHints[0] ?? ''
          if (resolved.readRef.kind === 'url' && hint.startsWith('image/') && (resolved.size ?? 0) < 20 * 1024 * 1024) {
            const img = new Image()
            img.decoding = 'async'
            img.src = resolved.readRef.url
          }
        } catch {
          // Prefetch is best-effort and must never surface (§17.1).
        }
      }
    })
    return () => controller.abort()
  }, [currentSession, index, latest, prefetchAdjacent, status])

  // ── UI visibility (§10.3) ──
  // Visible: always; Silent: never; Auto: shown on intent, hidden after idle. The user's pin switches between Auto
  // and Visible for this item list (a new `uiMode` from the host starts over); Silent stays the host's.
  const [pinChoice, setPinChoice] = useState<{ base: PreviewUIMode; pinned: boolean } | null>(null)
  const toolbarPinned = pinChoice && pinChoice.base === uiMode ? pinChoice.pinned : uiMode === 'visible'
  const effectiveUiMode: PreviewUIMode = uiMode === 'silent' ? 'silent' : toolbarPinned ? 'visible' : 'auto'
  const [autoVisible, setAutoVisible] = useState(false)
  const uiVisible = effectiveUiMode === 'visible' ? true : effectiveUiMode === 'silent' ? false : autoVisible
  const hideTimer = useRef<number | null>(null)
  // What keeps an auto toolbar shown besides the pin: keyboard focus in it, an open menu, and the pointer resting on
  // it (read as `:hover` when the time is up: an enter whose leave never came, the bar having moved, does not count).
  const holds = useRef(new Set<string>())
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const scheduleHide = useCallback(() => {
    if (hideTimer.current !== null) window.clearTimeout(hideTimer.current)
    const due = () => {
      hideTimer.current = null
      if (holds.current.size > 0) return
      if (toolbarRef.current?.matches(':hover')) hideTimer.current = window.setTimeout(due, AUTO_HIDE_MS)
      else setAutoVisible(false)
    }
    hideTimer.current = window.setTimeout(due, AUTO_HIDE_MS)
  }, [])
  const showUi = useCallback(() => {
    if (effectiveUiMode !== 'auto') return
    setAutoVisible(true)
    scheduleHide()
  }, [scheduleHide, effectiveUiMode])
  useEffect(() => () => {
    if (hideTimer.current !== null) window.clearTimeout(hideTimer.current)
    hideTimer.current = null
  }, [])
  useEffect(() => {
    // Shown without a countdown (a remount kept the state but dropped the timer): start one.
    if (autoVisible && effectiveUiMode === 'auto' && hideTimer.current === null && holds.current.size === 0) scheduleHide()
  }, [autoVisible, effectiveUiMode, scheduleHide])
  const holdUi = useCallback(
    (reason: string, on: boolean) => {
      if (on) {
        holds.current.add(reason)
        if (hideTimer.current !== null) window.clearTimeout(hideTimer.current)
        hideTimer.current = null
      } else if (holds.current.delete(reason) && holds.current.size === 0 && effectiveUiMode === 'auto') {
        scheduleHide()
      }
    },
    [scheduleHide, effectiveUiMode],
  )
  useEffect(() => {
    holdUi('menu', Boolean(menuAnchor))
  }, [menuAnchor, holdUi])
  const toggleToolbarPin = useCallback(() => {
    const pinned = !toolbarPinned
    setPinChoice({ base: uiMode, pinned })
    if (!pinned) {
      // Unpinned: still shown until the user is idle.
      setAutoVisible(true)
      if (holds.current.size === 0) scheduleHide()
    }
    latest.current.onUiModeChange?.(pinned ? 'visible' : 'auto')
  }, [latest, scheduleHide, toolbarPinned, uiMode])

  // ── info / find / fullscreen ──
  const [infoOpen, setInfoOpen] = useState(false)
  const [findCount, setFindCount] = useState(0)
  // Per-item transient UI (find bar, image view) keyed by the load key — a
  // new item starts clean without an effect-driven reset.
  const [transientState, setTransientState] = useState<TransientState>({ key: null, ...EMPTY_TRANSIENT })
  const transient: TransientState = transientState.key === loadKey ? transientState : { key: loadKey, ...EMPTY_TRANSIENT }
  const patchTransient = useCallback(
    (patch: Partial<Omit<TransientState, 'key'>> | ((prev: TransientState) => Partial<Omit<TransientState, 'key'>>)) => {
      setTransientState((prev) => {
        const base: TransientState = prev.key === loadKey ? prev : { key: loadKey, ...EMPTY_TRANSIENT }
        const next = typeof patch === 'function' ? patch(base) : patch
        // Pointer-move driven patches (image pan/pinch) often carry no change;
        // keep the previous state so the whole preview does not re-render.
        const unchanged = (Object.keys(next) as (keyof typeof next)[]).every((k) =>
          k === 'imageView' ? sameImageView(next.imageView ?? null, base.imageView) : next[k] === base[k],
        )
        if (unchanged) return prev
        return { ...base, ...next, key: loadKey }
      })
    },
    [loadKey],
  )
  const { findOpen, findQuery, findActive, imageView } = transient
  const setFindOpen = useCallback((open: boolean) => patchTransient({ findOpen: open }), [patchTransient])
  const handleViewChange = useCallback((view: ImageViewInfo | null) => patchTransient({ imageView: view }), [patchTransient])
  const [isFullscreen, setIsFullscreen] = useState(false)
  useEffect(() => {
    const update = () => setIsFullscreen(Boolean(document.fullscreenElement) && document.fullscreenElement === rootRef.current)
    document.addEventListener('fullscreenchange', update)
    return () => document.removeEventListener('fullscreenchange', update)
  }, [])
  const fullscreenAvailable = allowFullscreen && typeof document !== 'undefined' && Boolean(document.fullscreenEnabled)
  const toggleFullscreen = useCallback(() => {
    const root = rootRef.current
    if (!root || !fullscreenAvailable) return
    if (document.fullscreenElement === root) void document.exitFullscreen()
    else void root.requestFullscreen().catch(() => {})
  }, [fullscreenAvailable])

  // ── renderer controls ──
  const imageRef = useRef<ImageRendererHandle | null>(null)
  const mediaRef = useRef<MediaRendererHandle | null>(null)
  const zoomRef = useRef<ZoomHandle | null>(null)
  const [fontScale, setFontScale] = useState(1)

  // ── item info & events ──
  const itemInfo: PreviewItemInfo | null = useMemo(() => {
    if (!item || !currentSession) return null
    return { index, count: currentSession.items.length, id: item.id, title: item.title ?? refDisplayName(item.source), source: item.source }
  }, [currentSession, index, item])
  const displayName = current?.resolved?.displayName ?? itemInfo?.title ?? refDisplayName(source)

  const capabilities: PreviewCapabilities = useMemo(() => {
    const ready = status === 'ready'
    const isImage = ready && (rendererType === 'image' || rendererType === 'svg' || rendererType === 'diagram')
    const isText = ready && rendererType === 'text'
    // A PDF drawn by pdf.js and a sheet report their view; the built-in PDF viewer zooms by itself.
    const isZoomView = ready && (rendererType === 'pdf' || rendererType === 'sheet') && imageView !== null
    const hasRead = Boolean(current?.result?.readRef)
    return {
      zoom: isImage || isText || isZoomView,
      pan: isImage,
      textSelection: ready && (rendererType === 'text' || rendererType === 'html'),
      search: isText,
      playback: ready && (rendererType === 'audio' || rendererType === 'video'),
      rotate: isImage,
      previous: canPrev,
      next: canNext,
      export: allowExport && hasRead,
      openWith: Boolean(onRequestOpenWith),
      share: Boolean(onRequestShare),
      fullscreen: fullscreenAvailable,
    }
  }, [allowExport, canNext, canPrev, current?.result?.readRef, fullscreenAvailable, imageView, onRequestOpenWith, onRequestShare, rendererType, status])

  const capabilitiesKey = JSON.stringify(capabilities)
  useEffect(() => {
    latest.current.onCapabilitiesChanged?.(JSON.parse(capabilitiesKey) as PreviewCapabilities)
  }, [capabilitiesKey, latest])
  useEffect(() => {
    if (itemInfo) latest.current.onItemChanged?.(itemInfo)
  }, [itemInfo, latest])
  useEffect(() => {
    latest.current.onUiVisibilityChanged?.(uiVisible)
  }, [latest, uiVisible])
  useEffect(() => {
    if (status === 'ready' && current?.result && current.resolved && itemInfo && current.via) {
      latest.current.onReady?.({ ...itemInfo, rendererType: current.result.resultType, mediaType: current.result.mediaType, via: current.via, resolved: current.resolved, result: current.result })
    }
  }, [current, itemInfo, latest, status])
  useEffect(() => {
    if (status === 'error' && error) latest.current.onError?.(error, itemInfo)
  }, [error, itemInfo, latest, status])
  useEffect(() => {
    if (current?.progress) latest.current.onProgress?.(current.progress)
  }, [current?.progress, latest])

  const invokeAction = useCallback(
    (id: string) => {
      latest.current.onActionInvoked?.({ id, item: itemInfo })
    },
    [itemInfo, latest],
  )

  const requestExit = useCallback(() => {
    if (document.fullscreenElement === rootRef.current) {
      void document.exitFullscreen()
      return
    }
    invokeAction('exit')
    latest.current.onRequestExit?.()
  }, [invokeAction, latest])

  // Open with… and Share act on the original: its own media type, never a Pipeline result's (a DOCX shown as PDF).
  const originalRequest = useCallback(
    (): PreviewOpenWithRequest | null => (itemInfo ? { item: itemInfo, resolved: current?.resolved, mediaType: current?.classification?.mediaType } : null),
    [current?.classification?.mediaType, current?.resolved, itemInfo],
  )
  const requestOpenWith = useCallback(() => {
    const request = originalRequest()
    if (!request) return
    invokeAction('open-with')
    latest.current.onRequestOpenWith?.(request)
  }, [invokeAction, latest, originalRequest])
  const requestShare = useCallback(() => {
    const request = originalRequest()
    if (!request) return
    invokeAction('share')
    latest.current.onRequestShare?.(request)
  }, [invokeAction, latest, originalRequest])

  const downloadCurrent = useCallback(() => {
    // "Download original": the source's bytes, never a Pipeline result.
    const readRef = current?.resolved?.readRef ?? current?.result?.readRef
    if (!readRef || !capabilities.export) return
    invokeAction('download')
    const anchor = document.createElement('a')
    anchor.download = displayName
    anchor.rel = 'noopener'
    if (readRef.kind === 'url') {
      anchor.href = readRef.downloadUrl ?? readRef.url
      anchor.click()
      return
    }
    const url = URL.createObjectURL(readRef.blob)
    anchor.href = url
    anchor.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
  }, [capabilities.export, current?.resolved?.readRef, current?.result?.readRef, displayName, invokeAction])

  const copySource = useCallback(() => {
    if (!itemInfo) return
    invokeAction('copy-source')
    void navigator.clipboard?.writeText(sourceLabel(itemInfo.source)).catch(() => {})
  }, [invokeAction, itemInfo])

  // The item as a Zone reference (剪贴板规范 §2.2): dragged from the toolbar's handle to Files, a conversation or
  // another App, or copied from More. Content outside the Zone (a file of this device) has neither.
  const zoneItem = useMemo(
    () => (itemInfo ? zoneContentOfPreviewSource(itemInfo.source, { name: displayName, mime: current?.classification?.mediaType, size: current?.resolved?.size }) : null),
    [current?.classification?.mediaType, current?.resolved?.size, displayName, itemInfo],
  )
  const [finePointer] = useState(() => !window.matchMedia?.('(pointer: coarse)').matches)
  const dragItem = useCallback((event: React.DragEvent) => {
    if (!zoneItem) return
    writeTransferEvent(event.dataTransfer, contentRefsRepresentations([zoneItem]))
    event.dataTransfer.effectAllowed = 'copy'
  }, [zoneItem])
  const copyItem = useCallback(() => {
    if (!zoneItem) return
    invokeAction('copy-file')
    void copyWithCommand(contentRefsRepresentations([zoneItem]))
  }, [invokeAction, zoneItem])

  const zoomIn = useCallback(() => {
    if (rendererType === 'text') setFontScale((s) => clamp(s * 1.15, 0.5, 4))
    else if (rendererType === 'pdf' || rendererType === 'sheet') zoomRef.current?.zoomBy(ZOOM_STEP)
    else imageRef.current?.zoomBy(ZOOM_STEP)
  }, [rendererType])
  const zoomOut = useCallback(() => {
    if (rendererType === 'text') setFontScale((s) => clamp(s / 1.15, 0.5, 4))
    else if (rendererType === 'pdf' || rendererType === 'sheet') zoomRef.current?.zoomBy(1 / ZOOM_STEP)
    else imageRef.current?.zoomBy(1 / ZOOM_STEP)
  }, [rendererType])
  const fitToView = useCallback(() => {
    if (rendererType === 'text') setFontScale(1)
    else if (rendererType === 'pdf' || rendererType === 'sheet') zoomRef.current?.fit()
    else imageRef.current?.fit()
  }, [rendererType])
  const actualSize = useCallback(() => imageRef.current?.actualSize(), [])
  const rotate = useCallback(() => imageRef.current?.rotate(), [])

  useImperativeHandle(
    ref,
    () => ({
      next: () => step(1),
      previous: () => step(-1),
      goTo,
      zoomIn,
      zoomOut,
      fitToView,
      actualSize,
      rotate,
      toggleInfo: () => setInfoOpen((v) => !v),
      retry,
      focus: () => rootRef.current?.focus(),
      getCapabilities: () => capabilities,
      getStatus: () => status,
      getItem: () => itemInfo,
    }),
    [actualSize, capabilities, fitToView, goTo, itemInfo, retry, rotate, status, step, zoomIn, zoomOut],
  )

  useEffect(() => {
    if (autoFocus) rootRef.current?.focus({ preventScroll: true })
  }, [autoFocus])

  // ── keyboard (§10.7) ──
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    const inField = target.closest('input, textarea, [contenteditable="true"]') !== null
    const mod = event.ctrlKey || event.metaKey
    if (event.key === 'Escape') {
      if (inField && findOpen) {
        setFindOpen(false)
        rootRef.current?.focus()
        event.preventDefault()
        return
      }
      event.preventDefault()
      requestExit()
      return
    }
    if (inField || target.closest('video, audio')) return
    showUi()
    if (event.key === 'Tab' && !event.shiftKey && target === rootRef.current && !uiVisible && effectiveUiMode === 'auto') {
      // A hidden toolbar is inert: the first Tab shows it and moves into it.
      event.preventDefault()
      window.requestAnimationFrame(() => toolbarRef.current?.querySelector<HTMLElement>('button:not([disabled])')?.focus())
      return
    }
    const isMedia = rendererType === 'audio' || rendererType === 'video'
    switch (event.key) {
      case 'ArrowLeft':
        if (isMedia && status === 'ready') mediaRef.current?.seekBy(-5)
        else step(-1)
        event.preventDefault()
        return
      case 'ArrowRight':
        if (isMedia && status === 'ready') mediaRef.current?.seekBy(5)
        else step(1)
        event.preventDefault()
        return
      case 'PageUp':
      case '[':
        step(-1)
        event.preventDefault()
        return
      case 'PageDown':
      case ']':
        step(1)
        event.preventDefault()
        return
      case 'Home':
        goTo(0)
        event.preventDefault()
        return
      case 'End':
        if (currentSession) goTo(currentSession.items.length - 1)
        event.preventDefault()
        return
      case ' ':
        if (isMedia) {
          mediaRef.current?.togglePlay()
          event.preventDefault()
        }
        return
      case '+':
      case '=':
        if (capabilities.zoom && (rendererType !== 'text' || mod)) {
          zoomIn()
          event.preventDefault()
        }
        return
      case '-':
      case '_':
        if (capabilities.zoom && (rendererType !== 'text' || mod)) {
          zoomOut()
          event.preventDefault()
        }
        return
      case '0':
        if (capabilities.zoom) {
          fitToView()
          event.preventDefault()
        }
        return
      case '1':
        if (capabilities.pan) {
          actualSize()
          event.preventDefault()
        }
        return
      default:
        break
    }
    const key = event.key.toLowerCase()
    if (key === 'r' && !mod && capabilities.rotate) {
      rotate()
      event.preventDefault()
    } else if (key === 'i' && !mod && !inline) {
      setInfoOpen((v) => !v)
      event.preventDefault()
    } else if (key === 'f' && mod && capabilities.search) {
      setFindOpen(true)
      event.preventDefault()
    } else if (key === 'f' && !mod && capabilities.fullscreen) {
      toggleFullscreen()
      event.preventDefault()
    }
  }

  const handlePointerActivity = () => showUi()

  // ── render ──
  const contentLabel = error?.contentLabel ?? (current?.classification ? contentLabelOf(current.classification) : undefined)
  const showToolbar = effectiveUiMode !== 'silent' && uiVisible
  const transition = reducedMotion ? 'none' : 'opacity 160ms var(--cp-ease-smooth, ease)'
  const sessionCount = currentSession?.items.length ?? 0

  const toolbarHostActions = (hostActions ?? []).filter((a) => a.placement !== 'overflow')
  const overflowHostActions = (hostActions ?? []).filter((a) => a.placement === 'overflow')
  const runHostAction = (action: PreviewHostAction) => {
    invokeAction(action.id)
    if (itemInfo) action.onInvoke({ item: itemInfo, resolved: current?.resolved })
  }
  const closeMenuThen = (run: () => void) => {
    setMenuAnchor(null)
    run()
  }
  const zoomPercent = rendererType === 'text' ? Math.round(fontScale * 100) : imageView ? Math.round(imageView.scale * 100) : null

  return (
    <div
      ref={rootRef}
      data-testid={props['data-testid'] ?? 'content-preview'}
      data-status={status}
      data-renderer={rendererType ?? ''}
      data-via={current?.via ?? ''}
      data-item-index={index}
      data-item-count={sessionCount}
      data-degraded={current?.degraded ? 'true' : 'false'}
      data-ui-visible={showToolbar ? 'true' : 'false'}
      role="region"
      aria-roledescription={t('preview.a11y.region', 'content preview')}
      aria-label={displayName}
      aria-busy={status === 'resolving' || status === 'converting' || status === 'rendering'}
      tabIndex={0}
      className={clsx(
        'files-ui relative isolate flex h-full w-full min-h-0 select-none overflow-hidden bg-[color:var(--cp-bg-strong)] text-[color:var(--cp-text)] outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[color:var(--cp-focus-ring)]',
        className,
      )}
      style={style}
      onKeyDown={handleKeyDown}
      onPointerMove={handlePointerActivity}
      onPointerOver={handlePointerActivity}
      onPointerDown={handlePointerActivity}
      onFocus={handlePointerActivity}
      onTouchStart={handlePointerActivity}
    >
      {/* Content area */}
      <div className="relative min-h-0 min-w-0 flex-1">
        {status === 'ready' || status === 'rendering' ? (
          current?.result ? (
            <RendererSwitch
              key={loadKey ?? 'none'}
              result={current.result}
              displayName={displayName}
              fitMode={fitMode}
              uiMode={effectiveUiMode}
              compact={inline}
              autoPlay={autoPlay}
              reducedMotion={reducedMotion}
              fontScale={fontScale}
              find={{ open: findOpen, query: findQuery, active: findActive, onCount: setFindCount }}
              imageRef={imageRef}
              mediaRef={mediaRef}
              zoomRef={zoomRef}
              onViewChange={handleViewChange}
              onRendered={handleRendered}
              onFailure={handleRenderFailure}
              onDownload={downloadCurrent}
              onOpenWith={capabilities.openWith ? requestOpenWith : undefined}
              onActivity={showUi}
              t={t}
            />
          ) : null
        ) : null}

        {(status === 'resolving' || status === 'converting' || status === 'rendering') && (
          <StatusOverlay status={status} progress={current?.progress} t={t} reducedMotion={reducedMotion} />
        )}

        {status === 'error' && error ? (
          <ErrorState
            error={error}
            contentLabel={contentLabel}
            t={t}
            onRetry={error.retryable || error.kind === 'permission-denied' || error.kind === 'not-found' ? retry : undefined}
            onOpenWith={capabilities.openWith ? requestOpenWith : undefined}
            onDownload={allowExport && current?.resolved && error.kind !== 'permission-denied' && error.kind !== 'not-found' ? downloadCurrent : undefined}
            onInfo={error.kind !== 'permission-denied' && current?.resolved ? () => setInfoOpen(true) : undefined}
          />
        ) : null}

        {/* Session edge navigation */}
        {sessionCount > 1 && effectiveUiMode !== 'silent' && !inline ? (
          <>
            <EdgeNav side="left" visible={showToolbar} disabled={!canPrev} onClick={() => step(-1)} label={t('preview.action.previous', 'Previous')} transition={transition} />
            <EdgeNav side="right" visible={showToolbar} disabled={!canNext} onClick={() => step(1)} label={t('preview.action.next', 'Next')} transition={transition} />
          </>
        ) : null}

        {/* Toolbar (§10.8): a compact bar centred at the top, over the content; the host's title bar keeps the name */}
        {effectiveUiMode !== 'silent' && !inline ? (
          <div
            ref={toolbarRef}
            data-testid="content-preview-toolbar"
            data-pinned={toolbarPinned ? 'true' : 'false'}
            data-layout={toolbarLayout}
            role="toolbar"
            aria-label={t('preview.a11y.toolbar', 'Preview actions')}
            aria-hidden={!showToolbar}
            inert={!showToolbar}
            className={clsx(
              'absolute left-1/2 top-2 z-20 flex max-w-[calc(100%-16px)] -translate-x-1/2 items-center gap-0.5 rounded-[12px] border p-1',
              !showToolbar && 'pointer-events-none',
            )}
            style={{
              opacity: showToolbar ? 1 : 0,
              transition,
              borderColor: 'var(--files-hairline)',
              background: 'var(--cp-surface-opaque)',
              boxShadow: 'var(--files-float-shadow)',
            }}
            onPointerLeave={showUi}
            onFocus={(event) => holdUi('focus', event.target.matches(':focus-visible'))}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) holdUi('focus', false)
            }}
          >
            {zoneItem && finePointer ? (
              <span
                draggable
                onDragStart={dragItem}
                role="img"
                aria-label={t('preview.action.dragOut', 'Drag to another app')}
                title={t('preview.action.dragOut', 'Drag to another app')}
                data-testid="content-preview-drag"
                className="inline-flex h-8 w-6 shrink-0 cursor-grab items-center justify-center rounded-[8px] text-[color:var(--cp-muted)] transition-colors hover:bg-[color:var(--files-hover)] active:cursor-grabbing"
              >
                <GripVertical size={16} />
              </span>
            ) : null}
            {onRequestExit && !titleInHost ? (
              <ToolButton label={t('preview.action.exit', 'Exit preview')} shortcut="Esc" onClick={requestExit} testId="content-preview-exit">
                <X size={16} />
              </ToolButton>
            ) : null}
            {!titleInHost ? (
              <div className="min-w-0 max-w-[260px] shrink px-1.5" title={displayName}>
                <div className="truncate text-[13px] font-medium leading-4" data-testid="content-preview-title">
                  {displayName}
                </div>
                {sessionCount > 1 ? (
                  <div className="text-[12px] leading-4 tabular-nums text-[color:var(--cp-muted)]" data-testid="content-preview-counter">
                    {t('preview.counter', '{{index}} of {{count}}', { index: index + 1, count: sessionCount })}
                  </div>
                ) : null}
              </div>
            ) : sessionCount > 1 ? (
              <div className="shrink-0 px-2 text-[12px] tabular-nums text-[color:var(--cp-muted)]" data-testid="content-preview-counter">
                {t('preview.counter', '{{index}} of {{count}}', { index: index + 1, count: sessionCount })}
              </div>
            ) : null}
            {!titleInHost || sessionCount > 1 ? <ToolbarDivider /> : null}
            {capabilities.zoom && toolbarLayout !== 'tiny' ? (
              <div className="flex shrink-0 items-center">
                <ToolButton label={t('preview.action.zoomOut', 'Zoom out')} shortcut="-" onClick={zoomOut} testId="content-preview-zoom-out">
                  <ZoomOut size={16} />
                </ToolButton>
                <button
                  type="button"
                  className="h-8 min-w-[48px] rounded-[8px] px-1 text-[12px] font-medium tabular-nums text-[color:var(--cp-text)] transition-colors hover:bg-[color:var(--files-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)]"
                  onClick={fitToView}
                  title={t('preview.action.fit', 'Fit to view')}
                  data-testid="content-preview-zoom-level"
                >
                  {zoomPercent !== null ? `${zoomPercent}%` : '—'}
                </button>
                <ToolButton label={t('preview.action.zoomIn', 'Zoom in')} shortcut="+" onClick={zoomIn} testId="content-preview-zoom-in">
                  <ZoomIn size={16} />
                </ToolButton>
                {capabilities.pan && toolbarLayout === 'full' ? (
                  <ToolButton
                    label={imageView?.mode === 'fit' ? t('preview.action.actualSize', 'Actual size') : t('preview.action.fit', 'Fit to view')}
                    shortcut={imageView?.mode === 'fit' ? '1' : '0'}
                    onClick={imageView?.mode === 'fit' ? actualSize : fitToView}
                  >
                    {imageView?.mode === 'fit' ? <Maximize2 size={16} /> : <Minimize2 size={16} />}
                  </ToolButton>
                ) : null}
              </div>
            ) : null}
            {toolbarLayout === 'full' ? (
              <>
                {capabilities.rotate ? (
                  <ToolButton label={t('preview.action.rotate', 'Rotate')} shortcut="R" onClick={rotate} testId="content-preview-rotate">
                    <RotateCw size={16} />
                  </ToolButton>
                ) : null}
                {capabilities.search ? (
                  <ToolButton label={t('preview.action.find', 'Find')} shortcut={`${MOD_KEY}+F`} onClick={() => setFindOpen(true)} testId="content-preview-find">
                    <Search size={16} />
                  </ToolButton>
                ) : null}
                <ToolButton label={t('preview.action.info', 'Info')} shortcut="I" onClick={() => setInfoOpen((v) => !v)} active={infoOpen} testId="content-preview-info">
                  <Info size={16} />
                </ToolButton>
              </>
            ) : null}
            {capabilities.share ? (
              <ToolButton label={t('preview.action.share', 'Share')} onClick={requestShare} testId="content-preview-share">
                <Share2 size={16} />
              </ToolButton>
            ) : null}
            {toolbarLayout === 'full' ? (
              <>
                {capabilities.openWith ? (
                  <ToolButton label={t('preview.action.openWith', 'Open with…')} onClick={requestOpenWith} testId="content-preview-open-with">
                    <ExternalLink size={16} />
                  </ToolButton>
                ) : null}
                {toolbarHostActions.map((action) => (
                  <ToolButton
                    key={action.id}
                    label={action.label}
                    disabled={action.disabled}
                    onClick={() => runHostAction(action)}
                    testId={`content-preview-host-${action.id}`}
                  >
                    {action.icon ?? <span className="px-1 text-[12px]">{action.label}</span>}
                  </ToolButton>
                ))}
                <ToolButton
                  label={toolbarPinned ? t('preview.action.unpinToolbar', 'Hide the toolbar when idle') : t('preview.action.pinToolbar', 'Keep the toolbar shown')}
                  onClick={toggleToolbarPin}
                  active={toolbarPinned}
                  testId="content-preview-pin-toolbar"
                >
                  {toolbarPinned ? <PinOff size={16} /> : <Pin size={16} />}
                </ToolButton>
              </>
            ) : null}
            <ToolButton label={t('preview.action.more', 'More')} onClick={(event) => setMenuAnchor(event.currentTarget)} testId="content-preview-more">
              <MoreHorizontal size={16} />
            </ToolButton>
            <Menu anchorEl={menuAnchor} open={Boolean(menuAnchor)} onClose={() => setMenuAnchor(null)}>
              {toolbarLayout !== 'full'
                ? [
                    capabilities.zoom && toolbarLayout === 'tiny' ? (
                      <MenuItem key="fit" onClick={() => closeMenuThen(fitToView)} data-testid="content-preview-menu-fit">
                        <Minimize2 size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.fit', 'Fit to view')}
                      </MenuItem>
                    ) : null,
                    capabilities.rotate ? (
                      <MenuItem key="rotate" onClick={() => closeMenuThen(rotate)} data-testid="content-preview-menu-rotate">
                        <RotateCw size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.rotate', 'Rotate')}
                      </MenuItem>
                    ) : null,
                    capabilities.search ? (
                      <MenuItem key="find" onClick={() => closeMenuThen(() => setFindOpen(true))} data-testid="content-preview-menu-find">
                        <Search size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.find', 'Find')}
                      </MenuItem>
                    ) : null,
                    <MenuItem key="info" onClick={() => closeMenuThen(() => setInfoOpen((v) => !v))} data-testid="content-preview-menu-info">
                      <Info size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.info', 'Info')}
                    </MenuItem>,
                    capabilities.openWith ? (
                      <MenuItem key="open-with" onClick={() => closeMenuThen(requestOpenWith)} data-testid="content-preview-menu-open-with">
                        <ExternalLink size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.openWith', 'Open with…')}
                      </MenuItem>
                    ) : null,
                    ...toolbarHostActions.map((action) => (
                      <MenuItem key={action.id} disabled={action.disabled} onClick={() => closeMenuThen(() => runHostAction(action))} data-testid={`content-preview-menu-${action.id}`}>
                        {action.icon ? <span className="mr-3 inline-flex text-[color:var(--cp-muted)]">{action.icon}</span> : null}
                        {action.label}
                      </MenuItem>
                    )),
                    <MenuItem key="pin-toolbar" onClick={() => closeMenuThen(toggleToolbarPin)} data-testid="content-preview-menu-pin-toolbar">
                      {toolbarPinned ? <PinOff size={16} className="mr-3 text-[color:var(--cp-muted)]" /> : <Pin size={16} className="mr-3 text-[color:var(--cp-muted)]" />}
                      {toolbarPinned ? t('preview.action.unpinToolbar', 'Hide the toolbar when idle') : t('preview.action.pinToolbar', 'Keep the toolbar shown')}
                    </MenuItem>,
                  ]
                : null}
              {capabilities.export ? (
                <MenuItem onClick={() => closeMenuThen(downloadCurrent)}>
                  <Download size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.download', 'Download original')}
                </MenuItem>
              ) : null}
              {zoneItem ? (
                <MenuItem onClick={() => closeMenuThen(copyItem)} data-testid="content-preview-menu-copy-file">
                  <Files size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.copyFile', 'Copy file')}
                </MenuItem>
              ) : null}
              <MenuItem onClick={() => closeMenuThen(copySource)}>
                <Copy size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {t('preview.action.copySource', 'Copy path / object id')}
              </MenuItem>
              {capabilities.fullscreen ? (
                <MenuItem onClick={() => closeMenuThen(toggleFullscreen)}>
                  <Fullscreen size={16} className="mr-3 text-[color:var(--cp-muted)]" /> {isFullscreen ? t('preview.action.exitFullscreen', 'Exit fullscreen') : t('preview.action.fullscreen', 'Fullscreen')}
                  <kbd className="ml-auto min-w-5 rounded-[4px] border border-[color:var(--cp-border)] px-1 pl-1 text-center font-sans text-[11px] leading-4 text-[color:var(--cp-muted)]">F</kbd>
                </MenuItem>
              ) : null}
              {overflowHostActions.map((action) => (
                <MenuItem key={action.id} disabled={action.disabled} onClick={() => closeMenuThen(() => runHostAction(action))} data-testid={`content-preview-menu-${action.id}`}>
                  {action.icon ? <span className="mr-3 inline-flex text-[color:var(--cp-muted)]">{action.icon}</span> : null}
                  {action.label}
                </MenuItem>
              ))}
            </Menu>
          </div>
        ) : null}

        {/* Find bar (text renderer) */}
        {findOpen && capabilities.search ? (
          <FindBar
            query={findQuery}
            count={findCount}
            active={findActive}
            onChange={(value) => patchTransient({ findQuery: value, findActive: 0 })}
            onStep={(delta) => patchTransient((prev) => ({ findActive: findCount === 0 ? 0 : (prev.findActive + delta + findCount) % findCount }))}
            onClose={() => {
              setFindOpen(false)
              rootRef.current?.focus()
            }}
            t={t}
          />
        ) : null}

        {/* Fidelity note (Pipeline results, §6.4) */}
        {status === 'ready' && current?.result?.fidelityNote && showToolbar ? (
          <div
            className="pointer-events-none absolute left-1/2 top-[56px] z-10 max-w-[80%] -translate-x-1/2 rounded-[8px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-3 py-1 text-[12px] text-[color:var(--cp-muted)]"
            style={{ transition }}
            data-testid="content-preview-fidelity"
          >
            {current.result.fidelityNote}
          </div>
        ) : null}
      </div>

      {/* Info panel (§10.6) */}
      {infoOpen && !inline ? (
        <InfoPanel
          displayName={displayName}
          item={itemInfo}
          session={currentSession}
          current={current}
          contentLabel={contentLabel}
          onClose={() => setInfoOpen(false)}
          onCopy={copySource}
          t={t}
        />
      ) : null}

      {/* Screen-reader status */}
      <span className="sr-only" aria-live="polite" data-testid="content-preview-live">
        {status === 'ready'
          ? t('preview.a11y.ready', '{{name}} ready', { name: displayName })
          : status === 'error'
            ? error?.message ?? t('preview.error.generic', 'Preview failed')
            : t('preview.a11y.loading', 'Loading {{name}}', { name: displayName })}
      </span>
    </div>
  )
}

// ─── Toolbar pieces ───

type TranslateFn = ReturnType<typeof useI18n>['t']

function ToolButton({
  label,
  shortcut,
  onClick,
  children,
  disabled,
  active,
  testId,
}: {
  label: string
  shortcut?: string
  onClick: (event: React.MouseEvent<HTMLButtonElement>) => void
  children: ReactNode
  disabled?: boolean
  active?: boolean
  testId?: string
}) {
  const title = shortcut ? `${label} (${shortcut})` : label
  return (
    <button
      type="button"
      aria-label={label}
      aria-keyshortcuts={shortcut}
      aria-pressed={active}
      title={title}
      disabled={disabled}
      data-testid={testId}
      onClick={onClick}
      className={clsx(
        'inline-flex h-8 min-w-8 shrink-0 items-center justify-center rounded-[8px] px-1.5 text-[color:var(--cp-text)] transition-colors hover:bg-[color:var(--files-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)] disabled:opacity-40',
        active && 'bg-[color:var(--files-pressed)]',
      )}
    >
      {children}
    </button>
  )
}

function ToolbarDivider() {
  return <span aria-hidden="true" className="mx-1 h-4 w-px shrink-0 bg-[color:var(--files-hairline)]" />
}

function EdgeNav({
  side,
  visible,
  disabled,
  onClick,
  label,
  transition,
}: {
  side: 'left' | 'right'
  visible: boolean
  disabled: boolean
  onClick: () => void
  label: string
  transition: string
}) {
  return (
    <button
      type="button"
      aria-label={label}
      disabled={disabled}
      data-testid={side === 'left' ? 'content-preview-prev' : 'content-preview-next'}
      onClick={onClick}
      className={clsx(
        'absolute top-1/2 z-10 flex h-12 w-10 -translate-y-1/2 items-center justify-center rounded-[10px] border border-[color:var(--files-hairline)] bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_92%,transparent)] text-[color:var(--cp-text)] shadow-[var(--files-float-shadow)] transition-colors hover:bg-[color:var(--cp-surface-opaque)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)] disabled:opacity-25',
        side === 'left' ? 'left-3' : 'right-3',
        !visible && 'pointer-events-none',
      )}
      style={{ opacity: visible ? 1 : 0, transition }}
    >
      {side === 'left' ? <ChevronLeft size={20} /> : <ChevronRight size={20} />}
    </button>
  )
}

function FindBar({
  query,
  count,
  active,
  onChange,
  onStep,
  onClose,
  t,
}: {
  query: string
  count: number
  active: number
  onChange: (value: string) => void
  onStep: (delta: number) => void
  onClose: () => void
  t: TranslateFn
}) {
  const inputRef = useRef<HTMLInputElement | null>(null)
  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])
  return (
    <div
      className="absolute right-3 top-14 z-30 flex items-center gap-1 rounded-[10px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] py-1 pl-3 pr-1 shadow-[var(--files-float-shadow)]"
      data-testid="content-preview-findbar"
      onPointerDown={(event) => event.stopPropagation()}
    >
      <Search size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
      <input
        ref={inputRef}
        value={query}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            onStep(event.shiftKey ? -1 : 1)
            event.preventDefault()
          }
        }}
        placeholder={t('preview.find.placeholder', 'Find in text')}
        aria-label={t('preview.find.placeholder', 'Find in text')}
        className="w-40 bg-transparent px-1 text-[13px] text-[color:var(--cp-text)] outline-none placeholder:text-[color:var(--cp-muted)]"
      />
      <span className="min-w-[48px] text-center text-[12px] tabular-nums text-[color:var(--cp-muted)]" data-testid="content-preview-find-count">
        {query ? `${count === 0 ? 0 : active + 1} / ${count}` : ''}
      </span>
      <ToolButton label={t('preview.find.previous', 'Previous match')} onClick={() => onStep(-1)} disabled={count === 0}>
        <ChevronLeft size={16} />
      </ToolButton>
      <ToolButton label={t('preview.find.next', 'Next match')} onClick={() => onStep(1)} disabled={count === 0}>
        <ChevronRight size={16} />
      </ToolButton>
      <ToolButton label={t('common.close', 'Close')} onClick={onClose}>
        <X size={16} />
      </ToolButton>
    </div>
  )
}

function StatusOverlay({
  status,
  progress,
  t,
  reducedMotion,
}: {
  status: PreviewStatus
  progress?: PreviewProgress
  t: TranslateFn
  reducedMotion: boolean
}) {
  // Short loads never flash a spinner (§15.2): show after a small delay.
  const [shown, setShown] = useState(false)
  useEffect(() => {
    const timer = window.setTimeout(() => setShown(true), LOADING_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [])
  if (!shown) return null
  const converting = status === 'converting'
  const percent = progress?.total ? Math.round(((progress.completed ?? 0) / progress.total) * 100) : null
  return (
    <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center" data-testid="content-preview-loading" data-phase={status}>
      <div className="flex min-w-[220px] flex-col items-center gap-3 rounded-[12px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-6 py-5 shadow-[var(--files-float-shadow)]">
        <Loader2 size={20} className={clsx('text-[color:var(--cp-muted)]', !reducedMotion && 'animate-spin')} />
        <div className="text-[13px] font-medium">
          {converting ? t('preview.status.preparing', 'Preparing preview') : t('preview.status.loading', 'Loading')}
        </div>
        {converting && progress?.message ? <div className="text-[12px] text-[color:var(--cp-muted)]">{progress.message}</div> : null}
        {converting ? (
          <div className="h-1 w-full overflow-hidden rounded-[2px] bg-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)]" role="progressbar" aria-valuenow={percent ?? undefined} aria-valuemin={0} aria-valuemax={100}>
            <div
              className="h-full w-full origin-left rounded-[2px] bg-[color:var(--cp-accent)]"
              style={{ transform: `scaleX(${percent === null ? 0.4 : percent / 100})`, transition: reducedMotion ? 'none' : 'transform 200ms var(--cp-ease-smooth)' }}
            />
          </div>
        ) : null}
      </div>
    </div>
  )
}

function ErrorState({
  error,
  contentLabel,
  t,
  onRetry,
  onOpenWith,
  onDownload,
  onInfo,
}: {
  error: PreviewErrorState
  contentLabel?: string
  t: TranslateFn
  onRetry?: () => void
  onOpenWith?: () => void
  onDownload?: () => void
  onInfo?: () => void
}) {
  const icon =
    error.kind === 'permission-denied' ? <Lock size={32} strokeWidth={1.5} /> : error.kind === 'corrupted' ? <FileX size={32} strokeWidth={1.5} /> : error.kind === 'unsupported' ? <FileQuestion size={32} strokeWidth={1.5} /> : <AlertTriangle size={32} strokeWidth={1.5} />
  const title =
    error.kind === 'permission-denied'
      ? t('preview.error.permission', 'You need access to view this content')
      : error.kind === 'corrupted'
        ? t('preview.error.corrupted', 'This file may be damaged')
        : error.kind === 'unsupported'
          ? t('preview.error.unsupported', 'Preview is not available for this content')
          : error.kind === 'not-found'
            ? t('preview.error.notFound', 'This content no longer exists')
            : error.kind === 'timeout'
              ? t('preview.error.timeout', 'Preparing the preview took too long')
              : t('preview.error.generic', 'Preview failed')
  const detail = error.kind === 'permission-denied' ? t('preview.error.permissionBody', 'Ask the owner or the host app to grant access.') : error.message
  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center p-6" data-testid="content-preview-error" data-error-kind={error.kind} role="alert">
      <div className="flex max-w-md flex-col items-center gap-3 text-center">
        <div className="text-[color:var(--cp-muted)]">{icon}</div>
        <div className="mt-1 text-[16px] font-medium">{title}</div>
        {contentLabel && error.kind !== 'permission-denied' ? (
          <div className="font-[family-name:var(--files-mono)] text-[12px] text-[color:var(--cp-muted)]">{contentLabel}</div>
        ) : null}
        {detail ? <div className="text-[13px] leading-5 text-[color:var(--cp-muted)]">{detail}</div> : null}
        <div className="mt-1 flex flex-wrap justify-center gap-2">
          {onRetry ? (
            <ActionChip onClick={onRetry} testId="content-preview-retry">
              <RefreshCw size={16} /> {t('common.retry', 'Retry')}
            </ActionChip>
          ) : null}
          {onOpenWith ? (
            <ActionChip onClick={onOpenWith} testId="content-preview-error-open-with">
              <ExternalLink size={16} /> {t('preview.action.openWith', 'Open with…')}
            </ActionChip>
          ) : null}
          {onDownload ? (
            <ActionChip onClick={onDownload}>
              <Download size={16} /> {t('preview.action.download', 'Download original')}
            </ActionChip>
          ) : null}
          {onInfo ? (
            <ActionChip onClick={onInfo}>
              <Info size={16} /> {t('preview.action.info', 'Info')}
            </ActionChip>
          ) : null}
        </div>
      </div>
    </div>
  )
}

function ActionChip({ onClick, children, testId }: { onClick: () => void; children: ReactNode; testId?: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      data-testid={testId}
      className="files-btn"
    >
      {children}
    </button>
  )
}

function InfoPanel({
  displayName,
  item,
  session,
  current,
  contentLabel,
  onClose,
  onCopy,
  t,
}: {
  displayName: string
  item: PreviewItemInfo | null
  session: ResolvedSession | null
  current: LoadState | null
  contentLabel?: string
  onClose: () => void
  onCopy: () => void
  t: TranslateFn
}) {
  const resolved = current?.resolved
  const result = current?.result
  const media = current?.media
  const rows: Array<[string, ReactNode]> = [
    [t('preview.info.name', 'Name'), displayName],
    [t('preview.info.type', 'Type'), contentLabel ?? '—'],
  ]
  if (current?.via) {
    rows.push([
      t('preview.info.rendering', 'Rendering'),
      current.via === 'direct'
        ? t('preview.info.direct', 'Direct ({{renderer}})', { renderer: result?.resultType ?? '' })
        : t('preview.info.pipeline', 'Converted to {{type}}', { type: result?.mediaType ?? '' }),
    ])
  }
  if (result?.fidelityNote) rows.push([t('preview.info.fidelity', 'Fidelity'), result.fidelityNote])
  if (resolved?.size !== undefined) rows.push([t('preview.info.size', 'Size'), formatBytes(resolved.size)])
  if (media?.width && media.height) rows.push([t('preview.info.dimensions', 'Dimensions'), `${media.width} × ${media.height}`])
  if (media?.durationSeconds !== undefined) rows.push([t('preview.info.duration', 'Duration'), formatDuration(media.durationSeconds)])
  if (media?.pageCount) rows.push([t('preview.info.pages', 'Pages'), String(media.pageCount)])
  if (item) {
    rows.push([
      t('preview.info.source', 'Source'),
      <button type="button" onClick={onCopy} className="inline-flex max-w-full items-start gap-1 text-left font-[family-name:var(--files-mono)] text-[12px] text-[color:var(--files-link)] hover:underline" title={t('common.copy', 'Copy')}>
        <span className="break-all">{sourceLabel(item.source)}</span>
        <Copy size={14} className="mt-0.5 shrink-0" />
      </button>,
    ])
  }
  if (resolved?.inputObjectId) rows.push([t('preview.info.objectId', 'Object ID'), <span className="break-all font-[family-name:var(--files-mono)] text-[12px]">{resolved.inputObjectId}</span>])
  if (resolved?.sourceObjectId && resolved.sourceObjectId !== resolved.inputObjectId) {
    rows.push([t('preview.info.sourceObject', 'Source object'), <span className="break-all font-[family-name:var(--files-mono)] text-[12px]">{resolved.sourceObjectId}</span>])
  }
  if (resolved?.versionToken) rows.push([t('preview.info.version', 'Version'), <span className="font-[family-name:var(--files-mono)] text-[12px]">{resolved.versionToken}</span>])
  if (session && item) {
    rows.push([
      t('preview.info.session', 'Session'),
      t('preview.info.sessionBody', '{{kind}} · item {{index}} of {{count}} · {{nav}}', {
        kind: session.kind,
        index: item.index + 1,
        count: session.items.length,
        nav: session.navigation,
      }),
    ])
  }
  return (
    <aside
      className="z-30 flex w-[320px] max-w-full shrink-0 flex-col border-l border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)]"
      data-testid="content-preview-info-panel"
      aria-label={t('preview.action.info', 'Info')}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-[color:var(--files-hairline)] pl-4 pr-1">
        <span className="files-label">{t('preview.action.info', 'Info')}</span>
        <ToolButton label={t('common.close', 'Close')} onClick={onClose}>
          <X size={16} />
        </ToolButton>
      </div>
      <div className="flex-1 space-y-2 overflow-y-auto px-4 py-4">
        {rows.map(([label, value]) => (
          <div key={label} className="grid grid-cols-[96px_1fr] items-start gap-2 text-[13px] leading-5 tabular-nums">
            <span className="text-[color:var(--cp-muted)]">{label}</span>
            <span className="min-w-0 select-text break-words text-[color:var(--cp-text)]">{value}</span>
          </div>
        ))}
      </div>
    </aside>
  )
}

// ─── Renderer switch ───

interface FindState {
  open: boolean
  query: string
  active: number
  onCount: (count: number) => void
}

interface ImageRendererHandle {
  zoomBy(factor: number): void
  fit(): void
  actualSize(): void
  rotate(): void
}

interface MediaRendererHandle {
  togglePlay(): void
  seekBy(seconds: number): void
}

function RendererSwitch({
  result,
  displayName,
  fitMode,
  uiMode,
  compact,
  autoPlay,
  reducedMotion,
  fontScale,
  find,
  imageRef,
  mediaRef,
  zoomRef,
  onViewChange,
  onRendered,
  onFailure,
  onDownload,
  onOpenWith,
  onActivity,
  t,
}: {
  result: PreviewResult
  displayName: string
  fitMode: PreviewFitMode
  uiMode: PreviewUIMode
  compact: boolean
  autoPlay: boolean
  reducedMotion: boolean
  fontScale: number
  find: FindState
  imageRef: React.RefObject<ImageRendererHandle | null>
  mediaRef: React.RefObject<MediaRendererHandle | null>
  zoomRef: React.RefObject<ZoomHandle | null>
  onViewChange: (view: ImageViewInfo | null) => void
  onRendered: (media?: LoadState['media'], opts?: { degraded?: boolean }) => void
  onFailure: (failure: RenderFailure) => void
  onDownload: () => void
  onOpenWith?: () => void
  onActivity: () => void
  t: TranslateFn
}) {
  switch (result.resultType) {
    case 'image':
    case 'svg':
      return (
        <ImageRenderer
          ref={imageRef}
          readRef={result.readRef}
          alt={displayName}
          fitMode={fitMode}
          reducedMotion={reducedMotion}
          onViewChange={onViewChange}
          onRendered={onRendered}
          onFailure={onFailure}
        />
      )
    case 'text':
      return <TextRenderer readRef={result.readRef} fontScale={fontScale} find={find} onRendered={onRendered} onFailure={onFailure} t={t} />
    case 'html':
      return <HtmlRenderer readRef={result.readRef} title={displayName} onRendered={onRendered} onFailure={onFailure} />
    case 'audio':
    case 'video':
      return (
        <MediaRenderer
          ref={mediaRef}
          kind={result.resultType}
          readRef={result.readRef}
          mediaType={result.mediaType}
          title={displayName}
          uiMode={uiMode}
          compact={compact}
          autoPlay={autoPlay}
          onRendered={onRendered}
          onFailure={onFailure}
          onActivity={onActivity}
        />
      )
    case 'pdf':
      return <PdfRenderer ref={zoomRef} readRef={result.readRef} title={displayName} topInset={uiMode !== 'silent' && !compact ? TOOLBAR_BAND_PX : 0} onViewChange={onViewChange} onRendered={onRendered} onFailure={onFailure} onDownload={onDownload} onOpenWith={onOpenWith} t={t} />
    case 'diagram':
      return (
        <DiagramRenderer
          readRef={result.readRef}
          stage={(page, key, onStageRendered) => (
            <ImageRenderer
              key={key}
              ref={imageRef}
              readRef={page}
              alt={displayName}
              fitMode={fitMode}
              reducedMotion={reducedMotion}
              onViewChange={onViewChange}
              onRendered={onStageRendered}
              onFailure={onFailure}
            />
          )}
          textView={<TextRenderer readRef={result.readRef} fontScale={fontScale} find={find} onRendered={onRendered} onFailure={onFailure} t={t} />}
          onRendered={onRendered}
          onFailure={onFailure}
          t={t}
        />
      )
    case 'sheet': {
      const type = result.mediaType.split(';')[0].trim().toLowerCase()
      const delimited = type === 'text/csv' || type === 'text/tab-separated-values'
      const name = type === 'text/tab-separated-values' && !/\.tsv$/i.test(displayName) ? `${displayName}.tsv` : displayName
      return (
        <SheetRenderer
          ref={zoomRef}
          readRef={result.readRef}
          kind={delimited ? 'csv' : 'xlsx'}
          name={name}
          compact={compact}
          textView={delimited ? <TextRenderer readRef={result.readRef} fontScale={fontScale} find={find} onRendered={onRendered} onFailure={onFailure} t={t} /> : undefined}
          onViewChange={onViewChange}
          onRendered={onRendered}
          onFailure={onFailure}
          onDownload={onDownload}
          onOpenWith={onOpenWith}
          t={t}
        />
      )
    }
    default:
      return null
  }
}

// ─── Image / SVG renderer (§10.7 图片 / SVG) ───

interface ImageViewState {
  mode: 'fit' | 'cover' | 'custom'
  scale: number
  tx: number
  ty: number
  rotation: number
}

function fitScaleFor(container: { width: number; height: number }, natural: { width: number; height: number }, rotation: number, mode: 'fit' | 'cover') {
  const rotated = rotation % 180 !== 0
  const w = rotated ? natural.height : natural.width
  const h = rotated ? natural.width : natural.height
  if (!w || !h || !container.width || !container.height) return 1
  const sx = container.width / w
  const sy = container.height / h
  return mode === 'cover' ? Math.max(sx, sy) : Math.min(sx, sy)
}

function clampPan(state: ImageViewState, container: { width: number; height: number }, natural: { width: number; height: number }): ImageViewState {
  const rotated = state.rotation % 180 !== 0
  const w = (rotated ? natural.height : natural.width) * state.scale
  const h = (rotated ? natural.width : natural.height) * state.scale
  const limitX = Math.max(0, (w - container.width) / 2)
  const limitY = Math.max(0, (h - container.height) / 2)
  return { ...state, tx: clamp(state.tx, -limitX, limitX), ty: clamp(state.ty, -limitY, limitY) }
}

function ImageRenderer({
  ref,
  readRef,
  alt,
  fitMode,
  reducedMotion,
  onViewChange,
  onRendered,
  onFailure,
}: {
  ref: Ref<ImageRendererHandle>
  readRef: PreviewReadRef
  alt: string
  fitMode: PreviewFitMode
  reducedMotion: boolean
  onViewChange: (view: ImageViewInfo | null) => void
  onRendered: (media?: LoadState['media']) => void
  onFailure: (failure: RenderFailure) => void
}) {
  const url = useContentUrl(readRef)
  const containerRef = useRef<HTMLDivElement | null>(null)
  const container = useElementSize(containerRef)
  const [natural, setNatural] = useState<{ width: number; height: number } | null>(null)
  const initialMode: ImageViewState['mode'] = fitMode === 'cover' ? 'cover' : fitMode === 'actual-size' ? 'custom' : 'fit'
  const [view, setView] = useState<ImageViewState>({ mode: initialMode, scale: 1, tx: 0, ty: 0, rotation: 0 })
  const [dragging, setDragging] = useState(false)
  const pointers = useRef(new Map<number, { x: number; y: number }>())
  const drag = useRef<{ x: number; y: number; moved: boolean; button: number } | null>(null)
  const pinch = useRef<{ distance: number; scale: number } | null>(null)
  const suppressContextMenu = useRef(false)

  // Fit / cover follow the container; custom keeps its focal point but stays clamped (§10.5).
  const effective = useMemo<ImageViewState>(() => {
    if (!natural) return { ...view, scale: 1, tx: 0, ty: 0 }
    if (view.mode === 'custom') return clampPan(view, container, natural)
    return { ...view, scale: fitScaleFor(container, natural, view.rotation, view.mode), tx: 0, ty: 0 }
  }, [container, natural, view])

  const reportedView = useRef<ImageViewInfo | null>(null)
  useEffect(() => {
    if (!natural) return
    const fitScale = fitScaleFor(container, natural, effective.rotation, 'fit')
    const view: ImageViewInfo = {
      scale: effective.scale,
      mode: effective.mode === 'custom' && Math.abs(effective.scale - fitScale) < 0.001 ? 'fit' : effective.mode,
      rotation: effective.rotation,
    }
    // Panning changes tx/ty only; skip the parent update when the reported
    // info (scale/mode/rotation) is unchanged.
    if (sameImageView(reportedView.current, view)) return
    reportedView.current = view
    onViewChange(view)
  }, [container, effective, natural, onViewChange])

  const applyZoom = useCallback(
    (factor: number, focal?: { x: number; y: number }) => {
      if (!natural) return
      setView((prev) => {
        const base =
          prev.mode === 'custom'
            ? clampPan(prev, container, natural)
            : { ...prev, scale: fitScaleFor(container, natural, prev.rotation, prev.mode), tx: 0, ty: 0 }
        const nextScale = clamp(base.scale * factor, MIN_SCALE, MAX_SCALE)
        const ratio = nextScale / base.scale
        // Keep the point under the cursor stationary.
        const fx = focal ? focal.x - container.width / 2 : 0
        const fy = focal ? focal.y - container.height / 2 : 0
        const tx = fx - (fx - base.tx) * ratio
        const ty = fy - (fy - base.ty) * ratio
        return clampPan({ ...base, mode: 'custom', scale: nextScale, tx, ty }, container, natural)
      })
    },
    [container, natural],
  )
  const fit = useCallback(() => setView((prev) => ({ ...prev, mode: 'fit', tx: 0, ty: 0 })), [])
  const actualSize = useCallback(() => {
    if (!natural) return
    setView((prev) => clampPan({ ...prev, mode: 'custom', scale: 1, tx: 0, ty: 0 }, container, natural))
  }, [container, natural])
  const rotate = useCallback(() => {
    setView((prev) => (prev.mode === 'custom' ? { ...prev, rotation: (prev.rotation + 90) % 360 } : { ...prev, rotation: (prev.rotation + 90) % 360, tx: 0, ty: 0 }))
  }, [])
  const panBy = useCallback(
    (dx: number, dy: number) => {
      if (!natural) return
      setView((prev) => (prev.mode === 'custom' ? clampPan({ ...prev, tx: prev.tx + dx, ty: prev.ty + dy }, container, natural) : prev))
    },
    [container, natural],
  )

  useImperativeHandle(ref, () => ({ zoomBy: (factor) => applyZoom(factor), fit, actualSize, rotate }), [actualSize, applyZoom, fit, rotate])

  // Wheel: Ctrl/⌘ zooms at the cursor, plain wheel pans (trackpad). Non-passive
  // so the host page never scrolls; the handler is read through a ref so the
  // listener is attached once.
  const wheelHandler = useRef<(event: WheelEvent) => void>(() => {})
  useEffect(() => {
    wheelHandler.current = (event) => {
      const element = containerRef.current
      if (!element) return
      const rect = element.getBoundingClientRect()
      if (event.ctrlKey || event.metaKey) {
        applyZoom(Math.exp(-event.deltaY * 0.0025), { x: event.clientX - rect.left, y: event.clientY - rect.top })
      } else {
        panBy(-event.deltaX, -event.deltaY)
      }
    }
  }, [applyZoom, panBy])
  useEffect(() => {
    const element = containerRef.current
    if (!element) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      wheelHandler.current(event)
    }
    element.addEventListener('wheel', onWheel, { passive: false })
    return () => element.removeEventListener('wheel', onWheel)
  }, [])

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType === 'mouse' && event.button !== 0 && event.button !== 2) return
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY })
    event.currentTarget.setPointerCapture(event.pointerId)
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()]
      pinch.current = { distance: Math.hypot(a.x - b.x, a.y - b.y), scale: effective.scale }
      drag.current = null
      return
    }
    drag.current = { x: event.clientX, y: event.clientY, moved: false, button: event.button }
    if (event.button === 2) event.preventDefault()
  }

  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pointers.current.has(event.pointerId)) return
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY })
    if (!natural) return
    if (pinch.current && pointers.current.size >= 2) {
      const [a, b] = [...pointers.current.values()]
      const distance = Math.hypot(a.x - b.x, a.y - b.y)
      const rect = event.currentTarget.getBoundingClientRect()
      const focal = { x: (a.x + b.x) / 2 - rect.left, y: (a.y + b.y) / 2 - rect.top }
      const target = clamp((pinch.current.scale * distance) / pinch.current.distance, MIN_SCALE, MAX_SCALE)
      applyZoom(target / effective.scale, focal)
      return
    }
    const d = drag.current
    if (!d) return
    const dx = event.clientX - d.x
    const dy = event.clientY - d.y
    if (!d.moved && Math.hypot(dx, dy) < 3) return
    if (!d.moved) {
      d.moved = true
      setDragging(true)
    }
    d.x = event.clientX
    d.y = event.clientY
    if (d.button === 2) suppressContextMenu.current = true
    setView((prev) => {
      const base = prev.mode === 'custom' ? prev : { ...prev, mode: 'custom' as const, scale: fitScaleFor(container, natural, prev.rotation, prev.mode) }
      return clampPan({ ...base, tx: base.tx + dx, ty: base.ty + dy }, container, natural)
    })
  }

  const onPointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    pointers.current.delete(event.pointerId)
    if (pointers.current.size < 2) pinch.current = null
    if (pointers.current.size === 0) {
      drag.current = null
      setDragging(false)
    }
  }

  const transform = `translate(${effective.tx}px, ${effective.ty}px) rotate(${effective.rotation}deg) scale(${effective.scale})`
  return (
    <div
      ref={containerRef}
      className="absolute inset-0 overflow-hidden"
      style={{ touchAction: 'none', cursor: natural ? (dragging ? 'grabbing' : 'grab') : 'default' }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={() => {
        if (effective.mode === 'fit') actualSize()
        else fit()
      }}
      onContextMenu={(event) => {
        if (suppressContextMenu.current) {
          suppressContextMenu.current = false
          event.preventDefault()
        }
      }}
      data-testid="content-preview-image-stage"
    >
      {url ? (
        <img
          src={url}
          alt={alt}
          draggable={false}
          decoding="async"
          data-testid="content-preview-image"
          className="absolute left-1/2 top-1/2 max-w-none select-none"
          style={{
            width: natural?.width || undefined,
            height: natural?.height || undefined,
            transform: `translate(-50%, -50%) ${transform}`,
            transformOrigin: 'center',
            transition: reducedMotion || dragging ? 'none' : 'transform 120ms var(--cp-ease-smooth, ease)',
            opacity: natural ? 1 : 0,
            imageRendering: effective.scale > 4 ? 'pixelated' : 'auto',
          }}
          onLoad={(event) => {
            const img = event.currentTarget
            const width = img.naturalWidth || Math.min(container.width || 800, 800)
            const height = img.naturalHeight || Math.min(container.height || 600, 600)
            setNatural({ width, height })
            onRendered({ width: img.naturalWidth || undefined, height: img.naturalHeight || undefined })
          }}
          onError={() => onFailure('unsupported-encoding')}
        />
      ) : null}
    </div>
  )
}

// ─── Text renderer (§10.7 文本) ───

function TextRenderer({
  readRef,
  fontScale,
  find,
  onRendered,
  onFailure,
  t,
}: {
  readRef: PreviewReadRef
  fontScale: number
  find: FindState
  onRendered: (media?: LoadState['media']) => void
  onFailure: (failure: RenderFailure) => void
  t: TranslateFn
}) {
  const [text, setText] = useState<{ value: string; truncated: boolean; total: number | null } | null>(null)
  const scrollRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const controller = new AbortController()
    void readText(readRef, TEXT_READ_BUDGET, controller.signal)
      .then((result) => {
        if (controller.signal.aborted) return
        setText({ value: result.text, truncated: result.truncated, total: result.totalLength })
        onRendered()
      })
      .catch((err) => {
        if (controller.signal.aborted || isAbortError(err)) return
        onFailure('network')
      })
    return () => controller.abort()
  }, [onFailure, onRendered, readRef])

  const matches = useMemo(() => {
    if (!text || !find.open || !find.query) return []
    const needle = find.query.toLowerCase()
    const hay = text.value.toLowerCase()
    const out: number[] = []
    let from = 0
    while (out.length < 5000) {
      const at = hay.indexOf(needle, from)
      if (at < 0) break
      out.push(at)
      from = at + needle.length
    }
    return out
  }, [find.open, find.query, text])

  const { onCount } = find
  useEffect(() => {
    onCount(matches.length)
  }, [matches.length, onCount])

  useEffect(() => {
    if (!matches.length) return
    const active = scrollRef.current?.querySelector<HTMLElement>('[data-find-active="true"]')
    active?.scrollIntoView({ block: 'center' })
  }, [find.active, matches])

  const content = useMemo(() => {
    if (!text) return null
    if (!matches.length) return text.value
    const parts: ReactNode[] = []
    let cursor = 0
    const length = find.query.length
    matches.forEach((at, i) => {
      if (at > cursor) parts.push(text.value.slice(cursor, at))
      const active = i === find.active
      parts.push(
        <mark
          key={at}
          data-find-active={active ? 'true' : 'false'}
          className={clsx('rounded-sm', active ? 'bg-[color:var(--cp-warning)] text-black' : 'bg-[color:color-mix(in_srgb,var(--cp-warning)_45%,transparent)] text-inherit')}
        >
          {text.value.slice(at, at + length)}
        </mark>,
      )
      cursor = at + length
    })
    if (cursor < text.value.length) parts.push(text.value.slice(cursor))
    return parts
  }, [find.active, find.query.length, matches, text])

  return (
    <>
      <div ref={scrollRef} className="desktop-scrollbar absolute inset-0 overflow-auto bg-[color:var(--cp-surface-opaque)]" data-testid="content-preview-text">
        <pre
          className="select-text whitespace-pre-wrap break-words px-6 pb-10 pt-12 font-mono text-[color:var(--cp-text)]"
          style={{ fontSize: `${13 * fontScale}px`, lineHeight: 1.6, tabSize: 4 }}
        >
          {content}
        </pre>
      </div>
      {text?.truncated ? (
        // Bottom edge: never under the toolbar, never over the first lines (§15.2 progressive reads).
        <div
          className="pointer-events-none absolute inset-x-0 bottom-0 z-10 border-t border-[color:var(--files-hairline)] bg-[color:color-mix(in_srgb,var(--cp-warning)_16%,var(--cp-surface-opaque))] px-4 py-2 text-center text-[12px] tabular-nums text-[color:var(--cp-text)]"
          data-testid="content-preview-text-truncated"
        >
          {t('preview.text.truncated', 'Showing the first {{shown}} of {{total}}', {
            shown: formatBytes(TEXT_READ_BUDGET),
            total: text.total !== null ? formatBytes(text.total) : '?',
          })}
        </div>
      ) : null}
    </>
  )
}

// ─── HTML renderer (§10.7 HTML, §16 sandbox) ───

const HTML_CSP = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; font-src data:; media-src data: blob:; frame-src 'none'; form-action 'none';"

function sandboxHtml(html: string): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${HTML_CSP}"><base target="_self">`
  if (/<head[^>]*>/i.test(html)) return html.replace(/<head[^>]*>/i, (m) => `${m}${meta}`)
  if (/<html[^>]*>/i.test(html)) return html.replace(/<html[^>]*>/i, (m) => `${m}<head>${meta}</head>`)
  return `<!DOCTYPE html><html><head>${meta}</head><body>${html}</body></html>`
}

function HtmlRenderer({
  readRef,
  title,
  onRendered,
  onFailure,
}: {
  readRef: PreviewReadRef
  title: string
  onRendered: (media?: LoadState['media']) => void
  onFailure: (failure: RenderFailure) => void
}) {
  const [doc, setDoc] = useState<string | null>(null)
  useEffect(() => {
    const controller = new AbortController()
    void readAll(readRef, 8 * 1024 * 1024, controller.signal)
      .then((blob) => blob.text())
      .then((html) => {
        if (controller.signal.aborted) return
        setDoc(sandboxHtml(html))
      })
      .catch((err) => {
        if (controller.signal.aborted || isAbortError(err)) return
        onFailure(err instanceof PreviewError && err.code === 'TOO_LARGE' ? 'runtime' : 'network')
      })
    return () => controller.abort()
  }, [onFailure, readRef])
  if (doc === null) return null
  return (
    <iframe
      title={title}
      srcDoc={doc}
      // No scripts, no same-origin, no forms, no popups, no top navigation (§16.4).
      sandbox=""
      referrerPolicy="no-referrer"
      className="absolute inset-0 h-full w-full border-0 bg-white"
      data-testid="content-preview-html"
      onLoad={() => onRendered()}
    />
  )
}

// ─── Audio / Video renderer (§10.7 音频 / 视频) ───

function MediaRenderer({
  ref,
  kind,
  readRef,
  mediaType,
  title,
  uiMode,
  compact,
  autoPlay,
  onRendered,
  onFailure,
  onActivity,
}: {
  ref: Ref<MediaRendererHandle>
  kind: 'audio' | 'video'
  readRef: PreviewReadRef
  mediaType: string
  title: string
  uiMode: PreviewUIMode
  compact: boolean
  autoPlay: boolean
  onRendered: (media?: LoadState['media']) => void
  onFailure: (failure: RenderFailure) => void
  onActivity: () => void
}) {
  const url = useContentUrl(readRef)
  const elementRef = useRef<HTMLVideoElement | HTMLAudioElement | null>(null)
  useImperativeHandle(
    ref,
    () => ({
      togglePlay: () => {
        const el = elementRef.current
        if (!el) return
        if (el.paused) void el.play().catch(() => {})
        else el.pause()
        onActivity()
      },
      seekBy: (seconds) => {
        const el = elementRef.current
        if (!el || !Number.isFinite(el.duration)) return
        el.currentTime = clamp(el.currentTime + seconds, 0, el.duration)
        onActivity()
      },
    }),
    [onActivity],
  )
  const handleError = () => {
    const el = elementRef.current
    const code = el?.error?.code
    if (code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED) onFailure('unsupported-encoding')
    else if (code === MediaError.MEDIA_ERR_DECODE) onFailure('corrupted')
    else onFailure('network')
  }
  const handleMetadata = () => {
    const el = elementRef.current
    if (!el) return
    const video = el as HTMLVideoElement
    if (kind === 'video' && !video.videoWidth && !video.videoHeight) {
      onFailure('unsupported-encoding')
      return
    }
    onRendered({ width: video.videoWidth || undefined, height: video.videoHeight || undefined, durationSeconds: Number.isFinite(el.duration) ? el.duration : undefined })
  }
  if (!url) return null
  const showControls = uiMode !== 'silent'
  if (kind === 'video') {
    return (
      <div className="absolute inset-0 flex items-center justify-center bg-black" data-testid="content-preview-video-stage">
        <video
          ref={(el) => {
            elementRef.current = el
          }}
          src={url}
          title={title}
          controls={showControls}
          autoPlay={autoPlay}
          playsInline
          preload="metadata"
          className="max-h-full max-w-full"
          data-testid="content-preview-video"
          onLoadedMetadata={handleMetadata}
          onError={handleError}
          onClick={(event) => {
            if (!showControls) {
              const el = event.currentTarget
              if (el.paused) void el.play().catch(() => {})
              else el.pause()
            }
          }}
        >
          <source src={url} type={mediaType} />
        </video>
      </div>
    )
  }
  if (compact) {
    return (
      <div className="absolute inset-0 flex flex-col items-stretch justify-center gap-1 px-2" data-testid="content-preview-audio-stage">
        <div className="truncate px-1 text-[12px] font-medium">{title}</div>
        <audio
          ref={(el) => {
            elementRef.current = el
          }}
          src={url}
          title={title}
          controls={showControls}
          autoPlay={autoPlay}
          preload="metadata"
          className="w-full"
          data-testid="content-preview-audio"
          onLoadedMetadata={handleMetadata}
          onError={handleError}
        />
      </div>
    )
  }
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center gap-6 p-6" data-testid="content-preview-audio-stage">
      <div className="flex h-24 w-24 items-center justify-center rounded-[16px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)]" style={{ color: 'var(--files-type-violet)' }}>
        <Music size={40} strokeWidth={1.5} />
      </div>
      <div className="max-w-md truncate text-[15px] font-medium">{title}</div>
      <audio
        ref={(el) => {
          elementRef.current = el
        }}
        src={url}
        controls={showControls}
        autoPlay={autoPlay}
        preload="metadata"
        className="w-full max-w-md"
        data-testid="content-preview-audio"
        onLoadedMetadata={handleMetadata}
        onError={handleError}
      />
    </div>
  )
}

// ─── PDF (§10.7 / §23.3.1): the Runtime's viewer on a desktop, pdf.js on touch devices and where there is none ───

function PdfRenderer({
  ref,
  readRef,
  title,
  topInset,
  onViewChange,
  onRendered,
  onFailure,
  onDownload,
  onOpenWith,
  t,
}: {
  ref: Ref<ZoomHandle>
  readRef: PreviewReadRef
  title: string
  /** The browser's viewer draws its own toolbar at the top: it starts below the component's (a fixed band, so the
   * content never moves when the component's toolbar shows or hides). */
  topInset: number
  onViewChange: (view: ImageViewInfo | null) => void
  onRendered: (media?: LoadState['media'], opts?: { degraded?: boolean }) => void
  onFailure: (failure: RenderFailure) => void
  onDownload: () => void
  onOpenWith?: () => void
  t: TranslateFn
}) {
  const [pdfjs, setPdfjs] = useState(() => pdfUsesPdfjs())
  const fallBack = useCallback(() => setPdfjs(true), [])
  if (pdfjs) {
    return <PdfJsRenderer ref={ref} readRef={readRef} title={title} onViewChange={onViewChange} onRendered={onRendered} onFailure={onFailure} onDownload={onDownload} onOpenWith={onOpenWith} t={t} />
  }
  return <PDFIframeRenderer ref={ref} readRef={readRef} title={title} topInset={topInset} onRendered={onRendered} onUnavailable={fallBack} />
}

function PDFIframeRenderer({
  ref,
  readRef,
  title,
  topInset,
  onRendered,
  onUnavailable,
}: {
  ref: Ref<ZoomHandle>
  readRef: PreviewReadRef
  title: string
  topInset: number
  onRendered: (media?: LoadState['media']) => void
  /** The viewer did not take the content: pdf.js draws it instead. */
  onUnavailable: () => void
}) {
  const [phase, setPhase] = useState<'preflight' | 'loading' | 'ready'>('preflight')
  // The viewer is opaque: a zoom asked through the component reloads it with the open parameter.
  const [zoom, setZoom] = useState<number | null>(null)
  const url = useContentUrl(readRef)
  const rendered = useRef(false)
  useImperativeHandle(ref, () => ({ zoomBy: (factor) => setZoom((value) => clamp((value ?? 1) * factor, 0.25, 5)), fit: () => setZoom(null) }), [])

  useEffect(() => {
    const controller = new AbortController()
    void (async () => {
      try {
        // URL pre-check: the bytes must be a PDF and the response must not force a download.
        const probe = await readProbe(readRef, 1024, controller.signal)
        const head = String.fromCharCode(...probe.bytes.subarray(0, 5))
        const type = (probe.contentType ?? '').split(';')[0].trim().toLowerCase()
        if (head !== '%PDF-' && type !== 'application/pdf') {
          onUnavailable()
          return
        }
      } catch {
        // A failed pre-check is not proof of failure — let the viewer try.
      }
      if (!controller.signal.aborted) setPhase('loading')
    })()
    return () => controller.abort()
  }, [onUnavailable, readRef])

  useEffect(() => {
    if (phase !== 'loading') return
    const timer = window.setTimeout(() => {
      if (!rendered.current) onUnavailable()
    }, PDF_LOAD_TIMEOUT_MS)
    return () => window.clearTimeout(timer)
  }, [onUnavailable, phase])

  if (phase === 'preflight' || !url) return null
  const src = zoom === null ? url : `${url}#zoom=${Math.round(zoom * 100)}`
  return (
    // The viewer is an opaque renderer: no DOM access, no private messaging (§23.3.1 rule 5).
    // Chromium disables its PDF plugin inside sandboxed frames, so `sandbox` is deliberately absent;
    // the iframe stays on a system-controlled read endpoint with a short-lived, permission-bound URL.
    <iframe
      key={src}
      title={title}
      src={src}
      referrerPolicy="no-referrer"
      className="absolute inset-x-0 bottom-0 w-full border-0 bg-[color:var(--cp-surface-2-opaque)]"
      style={{ top: topInset, height: `calc(100% - ${topInset}px)` }}
      data-testid="content-preview-pdf"
      onLoad={() => {
        rendered.current = true
        setPhase('ready')
        onRendered()
      }}
      onError={onUnavailable}
    />
  )
}
