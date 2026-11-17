/**
 * The computer behind a Desktop Lite node (NFSP `host` realm), unlocked per
 * Files window with BuckyOS sudo.
 *
 * Each window holds its own connection under a random session key; its
 * locations are `device://<key>/<path>`, so the global reader registry, the
 * folder-op router and the Preview resolvers find the window's connection from
 * the URL alone. The sudo token lives only in that connection (memory): closing
 * the window drops it, and nothing is shared with other windows.
 */

import type { ContentRef } from '../../../components/preview/types'
import type { FileEntry } from '../types'
import type { FolderReader, LocationCapabilities } from './FolderReader'
import type { FolderWriteOps } from './folderOps'
import type { FolderChildrenPage } from './sidebarSources'
import type { TransferTask, UiError } from './state'
import type { TransferControls } from './transfers'
import { registerReaderProvider } from './readerRegistry'
import { deviceUrl, parseDeviceUrl } from './urls'

export const SUDO_REQUIRED = 'SUDO_REQUIRED'
export const HOST_SOURCE_KIND = 'buckyos-host-file'

export interface HostGrant {
  token: string
  expiresAtMs: number
}

/** One unlocked window's view of the computer. */
export interface HostConnection {
  reader(url: string): FolderReader
  ops: FolderWriteOps
  children(url: string, cursor?: string): Promise<FolderChildrenPage>
  /** Upload executor for a transfer into a computer folder. */
  upload?: (task: TransferTask, controls: TransferControls) => Promise<FileEntry>
  close(): void
}

export interface HostBackend {
  /** Opens a sudo session with a fresh grant; fails with SUDO_REQUIRED when the grant is not accepted. */
  connect(session: string, grant: HostGrant): Promise<HostConnection>
}

let backend: HostBackend | null = null
const connections = new Map<string, { connection: HostConnection; grant: HostGrant }>()
const changeListeners = new Map<string, Set<() => void>>()

function changed(session: string) {
  changeListeners.get(session)?.forEach((listener) => listener())
}

export function registerHostBackend(next: HostBackend): () => void {
  backend = next
  return () => {
    if (backend === next) backend = null
  }
}

/** Thrown for a locked window's computer (an Error, so lists can carry it; also a UiError). */
export function sudoRequiredError(): Error & UiError {
  const err = new Error('Unlock this computer to see its folders') as Error & UiError
  err.name = SUDO_REQUIRED
  return Object.assign(err, {
    code: SUDO_REQUIRED,
    messageKey: 'filebrowser.host.sudoRequired',
    fallback: 'Unlock this computer to see its folders',
    retryable: true,
  })
}

export function isSudoRequired(err: unknown): boolean {
  if (!err || typeof err !== 'object') return false
  const value = err as { code?: unknown; name?: unknown }
  return value.code === SUDO_REQUIRED || value.name === SUDO_REQUIRED
}

export function newHostSessionKey(): string {
  return `w${crypto.randomUUID().replace(/-/g, '').slice(0, 10)}`
}

export function hostRootUrl(session: string): string {
  return deviceUrl(session)
}

export async function unlockHost(session: string, grant: HostGrant): Promise<void> {
  if (!backend) throw sudoRequiredError()
  const connection = await backend.connect(session, grant)
  connections.get(session)?.connection.close()
  connections.set(session, { connection, grant })
  changed(session)
}

/** Locks the window's computer again (token expired, window closed, unlock cancelled). */
export function lockHost(session: string) {
  const held = connections.get(session)
  if (!held) return
  connections.delete(session)
  held.connection.close()
  changed(session)
}

export function isHostUnlocked(session: string): boolean {
  const held = connections.get(session)
  return !!held && held.grant.expiresAtMs > Date.now()
}

export function hostGrantExpiry(session: string): number | null {
  return connections.get(session)?.grant.expiresAtMs ?? null
}

