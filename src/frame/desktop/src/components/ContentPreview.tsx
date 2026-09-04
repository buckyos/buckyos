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
import clsx from 'clsx'
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  ExternalLink,
  FileQuestion,
  FileX,
  Fullscreen,
  Info,
  Loader2,
  Lock,
  Maximize2,
  Minimize2,
  MoreHorizontal,
  Music,
  RefreshCw,
  RotateCw,
  Search,
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
import { useI18n } from '../i18n/provider'
import { readAll, readProbe, readText } from './preview/io'
import {
  classifyMedia,
  contentLabelOf,
  decideDirect,
  detectRuntimeProfile,
  ensureRuntimeProfile,
  TEXT_READ_BUDGET,
  type MediaClassification,
} from './preview/mediaTypes'
import { ensurePreviewProvider } from './preview/provider'
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
  isAbortError,
  isBlobRef,
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
  type PreviewSessionContext,
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
  PreviewStatus,
  PreviewUIMode,
} from './preview/types'

// ─── Public surface ───

export interface ContentPreviewProps {
  source: ContentRef
  session?: PreviewSessionContext
  /** Auto (default): content only until the user shows intent. */
  uiMode?: PreviewUIMode
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
  if (isBlobRef(ref)) return ref.value.name ?? 'blob'
  return ref.kind
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
    return { kind, code: err.code, message: err.message, retryable: err.retryable, contentLabel }
  }
  if (err instanceof DOMException && err.name === 'AbortError') {
    return { kind: 'cancelled', code: 'CANCELLED', message: 'Cancelled', retryable: false, contentLabel }
  }
  return { kind: 'error', code: 'INTERNAL', message: err instanceof Error ? err.message : String(err), retryable: true, contentLabel }
}

