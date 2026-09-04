/**
 * BuckyOS Preview — product-level protocol types.
 *
 * Source of truth: product/bucky_file/BuckyOS Preview App-Component PRD.md
 * (§12 conceptual interface, §23.3 Source Resolver, §23.6 work records,
 * §23.7 result contract). These shapes are consumed by the Preview Component
 * (`components/ContentPreview.tsx`), the Preview App (`app/preview/`) and
 * every host that embeds the component. No NFSP / mock wire type crosses this
 * boundary — providers map onto these.
 */

import type { ReactNode } from 'react'

// ─── Content Source (§10.2, §12) ───

export interface CyfsPathRef {
  kind: 'cyfs-path'
  /** `cyfs:///home/x` (canonical) — bare `/home/x` is accepted and normalized. */
  path: string
  version?: string
}

export interface ObjectIdRef {
  kind: 'object-id'
  objectId: string
  version?: string
}

/**
 * Host-provided, already-resolved content (drag & drop, clipboard, an IM
 * attachment held in memory). Reserved extension slot per §10.2 — it must
 * never be required by a host that only has paths / object ids.
 */
export interface BlobRef {
  kind: 'blob'
  value: { blob: Blob; name?: string; version?: string }
}

/** Future extension slot (§12): unknown kinds resolve to Unsupported. */
export interface ExtensionRef {
  kind: string
  value?: unknown
}

export type ContentRef = CyfsPathRef | ObjectIdRef | BlobRef | ExtensionRef

export function isCyfsPathRef(ref: ContentRef): ref is CyfsPathRef {
  return ref.kind === 'cyfs-path' && typeof (ref as CyfsPathRef).path === 'string'
}

export function isObjectIdRef(ref: ContentRef): ref is ObjectIdRef {
  return ref.kind === 'object-id' && typeof (ref as ObjectIdRef).objectId === 'string'
}

export function isBlobRef(ref: ContentRef): ref is BlobRef {
  const value = (ref as BlobRef).value
  return ref.kind === 'blob' && !!value && typeof value === 'object' && 'blob' in value
}

// ─── UI policy (§10.3, §10.5) ───

export type PreviewUIMode = 'auto' | 'visible' | 'silent'
export type PreviewFitMode = 'contain' | 'cover' | 'actual-size'
export type NavigationMode = 'wrap' | 'bounded'

// ─── Session Context (§11, §12) ───

export interface PreviewSessionItemInput {
  /** Stable id inside the session; defaults to the source identity. */
  id?: string
  source: ContentRef
  title?: string
}

/**
 * P1 Provider-based session — accepted now so IM hosts can already pass a
 * paged enumerator; the component only uses `listItems()` in v1.
 */
export interface PreviewSessionProvider {
  listItems(): Promise<PreviewSessionItemInput[]>
  /** Optional change signal; the component re-lists and keeps the current item. */
  subscribe?(listener: () => void): () => void
}

export type PreviewSessionContext =
  | { kind: 'single' }
  | {
      kind: 'container'
      container: ContentRef
      current: ContentRef
      sort?: unknown
      filter?: unknown
      snapshot?: boolean
      navigation?: NavigationMode
    }
  | {
      kind: 'list'
      sessionId?: string
      version?: string
      items: PreviewSessionItemInput[]
      currentIndex: number
      navigation?: NavigationMode
    }
  | {
      kind: 'provider'
      sessionId: string
      currentItemId: string
      provider: PreviewSessionProvider
      navigation?: NavigationMode
    }

// ─── Resolved source (§23.3) ───

/**
 * Permission-bound read reference. `url` must inherit the caller's
 * permissions (never a long-lived public token); `blob` is host memory.
 */
export type PreviewReadRef =
  | {
      kind: 'url'
      url: string
      /** URL variant that forces a download (`Content-Disposition: attachment`). */
      downloadUrl?: string
    }
  | { kind: 'blob'; blob: Blob }

