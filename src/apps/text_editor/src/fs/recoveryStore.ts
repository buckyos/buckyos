import type { TransferableContentRef } from 'buckyos/content'
import { MAX_FILE_SIZE, type TextFormat } from './codec.ts'
import type { BufferHeader } from './bufferStore.ts'
import { isMissingObject, type AppData } from './appData.ts'
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
/** `id`: the entry's key prefix `<YYYYMMDDTHHMMSS>-<uuid>` in the `recovery` bucket, holding `<fileName>` and `entry.json`. */
export interface RecoveryRecord { id: string; entry: RecoveryEntry; size: number; incomplete?: boolean }
export class RecoveryStore {
  data: AppData
  constructor(data: AppData) { this.data = data }
  async archive(bytes: Uint8Array, entry: Omit<RecoveryEntry, 'format' | 'fileName' | 'createdAt'>): Promise<RecoveryRecord> {
    const name = entry.displayName.replace(/[\/\\\0]/g, '_') || 'untitled.txt'
    const fileName = name === 'entry.json' ? 'entry (content).json' : name
    const createdAt = Date.now()
    const id = `${new Date(createdAt).toISOString().replace(/[-:]/g, '').slice(0, 15)}-${crypto.randomUUID()}`
    const full: RecoveryEntry = { ...entry, format: 'buckyos.text-editor.recovery/1', fileName, createdAt }
    await this.data.put('recovery', `${id}/${fileName}`, bytes)
    await this.writeEntry(id, full)
    return { id, entry: full, size: bytes.length }
  }
  private async writeEntry(id: string, entry: RecoveryEntry): Promise<void> {
    await this.data.put('recovery', `${id}/entry.json`, new TextEncoder().encode(JSON.stringify(entry)), 'application/json')
  }
  async annotate(record: RecoveryRecord, conflictCopyPath: string): Promise<void> {
    const entry = { ...record.entry, conflictCopyPath }
    await this.writeEntry(record.id, entry)
    record.entry = entry
  }
  async read(record: RecoveryRecord): Promise<Uint8Array> { return this.data.read('recovery', `${record.id}/${record.entry.fileName}`, MAX_FILE_SIZE) }
  async remove(record: RecoveryRecord): Promise<void> {
    for (const object of await this.data.list('recovery', `${record.id}/`)) await this.data.remove('recovery', object.key)
  }
  async list(): Promise<RecoveryRecord[]> {
    const entries = new Map<string, Array<{ name: string; size: number; createdAt: number }>>()
    for (const object of await this.data.list('recovery')) {
      const split = object.key.indexOf('/')
      if (split <= 0) continue
      const id = object.key.slice(0, split)
      entries.set(id, [...entries.get(id) ?? [], { name: object.key.slice(split + 1), size: object.size, createdAt: object.createdAt }])
    }
    const records: RecoveryRecord[] = []
    for (const [id, files] of entries) {
      let entry: RecoveryEntry | undefined
      if (files.some(f => f.name === 'entry.json')) {
        try { entry = JSON.parse(new TextDecoder().decode(await this.data.read('recovery', `${id}/entry.json`, 65536))) } catch (e) { if (!isMissingObject(e) && !(e instanceof SyntaxError)) throw e }
      }
      if (entry?.format === 'buckyos.text-editor.recovery/1') {
        records.push({ id, entry, size: files.find(f => f.name === entry!.fileName)?.size ?? 0 })
      } else {
        for (const file of files.filter(f => f.name !== 'entry.json')) {
          records.push({ id, size: file.size, incomplete: true, entry: { format: 'buckyos.text-editor.recovery/1', reason: 'discarded', side: 'mine', displayName: file.name, fileName: file.name, createdAt: file.createdAt, encoding: 'utf-8', bom: false, eol: '\n', origin: { device: '' } } })
        }
      }
    }
    return records.sort((a, b) => b.entry.createdAt - a.entry.createdAt)
  }
  async cleanup(days: { discarded: number; takenOver: number }): Promise<void> {
    for (const record of await this.list()) {
      const retention = record.entry.reason === 'discarded' ? days.discarded : record.entry.reason === 'taken-over' ? days.takenOver : 0
      if (!record.incomplete && retention > 0 && Date.now() - record.entry.createdAt > retention * 86400000) await this.remove(record)
    }
  }
}
export function conflictCopyName(name: string, date: Date, language: string, sequence = 1): string {
  const dot = name.lastIndexOf('.'); const stem = dot > 0 ? name.slice(0, dot) : name; const ext = dot > 0 ? name.slice(dot) : ''
  const pad = (n: number) => String(n).padStart(2, '0')
  const stamp = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`
  return `${stem} (${language.startsWith('zh') ? '冲突副本' : 'conflict copy'} ${stamp})${sequence > 1 ? ` ${sequence}` : ''}${ext}`
}
