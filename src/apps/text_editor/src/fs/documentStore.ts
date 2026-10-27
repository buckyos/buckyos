import { NfspClient, NfspError, type NodeInfo, type Entry, type OpenWriteResult } from 'buckyos/nfsp'
import { contentRefString, type TransferableContentRef } from 'buckyos/content'
import { ContentError, MAX_FILE_SIZE, normalizeEtag } from './codec.ts'
export interface FileSnapshot { source: TransferableContentRef; info: NodeInfo; etag: string; size: number }
export interface FileData extends FileSnapshot { bytes: Uint8Array }
export interface WriteResult { source: TransferableContentRef; etag: string; size: number }
export class SaveConflict extends Error {
  kind: 'external-modified' | 'deleted' | 'lease-busy' | 'copy-failed'
  constructor(kind: SaveConflict['kind']) { super(kind); this.kind = kind }
}
export type ErrorCategory = 'network' | 'permission' | 'not-found' | 'not-dir' | 'quota' | 'other'
export function errorCategory(error: unknown): ErrorCategory {
  if (error instanceof SaveError) return error.category
  if (!(error instanceof NfspError)) return error instanceof TypeError ? 'network' : 'other'
  if (outcomeUnknown(error)) return 'network'
  if (['NOT_FOUND', 'STALE'].includes(error.code)) return 'not-found'
  if (['PERMISSION_DENIED', 'POLICY_DENIED'].includes(error.code)) return 'permission'
  if (['NAMESPACE_CONFLICT', 'NOT_A_CONTAINER'].includes(error.code)) return 'not-dir'
  if (error.code === 'QUOTA_EXCEEDED') return 'quota'
  return 'other'
}
/** The request may have reached the server: the browser lost the connection, or the gateway gave up on it. */
const outcomeUnknown = (error: unknown): boolean => !(error instanceof NfspError) ? error instanceof TypeError : error.code === 'INTERNAL' && error.httpStatus >= 502 && error.httpStatus <= 504
/**
 * A save that failed at a known stage. `prepare` and `upload` never touched the target; after a failed `commit`,
 * `unknown` says the server may still have applied it (the next save checks the disk before writing again).
 */
export class SaveError extends Error {
  stage: 'prepare' | 'upload' | 'commit'; path: string; category: ErrorCategory; unknown: boolean
  constructor(stage: SaveError['stage'], path: string, cause: unknown, unknown = false) {
    super(cause instanceof Error ? cause.message : String(cause), { cause })
    this.stage = stage; this.path = path; this.category = errorCategory(cause); this.unknown = unknown
  }
}
export interface DocumentStore {
  stat(source: TransferableContentRef): Promise<FileSnapshot>
  read(source: TransferableContentRef, maxSize?: number): Promise<FileData>
  /** `createParents`: a formal save may create the missing folders on the way to `path`. */
  write(path: string, bytes: Uint8Array, expected: string | null, options?: { createParents?: boolean }): Promise<WriteResult>
  list(path: string): Promise<Entry[]>
}
export function splitPath(path: string): { parent: string; name: string } {
  const index = path.lastIndexOf('/')
  const name = path.slice(index + 1)
  if (!path.startsWith('cyfs:///') || !name || name === '.' || name === '..' || /[\\\0]/.test(name)) throw new Error('invalid-path')
  return { parent: path.slice(0, index), name }
}
const conflictOf = (e: unknown): SaveConflict | undefined =>
  e instanceof SaveConflict ? e : e instanceof NfspError && e.code === 'LEASE_CONFLICT' ? new SaveConflict('lease-busy') : e instanceof NfspError && e.code === 'TARGET_MISMATCH' ? new SaveConflict('external-modified') : undefined
