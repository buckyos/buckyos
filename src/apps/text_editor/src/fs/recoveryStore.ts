import type { TransferableContentRef } from 'buckyos/content'
import type { TextFormat } from './codec.ts'
import type { BufferHeader } from './bufferStore.ts'
import { isMissing, type DocumentStore } from './documentStore.ts'
export interface RecoveryEntry extends Omit<TextFormat, 'mixedEol'> {
  format: 'buckyos.text-editor.recovery/1'
  reason: 'conflict-reload' | 'conflict-overwrite' | 'deleted-close' | 'discarded' | 'taken-over'
  side: 'mine' | 'disk'
  source?: TransferableContentRef
  displayName: string
  fileName: string
  baseEtag?: string
  diskEtag?: string
  conflictCopyPath?: string
  createdAt: number
  origin: BufferHeader['origin']
}
export interface RecoveryRecord { path: string; entry: RecoveryEntry; size: number; incomplete?: boolean }
export class RecoveryStore {
  root: string; store: DocumentStore
  constructor(root: string, store: DocumentStore) { this.root = root; this.store = store }
  async archive(bytes: Uint8Array, entry: Omit<RecoveryEntry, 'format' | 'fileName' | 'createdAt'>): Promise<RecoveryRecord> {
    const name = entry.displayName.replace(/[\/\\\0]/g, '_') || 'untitled.txt'
    const fileName = name === 'entry.json' ? 'entry (content).json' : name
    const createdAt = Date.now()
    const path = `${this.root}/recovery/${new Date(createdAt).toISOString().replace(/[-:]/g, '').slice(0, 15)}-${crypto.randomUUID()}`
    const full: RecoveryEntry = { ...entry, format: 'buckyos.text-editor.recovery/1', fileName, createdAt }
    await this.store.mkdir(path)
    await this.store.write(`${path}/${fileName}`, bytes, null)
    await this.store.write(`${path}/entry.json`, new TextEncoder().encode(JSON.stringify(full)), null)
    return { path, entry: full, size: bytes.length }
  }
  async annotate(record: RecoveryRecord, conflictCopyPath: string): Promise<void> {
    const path = `${record.path}/entry.json`
    const current = await this.store.stat({ kind: 'cyfs-path', path })
    const entry = { ...record.entry, conflictCopyPath }
    await this.store.write(path, new TextEncoder().encode(JSON.stringify(entry)), current.etag)
    record.entry = entry
  }
  async list(): Promise<RecoveryRecord[]> {
    const records: RecoveryRecord[] = []
    for (const child of await this.store.list(`${this.root}/recovery`)) {
      if (child.target.kind !== 'dir') continue
      const path = `${this.root}/recovery/${child.name}`
      const files = await this.store.list(path)
      let entry: RecoveryEntry | undefined
      try { entry = JSON.parse(new TextDecoder().decode((await this.store.read({ kind: 'cyfs-path', path: `${path}/entry.json` })).bytes)) } catch (e) { if (!isMissing(e) && !(e instanceof SyntaxError)) throw e }
      if (entry?.format === 'buckyos.text-editor.recovery/1') {
        records.push({ path, entry, size: files.find(f => f.name === entry!.fileName)?.target.attrs?.size ?? 0 })
      } else {
        for (const file of files.filter(f => f.name !== 'entry.json' && f.target.kind === 'file')) {
          records.push({ path, size: file.target.attrs?.size ?? 0, incomplete: true, entry: { format: 'buckyos.text-editor.recovery/1', reason: 'discarded', side: 'mine', displayName: file.name, fileName: file.name, createdAt: (file.target.attrs?.mtime ?? 0) * 1000, encoding: 'utf-8', bom: false, eol: '\n', origin: { device: '' } } })
        }
      }
    }
    return records.sort((a, b) => b.entry.createdAt - a.entry.createdAt)
  }
  async cleanup(days: { discarded: number; takenOver: number }): Promise<void> {
    for (const record of await this.list()) {
      const retention = record.entry.reason === 'discarded' ? days.discarded : record.entry.reason === 'taken-over' ? days.takenOver : 0
      if (!record.incomplete && retention > 0 && Date.now() - record.entry.createdAt > retention * 86400000) await this.store.remove(record.path, true)
    }
  }
}
export function conflictCopyName(name: string, date: Date, language: string, sequence = 1): string {
  const dot = name.lastIndexOf('.'); const stem = dot > 0 ? name.slice(0, dot) : name; const ext = dot > 0 ? name.slice(dot) : ''
  const pad = (n: number) => String(n).padStart(2, '0')
  const stamp = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`
  return `${stem} (${language.startsWith('zh') ? '冲突副本' : 'conflict copy'} ${stamp})${sequence > 1 ? ` ${sequence}` : ''}${ext}`
}