export interface ResolvedPreviewSource {
  originalSource: ContentRef
  /** FileObject / wrapper identity — provenance only. */
  sourceObjectId?: string
  /** Immutable identity of the bytes; required before any Pipeline work. */
  inputObjectId?: string
  /** Live-path ETag / revision — re-validation only, never a cache identity. */
  versionToken?: string
  displayName?: string
  size?: number
  /** `file` | `dir` | `collection` | … (provider vocabulary, display only). */
  objectType?: string
  /** MIME / extension hints — hints only, never the sole basis (§9.5). */
  mediaTypeHints: string[]
  readRef: PreviewReadRef
  /** Parent container, when the provider knows it (Smart Window relevance). */
  containerRef?: ContentRef
  /** Provider-specific opaque handle (e.g. NFSP ref) for later calls. */
  providerHandle?: unknown
}

// ─── Standard result types (§9.2) ───

export type PreviewRendererType =
  | 'image'
  | 'svg'
  | 'text'
  | 'html'
  | 'audio'
  | 'video'
  | 'pdf'

/** §9.4 success payload of a Pipeline (or the Direct path's synthesized one). */
export interface PreviewResult {
  resultType: PreviewRendererType
  readRef: PreviewReadRef
  mediaType: string
  /** Version of the source this result was derived from. */
  sourceVersion?: string
  width?: number
  height?: number
  durationSeconds?: number
  pageCount?: number
  /** Human-readable fidelity / degradation note (§6.4). */
  fidelityNote?: string
  progressive?: boolean
  cacheable?: boolean
  fallbacks?: PreviewResult[]
}

// ─── Pipeline work (§23.6 / §23.7) ───

export interface PreviewWorkError {
  code: string
  message: string
  retryable: boolean
  /** Epoch millis after which a retry is meaningful. */
  retryAfter?: number
}

export interface PreviewWorkProgress {
  completed: number
  total?: number
  message?: string
}

export type PreviewWorkState =
  | {
      workKey: string
      state: 'processing'
      attemptId?: string
      taskId?: string
      progress?: PreviewWorkProgress
      retryAfterMs?: number
    }
  | { workKey: string; state: 'completed'; attemptId?: string; result: PreviewResult }
  | { workKey: string; state: 'failed'; attemptId?: string; error: PreviewWorkError }

export type PreviewUnsupportedReason =
  | 'no-renderer'
  | 'no-pipeline'
  | 'runtime-unsupported'
  | 'too-large'
  | 'policy'
  | 'not-content'

export type PreviewResolution =
  | { kind: 'direct'; source: ResolvedPreviewSource; rendererType: PreviewRendererType; mediaType: string }
  | ({ kind: 'pipeline' } & PreviewWorkState)
  | { kind: 'unsupported'; reason: PreviewUnsupportedReason; detail?: string }

/** What the current Runtime can consume — the Pipeline Target Profile input. */
export interface PreviewRuntimeProfile {
  acceptTypes: PreviewRendererType[]
  imageMediaTypes: string[]
  videoMediaTypes: string[]
  audioMediaTypes: string[]
  pdfInline: boolean
}

export interface PreviewTargetProfile {
  purpose: 'preview' | 'thumbnail'
  viewport: { width: number; height: number; dpr: number }
  quality: 'fast' | 'balanced' | 'best'
  page?: number
  timeRange?: { start: number; end?: number }
}

export interface EnsurePreviewWorkRequest {
  source: ResolvedPreviewSource
  runtimeProfile: PreviewRuntimeProfile
  targetProfile: PreviewTargetProfile
  options?: {
    retry?: boolean
    expectedAttemptId?: string
    signal?: AbortSignal
  }
}

// ─── Errors ───

export type PreviewErrorCode =
  | 'NOT_FOUND'
  | 'PERMISSION_DENIED'
  | 'CORRUPTED'
  | 'UNSUPPORTED'
  | 'TIMEOUT'
  | 'NETWORK'
  | 'CANCELLED'
  | 'PIPELINE_FAILED'
  | 'TOO_LARGE'
  | 'INVALID_SOURCE'
  | 'INTERNAL'

