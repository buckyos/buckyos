import type { TransferableContentRef } from 'buckyos/content'
import { MAX_FILE_SIZE, type TextFormat } from './codec.ts'
import { isMissing, sourceKey, type DocumentStore } from './documentStore.ts'
import { LocalStore, holdLock } from './localStore.ts'
export interface BufferHeader extends Omit<TextFormat, 'mixedEol'> {
  format: 'buckyos.text-editor.buffer/1'
  bufferId: string
  source?: TransferableContentRef
  displayName: string
  base?: { etag: string; size: number }
  language?: string
  createdAt: number
  updatedAt: number
  origin: { device: string; windowId?: string }
}
export interface BufferRecord { header: BufferHeader; text: string; path: string; version: number; active?: boolean }
export function encodeBuffer(record: Pick<BufferRecord, 'header' | 'text'>): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(record.header) + '\n' + record.text)
}
export function decodeBuffer(bytes: Uint8Array): Pick<BufferRecord, 'header' | 'text'> {
  const value = new TextDecoder('utf-8', { fatal: true }).decode(bytes); const split = value.indexOf('\n')
  if (split < 0 || split > 65536) throw new Error('invalid-buffer')
  const header = JSON.parse(value.slice(0, split)) as BufferHeader
  if (header.format !== 'buckyos.text-editor.buffer/1' || !header.bufferId || !Number.isFinite(header.updatedAt) || !['\n', '\r', '\r\n'].includes(header.eol)) throw new Error('invalid-buffer')
  return { header, text: value.slice(split + 1) }
}
export async function pathKey(source?: TransferableContentRef): Promise<string> {
  if (!source) return 'untitled'
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(sourceKey(source))))
  const alphabet = 'abcdefghijklmnopqrstuvwxyz234567'; let bits = 0; let value = 0; let out = ''
  for (const byte of digest) { value = value << 8 | byte; bits += 8; while (bits >= 5) { out += alphabet[value >>> (bits -= 5) & 31]; if (out.length === 16) return out } }
  return out
}
export class BufferStore {
  private queue = new Map<string, Promise<void>>()
  private locks = new Map<string, () => Promise<void>>()
  root: string; store: DocumentStore; local: LocalStore
  constructor(root: string, store: DocumentStore, local: LocalStore) { this.root = root; this.store = store; this.local = local }
  async create(header: BufferHeader, text: string, version: number): Promise<BufferRecord> {
    const path = `${this.root}/buffers/${await pathKey(header.source)}.${header.bufferId}.buf`
    const release = await holdLock(`te.buffer.${header.bufferId}`)
    if (!release) throw new Error('buffer-in-use')
    this.locks.set(header.bufferId, release)
    return { header, text, path, version }
  }
  async localWrite(record: BufferRecord): Promise<void> { await this.local.put(`buffer:${record.path}`, record) }
  flush(record: BufferRecord): Promise<void> {
    const run = (this.queue.get(record.path) ?? Promise.resolve()).catch(() => {}).then(async () => {
      let etag: string | null = null
      try { etag = (await this.store.stat({ kind: 'cyfs-path', path: record.path })).etag } catch (e) { if (!isMissing(e)) throw e }
      await this.store.write(record.path, encodeBuffer(record), etag)
      const pending = await this.local.get<BufferRecord>(`buffer:${record.path}`)
      if (pending && pending.version <= record.version) await this.local.delete(`buffer:${record.path}`)
    })
    this.queue.set(record.path, run)
    return run
  }
  async remove(record: BufferRecord): Promise<void> {
    await this.queue.get(record.path)?.catch(() => {})
    await this.store.remove(record.path)
    await this.local.delete(`buffer:${record.path}`)
    await this.locks.get(record.header.bufferId)?.(); this.locks.delete(record.header.bufferId)
  }
  release(record: BufferRecord): void { this.locks.get(record.header.bufferId)?.(); this.locks.delete(record.header.bufferId) }
  async active(record: BufferRecord): Promise<boolean> {
    if (this.locks.has(record.header.bufferId)) return false
    const release = await holdLock(`te.buffer.${record.header.bufferId}`)
    if (!release) return true
    await release(); return false
  }
  async list(): Promise<BufferRecord[]> {
    const local = await this.local.entries<BufferRecord>('buffer:')
    const records = new Map(local.map(([, r]) => [r.path, r]))
    for (const entry of await this.store.list(`${this.root}/buffers`)) {
      if (!entry.name.endsWith('.buf')) continue
      const path = `${this.root}/buffers/${entry.name}`
      const data = await this.store.read({ kind: 'cyfs-path', path }, MAX_FILE_SIZE * 3 + 65536)
      const decoded = decodeBuffer(data.bytes)
      if (!records.has(path) || records.get(path)!.header.updatedAt < decoded.header.updatedAt) records.set(path, { ...decoded, path, version: 0 })
    }
    for (const record of records.values()) record.active = await this.active(record)
    return [...records.values()].sort((a, b) => b.header.updatedAt - a.header.updatedAt)
  }
  async replay(): Promise<void> {
    for (const [, record] of await this.local.entries<BufferRecord>('buffer:')) {
      if (await this.active(record)) continue
      let newer = false
      try { newer = decodeBuffer((await this.store.read({ kind: 'cyfs-path', path: record.path }, MAX_FILE_SIZE * 3 + 65536)).bytes).header.updatedAt > record.header.updatedAt } catch (e) { if (!isMissing(e)) throw e }
      if (newer) await this.local.delete(`buffer:${record.path}`)
      else await this.flush(record)
    }
  }
}
