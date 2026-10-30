/* The SQLite query function for Deno hosts (the CLI, tests): `node:sqlite`, read-only. Integers come back as bigints
 * and are handed on as numbers where they are safe (the importer keeps larger ones as decimal text). */

import { DatabaseSync } from 'node:sqlite'
import type { SqliteQuery } from '../sqlite.ts'

export function openSqliteFile(path: string): { query: SqliteQuery; close: () => void } {
  const db = new DatabaseSync(path, { readOnly: true })
  const query: SqliteQuery = (sql) => {
    const statement = db.prepare(sql)
    statement.setReadBigInts(true)
    const columns = statement.columns().map((c) => c.name)
    const rows = (statement.all() as Record<string, unknown>[]).map((row) => columns.map((c) => {
      const v = row[c]
      return typeof v === 'bigint' && v <= BigInt(Number.MAX_SAFE_INTEGER) && v >= -BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v
    }))
    return { columns, rows }
  }
  return { query, close: () => db.close() }
}
