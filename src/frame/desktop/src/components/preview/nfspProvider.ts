/**
 * NFSP-backed Preview provider — nfs_server is the P0 Pipeline Provider
 * (PRD §23.1). Source Resolver = `resolve` (base/ident/access), container
 * enumeration = `list`, read references = the data plane `/nfs/v1/read/*`.
 *
 * Pipeline calls map onto the `repr` family (`repr` = ensure, `get_repr` =
 * query) that §23.7 leaves for the API Spec to freeze. The wire shape below
 * is PROVISIONAL: nfs_server does not advertise `repr` yet, so this provider
 * gates the Pipeline on the hello feature list and plans every non-Direct
 * format as Unsupported until the server ships it — never a fake result.
 */

import { NfspError, type NodeInfo, type WireRef } from '../../api/nfsp_client'
import { effectiveListLimit, ensureSession, nfspClient } from '../../app/filebrowser/data/nfsp/client'
import { extensionOf, mediaTypeFromExtension } from './mediaTypes'
import { cyfsPathToLocal, normalizeCyfsPath, parentCyfsPath } from './session'
import {
  isBlobRef,
  isCyfsPathRef,
  isObjectIdRef,
  PreviewError,
  type ContentRef,
  type PreviewProvider,
  type PreviewReadRef,
  type PreviewRendererType,
  type PreviewResult,
  type PreviewSessionItemInput,
  type PreviewWorkState,
} from './types'

const REPR_FEATURE = 'repr'
const WANT = ['base', 'ident', 'access'] as const

function toPreviewError(err: unknown): PreviewError {
  if (err instanceof PreviewError) return err
  if (err instanceof DOMException && err.name === 'AbortError') return new PreviewError('CANCELLED', 'Cancelled')
  if (err instanceof NfspError) {
    switch (err.code) {
      case 'NOT_FOUND':
      case 'STALE':
        return new PreviewError('NOT_FOUND', 'This content no longer exists', { detail: err.code })
      case 'PERMISSION_DENIED':
        return new PreviewError('PERMISSION_DENIED', 'You do not have access to this content', { detail: err.code })
      case 'UNSUPPORTED':
        return new PreviewError('UNSUPPORTED', 'The server does not support this operation', { detail: err.code })
      case 'NEED_PULL':
        return new PreviewError('NOT_FOUND', 'The content is not available on this node yet', { detail: err.code, retryable: true })
      default:
        return new PreviewError(err.httpStatus >= 500 ? 'NETWORK' : 'INTERNAL', 'The content server rejected the request', { detail: err.code })
    }
  }
  return new PreviewError('NETWORK', 'Could not reach the content server', { detail: String(err) })
}

function nodeIdOf(info: NodeInfo): string {
  if (info.node_id) return info.node_id
  const ref = info.ref
  return ref.type === 'live' ? ref.node_id : ref.obj_id
}

function absoluteUrl(url: string): string {
  return /^[a-z][a-z0-9+.-]*:/i.test(url) ? url : `${nfspClient().raw.baseUrl}${url}`
}

function readRefOf(info: NodeInfo, name: string | undefined): PreviewReadRef {
  const access = info.access_urls?.find((u) => u.kind === 'read')
  const nodeId = nodeIdOf(info)
  const raw = nfspClient().raw
  return {
    kind: 'url',
    url: access ? absoluteUrl(access.url) : raw.readUrl(nodeId),
    downloadUrl: raw.readUrl(nodeId, { download: true, name }),
  }
}

function locatorOf(source: ContentRef): string | WireRef {
  if (isCyfsPathRef(source)) return cyfsPathToLocal(source.path)
  if (isObjectIdRef(source)) return { type: 'object', obj_id: source.objectId }
  throw new PreviewError('INVALID_SOURCE', `Unsupported source kind "${source.kind}"`)
}

async function resolveNode(source: ContentRef, signal?: AbortSignal): Promise<NodeInfo> {
  await ensureSession()
  if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
  let info: NodeInfo | null
  try {
    info = await nfspClient().resolve(locatorOf(source), [...WANT])
  } catch (err) {
    throw toPreviewError(err)
  }
  if (!info) throw new PreviewError('NOT_FOUND', 'This content no longer exists')
  return info
}

