/**
 * Mock host realm (`?fbHost=1`): the computer behind a Desktop Lite node.
 *
 * - The sudo dialog's password check is injected (`mockHostRequestPassword`):
 *   "wrong" fails, anything else unlocks; `?fbHostTtl=<seconds>` shortens the
 *   grant so expiry can be exercised without waiting 30 minutes.
 * - `/System` and `/Users/alice/Library` are folders this account cannot open.
 * - Copies between the computer and the mock Zone write both in-memory trees.
 */

import type { SudoByPasswordParams, SudoGrant } from '../../../components/sudo'
import { registerPreviewSourceResolver } from '../../../components/preview/hostSources'
import { extensionRefField } from '../../../components/preview/types'
import { extensionOf, mediaTypeFromExtension } from 'buckyos/content'
import type { FileEntry, HostInfo } from '../types'
import type { FileItem, FileItemPage, FolderReader, ListQuery, LocationCapabilities } from '../data/FolderReader'
import { classifyFileKind } from '../data/fileKinds'
import { operationError, runEntryBatch, type BatchOptions, type FolderWriteOps, type OperationResult } from '../data/folderOps'
import { HOST_SOURCE_KIND, lockHost, registerHostBackend, sudoRequiredError, type HostConnection, type HostGrant } from '../data/hostRealm'
import { invalidateMockPath, mockDelay } from '../data/mockReader'
import { sortEntriesForQuery } from '../data/sortEntries'
import { DEVICE_SCHEME, deviceUrl, parseDeviceUrl } from '../data/urls'
import { mockAddEntry, mockEntriesAtPath } from './data'

function param(name: string): string | null {
  try {
    return new URLSearchParams(window.location.search).get(name)
  } catch {
    return null
  }
}

export function mockHostInfo(): HostInfo | null {
  return param('fbHost') ? { label: 'Studio-Mac' } : null
}

/** The sudo dialog's password check in mock runs. */
export async function mockHostRequestPassword(params: SudoByPasswordParams & { expiresInSecs?: number }): Promise<SudoGrant> {
  await mockDelay(60, 120)
  if (params.password === 'wrong') throw new Error('INCORRECT_PASSWORD')
  const ttl = Number(param('fbHostTtl')) || params.expiresInSecs || 1800
  const expiresAtMs = Date.now() + ttl * 1000
  return { sessionToken: `mock-sudo-${crypto.randomUUID()}`, expiresAtMs, expiresInSeconds: ttl, username: params.username, appid: params.appid, aud: params.aud }
}

// ─── The computer's files (host paths, `/Users/alice/...`) ───

interface HostNode {
  kind: FileEntry['kind']
  size?: number
  mtime: string
  locked?: boolean
}

const now = new Date('2026-11-12T09:30:00Z').toISOString()
const tree = new Map<string, HostNode>()
const dir = (path: string, locked?: boolean) => tree.set(path, { kind: 'folder', mtime: now, locked })
const file = (path: string, size: number) => tree.set(path, { kind: classifyFileKind(path), size, mtime: now })
for (const path of ['/Applications', '/Users', '/Users/alice', '/Users/alice/Desktop', '/Users/alice/Documents', '/Users/alice/Documents/Projects', '/Users/alice/Documents/Projects/2026', '/Users/alice/Pictures', '/tmp']) dir(path)
dir('/System', true)
dir('/Users/alice/Library', true)
file('/Users/alice/Documents/Report Q3.pdf', 182_000)
file('/Users/alice/Documents/Budget 2026.xlsx', 24_000)
file('/Users/alice/Documents/notes.md', 1_200)
file('/Users/alice/Documents/Projects/2026/plan.docx', 40_000)
file('/Users/alice/Pictures/Beach 1.jpg', 2_400_000)
file('/Users/alice/Pictures/Clip.mp4', 9_800_000)
file('/Users/alice/Desktop/installer.dmg', 54_000_000)
file('/tmp/a b#1.txt', 12)

const parentOf = (path: string) => path.slice(0, path.lastIndexOf('/')) || '/'
const childOf = (parent: string, name: string) => `${parent === '/' ? '' : parent}/${name}`
const childrenOf = (path: string) => [...tree.keys()].filter((key) => key !== '/' && parentOf(key) === path)
const hostPathOf = (url: string) => parseDeviceUrl(url)?.path ?? '/'

function entryOf(session: string, path: string): FileEntry {
  const node = tree.get(path)!
  const name = path.slice(path.lastIndexOf('/') + 1)
  return {
    id: `host:${path}`,
    name,
    kind: node.kind,
    path: deviceUrl(session, path),
    modifiedAt: node.mtime,
    sizeBytes: node.kind === 'folder' ? undefined : node.size,
    accessLevel: node.locked ? 'none' : 'manage',
    operations: { copy: node.locked ? 'unsupported' : 'available' },
  }
}

