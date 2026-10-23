import type { TransferableContentRef } from 'buckyos/content'
import { encodeText, decodeText, type TextFormat } from './codec.ts'
import { SaveConflict, holdsBytes, isMissing, splitPath, type DocumentStore, type FileSnapshot, type WriteResult } from './documentStore.ts'
import { conflictCopyName, type RecoveryEntry, type RecoveryRecord } from './recoveryStore.ts'
export interface SaveDocument extends TextFormat {
  source?: TransferableContentRef; name: string; text: string; base?: { etag: string; size: number }; readOnly: boolean
}
export interface SaveServices {
  store: DocumentStore
  flush: () => Promise<void>
  archive: (bytes: Uint8Array, entry: Omit<RecoveryEntry, 'format' | 'fileName' | 'createdAt'>) => Promise<RecoveryRecord>
  annotate: (record: RecoveryRecord, path: string) => Promise<void>
  origin: RecoveryEntry['origin']
}
export async function archiveMine(doc: SaveDocument, services: SaveServices, reason: RecoveryEntry['reason']): Promise<void> {
  await services.flush()
  await services.archive(encodeText(doc.text, doc), { source: doc.source, encoding: doc.encoding, bom: doc.bom, eol: doc.eol, displayName: doc.name, reason, side: 'mine', baseEtag: doc.base?.etag, origin: services.origin })
}
/**
 * The draft is already safe in the local buffer when this runs (the caller persisted it); the remote buffer copy
 * is not awaited here, so a slow buffer sync never delays or fails the save itself.
 */
export async function saveDocument(doc: SaveDocument, services: SaveServices,
  options: { path?: string; overwrite?: boolean; conflictCopy?: boolean; recreate?: boolean; locale?: string } = {}): Promise<WriteResult> {
  if (!options.path && (doc.readOnly || doc.source?.kind !== 'cyfs-path')) throw new Error('save-as-required')
  const path = options.path ?? (doc.source as { path: string }).path
  const source: TransferableContentRef = { kind: 'cyfs-path', path }
  const bytes = encodeText(doc.text, doc)
  let disk: FileSnapshot | undefined
  try { disk = await services.store.stat(source) } catch (e) { if (!isMissing(e)) throw e }
  const saveAs = path !== (doc.source?.kind === 'cyfs-path' ? doc.source.path : undefined)
  if (!disk && !saveAs && !options.recreate) throw new SaveConflict('deleted')
  if (disk && (saveAs || disk.etag !== doc.base?.etag) && !options.overwrite) {
    if (!saveAs && await holdsBytes(services.store, source, bytes)) return { source, etag: disk.etag, size: disk.size }
    throw new SaveConflict('external-modified')
  }
  if (disk && options.overwrite) {
    const original = await services.store.read(source)
    let format: TextFormat = { encoding: 'utf-8', bom: false, eol: '\n', mixedEol: false }
    try { format = decodeText(original.bytes) } catch {}
    const record = await services.archive(original.bytes, { ...format, source, displayName: splitPath(path).name,
      reason: 'conflict-overwrite', side: 'disk', diskEtag: original.etag, origin: services.origin })
    disk = original
    if (options.conflictCopy) {
      const { parent, name } = splitPath(path); const now = new Date(); let copied = false
      for (let sequence = 1; sequence <= 100; sequence++) {
        const copyPath = `${parent}/${conflictCopyName(name, now, options.locale ?? 'en', sequence)}`
        try {
          await services.store.write(copyPath, original.bytes, null)
          await services.annotate(record, copyPath); copied = true; break
        } catch (e) {
          if (e instanceof SaveConflict && e.kind === 'external-modified') continue
          throw new SaveConflict('copy-failed')
        }
      }
      if (!copied) throw new SaveConflict('copy-failed')
    }
  }
  return services.store.write(path, bytes, disk?.etag ?? null, { createParents: true })
}