// ─── Provisional `repr` wire shape (to be frozen by the NFSP API Spec) ───

interface ReprWireProgress {
  completed: number
  total?: number
  message?: string
}

interface ReprWireResult {
  media_type: string
  result_type?: string
  read_url?: string
  result_node_id?: string
  result_obj_id?: string
  source_version?: string
  width?: number
  height?: number
  duration?: number
  page_count?: number
  fidelity?: string
  progressive?: boolean
}

interface ReprWireRecord {
  work_key: string
  state: 'processing' | 'completed' | 'failed' | 'unsupported'
  attempt_id?: string
  task_id?: string
  progress?: ReprWireProgress
  retry_after_ms?: number
  result?: ReprWireResult
  error?: { code: string; message: string; retryable?: boolean; retry_after?: number }
  reason?: string
}

function rendererOf(result: ReprWireResult): PreviewRendererType {
  const declared = result.result_type as PreviewRendererType | undefined
  if (declared) return declared
  const type = result.media_type
  if (type === 'image/svg+xml') return 'svg'
  if (type.startsWith('image/')) return 'image'
  if (type === 'text/html') return 'html'
  if (type.startsWith('text/')) return 'text'
  if (type === 'application/pdf') return 'pdf'
  if (type.startsWith('audio/')) return 'audio'
  return 'video'
}

function mapWireResult(result: ReprWireResult): PreviewResult {
  const raw = nfspClient().raw
  const nodeId = result.result_node_id ?? result.result_obj_id
  const url = result.read_url ? absoluteUrl(result.read_url) : nodeId ? raw.readUrl(nodeId) : null
  if (!url) throw new PreviewError('PIPELINE_FAILED', 'The Pipeline result has no readable reference')
  return {
    resultType: rendererOf(result),
    readRef: { kind: 'url', url, downloadUrl: nodeId ? raw.readUrl(nodeId, { download: true }) : undefined },
    mediaType: result.media_type,
    sourceVersion: result.source_version,
    width: result.width,
    height: result.height,
    durationSeconds: result.duration,
    pageCount: result.page_count,
    fidelityNote: result.fidelity,
    progressive: result.progressive,
    cacheable: true,
  }
}

function mapWireRecord(record: ReprWireRecord): PreviewWorkState {
  if (record.state === 'completed' && record.result) {
    return { workKey: record.work_key, state: 'completed', attemptId: record.attempt_id, result: mapWireResult(record.result) }
  }
  if (record.state === 'failed') {
    return {
      workKey: record.work_key,
      state: 'failed',
      attemptId: record.attempt_id,
      error: {
        code: record.error?.code ?? 'PIPELINE_FAILED',
        message: record.error?.message ?? 'The Pipeline failed',
        retryable: record.error?.retryable ?? false,
        retryAfter: record.error?.retry_after,
      },
    }
  }
  return {
    workKey: record.work_key,
    state: 'processing',
    attemptId: record.attempt_id,
    taskId: record.task_id,
    progress: record.progress,
    retryAfterMs: record.retry_after_ms,
  }
}