const CAPABILITIES: LocationCapabilities = {
  kind: 'folder',
  acceptsContent: true,
  acceptsReferences: false,
  removal: 'destroy',
  canReorder: false,
  sortKeys: ['name', 'size', 'modified'],
  defaultSortKey: 'name',
}

/** A grant that expired locks the window; every call checks it like the server does. */
function connectionFor(session: string, grant: HostGrant): HostConnection {
  const live = async () => {
    await mockDelay(20, 60)
    if (grant.expiresAtMs <= Date.now()) {
      lockHost(session)
      throw sudoRequiredError()
    }
  }
  const requireDir = (path: string) => {
    const node = path === '/' ? { kind: 'folder' as const, locked: false } : tree.get(path)
    if (!node || node.kind !== 'folder') throw operationError('NOT_FOUND', 'This folder no longer exists')
    if (node.locked) throw operationError('PERMISSION_DENIED', 'You do not have access to this item')
  }
  const watchers = new Map<string, Set<() => void>>()
  const touched = (path: string) => {
    watchers.get(path)?.forEach((listener) => listener())
  }

  class MockHostReader implements FolderReader {
    readonly url: string
    readonly capabilities = CAPABILITIES
    constructor(url: string) {
      this.url = url
    }
    async list(query: ListQuery): Promise<FileItemPage> {
      await live()
      const path = hostPathOf(this.url)
      requireDir(path)
      const entries = sortEntriesForQuery(childrenOf(path).map((child) => entryOf(session, child)), query.sortKey, query.sortDir, query.foldersFirst)
      const slice = entries.slice(query.offset, query.offset + query.limit)
      return { items: slice.map((entry): FileItem => ({ key: entry.id, entry })), totalCount: entries.length, hasMore: query.offset + slice.length < entries.length }
    }
    async getItem(key: string) {
      const path = key.slice('host:'.length)
      return tree.has(path) ? { key, entry: entryOf(session, path) } : null
    }
    watch(onInvalidate: () => void) {
      const path = hostPathOf(this.url)
      let set = watchers.get(path)
      if (!set) watchers.set(path, (set = new Set()))
      set.add(onInvalidate)
      return () => { set.delete(onInvalidate) }
    }
    dispose() {}
  }

  /** Copies a subtree between the computer and the mock Zone (either direction). */
  const copyInto = (entry: FileEntry, target: string, name: string) => {
    const fromHost = entry.path.startsWith(DEVICE_SCHEME)
    const toHost = target.startsWith(DEVICE_SCHEME)
    const source = fromHost ? hostPathOf(entry.path) : entry.path
    const sourceNode = fromHost ? tree.get(source) : null
    if (fromHost && (!sourceNode || sourceNode.locked)) throw operationError('PERMISSION_DENIED', 'The source is not readable')
    const visit = (from: string, kind: FileEntry['kind'], size: number | undefined, to: string, toName: string) => {
      const destination = childOf(to, toName)
      if (toHost) tree.set(destination, { kind, size, mtime: new Date().toISOString() })
      else mockAddEntry({ id: crypto.randomUUID(), name: toName, kind, path: destination, modifiedAt: new Date().toISOString(), sizeBytes: size })
      if (kind !== 'folder') return
      if (fromHost) for (const child of childrenOf(from)) { const node = tree.get(child)!; if (!node.locked) visit(child, node.kind, node.size, destination, child.slice(child.lastIndexOf('/') + 1)) }
      else for (const child of mockEntriesAtPath(from) ?? []) visit(child.path, child.kind, child.sizeBytes, destination, child.name)
    }
    const to = toHost ? hostPathOf(target) : target
    visit(source, entry.kind, entry.sizeBytes, to, name)
    if (toHost) touched(to)
    else invalidateMockPath(to)
  }

  const statEntry = async (parent: string, name: string): Promise<FileEntry | null> => {
    if (!parent.startsWith(DEVICE_SCHEME)) {
      const found = (mockEntriesAtPath(parent) ?? []).find((entry) => entry.name === name)
      return found ? { ...found } : null
    }
    await live()
    const path = childOf(hostPathOf(parent), name)
    return tree.has(path) ? entryOf(session, path) : null
  }

  const ops: FolderWriteOps = {
    supportsCopy: true,
    async copyEntries(entries: FileEntry[], target: string, options: BatchOptions = {}) {
      await live()
      options.onTask?.({ taskId: crypto.randomUUID(), total: entries.length, cancelling: false })
      const results: OperationResult[] = await runEntryBatch(entries, async (entry, name) => {
        await mockDelay(20, 60)
        if (target.startsWith(DEVICE_SCHEME)) requireDir(hostPathOf(target))
        else if (!mockEntriesAtPath(target)) throw operationError('NOT_FOUND', 'The destination folder no longer exists')
        copyInto(entry, target, name)
      }, target, options, 'copy')
      return results
    },
    async nameExists(parent, name) {
      return (await statEntry(parent, name)) !== null
    },
    statEntry,
    async createFolder(parent, name) {
      await live()
      const path = hostPathOf(parent)
      requireDir(path)
      if (tree.has(childOf(path, name))) throw operationError('CONFLICT', 'This name already exists here')
      dir(childOf(path, name))
      touched(path)
    },
    async renameEntry(entry, name) {
      await live()
      const from = hostPathOf(entry.path)
      const to = childOf(parentOf(from), name)
      if (tree.has(to)) throw operationError('CONFLICT', 'This name already exists here')
      for (const key of [...tree.keys()]) {
        if (key === from || key.startsWith(`${from}/`)) {
          tree.set(to + key.slice(from.length), tree.get(key)!)
          tree.delete(key)
        }
      }
      touched(parentOf(from))
    },
    deleteEntries(entries, options) {
      return runEntryBatch(entries, async (entry) => {
        await live()
        const path = hostPathOf(entry.path)
        if (tree.get(path)?.locked) throw operationError('PERMISSION_DENIED', 'You do not have permission to change this item')
        for (const key of [...tree.keys()]) if (key === path || key.startsWith(`${path}/`)) tree.delete(key)
        touched(parentOf(path))
      }, undefined, options)
    },
    moveEntries(entries, target, options) {
      return runEntryBatch(entries, async (entry, name) => {
        await live()
        const from = hostPathOf(entry.path)
        const to = childOf(hostPathOf(target), name)
        for (const key of [...tree.keys()]) {
          if (key === from || key.startsWith(`${from}/`)) {
            tree.set(to + key.slice(from.length), tree.get(key)!)
            tree.delete(key)
          }
        }
        touched(parentOf(from))
        touched(hostPathOf(target))
      }, target, options)
    },
    downloadUrl(entry) {
      return entry.kind === 'folder' ? null : `data:application/octet-stream,${encodeURIComponent(`mock content of ${entry.name}\n`)}`
    },
    async readContent(path) {
      await live()
      return new Blob([`mock content of ${hostPathOf(path)}\n`])
    },
  }

  return {
    reader: (url) => new MockHostReader(url),
    ops,
    async upload(task, controls) {
      await live()
      const parent = hostPathOf(task.targetUrl)
      requireDir(parent)
      const path = childOf(parent, task.candidate.name)
      if (tree.has(path)) throw { code: 'NAMESPACE_CONFLICT', messageKey: 'filebrowser.transfer.conflict', fallback: `"${task.candidate.name}" already exists here`, retryable: true }
      controls.setStatus('uploading')
      await mockDelay(80, 160)
      controls.setProgress(task.totalBytes)
      tree.set(path, { kind: classifyFileKind(task.candidate.name, task.candidate.mimeType), size: task.candidate.sizeBytes, mtime: new Date().toISOString() })
      touched(parent)
      return entryOf(session, path)
    },
    async children(url) {
      await live()
      const path = hostPathOf(url)
      requireDir(path)
      return {
        folders: childrenOf(path)
          .filter((child) => tree.get(child)!.kind === 'folder')
          .sort((a, b) => a.localeCompare(b))
          .map((child) => ({ name: child.slice(child.lastIndexOf('/') + 1), url: deviceUrl(session, child), locked: tree.get(child)!.locked })),
      }
    },
    close() {
      watchers.clear()
    },
  }
}

