import { NfspClient, NfspError, type NodeInfo, type Entry, type OpenWriteResult } from 'buckyos/nfsp'
import { contentRefString, type TransferableContentRef } from 'buckyos/content'
import { MAX_FILE_SIZE, normalizeEtag } from './codec.ts'
export interface FileSnapshot { source: TransferableContentRef; info: NodeInfo; etag: string; size: number }
export interface FileData extends FileSnapshot { bytes: Uint8Array }
export class SaveConflict extends Error {
  kind: 'external-modified' | 'deleted' | 'lease-busy' | 'copy-failed'
  constructor(kind: SaveConflict['kind']) { super(kind); this.kind = kind }
}
export interface DocumentStore {
  stat(source: TransferableContentRef): Promise<FileSnapshot>
  read(source: TransferableContentRef, maxSize?: number): Promise<FileData>
  write(path: string, bytes: Uint8Array, expected: string | null): Promise<FileSnapshot>
  list(path: string): Promise<Entry[]>
  mkdir(path: string): Promise<void>
  remove(path: string, recursive?: boolean): Promise<void>
}
export function splitPath(path: string): { parent: string; name: string } {
  const index = path.lastIndexOf('/')
  const name = path.slice(index + 1)
  if (!path.startsWith('cyfs:///') || !name || name === '.' || name === '..' || /[\\\0]/.test(name)) throw new Error('invalid-path')
  return { parent: path.slice(0, index), name }
}
export const isMissing = (error: unknown): boolean => error instanceof SaveConflict && error.kind === 'deleted' || error instanceof NfspError && ['NOT_FOUND', 'STALE'].includes(error.code)
export class NfspDocumentStore implements DocumentStore {
  client: NfspClient
  private connecting?: Promise<unknown>
  constructor(client: NfspClient) { this.client = client }
  async connect(): Promise<void> {
    if (this.client.sessionId) return
    this.connecting ??= this.client.hello().finally(() => { this.connecting = undefined })
    await this.connecting
  }
  private async call<T>(operation: () => Promise<T>): Promise<T> {
    await this.connect()
    try { return await operation() }
    catch (error) {
      if (error instanceof NfspError && /session/i.test(error.message) && ['PERMISSION_DENIED', 'INVALID_SESSION'].includes(error.code)) {
        this.connecting ??= this.client.hello().finally(() => { this.connecting = undefined })
        await this.connecting
        return operation()
      }
      throw error
    }
  }
  async stat(source: TransferableContentRef): Promise<FileSnapshot> {
    return this.call(async () => {
      const info = await this.client.resolve(source.kind === 'cyfs-path' ? source.path : { type: 'object', obj_id: source.objectId }, ['base', 'ident', 'access'])
      if (info.kind !== 'file') throw new Error('not-a-file')
      return { source, info, etag: normalizeEtag(info.etag), size: info.size ?? 0 }
    })
  }
  async read(source: TransferableContentRef, maxSize = MAX_FILE_SIZE): Promise<FileData> {
    const snapshot = await this.stat(source)
    if (snapshot.size > maxSize) throw new Error('too-large')
    const node = snapshot.info.node_id ?? (snapshot.info.ref.type === 'live' ? snapshot.info.ref.node_id : snapshot.info.ref.obj_id)
    const response = await this.client.readFile(node)
    const reader = response.body?.getReader()
    if (!reader) throw new Error('read-failed')
    const chunks: Uint8Array[] = []; let size = 0
    try {
      for (;;) {
        const result = await reader.read(); if (result.done) break
        size += result.value.length
        if (size > maxSize) throw new Error('too-large')
        chunks.push(result.value)
      }
    } finally { await reader.cancel() }
    const bytes = new Uint8Array(size); let offset = 0
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length }
    const readEtag = normalizeEtag(response.headers.get('etag') ?? undefined)
    if (readEtag && readEtag !== snapshot.etag) throw new SaveConflict('external-modified')
    return { ...snapshot, bytes }
  }
  async write(path: string, bytes: Uint8Array, expected: string | null): Promise<FileSnapshot> {
    return this.call(async () => {
      const { parent, name } = splitPath(path)
      let snapshot: FileSnapshot | undefined
      try { snapshot = await this.stat({ kind: 'cyfs-path', path }) } catch (e) { if (!isMissing(e)) throw e }
      if (expected !== null && !snapshot) throw new SaveConflict('deleted')
      if ((snapshot?.etag ?? null) !== expected) throw new SaveConflict('external-modified')
      let lease: OpenWriteResult | undefined
      try {
        lease = snapshot ? await this.client.openWrite({ ref: snapshot.info.ref }) :
          await this.client.openWrite({ parentRef: (await this.client.resolve(parent)).ref, name, size: bytes.length })
        if (lease.target.exists !== !!snapshot) throw new SaveConflict('external-modified')
        if (snapshot && (await this.stat({ kind: 'cyfs-path', path })).etag !== expected) throw new SaveConflict('external-modified')
        await this.client.uploadContent(lease.fb_handle, bytes)
        const committed = await this.client.commitFile(parent, name, { fbHandle: lease.fb_handle, leaseId: lease.lease.lease_id })
        const info = await this.client.stat(committed.ref, { want: ['base', 'ident', 'access'] })
        return { source: { kind: 'cyfs-path', path }, info, etag: normalizeEtag(info.etag), size: info.size ?? bytes.length }
      } catch (e) {
        if (e instanceof NfspError && e.code === 'LEASE_CONFLICT') throw new SaveConflict('lease-busy')
        if (e instanceof NfspError && e.code === 'TARGET_MISMATCH') throw new SaveConflict('external-modified')
        throw e
      } finally {
        if (lease) await this.client.abortWrite(lease.lease.lease_id)
      }
    })
  }
  async list(path: string): Promise<Entry[]> {
    return this.call(async () => {
      const entries: Entry[] = []; let cursor: string | undefined
      do { const listing = await this.client.list(path, { limit: 1000, cursor }, ['base', 'ident']); entries.push(...listing.entries); cursor = listing.next_cursor } while (cursor)
      return entries
    })
  }
  async mkdir(path: string): Promise<void> { await this.call(() => this.client.mkdir(path)) }
  async remove(path: string, recursive = false): Promise<void> {
    const { parent, name } = splitPath(path)
    try { await this.call(() => this.client.delete(parent, name, { recursive })) } catch (e) { if (!isMissing(e)) throw e }
  }
}
export const sourceKey = (source?: TransferableContentRef): string => source ? contentRefString(source) : 'untitled'