export function createNfspPreviewProvider(): PreviewProvider {
  return {
    id: 'nfsp',

    async resolvePreviewSource(source, { signal }) {
      if (isBlobRef(source)) {
        const { blob, name } = source.value
        const hint = blob.type || mediaTypeFromExtension(extensionOf(name))
        return {
          originalSource: source,
          displayName: name ?? 'Dropped content',
          size: blob.size,
          objectType: 'file',
          mediaTypeHints: hint ? [hint] : [],
          readRef: { kind: 'blob', blob },
        }
      }
      const info = await resolveNode(source, signal)
      const name = info.name ?? (isCyfsPathRef(source) ? cyfsPathToLocal(source.path).split('/').pop() : undefined)
      const hint = name ? mediaTypeFromExtension(extensionOf(name)) : undefined
      const containerPath = isCyfsPathRef(source) ? parentCyfsPath(normalizeCyfsPath(source.path)) : null
      return {
        originalSource: source,
        sourceObjectId: info.ref.type === 'object' ? info.ref.obj_id : undefined,
        inputObjectId: info.obj_id,
        versionToken: info.etag ?? info.revision,
        displayName: name,
        size: info.size,
        objectType: info.kind,
        mediaTypeHints: hint ? [hint] : [],
        readRef: readRefOf(info, name),
        containerRef: containerPath ? { kind: 'cyfs-path', path: containerPath } : undefined,
        providerHandle: info.ref,
      }
    },

    async enumerateContainer(container, { signal }) {
      await ensureSession()
      const client = nfspClient()
      const at = locatorOf(container)
      const containerLocal = isCyfsPathRef(container) ? cyfsPathToLocal(container.path) : null
      const items: PreviewSessionItemInput[] = []
      let cursor: string | undefined
      const limit = effectiveListLimit(500)
      const MAX_ITEMS = 5000
      for (;;) {
        if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
        let listing
        try {
          listing = await client.list(at, { cursor, limit, order: 'name' }, ['base'])
        } catch (err) {
          throw toPreviewError(err)
        }
        if (!listing) throw new PreviewError('NOT_FOUND', 'This folder no longer exists')
        for (const entry of listing.entries) {
          if (entry.target.kind !== 'file') continue
          const canonical = entry.canonical_path
          if (canonical?.startsWith('cyfs://')) {
            items.push({ id: entry.entry_ref, source: { kind: 'cyfs-path', path: normalizeCyfsPath(canonical) }, title: entry.name })
          } else if (containerLocal !== null) {
            const path = containerLocal === '/' ? `/${entry.name}` : `${containerLocal}/${entry.name}`
            items.push({ id: entry.entry_ref, source: { kind: 'cyfs-path', path: normalizeCyfsPath(path) }, title: entry.name })
          } else if (entry.target.ref.type === 'object') {
            items.push({ id: entry.entry_ref, source: { kind: 'object-id', objectId: entry.target.ref.obj_id }, title: entry.name })
          }
          if (items.length >= MAX_ITEMS) return items
        }
        if (!listing.next_cursor || listing.entries.length === 0) return items
        cursor = listing.next_cursor
      }
    },

    async ensurePreviewWork(request) {
      await ensureSession()
      const raw = nfspClient().raw
      if (!raw.features.includes(REPR_FEATURE)) {
        return { kind: 'unsupported', reason: 'no-pipeline', detail: 'The content server has no Preview Pipeline for this format yet' }
      }
      const { source, runtimeProfile, targetProfile, options } = request
      if (!source.inputObjectId && !source.providerHandle) {
        return { kind: 'unsupported', reason: 'not-content', detail: 'The source has no stable content identity' }
      }
      try {
        const record = (await raw.call('repr', {
          at: source.providerHandle ? { ref: source.providerHandle as WireRef } : undefined,
          args: {
            purpose: targetProfile.purpose,
            input_obj_id: source.inputObjectId,
            version_token: source.versionToken,
            accept: runtimeProfile.acceptTypes,
            runtime: {
              image: runtimeProfile.imageMediaTypes,
              video: runtimeProfile.videoMediaTypes,
              audio: runtimeProfile.audioMediaTypes,
              pdf_inline: runtimeProfile.pdfInline,
            },
            target: {
              width: targetProfile.viewport.width,
              height: targetProfile.viewport.height,
              dpr: targetProfile.viewport.dpr,
              quality: targetProfile.quality,
              page: targetProfile.page,
              time_range: targetProfile.timeRange,
            },
            retry: options?.retry,
            expected_attempt_id: options?.expectedAttemptId,
          },
        })) as ReprWireRecord
        if (record.state === 'unsupported') {
          return { kind: 'unsupported', reason: 'no-pipeline', detail: record.reason }
        }
        return mapWireRecord(record)
      } catch (err) {
        const mapped = toPreviewError(err)
        if (mapped.code === 'UNSUPPORTED') {
          return { kind: 'unsupported', reason: 'no-pipeline', detail: mapped.detail }
        }
        throw mapped
      }
    },

    async getPreviewWork(workKey, { signal }) {
      if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
      try {
        const record = (await nfspClient().raw.call('get_repr', { args: { work_key: workKey } })) as ReprWireRecord
        return mapWireRecord(record)
      } catch (err) {
        throw toPreviewError(err)
      }
    },
  }
}