async function mockHostImage(name: string): Promise<Blob> {
  const canvas = new OffscreenCanvas(96, 96)
  const context = canvas.getContext('2d')
  if (!context) return new Blob([`mock content of ${name}\n`])
  context.fillStyle = '#4a90d9'
  context.fillRect(0, 0, 96, 96)
  return canvas.convertToBlob({ type: 'image/png' })
}

export function registerMockHostBackend() {
  registerHostBackend({
    async connect(session, grant) {
      await mockDelay(30, 60)
      if (!mockHostInfo()) throw sudoRequiredError()
      return connectionFor(session, grant)
    },
  })
  registerPreviewSourceResolver(HOST_SOURCE_KIND, {
    async resolvePreviewSource(source) {
      const url = extensionRefField(source, 'url') ?? ''
      const name = extensionRefField(source, 'name') ?? url.slice(url.lastIndexOf('/') + 1)
      const path = hostPathOf(url)
      if (!tree.has(path)) throw sudoRequiredError()
      const hint = mediaTypeFromExtension(extensionOf(name))
      const blob = tree.get(path)!.kind === 'image' ? await mockHostImage(name) : new Blob([`mock content of ${path}\n`], { type: 'text/plain' })
      return { originalSource: source, displayName: name, size: blob.size, objectType: 'file', mediaTypeHints: hint ? [hint] : [], readRef: { kind: 'blob', blob } }
    },
  })
}