function unsupportedState(reason: PreviewUnsupportedReason, contentLabel: string, detail?: string): PreviewErrorState {
  return { kind: 'unsupported', code: `UNSUPPORTED_${reason.toUpperCase().replace(/-/g, '_')}`, message: detail ?? '', retryable: false, contentLabel }
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

// ─── Component ───

export function ContentPreview(props: ContentPreviewProps) {
  const {
    source,
    session: sessionContext,
    uiMode = 'auto',
    fitMode = 'contain',
    hostActions,
    onRequestExit,
    onRequestOpenWith,
    allowFullscreen = true,
    allowExport = true,
    prefetchAdjacent = true,
    autoFocus = false,
    className,
    style,
    ref,
  } = props
  const { t } = useI18n()
  const reducedMotion = useReducedMotion()
  // Latest props for async work and event emitters (never read during render).
  const latest = useRef(props)
  useEffect(() => {
    latest.current = props
  })
  const rootRef = useRef<HTMLDivElement | null>(null)

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
          if (probeErr instanceof PreviewError && (probeErr.code === 'PERMISSION_DENIED' || probeErr.code === 'NOT_FOUND')) throw probeErr
          // A failed probe is not fatal: fall back to hints.
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
        let direct = skipDirect ? ({ ok: false, reason: 'runtime-unsupported' } as const) : decideDirect(classification, resolved.size, runtime)
        // PDF P0 (§23.3.1): the original PDF only ever takes the Direct path — when the
        // Runtime cannot embed it, the PDFIframeRenderer shows the degraded state itself.
        if (!direct.ok && direct.reason === 'runtime-unsupported' && classification.rendererType === 'pdf' && !skipDirect) {
          direct = { ok: true }
        }

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
        let state = await provider.ensurePreviewWork({
          source: resolved,
          runtimeProfile: runtime,
          targetProfile,
          options: { signal, retry: !!retry, expectedAttemptId: retry?.expectedAttemptId },
        })
        if (signal.aborted) return
        if ('kind' in state) {
          const failure = skipDirect
          const label = contentLabelOf(classification)
          if (failure === 'corrupted' || (failure && classification.confirmed)) {
            update({ phase: 'error', resolved, classification, error: { kind: 'corrupted', code: 'CORRUPTED', message: t('preview.error.corruptedBody', 'The content could not be decoded and may be damaged'), retryable: false, contentLabel: label } })
          } else {
            update({ phase: 'error', resolved, classification, error: unsupportedState(state.reason, label, state.detail) })
          }
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
      const state = load && load.key === loadKey ? load : null
      if (state?.via === 'direct' && (failure === 'unsupported-encoding' || failure === 'corrupted') && !skipDirectRef.current.has(item.id)) {
        // Direct decode failed → back to the Pipeline Planner (§23.3).
        skipDirectRef.current.set(item.id, failure)
        setFallbackNonce((n) => n + 1)
        return
      }
      const label = state?.classification ? contentLabelOf(state.classification) : undefined
      const err: PreviewErrorState =
        failure === 'network'
          ? { kind: 'error', code: 'NETWORK', message: t('preview.error.networkBody', 'The content could not be loaded'), retryable: true, contentLabel: label }
          : failure === 'runtime'
            ? { kind: 'unsupported', code: 'RUNTIME_UNSUPPORTED', message: t('preview.error.runtimeBody', 'This Runtime cannot display the content'), retryable: false, contentLabel: label }
            : { kind: 'corrupted', code: 'CORRUPTED', message: t('preview.error.corruptedBody', 'The content could not be decoded and may be damaged'), retryable: false, contentLabel: label }
      setLoad((prev) => (prev && prev.key === loadKey ? { ...prev, phase: 'error', error: err } : prev))
    },
    [item, load, loadKey, t],
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
    setRetryNonce((n) => n + 1)
  }, [current?.work, item])

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
  // Visible: always; Silent: never; Auto: shown on intent, hidden after idle.
  const [autoVisible, setAutoVisible] = useState(false)
  const uiVisible = uiMode === 'visible' ? true : uiMode === 'silent' ? false : autoVisible
  const hideTimer = useRef<number | null>(null)
  const uiPinnedRef = useRef(false)
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const scheduleHide = useCallback(() => {
    if (hideTimer.current !== null) window.clearTimeout(hideTimer.current)
    hideTimer.current = window.setTimeout(() => {
      hideTimer.current = null
      if (!uiPinnedRef.current) setAutoVisible(false)
    }, AUTO_HIDE_MS)
  }, [])
  const showUi = useCallback(() => {
    if (uiMode !== 'auto') return
    setAutoVisible(true)
    scheduleHide()
  }, [scheduleHide, uiMode])
  useEffect(() => () => {
    if (hideTimer.current !== null) window.clearTimeout(hideTimer.current)
  }, [])
  const pinUi = useCallback(
    (pinned: boolean) => {
      uiPinnedRef.current = pinned
      if (pinned) {
        if (hideTimer.current !== null) window.clearTimeout(hideTimer.current)
        hideTimer.current = null
      } else if (uiMode === 'auto') {
        scheduleHide()
      }
    },
    [scheduleHide, uiMode],
  )
  useEffect(() => {
    // Open menus keep the toolbar around.
    pinUi(Boolean(menuAnchor))
  }, [menuAnchor, pinUi])

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
  const [fontScale, setFontScale] = useState(1)

  // ── item info & events ──
  const itemInfo: PreviewItemInfo | null = useMemo(() => {
    if (!item || !currentSession) return null
    return { index, count: currentSession.items.length, id: item.id, title: item.title ?? refDisplayName(item.source), source: item.source }
  }, [currentSession, index, item])
  const displayName = current?.resolved?.displayName ?? itemInfo?.title ?? refDisplayName(source)

  const capabilities: PreviewCapabilities = useMemo(() => {
    const ready = status === 'ready'
    const isImage = ready && (rendererType === 'image' || rendererType === 'svg')
    const isText = ready && rendererType === 'text'
    const hasRead = Boolean(current?.result?.readRef)
    return {
      zoom: isImage || isText,
      pan: isImage,
      textSelection: ready && (rendererType === 'text' || rendererType === 'html'),
      search: isText,
      playback: ready && (rendererType === 'audio' || rendererType === 'video'),
      rotate: isImage,
      previous: canPrev,
      next: canNext,
      export: allowExport && hasRead,
      openWith: Boolean(onRequestOpenWith),
      fullscreen: fullscreenAvailable,
    }
  }, [allowExport, canNext, canPrev, current?.result?.readRef, fullscreenAvailable, onRequestOpenWith, rendererType, status])

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

  const requestOpenWith = useCallback(() => {
    if (!itemInfo) return
    invokeAction('open-with')
    latest.current.onRequestOpenWith?.({ item: itemInfo, resolved: current?.resolved, mediaType: current?.result?.mediaType ?? current?.classification?.mediaType })
  }, [current?.classification?.mediaType, current?.resolved, current?.result?.mediaType, invokeAction, itemInfo, latest])

  const downloadCurrent = useCallback(() => {
    const readRef = current?.result?.readRef ?? current?.resolved?.readRef
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

  const zoomIn = useCallback(() => {
    if (rendererType === 'text') setFontScale((s) => clamp(s * 1.15, 0.5, 4))
    else imageRef.current?.zoomBy(ZOOM_STEP)
  }, [rendererType])
  const zoomOut = useCallback(() => {
    if (rendererType === 'text') setFontScale((s) => clamp(s / 1.15, 0.5, 4))
    else imageRef.current?.zoomBy(1 / ZOOM_STEP)
  }, [rendererType])
  const fitToView = useCallback(() => {
    if (rendererType === 'text') setFontScale(1)
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
    if (inField) return
    showUi()
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
    } else if (key === 'i' && !mod) {
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
  const showToolbar = uiMode !== 'silent' && uiVisible
  const transition = reducedMotion ? 'none' : 'opacity 160ms var(--cp-ease-smooth, ease)'
  const sessionCount = currentSession?.items.length ?? 0

  const toolbarHostActions = (hostActions ?? []).filter((a) => a.placement !== 'overflow')
  const overflowHostActions = (hostActions ?? []).filter((a) => a.placement === 'overflow')
  const zoomPercent = rendererType === 'text' ? Math.round(fontScale * 100) : imageView ? Math.round(imageView.scale * 100) : null

  return (
    <div
      ref={rootRef}
      data-testid={props['data-testid'] ?? 'content-preview'}
      data-status={status}
      data-renderer={rendererType ?? ''}
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
        'relative isolate flex h-full w-full min-h-0 select-none overflow-hidden bg-[color:var(--cp-bg-strong)] text-[color:var(--cp-text)] outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[color:var(--cp-focus-ring)]',
        className,
      )}
      style={style}
      onKeyDown={handleKeyDown}
      onPointerMove={handlePointerActivity}
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
              uiMode={uiMode}
              reducedMotion={reducedMotion}
              fontScale={fontScale}
              find={{ open: findOpen, query: findQuery, active: findActive, onCount: setFindCount }}
              imageRef={imageRef}
              mediaRef={mediaRef}
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
            onRetry={error.retryable ? retry : undefined}
            onOpenWith={capabilities.openWith ? requestOpenWith : undefined}
            onDownload={allowExport && current?.resolved && error.kind !== 'permission-denied' && error.kind !== 'not-found' ? downloadCurrent : undefined}
            onInfo={error.kind !== 'permission-denied' && current?.resolved ? () => setInfoOpen(true) : undefined}
          />
        ) : null}

        {/* Session edge navigation */}
        {sessionCount > 1 && uiMode !== 'silent' ? (
          <>
            <EdgeNav side="left" visible={showToolbar} disabled={!canPrev} onClick={() => step(-1)} label={t('preview.action.previous', 'Previous')} transition={transition} />
            <EdgeNav side="right" visible={showToolbar} disabled={!canNext} onClick={() => step(1)} label={t('preview.action.next', 'Next')} transition={transition} />
          </>
        ) : null}

        {/* Toolbar (§10.8) */}
        {uiMode !== 'silent' ? (
          <div
            data-testid="content-preview-toolbar"
            role="toolbar"
            aria-label={t('preview.a11y.toolbar', 'Preview actions')}
            aria-hidden={!showToolbar}
            className={clsx(
              'absolute inset-x-0 top-0 z-20 flex items-center gap-1 px-2 py-1.5',
              !showToolbar && 'pointer-events-none',
            )}
            style={{
              opacity: showToolbar ? 1 : 0,
              transition,
              background: 'linear-gradient(180deg, color-mix(in srgb, var(--cp-bg-strong) 92%, transparent), color-mix(in srgb, var(--cp-bg-strong) 55%, transparent) 70%, transparent)',
            }}
            onPointerEnter={() => pinUi(true)}
            onPointerLeave={() => pinUi(false)}
          >
            {onRequestExit ? (
              <ToolButton label={t('preview.action.exit', 'Exit preview')} shortcut="Esc" onClick={requestExit} testId="content-preview-exit">
                <X size={16} />
              </ToolButton>
            ) : null}
            <div className="min-w-0 flex-1 px-1">
              <div className="truncate text-[13px] font-medium" title={displayName}>
                {displayName}
              </div>
              {sessionCount > 1 ? (
                <div className="text-[11px] text-[color:var(--cp-muted)]" data-testid="content-preview-counter">
                  {t('preview.counter', '{{index}} of {{count}}', { index: index + 1, count: sessionCount })}
                </div>
              ) : null}
            </div>
            {capabilities.zoom ? (
              <div className="flex items-center gap-0.5 rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface-2)_70%,transparent)] px-1">
                <ToolButton label={t('preview.action.zoomOut', 'Zoom out')} shortcut="-" onClick={zoomOut} testId="content-preview-zoom-out">
                  <ZoomOut size={15} />
                </ToolButton>
                <button
                  type="button"
                  className="min-w-[44px] rounded px-1 text-[11px] font-medium tabular-nums text-[color:var(--cp-text)] hover:bg-[color:color-mix(in_srgb,var(--cp-surface-2)_90%,transparent)]"
                  onClick={fitToView}
                  title={t('preview.action.fit', 'Fit to view')}
                  data-testid="content-preview-zoom-level"
                >
                  {zoomPercent !== null ? `${zoomPercent}%` : '—'}
                </button>
                <ToolButton label={t('preview.action.zoomIn', 'Zoom in')} shortcut="+" onClick={zoomIn} testId="content-preview-zoom-in">
                  <ZoomIn size={15} />
                </ToolButton>
                {capabilities.pan ? (
                  <ToolButton
                    label={imageView?.mode === 'fit' ? t('preview.action.actualSize', 'Actual size') : t('preview.action.fit', 'Fit to view')}
                    shortcut={imageView?.mode === 'fit' ? '1' : '0'}
                    onClick={imageView?.mode === 'fit' ? actualSize : fitToView}
                  >
                    {imageView?.mode === 'fit' ? <Maximize2 size={15} /> : <Minimize2 size={15} />}
                  </ToolButton>
                ) : null}
              </div>
            ) : null}
            {capabilities.rotate ? (
              <ToolButton label={t('preview.action.rotate', 'Rotate')} shortcut="R" onClick={rotate} testId="content-preview-rotate">
                <RotateCw size={15} />
              </ToolButton>
            ) : null}
            {capabilities.search ? (
              <ToolButton label={t('preview.action.find', 'Find')} shortcut={`${MOD_KEY}+F`} onClick={() => setFindOpen(true)} testId="content-preview-find">
                <Search size={15} />
              </ToolButton>
            ) : null}
            <ToolButton label={t('preview.action.info', 'Info')} shortcut="I" onClick={() => setInfoOpen((v) => !v)} active={infoOpen} testId="content-preview-info">
              <Info size={15} />
            </ToolButton>
            {capabilities.openWith ? (
              <ToolButton label={t('preview.action.openWith', 'Open with…')} onClick={requestOpenWith} testId="content-preview-open-with">
                <ExternalLink size={15} />
              </ToolButton>
            ) : null}
            {toolbarHostActions.map((action) => (
              <ToolButton
                key={action.id}
                label={action.label}
                disabled={action.disabled}
                onClick={() => {
                  invokeAction(action.id)
                  if (itemInfo) action.onInvoke({ item: itemInfo, resolved: current?.resolved })
                }}
                testId={`content-preview-host-${action.id}`}
              >
                {action.icon ?? <span className="px-1 text-[11px]">{action.label}</span>}
              </ToolButton>
            ))}
            <ToolButton label={t('preview.action.more', 'More')} onClick={(event) => setMenuAnchor(event.currentTarget)} testId="content-preview-more">
              <MoreHorizontal size={15} />
            </ToolButton>
            <Menu anchorEl={menuAnchor} open={Boolean(menuAnchor)} onClose={() => setMenuAnchor(null)}>
              {capabilities.export ? (
                <MenuItem
                  onClick={() => {
                    setMenuAnchor(null)
                    downloadCurrent()
                  }}
                >
                  <Download size={14} className="mr-2" /> {t('preview.action.download', 'Download original')}
                </MenuItem>
              ) : null}
              <MenuItem
                onClick={() => {
                  setMenuAnchor(null)
                  copySource()
                }}
              >
                <Copy size={14} className="mr-2" /> {t('preview.action.copySource', 'Copy path / object id')}
              </MenuItem>
              {capabilities.fullscreen ? (
                <MenuItem
                  onClick={() => {
                    setMenuAnchor(null)
                    toggleFullscreen()
                  }}
                >
                  <Fullscreen size={14} className="mr-2" /> {isFullscreen ? t('preview.action.exitFullscreen', 'Exit fullscreen') : t('preview.action.fullscreen', 'Fullscreen')}
                  <span className="ml-auto pl-4 text-[11px] text-[color:var(--cp-muted)]">F</span>
                </MenuItem>
              ) : null}
              {overflowHostActions.map((action) => (
                <MenuItem
                  key={action.id}
                  disabled={action.disabled}
                  onClick={() => {
                    setMenuAnchor(null)
                    invokeAction(action.id)
                    if (itemInfo) action.onInvoke({ item: itemInfo, resolved: current?.resolved })
                  }}
                >
                  {action.icon ? <span className="mr-2 inline-flex">{action.icon}</span> : null}
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
            className="pointer-events-none absolute bottom-3 left-1/2 z-10 max-w-[80%] -translate-x-1/2 rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_92%,transparent)] px-3 py-1 text-[11px] text-[color:var(--cp-muted)] shadow"
            style={{ transition }}
            data-testid="content-preview-fidelity"
          >
            {current.result.fidelityNote}
          </div>
        ) : null}
      </div>

      {/* Info panel (§10.6) */}
      {infoOpen ? (
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
        'inline-flex h-8 min-w-8 items-center justify-center rounded-full px-1.5 text-[color:var(--cp-text)] transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-surface-2)_90%,transparent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)] disabled:opacity-40',
        active && 'bg-[color:color-mix(in_srgb,var(--cp-accent)_22%,transparent)] text-[color:var(--cp-accent)]',
      )}
    >
      {children}
    </button>
  )
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
        'absolute top-1/2 z-10 flex h-14 w-11 -translate-y-1/2 items-center justify-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_80%,transparent)] text-[color:var(--cp-text)] shadow hover:bg-[color:var(--cp-surface-opaque)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)] disabled:opacity-25',
        side === 'left' ? 'left-2' : 'right-2',
        !visible && 'pointer-events-none',
      )}
      style={{ opacity: visible ? 1 : 0, transition }}
    >
      {side === 'left' ? <ChevronLeft size={22} /> : <ChevronRight size={22} />}
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
      className="absolute right-3 top-12 z-30 flex items-center gap-1 rounded-full border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-2 py-1 shadow"
      data-testid="content-preview-findbar"
      onPointerDown={(event) => event.stopPropagation()}
    >
      <Search size={13} className="text-[color:var(--cp-muted)]" />
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
        className="w-40 bg-transparent text-[12px] text-[color:var(--cp-text)] outline-none placeholder:text-[color:var(--cp-muted)]"
      />
      <span className="min-w-[48px] text-center text-[11px] tabular-nums text-[color:var(--cp-muted)]" data-testid="content-preview-find-count">
        {query ? `${count === 0 ? 0 : active + 1} / ${count}` : ''}
      </span>
      <ToolButton label={t('preview.find.previous', 'Previous match')} onClick={() => onStep(-1)} disabled={count === 0}>
        <ChevronLeft size={14} />
      </ToolButton>
      <ToolButton label={t('preview.find.next', 'Next match')} onClick={() => onStep(1)} disabled={count === 0}>
        <ChevronRight size={14} />
      </ToolButton>
      <ToolButton label={t('common.close', 'Close')} onClick={onClose}>
        <X size={14} />
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
      <div className="flex min-w-[220px] flex-col items-center gap-3 rounded-2xl bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_88%,transparent)] px-6 py-5 shadow-[var(--cp-panel-shadow)]">
        <Loader2 size={22} className={clsx('text-[color:var(--cp-accent)]', !reducedMotion && 'animate-spin')} />
        <div className="text-[13px] font-medium">
          {converting ? t('preview.status.preparing', 'Preparing preview') : t('preview.status.loading', 'Loading')}
        </div>
        {converting && progress?.message ? <div className="text-[11px] text-[color:var(--cp-muted)]">{progress.message}</div> : null}
        {converting ? (
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)]" role="progressbar" aria-valuenow={percent ?? undefined} aria-valuemin={0} aria-valuemax={100}>
            <div
              className={clsx('h-full rounded-full bg-[color:var(--cp-accent)]', percent === null && !reducedMotion && 'animate-pulse')}
              style={{ width: percent === null ? '40%' : `${percent}%`, transition: reducedMotion ? 'none' : 'width 200ms ease' }}
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
    error.kind === 'permission-denied' ? <Lock size={28} /> : error.kind === 'corrupted' ? <FileX size={28} /> : error.kind === 'unsupported' ? <FileQuestion size={28} /> : <AlertTriangle size={28} />
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
        <div className="flex h-16 w-16 items-center justify-center rounded-3xl bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)] text-[color:var(--cp-muted)]">{icon}</div>
        <div className="text-[15px] font-semibold">{title}</div>
        {contentLabel && error.kind !== 'permission-denied' ? (
          <div className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)] px-2.5 py-0.5 font-mono text-[11px] text-[color:var(--cp-muted)]">{contentLabel}</div>
        ) : null}
        {detail ? <div className="text-[12px] leading-5 text-[color:var(--cp-muted)]">{detail}</div> : null}
        <div className="mt-1 flex flex-wrap justify-center gap-2">
          {onRetry ? (
            <ActionChip onClick={onRetry} testId="content-preview-retry">
              <RefreshCw size={13} /> {t('common.retry', 'Retry')}
            </ActionChip>
          ) : null}
          {onOpenWith ? (
            <ActionChip onClick={onOpenWith} testId="content-preview-error-open-with">
              <ExternalLink size={13} /> {t('preview.action.openWith', 'Open with…')}
            </ActionChip>
          ) : null}
          {onDownload ? (
            <ActionChip onClick={onDownload}>
              <Download size={13} /> {t('preview.action.download', 'Download original')}
            </ActionChip>
          ) : null}
          {onInfo ? (
            <ActionChip onClick={onInfo}>
              <Info size={13} /> {t('preview.action.info', 'Info')}
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
      className="inline-flex items-center gap-1.5 rounded-full border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-3 py-1.5 text-[12px] font-medium text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)] hover:text-[color:var(--cp-accent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)]"
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
      <button type="button" onClick={onCopy} className="inline-flex max-w-full items-start gap-1 text-left font-mono text-[11px] text-[color:var(--cp-accent)] hover:underline" title={t('common.copy', 'Copy')}>
        <span className="break-all">{sourceLabel(item.source)}</span>
        <Copy size={11} className="mt-0.5 shrink-0" />
      </button>,
    ])
  }
  if (resolved?.inputObjectId) rows.push([t('preview.info.objectId', 'Object ID'), <span className="break-all font-mono text-[11px]">{resolved.inputObjectId}</span>])
  if (resolved?.sourceObjectId && resolved.sourceObjectId !== resolved.inputObjectId) {
    rows.push([t('preview.info.sourceObject', 'Source object'), <span className="break-all font-mono text-[11px]">{resolved.sourceObjectId}</span>])
  }
  if (resolved?.versionToken) rows.push([t('preview.info.version', 'Version'), <span className="font-mono text-[11px]">{resolved.versionToken}</span>])
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
      className="z-30 flex w-[300px] max-w-full shrink-0 flex-col border-l border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)]"
      data-testid="content-preview-info-panel"
      aria-label={t('preview.action.info', 'Info')}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <div className="flex items-center justify-between border-b border-[color:var(--cp-border)] px-3 py-2">
        <span className="text-[12px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">{t('preview.action.info', 'Info')}</span>
        <ToolButton label={t('common.close', 'Close')} onClick={onClose}>
          <X size={14} />
        </ToolButton>
      </div>
      <div className="flex-1 space-y-2 overflow-y-auto px-3 py-3">
        {rows.map(([label, value]) => (
          <div key={label} className="grid grid-cols-[84px_1fr] items-start gap-2 text-[12px] leading-5">
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
  reducedMotion,
  fontScale,
  find,
  imageRef,
  mediaRef,
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
  reducedMotion: boolean
  fontScale: number
  find: FindState
  imageRef: React.RefObject<ImageRendererHandle | null>
  mediaRef: React.RefObject<MediaRendererHandle | null>
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
          onRendered={onRendered}
          onFailure={onFailure}
          onActivity={onActivity}
        />
      )
    case 'pdf':
      return <PDFIframeRenderer readRef={result.readRef} title={displayName} onRendered={onRendered} onDownload={onDownload} onOpenWith={onOpenWith} t={t} />
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

  useEffect(() => {
    if (!natural) return
    const fitScale = fitScaleFor(container, natural, effective.rotation, 'fit')
    onViewChange({
      scale: effective.scale,
      mode: effective.mode === 'custom' && Math.abs(effective.scale - fitScale) < 0.001 ? 'fit' : effective.mode,
      rotation: effective.rotation,
    })
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
          className="pointer-events-none absolute inset-x-0 bottom-0 z-10 border-t border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-warning)_18%,var(--cp-surface-opaque))] px-4 py-1.5 text-center text-[11px] text-[color:var(--cp-text)]"
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
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center gap-6 p-6" data-testid="content-preview-audio-stage">
      <div className="flex h-28 w-28 items-center justify-center rounded-[36px] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_30%,var(--cp-surface))] text-[color:var(--cp-accent)]">
        <Music size={44} />
      </div>
      <div className="max-w-md truncate text-[14px] font-medium">{title}</div>
      <audio
        ref={(el) => {
          elementRef.current = el
        }}
        src={url}
        controls={showControls}
        preload="metadata"
        className="w-full max-w-md"
        data-testid="content-preview-audio"
        onLoadedMetadata={handleMetadata}
        onError={handleError}
      />
    </div>
  )
}

// ─── PDF (P0: Runtime built-in viewer in an iframe, §10.7 / §23.3.1) ───

function PDFIframeRenderer({
  readRef,
  title,
  onRendered,
  onDownload,
  onOpenWith,
  t,
}: {
  readRef: PreviewReadRef
  title: string
  onRendered: (media?: LoadState['media'], opts?: { degraded?: boolean }) => void
  onDownload: () => void
  onOpenWith?: () => void
  t: TranslateFn
}) {
  const [phase, setPhase] = useState<'preflight' | 'loading' | 'ready' | 'degraded'>('preflight')
  const url = useContentUrl(readRef)
  const rendered = useRef(false)
  const degrade = useCallback(() => {
    setPhase('degraded')
    onRendered(undefined, { degraded: true })
  }, [onRendered])

  useEffect(() => {
    const controller = new AbortController()
    void (async () => {
      const runtime = detectRuntimeProfile()
      if (!runtime.pdfInline) {
        degrade()
        return
      }
      try {
        // URL pre-check: the bytes must be a PDF and the response must not force a download.
        const probe = await readProbe(readRef, 1024, controller.signal)
        const head = String.fromCharCode(...probe.bytes.subarray(0, 5))
        const type = (probe.contentType ?? '').split(';')[0].trim().toLowerCase()
        if (head !== '%PDF-' && type !== 'application/pdf') {
          degrade()
          return
        }
      } catch {
        // A failed pre-check is not proof of failure — let the viewer try.
      }
      if (!controller.signal.aborted) setPhase('loading')
    })()
    return () => controller.abort()
  }, [degrade, readRef])

  useEffect(() => {
    if (phase !== 'loading') return
    const timer = window.setTimeout(() => {
      if (!rendered.current) degrade()
    }, PDF_LOAD_TIMEOUT_MS)
    return () => window.clearTimeout(timer)
  }, [degrade, phase])

  if (phase === 'degraded') {
    return (
      <div className="absolute inset-0 flex items-center justify-center p-6" data-testid="content-preview-pdf-degraded">
        <div className="flex max-w-md flex-col items-center gap-3 text-center">
          <div className="flex h-16 w-16 items-center justify-center rounded-3xl bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)] text-[color:var(--cp-muted)]">
            <FileQuestion size={28} />
          </div>
          <div className="text-[15px] font-semibold">{t('preview.pdf.degraded', 'This Runtime cannot embed a PDF preview')}</div>
          <div className="text-[12px] text-[color:var(--cp-muted)]">{t('preview.pdf.degradedBody', 'Download the file or open it with another application.')}</div>
          <div className="mt-1 flex gap-2">
            <ActionChip onClick={onDownload}>
              <Download size={13} /> {t('preview.action.download', 'Download original')}
            </ActionChip>
            {onOpenWith ? (
              <ActionChip onClick={onOpenWith}>
                <ExternalLink size={13} /> {t('preview.action.openWith', 'Open with…')}
              </ActionChip>
            ) : null}
          </div>
        </div>
      </div>
    )
  }
  if (phase === 'preflight' || !url) return null
  return (
    // The viewer is an opaque renderer: no DOM access, no private messaging (§23.3.1 rule 5).
    // Chromium disables its PDF plugin inside sandboxed frames, so `sandbox` is deliberately absent;
    // the iframe stays on a system-controlled read endpoint with a short-lived, permission-bound URL.
    <iframe
      title={title}
      src={url}
      referrerPolicy="no-referrer"
      className="absolute inset-0 h-full w-full border-0 bg-[color:var(--cp-surface-2-opaque)]"
      data-testid="content-preview-pdf"
      onLoad={() => {
        rendered.current = true
        setPhase('ready')
        onRendered()
      }}
      onError={degrade}
    />
  )
}
