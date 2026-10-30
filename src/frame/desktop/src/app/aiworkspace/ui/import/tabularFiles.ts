/* Opening an import source (表格数据导入 TODO §5): what kind of file it is (`@table/files`), CSV and TSV read here
 * (encoding and delimiter sniffed, or the wizard's choice), workbooks and SQLite files through the reader Worker
 * (tableReader.worker.ts), a paste from its HTML table or its tab-separated text. Errors carry the words the wizard
 * shows. */

import { CSV_LIMIT, MAX_ROWS, readCsv, readHtmlTable, readPastedText, tabularKind, tidy, type CsvRead, type Delimiter, type Encoding, type Grid, type SheetInfo, type SqliteTable } from '@table/mod'
import { aiwsT } from '../../i18n'
import type { ReaderRequest, ReaderResponse } from './tableReader.worker'
import type { TableImportSource } from './tableImportRequest'

/** SQLite files larger than this are not opened (memory, §5.4). */
export const SQLITE_LIMIT = 256 * 1024 * 1024
/** The original file can be kept (an asset) up to this size. */
export const KEEP_LIMIT = 32 * 1024 * 1024

export class TableReadError extends Error {}

type Pending = { resolve: (r: ReaderResponse & { ok: true }) => void; reject: (e: Error) => void }
type Unid<T> = T extends unknown ? Omit<T, 'id'> : never

/** The reader Worker of one wizard. */
export class TableReader {
  private worker: Worker | null = null
  private next = 1
  private readonly pending = new Map<number, Pending>()
  call(request: Unid<ReaderRequest>, transfer: Transferable[] = []): Promise<ReaderResponse & { ok: true }> {
    if (!this.worker) {
      this.worker = new Worker(new URL('./tableReader.worker.ts', import.meta.url), { type: 'module', name: 'aiworkspace-table-reader' })
      this.worker.onmessage = (event: MessageEvent<ReaderResponse>) => {
        const p = this.pending.get(event.data.id)
        if (!p) return
        this.pending.delete(event.data.id)
        if (event.data.ok) p.resolve(event.data)
        else p.reject(new TableReadError(event.data.code === 'encrypted' ? aiwsT('aiworkspace.import.encrypted') : aiwsT('aiworkspace.import.unreadable', { reason: event.data.error })))
      }
      this.worker.onerror = (event) => { for (const p of this.pending.values()) p.reject(new TableReadError(aiwsT('aiworkspace.import.unreadable', { reason: event.message }))); this.pending.clear() }
    }
    const id = this.next++
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
      this.worker!.postMessage({ ...request, id }, transfer)
    })
  }
  dispose() { this.worker?.terminate(); this.worker = null; this.pending.clear() }
}

export type OpenedSource =
  | { kind: 'clipboard'; name: string; grid: Grid }
  | { kind: 'csv' | 'tsv'; name: string; bytes: Uint8Array; read: CsvRead }
  | { kind: 'xlsx'; name: string; bytes: Uint8Array; sheets: SheetInfo[] }
  | { kind: 'sqlite'; name: string; bytes: Uint8Array; tables: SqliteTable[] }

/** A file name without its extension (the table's title). */
export const baseName = (name: string) => name.replace(/\.[A-Za-z0-9]{1,8}$/, '') || name

/** Read the source as far as the wizard needs: a paste or a CSV completely, a workbook's sheet list, a SQLite file's tables. */
export async function openSource(source: TableImportSource, reader: TableReader, pastedName: string, csv: { encoding?: Encoding | 'auto'; delimiter?: Delimiter | 'auto' } = {}): Promise<OpenedSource> {
  if (source.kind === 'clipboard') {
    const grid = (source.html ? readHtmlTable(pastedName, source.html) : null) ?? (source.text ? readPastedText(pastedName, source.text) : null)
    if (!grid) throw new TableReadError(aiwsT('aiworkspace.import.empty'))
    return { kind: 'clipboard', name: pastedName, grid: tidy(grid) }
  }
  const { name, bytes } = source
  const kind = tabularKind(name, bytes)
  switch (kind) {
    case 'xls': throw new TableReadError(aiwsT('aiworkspace.import.refused.xls'))
    case 'numbers': throw new TableReadError(aiwsT('aiworkspace.import.refused.numbers'))
    case 'ods': throw new TableReadError(aiwsT('aiworkspace.import.refused.ods'))
    case 'xlsx': {
      const r = await reader.call({ op: 'open-xlsx', bytes: bytes.slice() })
      return { kind, name, bytes, sheets: r.sheets ?? [] }
    }
    case 'sqlite': {
      if (bytes.length > SQLITE_LIMIT) throw new TableReadError(aiwsT('aiworkspace.import.fileTooLarge', { size: 256 }))
      const r = await reader.call({ op: 'open-sqlite', bytes: bytes.slice() })
      return { kind, name, bytes, tables: r.tables ?? [] }
    }
    default: {
      if (bytes.length > CSV_LIMIT) throw new TableReadError(aiwsT('aiworkspace.import.fileTooLarge', { size: 64 }))
      const read = readCsv(name, bytes, csv)
      return { kind: kind === 'tsv' ? 'tsv' : 'csv', name, bytes, read: { ...read, grid: tidy(read.grid) } }
    }
  }
}

/** One sheet or table as a grid (one row more than an import takes, so going over the limit shows). */
export async function readPart(reader: TableReader, name: string): Promise<Grid> {
  const r = await reader.call({ op: 'read', name, limit: MAX_ROWS + 2 })
  return tidy(r.grid!)
}