export class PreviewError extends Error {
  readonly code: PreviewErrorCode
  readonly retryable: boolean
  readonly detail?: string

  constructor(code: PreviewErrorCode, message: string, opts?: { retryable?: boolean; detail?: string }) {
    super(message)
    this.name = 'PreviewError'
    this.code = code
    this.retryable = opts?.retryable ?? (code === 'NETWORK' || code === 'TIMEOUT' || code === 'INTERNAL')
    this.detail = opts?.detail
  }
}

export function isAbortError(err: unknown): boolean {
  return (
    (err instanceof DOMException && err.name === 'AbortError') ||
    (err instanceof PreviewError && err.code === 'CANCELLED')
  )
}

// ─── Provider contract (§23.7: resolve / ensure / get) ───

/**
 * The three logical operations between the Preview Controller and the
 * system (P0: nfs_server). A provider is installed once per runtime — the
 * component never talks to NFSP or to mocks directly.
 */
export interface PreviewProvider {
  readonly id: string
  resolvePreviewSource(
    source: ContentRef,
    opts: { signal?: AbortSignal; permissionsContext?: unknown },
  ): Promise<ResolvedPreviewSource>
  /**
   * Enumerate the Session Items of a container (folder, ZIP directory, object
   * container). Only leaf content is returned; ordering is the container's
   * natural order (§11.3 sort/filter are passed through when supported).
   */
  enumerateContainer(
    container: ContentRef,
    opts: { signal?: AbortSignal; sort?: unknown; filter?: unknown },
  ): Promise<PreviewSessionItemInput[]>
  /** Idempotent get-or-create of Pipeline work; only called when Direct is impossible. */
  ensurePreviewWork(request: EnsurePreviewWorkRequest): Promise<PreviewWorkState | { kind: 'unsupported'; reason: PreviewUnsupportedReason; detail?: string }>
  /** Query only — never creates a new attempt. */
  getPreviewWork(workKey: string, opts: { signal?: AbortSignal }): Promise<PreviewWorkState>
}

// ─── Component surface (§10.6, §12.1, §12.2) ───

export interface PreviewCapabilities {
  zoom: boolean
  pan: boolean
  textSelection: boolean
  search: boolean
  playback: boolean
  rotate: boolean
  previous: boolean
  next: boolean
  export: boolean
  openWith: boolean
  fullscreen: boolean
}

export type PreviewStatus =
  | 'idle'
  | 'resolving'
  | 'converting'
  | 'rendering'
  | 'ready'
  | 'error'

export type PreviewErrorKind =
  | 'unsupported'
  | 'permission-denied'
  | 'corrupted'
  | 'not-found'
  | 'cancelled'
  | 'timeout'
  | 'error'

export interface PreviewErrorState {
  kind: PreviewErrorKind
  code: string
  message: string
  retryable: boolean
  /** Extension / media type shown in the Unsupported state (§15.1). */
  contentLabel?: string
}

export interface PreviewItemInfo {
  index: number
  count: number
  id: string
  title: string
  source: ContentRef
}

export interface PreviewProgress {
  phase: 'resolving' | 'converting' | 'rendering'
  completed?: number
  total?: number
  message?: string
}

export interface PreviewReadyInfo extends PreviewItemInfo {
  rendererType: PreviewRendererType
  mediaType: string
  via: 'direct' | 'pipeline'
  resolved: ResolvedPreviewSource
  result: PreviewResult
}

export interface PreviewHostAction {
  id: string
  label: string
  icon?: ReactNode
  /** `toolbar` shows inline (controlled slot), `overflow` goes to the ⋯ menu. */
  placement?: 'toolbar' | 'overflow'
  disabled?: boolean
  onInvoke: (context: { item: PreviewItemInfo; resolved?: ResolvedPreviewSource }) => void
}

export interface PreviewOpenWithRequest {
  item: PreviewItemInfo
  resolved?: ResolvedPreviewSource
  /** Best-known media type at the time of the request. */
  mediaType?: string
}
