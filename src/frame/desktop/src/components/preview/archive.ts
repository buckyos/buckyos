/**
 * Archives in Preview (notepads/archive-preview-files-todo.md T6, and
 * archive-preview-more-formats-todo.md for 7z / tar): whether a source opens
 * as an archive directory, and the addresses its folders and members
 * use. Path sources stay `dfs://<file>/@/archive/...` (live); object sources
 * (chat attachments) are `archive://<objId>/archive/...` folders and
 * `{ object-id, innerPath }` members (fixed).
 */

import { getActiveSessionToken } from 'buckyos'
import type { LocatorLike } from '../../api/nfsp_client'
import { ensureSession, nfspBaseUrl, nfspClient } from '../../app/filebrowser/data/nfsp/client'
import {
  archiveObjectUrl,
  archiveRootUrl,
  archiveSplit,
  dfsPathOf,
  DFS_SCHEME,
  parentInnerPath,
  parseArchiveObjectUrl,
} from '../../app/filebrowser/data/urls'
import { readFileObjectMeta } from './ndmSource'
import { ensurePreviewProvider } from './provider'
import { cyfsPathToLocal, normalizeCyfsPath } from './session'
import { extensionRefField, isCyfsPathRef, isObjectIdRef, type ContentRef } from './types'

export interface ArchiveTarget {
  /** The folder page shown first (a Files location url). */
  dirUrl: string
  /** A member the source named directly (a deep link): shown over its folder. */
  file?: ContentRef
  /** The archive's name (the root crumb). */
  name: string
}

/** Whether the server opens files of this name (`hello.archive`). Null in the mock runtime. */
async function archiveSupport(): Promise<((name: string) => boolean) | null> {
  const provider = await ensurePreviewProvider()
  if (provider.id !== 'nfsp') return null
  await ensureSession()
  const raw = nfspClient().raw
  return raw.archiveCapability ? (name: string) => raw.opensArchive(name) : null
}

/** What a source opens as when it is an archive or something inside one; null = an ordinary item. */
export async function archiveTargetOf(source: ContentRef, signal?: AbortSignal): Promise<ArchiveTarget | null> {
  const opens = await archiveSupport().catch(() => null)
  if (!opens) return null
  const client = nfspClient()
  if (isCyfsPathRef(source)) {
    const local = cyfsPathToLocal(source.path)
    const split = archiveSplit(local)
    if (split) {
      const info = await client.resolve(local, ['base'])
      const name = split.outer.split('/').pop() ?? ''
      if (!info) return null
      if (info.kind === 'dir') return { dirUrl: `${DFS_SCHEME}${local}`, name }
      return { dirUrl: `${DFS_SCHEME}${local.slice(0, local.lastIndexOf('/'))}`, file: source, name }
    }
    const name = local.split('/').pop() ?? ''
    if (!opens(name)) return null
    const info = await client.resolve(local, ['base'])
    return info?.kind === 'file' ? { dirUrl: archiveRootUrl(local), name } : null
  }
  if (isObjectIdRef(source)) {
    if (source.innerPath) {
      const at = { type: 'object' as const, obj_id: source.objectId, inner_path: source.innerPath }
      const [info, root] = await Promise.all([client.resolve(at, ['base']), client.resolve({ ...at, inner_path: '/archive' }, ['base'])])
      if (!info) return null
      const name = root?.name ?? ''
      if (info.kind === 'dir') return { dirUrl: archiveObjectUrl(source.objectId, source.innerPath), name }
      return { dirUrl: archiveObjectUrl(source.objectId, parentInnerPath(source.innerPath) ?? '/archive'), file: source, name }
    }
    const endpoint = nfspBaseUrl()
    const token = new URL(endpoint).origin === window.location.origin ? await getActiveSessionToken().catch(() => null) : null
    const meta = await readFileObjectMeta(source.objectId, endpoint, token, signal).catch(() => null)
    return meta && opens(meta.name) ? { dirUrl: archiveObjectUrl(source.objectId), name: meta.name } : null
  }
  // A host-resolved object (a MessageHub attachment): its resolver hands the object to the viewer's own session.
  const name = extensionRefField(source, 'name')
  if (!name || !opens(name)) return null
  const provider = await ensurePreviewProvider()
  const resolved = await provider.resolvePreviewSource(source, { signal }).catch(() => null)
  if (!resolved?.pipelineObjectId) return null
  const objectId = await resolved.pipelineObjectId().catch(() => null)
  return objectId ? { dirUrl: archiveObjectUrl(objectId), name: resolved.displayName ?? name } : null
}

/** The NFSP locator of an archive folder url. */
export function locatorOfArchiveUrl(url: string): LocatorLike {
  const object = parseArchiveObjectUrl(url)
  if (object) return { type: 'object', obj_id: object.objectId, inner_path: object.innerPath }
  return dfsPathOf(url) ?? url
}

/** Preview source of a member (or folder) path from an archive listing. */
export function archiveSourceOf(path: string): ContentRef {
  const object = parseArchiveObjectUrl(path)
  if (object) return { kind: 'object-id', objectId: object.objectId, innerPath: object.innerPath }
  return { kind: 'cyfs-path', path: normalizeCyfsPath(dfsPathOf(path) ?? path) }
}


export function isArchiveRootUrl(url: string): boolean {
  const object = parseArchiveObjectUrl(url)
  if (object) return object.innerPath === '/archive'
  const split = archiveSplit(dfsPathOf(url) ?? '')
  return !!split && !split.member
}

/** The folder above an archive folder; null at the archive root. */
export function archiveParentUrl(url: string): string | null {
  const object = parseArchiveObjectUrl(url)
  if (object) {
    const parent = parentInnerPath(object.innerPath)
    return parent ? archiveObjectUrl(object.objectId, parent) : null
  }
  const split = archiveSplit(dfsPathOf(url) ?? '')
  if (!split || !split.member) return null
  return archiveRootUrl(split.outer, split.member.split('/').slice(0, -1).join('/'))
}

/** What "Copy reference" puts on the clipboard: the member's own address. */
export function archiveReferenceText(path: string): string {
  const object = parseArchiveObjectUrl(path)
  if (object) return `obj://${object.objectId}${object.innerPath.split('/').slice(1).map((s) => `/${encodeURIComponent(s)}`).join('')}`
  return normalizeCyfsPath(dfsPathOf(path) ?? path)
}

export async function unlockArchive(url: string, password: string): Promise<void> {
  await ensureSession()
  await nfspClient().raw.archiveUnlock(locatorOfArchiveUrl(url), password)
}

/** The root folder url of the archive an archive folder url is in. */
export function archiveRootOfUrl(url: string): string {
  const object = parseArchiveObjectUrl(url)
  if (object) return archiveObjectUrl(object.objectId)
  const split = archiveSplit(dfsPathOf(url) ?? '')
  return split ? archiveRootUrl(split.outer) : url
}

export async function forgetArchivePassword(url: string): Promise<void> {
  await ensureSession()
  await nfspClient().raw.archiveForget(locatorOfArchiveUrl(archiveRootOfUrl(url)))
}