/** Whether `source` holds exactly `bytes` now (false when that cannot be read either): a save whose answer was lost, or one that changes nothing. */
export async function holdsBytes(store: Pick<DocumentStore, 'read'>, source: TransferableContentRef, bytes: Uint8Array): Promise<boolean> {
  try { const file = await store.read(source, bytes.length); return file.bytes.length === bytes.length && file.bytes.every((b, i) => b === bytes[i]) } catch { return false }
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
    catch (thrown) {
      const error = thrown instanceof SaveError && thrown.stage === 'prepare' ? thrown.cause : thrown
      if (error instanceof NfspError && /session/i.test(error.message) && ['PERMISSION_DENIED', 'INVALID_SESSION'].includes(error.code)) {
        this.connecting ??= this.client.hello().finally(() => { this.connecting = undefined })
        await this.connecting
        return operation()
      }
      throw thrown
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
    if (snapshot.size > maxSize) throw new ContentError('too-large')
    const node = snapshot.info.node_id ?? (snapshot.info.ref.type === 'live' ? snapshot.info.ref.node_id : snapshot.info.ref.obj_id)
    const response = await this.client.readFile(node)
    const reader = response.body?.getReader()
    if (!reader) throw new Error('read-failed')
    const chunks: Uint8Array[] = []; let size = 0
    try {
      for (;;) {
        const result = await reader.read(); if (result.done) break
        size += result.value.length
        if (size > maxSize) throw new ContentError('too-large')
        chunks.push(result.value)
      }
    } finally { await reader.cancel() }
    const bytes = new Uint8Array(size); let offset = 0
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length }
    const readEtag = normalizeEtag(response.headers.get('etag') ?? undefined)
    if (readEtag && readEtag !== snapshot.etag) throw new SaveConflict('external-modified')
    return { ...snapshot, bytes }
  }
  async write(path: string, bytes: Uint8Array, expected: string | null, options: { createParents?: boolean } = {}): Promise<WriteResult> {
    return this.call(async () => {
      const { parent, name } = splitPath(path)
      const source: TransferableContentRef = { kind: 'cyfs-path', path }
      let snapshot: FileSnapshot | undefined
      try { snapshot = await this.stat(source) } catch (e) { if (!isMissing(e)) throw new SaveError('prepare', path, e) }
      if (expected !== null && !snapshot) throw new SaveConflict('deleted')
      if ((snapshot?.etag ?? null) !== expected) throw new SaveConflict('external-modified')
      let lease: OpenWriteResult | undefined
      const release = () => { if (lease) void this.client.abortWrite(lease.lease.lease_id).catch(() => {}) }
      try {
        lease = snapshot ? await this.client.openWrite({ ref: snapshot.info.ref }) :
          await this.client.openWrite({ parentRef: await this.folder(parent, options.createParents === true), name, size: bytes.length })
        if (lease.target.exists !== !!snapshot) throw new SaveConflict('external-modified')
        if (snapshot && (await this.stat(source)).etag !== expected) throw new SaveConflict('external-modified')
      } catch (e) { release(); throw conflictOf(e) ?? (e instanceof SaveConflict || e instanceof SaveError ? e : new SaveError('prepare', path, e)) }
      try { await this.client.uploadContent(lease.fb_handle, bytes) }
      catch (e) { release(); throw conflictOf(e) ?? new SaveError('upload', path, e) }
      try { await this.client.commitFile(parent, name, { fbHandle: lease.fb_handle, leaseId: lease.lease.lease_id }) }
      catch (e) {
        const conflict = conflictOf(e)
        if (conflict) { release(); throw conflict }
        if (!outcomeUnknown(e)) { release(); throw new SaveError('commit', path, e) }
        // The answer was lost: what is on disk now decides whether this save happened.
        if (!await holdsBytes(this, source, bytes)) { release(); throw new SaveError('commit', path, e, true) }
      }
      // Committed. A failed stat only costs the new etag; the next disk check adopts it (see Workspace.checkDisk).
      for (const delay of [0, 500, 1500]) {
        if (delay) await new Promise(resolve => setTimeout(resolve, delay))
        try { const info = await this.stat(source); return { source, etag: info.etag, size: info.size } } catch {}
      }
      return { source, etag: '', size: bytes.length }
    })
  }
  private async folder(path: string, create: boolean): Promise<NodeInfo['ref']> {
    let info: NodeInfo
    try { info = await this.client.resolve(path) }
    catch (e) {
      if (!create || !isMissing(e)) throw new SaveError('prepare', path, e)
      try { await this.client.mkdir(path) }
      catch (made) { if (errorCategory(made) !== 'other') throw new SaveError('prepare', path, made) }
      try { info = await this.client.resolve(path) } catch (again) { throw new SaveError('prepare', path, again) }
    }
    if (info.kind !== 'dir') throw new SaveError('prepare', path, new NfspError({ code: 'NOT_A_CONTAINER', message: `${path} is not a folder` }, 400))
    return info.ref
  }
  async list(path: string): Promise<Entry[]> {
    return this.call(async () => {
      const entries: Entry[] = []; let cursor: string | undefined
      do { const listing = await this.client.list(path, { limit: 1000, cursor }, ['base', 'ident']); entries.push(...listing.entries); cursor = listing.next_cursor } while (cursor)
      return entries
    })
  }
}
export const sourceKey = (source?: TransferableContentRef): string => source ? contentRefString(source) : 'untitled'