/** Fires when the window's computer locks or unlocks. */
export function onHostChanged(session: string, listener: () => void): () => void {
  let set = changeListeners.get(session)
  if (!set) {
    set = new Set()
    changeListeners.set(session, set)
  }
  set.add(listener)
  return () => {
    set.delete(listener)
    if (!set.size) changeListeners.delete(session)
  }
}

/** The live connection behind a device URL; throws SUDO_REQUIRED when the window is locked. */
export function hostConnectionOf(url: string): HostConnection {
  const parts = parseDeviceUrl(url)
  const held = parts ? connections.get(parts.session) : undefined
  if (!held) throw sudoRequiredError()
  if (held.grant.expiresAtMs <= Date.now()) {
    lockHost(parts!.session)
    throw sudoRequiredError()
  }
  return held.connection
}

/** Called by backends when the server answers SUDO_REQUIRED: the window relocks. */
export function reportHostLocked(url: string) {
  const parts = parseDeviceUrl(url)
  if (parts) lockHost(parts.session)
}

export function hostChildren(url: string, cursor?: string): Promise<FolderChildrenPage> {
  try {
    return hostConnectionOf(url).children(url, cursor)
  } catch (err) {
    return Promise.reject(err)
  }
}

/** Preview / thumbnail source of a file on the computer (never a cyfs:// path). */
export function hostContentRef(entry: Pick<FileEntry, 'path' | 'name'>): ContentRef {
  return { kind: HOST_SOURCE_KIND, value: { url: entry.path, name: entry.name } }
}

const LOCKED_CAPABILITIES: LocationCapabilities = {
  kind: 'folder',
  acceptsContent: false,
  acceptsReferences: false,
  removal: null,
  canReorder: false,
  sortKeys: ['name'],
  sortDirs: ['asc'],
  defaultSortKey: 'name',
}

/**
 * Reader for a device URL. It binds to the window's live connection on every
 * page, so a location opened while locked starts listing once unlocked, and a
 * relock (expired token) turns the view back into "Unlock".
 */
class HostLocationReader implements FolderReader {
  readonly url: string
  private inner: FolderReader | null = null
  private bound: HostConnection | null = null
  private listeners = new Set<() => void>()
  private innerUnwatch: (() => void) | null = null
  private sessionUnwatch: (() => void) | null = null

  constructor(url: string) {
    this.url = url
  }

  private current(): FolderReader {
    const connection = hostConnectionOf(this.url)
    if (connection !== this.bound) {
      this.release()
      this.bound = connection
      this.inner = connection.reader(this.url)
      if (this.listeners.size) this.innerUnwatch = this.inner.watch(() => this.notify())
    }
    return this.inner!
  }

  private release() {
    this.innerUnwatch?.()
    this.innerUnwatch = null
    this.inner?.dispose()
    this.inner = null
    this.bound = null
  }

  private notify() {
    this.listeners.forEach((listener) => listener())
  }

  get capabilities(): LocationCapabilities {
    return this.inner?.capabilities ?? LOCKED_CAPABILITIES
  }

  get meta() {
    return this.inner?.meta
  }

  async list(query: Parameters<FolderReader['list']>[0]) {
    const reader = this.current()
    try {
      return await reader.list(query)
    } catch (err) {
      if (isSudoRequired(err)) reportHostLocked(this.url)
      throw err
    }
  }

  async getItem(key: string) {
    return this.inner ? this.inner.getItem(key) : null
  }

  watch(onInvalidate: () => void) {
    this.listeners.add(onInvalidate)
    if (!this.sessionUnwatch) {
      const parts = parseDeviceUrl(this.url)
      if (parts) this.sessionUnwatch = onHostChanged(parts.session, () => this.notify())
    }
    if (this.inner && !this.innerUnwatch) this.innerUnwatch = this.inner.watch(() => this.notify())
    return () => {
      this.listeners.delete(onInvalidate)
    }
  }

  dispose() {
    this.release()
    this.sessionUnwatch?.()
    this.sessionUnwatch = null
    this.listeners.clear()
  }
}

export function registerHostReaders() {
  return registerReaderProvider({
    scheme: 'device',
    create: (url) => new HostLocationReader(url),
  })
}
