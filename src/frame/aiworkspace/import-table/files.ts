/* Which tabular file a file is (表格数据导入 TODO §5.1): by extension, then by its bytes (`PK\x03\x04` with
 * `xl/workbook.xml` → xlsx, `SQLite format 3\0` → sqlite). `.xls`, `.numbers` and `.ods` are recognised so the
 * user is told to save as xlsx / CSV or to copy and paste instead (Q3). */

import { isSqlite } from './sqlite.ts'
import { sniffWorkbook } from './xlsx.ts'

export type TabularKind = 'csv' | 'tsv' | 'xlsx' | 'sqlite'
export type RefusedKind = 'xls' | 'numbers' | 'ods'

/** The file picker's `accept` list for "import table data". */
export const TABULAR_ACCEPT = '.csv,.tsv,.txt,.xlsx,.xlsm,.sqlite,.sqlite3,.db,text/csv,text/tab-separated-values,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,application/vnd.sqlite3,application/x-sqlite3'

export function tabularKind(name: string, bytes: Uint8Array): TabularKind | RefusedKind | null {
  const lower = name.toLowerCase()
  const sniffed = sniffWorkbook(bytes)
  if (isSqlite(bytes)) return 'sqlite'
  if (sniffed) return sniffed
  if (/\.(xls)$/.test(lower)) return 'xls'
  if (/\.numbers$/.test(lower)) return 'numbers'
  if (/\.ods$/.test(lower)) return 'ods'
  if (lower.endsWith('.csv')) return 'csv'
  if (lower.endsWith('.tsv') || lower.endsWith('.tab')) return 'tsv'
  if (lower.endsWith('.txt')) {
    // a text file is a table when its first lines are tab-separated
    const head = new TextDecoder('utf-8', { fatal: false }).decode(bytes.subarray(0, 4096))
    const lines = head.split(/\r?\n/).slice(0, 5).filter((l) => l.trim() !== '')
    return lines.length > 0 && lines.every((l) => l.includes('\t')) ? 'tsv' : null
  }
  return null
}
