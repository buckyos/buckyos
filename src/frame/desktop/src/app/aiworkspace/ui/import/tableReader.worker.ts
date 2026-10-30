/* Reading workbooks and SQLite files off the main thread (表格数据导入 TODO §5.3, §5.4): an xlsx's XML parts and a
 * SQLite file (opened in memory with sqlite-wasm's `sqlite3_deserialize`, read-only, no OPFS) are read here and only
 * grids go back. One file at a time: `open` replaces what was open before. */

import sqlite3InitModule, { type Database, type Sqlite3Static } from '@sqlite.org/sqlite-wasm'
import sqliteWasmUrl from '@sqlite.org/sqlite-wasm/sqlite3.wasm?url'
import { listSqliteTables, openXlsx, readSqliteTable, type Grid, type SheetInfo, type SqliteQuery, type SqliteTable, type Workbook } from '@table/mod'

export type ReaderRequest =
  | { id: number; op: 'open-xlsx'; bytes: Uint8Array }
  | { id: number; op: 'open-sqlite'; bytes: Uint8Array }
  | { id: number; op: 'list-tables'; views: boolean }
  | { id: number; op: 'read'; name: string; limit: number }

export type ReaderResponse =
  | { id: number; ok: true; sheets?: SheetInfo[]; tables?: SqliteTable[]; grid?: Grid }
  | { id: number; ok: false; error: string; code?: string }

let workbook: Workbook | null = null
let db: Database | null = null
let sqlite3: Sqlite3Static | null = null

async function sqlite(): Promise<Sqlite3Static> {
  if (sqlite3) return sqlite3
  const start = sqlite3InitModule as unknown as (options: { locateFile: () => string; print: (text: string) => void; printErr: (text: string) => void }) => Promise<Sqlite3Static>
  sqlite3 = await start({ locateFile: () => sqliteWasmUrl, print: () => undefined, printErr: () => undefined })
  return sqlite3
}

const query: SqliteQuery = (sql) => {
  if (!db) throw new Error('no database')
  const columns: string[] = []
  const rows = db.exec(sql, { rowMode: 'array', returnValue: 'resultRows', columnNames: columns }) as unknown[][]
  return { columns, rows }
}

function close() {
  workbook = null
  if (db) { try { db.close() } catch { /* already closed */ } db = null }
}

async function handle(request: ReaderRequest): Promise<ReaderResponse> {
  switch (request.op) {
    case 'open-xlsx': {
      close()
      workbook = await openXlsx(request.bytes)
      return { id: request.id, ok: true, sheets: workbook.sheets }
    }
    case 'open-sqlite': {
      close()
      const s = await sqlite()
      db = new s.oo1.DB()
      const pointer = s.wasm.allocFromTypedArray(request.bytes)
      const rc = s.capi.sqlite3_deserialize(db.pointer!, 'main', pointer, request.bytes.length, request.bytes.length, s.capi.SQLITE_DESERIALIZE_FREEONCLOSE | s.capi.SQLITE_DESERIALIZE_READONLY)
      if (rc !== 0) { close(); return { id: request.id, ok: false, error: `sqlite3_deserialize: ${rc}` } }
      return { id: request.id, ok: true, tables: await listSqliteTables(query) }
    }
    case 'list-tables': return { id: request.id, ok: true, tables: await listSqliteTables(query, request.views) }
    case 'read': {
      if (workbook) {
        const sheet = workbook.sheets.find((s) => s.name === request.name)
        if (!sheet) return { id: request.id, ok: false, error: `no sheet ${request.name}` }
        return { id: request.id, ok: true, grid: await workbook.read(sheet, request.limit) }
      }
      return { id: request.id, ok: true, grid: await readSqliteTable(query, request.name, request.limit) }
    }
  }
}

self.onmessage = (event: MessageEvent<ReaderRequest>) => {
  const request = event.data
  handle(request).then((response) => self.postMessage(response), (error: unknown) => {
    const code = error && typeof error === 'object' && 'code' in error ? String((error as { code: unknown }).code) : undefined
    self.postMessage({ id: request.id, ok: false, error: error instanceof Error ? error.message : String(error), ...(code ? { code } : {}) } satisfies ReaderResponse)
  })
}
