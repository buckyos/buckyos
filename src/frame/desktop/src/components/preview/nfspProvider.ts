/**
 * NFSP-backed Preview provider — nfs_server is the P0 Pipeline Provider
 * (PRD §23.1). Source Resolver = `resolve` (base/ident/access), container
 * enumeration = `list`, read references = the data plane `/nfs/v1/read/*`.
 *
 * Pipeline calls map onto `repr` (ensure) / `get_repr` (query). nfs_server
 * advertises `repr` only while its converter (ffmpeg / ffprobe) is present;
 * without it every non-Direct format plans as Unsupported with the node's
 * reason — never a fake result. Results are read from the `preview-cache`
 * bucket through the URL (with a short-lived token) the node signs.
 */

import { bucket, getActiveSessionToken } from 'buckyos'
import { NfspError, type NodeInfo, type ReprRecord, type ReprResult, type ReprSource, type WireRef } from '../../api/nfsp_client'
import { effectiveListLimit, ensureSession, nfspBaseUrl, nfspClient } from '../../app/filebrowser/data/nfsp/client'
import { extensionOf, mediaTypeFromExtension } from './mediaTypes'
import { asBucketObjectRef, bucketRefOf, resolveBucketSource } from './bucketSource'
import { resolveNdmSource } from './ndmSource'
import { cyfsPathToLocal, normalizeCyfsPath, parentCyfsPath } from './session'
import {
  isBlobRef,
  isCyfsPathRef,
  isObjectIdRef,
  PreviewError,
  type ContentRef,
  type PreviewProvider,
  type PreviewReadRef,
  type PreviewResult,
  type PreviewSessionItemInput,
  type PreviewWorkState,
  type ResolvedPreviewSource,
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

// ─── `repr` wire (frozen with nfs_server preview/rpc.rs and websdk nfsp.ts) ───

function mapWireResult(result: ReprResult): PreviewResult {
  return {
    resultType: result.result_type,
    readRef: { kind: 'url', url: absoluteUrl(result.read_url) },
    mediaType: result.media_type,
    sourceVersion: result.source_version,
    width: result.width,
    height: result.height,
    durationSeconds: result.duration,
    fidelityNote: result.fidelity,
    cacheable: true,
  }
}

function mapWireRecord(record: Exclude<ReprRecord, { state: 'unsupported' }>): PreviewWorkState {
  if (record.state === 'completed') {
    return { workKey: record.work_key, state: 'completed', attemptId: record.attempt_id, result: mapWireResult(record.result) }
  }
  if (record.state === 'failed') {
    return {
      workKey: record.work_key,
      state: 'failed',
      attemptId: record.attempt_id,
      error: { code: record.error.code, message: record.error.message, retryable: record.error.retryable, retryAfter: record.error.retry_after },
    }
  }
  return { workKey: record.work_key, state: 'processing', attemptId: record.attempt_id, progress: record.progress, retryAfterMs: record.retry_after_ms }
}

/** Where the node finds the bytes: an NFSP ref, a bucket binding or a named object. */
function reprTarget(resolved: ResolvedPreviewSource): { at: WireRef | null; source?: ReprSource } | null {
  const bucketRef = bucketRefOf(resolved)
  if (bucketRef) {
    const parsed = bucket.parseBucketObjUrl(bucketRef.url)
    if (!parsed) return null
    return { at: null, source: { bucket: { bucket_id: parsed.bucketId, key: parsed.key, version: bucketRef.version, access_token: bucketRef.token } } }
  }
  if (resolved.providerHandle) return { at: resolved.providerHandle as WireRef }
  if (isObjectIdRef(resolved.originalSource) && resolved.sourceObjectId) return { at: null, source: { obj_id: resolved.sourceObjectId } }
  return null
}

export function createNfspPreviewProvider(): PreviewProvider {
  return {
    id: 'nfsp',

    async resolvePreviewSource(source, { signal }) {
      const bucketRef = asBucketObjectRef(source)
      if (bucketRef) return resolveBucketSource(bucketRef, signal)
      if (isObjectIdRef(source)) {
        const endpoint = nfspBaseUrl()
        const token = new URL(endpoint).origin === window.location.origin ? await getActiveSessionToken() : null
        return resolveNdmSource(source, endpoint, token, signal)
      }
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
        const capability = raw.reprCapability
        const detail = capability && !capability.available ? capability.detail : undefined
        return { kind: 'unsupported', reason: 'no-pipeline', detail: detail ?? 'The content server has no Preview Pipeline' }
      }
      const { source, runtimeProfile, targetProfile, options } = request
      const target = reprTarget(source)
      if (!target) return { kind: 'unsupported', reason: 'not-content', detail: 'The source has no stable content identity' }
      try {
        const record = await raw.repr(target.at, {
          purpose: targetProfile.purpose,
          source: target.source,
          runtime: {
            accept: runtimeProfile.acceptTypes,
            image: runtimeProfile.imageMediaTypes,
            video: runtimeProfile.videoMediaTypes,
            audio: runtimeProfile.audioMediaTypes,
            video_codecs: runtimeProfile.videoCodecs,
            heic: runtimeProfile.heic,
            pdf_inline: runtimeProfile.pdfInline,
          },
          retry: options?.retry || undefined,
          expected_attempt_id: options?.expectedAttemptId,
        })
        if (record.state === 'unsupported') return { kind: 'unsupported', reason: record.reason, detail: record.detail }
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
        const record = await nfspClient().raw.getRepr(workKey)
        if (record.state === 'unsupported') throw new PreviewError('UNSUPPORTED', record.detail ?? 'No Preview Pipeline for this content')
        return mapWireRecord(record)
      } catch (err) {
        throw toPreviewError(err)
      }
    },
  }
}
