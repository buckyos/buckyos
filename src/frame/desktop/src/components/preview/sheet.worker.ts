/* Reads a workbook for the sheet renderer with the table importer's own readers (`@table`): xlsx / xlsm and
 * csv / tsv. The file is the user's: it is parsed here, off the main thread, and what a zip may inflate to is capped. */

import { readCsv } from '@table/csv.ts'
import type { Grid } from '@table/grid.ts'
import { openXlsx, XlsxError, type InflateBudget, type SheetInfo, type Workbook } from '@table/xlsx.ts'
import type { SheetData, SheetErrorCode, SheetRequest, SheetResponse } from './sheetData'

const INFLATE_LIMIT = 512 * 1024 * 1024
const COLUMN_LIMIT = 256
const WIDTH_SAMPLE_ROWS = 500

let rowLimit = 0
let read: ((index: number) => Promise<Grid>) | null = null

const textWidth = (text: string) => {
  let width = 0
  for (let i = 0; i < text.length && width < 48; i += 1) width += text.charCodeAt(i) > 0x2e7f ? 2 : 1
  return width
}

function toSheet(grid: Grid): SheetData {
  const shown = grid.rows.length > rowLimit ? grid.rows.slice(0, rowLimit) : grid.rows
  let width = 0
  for (const row of shown) width = Math.max(width, row.length)
  for (const m of grid.merges ?? []) if (m[0] < shown.length) width = Math.max(width, m[3] + 1)
  const columns = Math.min(width, COLUMN_LIMIT)
  const chars = new Array<number>(columns).fill(4)
  const rows = shown.map((row, r) => {
    const out = new Array<string>(columns)
    for (let c = 0; c < columns; c += 1) {
      const text = row[c]?.text ?? ''
      out[c] = text
      if (r < WIDTH_SAMPLE_ROWS && text) chars[c] = Math.max(chars[c], textWidth(text))
    }
    return out
  })
  const merges: SheetData['merges'] = []
  for (const [r0, c0, r1, c1] of grid.merges ?? []) {
    if (r0 >= rows.length || c0 >= columns) continue
    merges.push([r0, c0, Math.min(r1, rows.length - 1), Math.min(c1, columns - 1)])
  }
  return {
    rows,
    columns,
    widths: chars.map((n) => Math.min(n, 40) * 7.5 + 18),
    merges,
    truncated: grid.rows.length > rowLimit || !!grid.truncated || width > COLUMN_LIMIT,
  }
}

async function open(request: Extract<SheetRequest, { type: 'open' }>): Promise<string[]> {
  const bytes = new Uint8Array(request.bytes)
  rowLimit = request.rowLimit
  if (request.kind === 'csv') {
    const { grid } = readCsv(request.name, bytes)
    read = async () => grid
    return [grid.name]
  }
  // An OLE2 container named .xlsx is an encrypted workbook.
  if (bytes.length > 8 && bytes[0] === 0xd0 && bytes[1] === 0xcf && bytes[2] === 0x11 && bytes[3] === 0xe0) throw new XlsxError('encrypted', 'the workbook is encrypted')
  const budget: InflateBudget = { left: INFLATE_LIMIT }
  const workbook: Workbook = await openXlsx(bytes, budget)
  const sheets: SheetInfo[] = workbook.sheets.filter((sheet) => !sheet.hidden)
  if (sheets.length === 0) throw new XlsxError('corrupt', 'no visible sheet')
  read = (index) => workbook.read(sheets[index], rowLimit)
  return sheets.map((sheet) => sheet.name)
}

function errorCode(err: unknown): SheetErrorCode {
  if (err instanceof XlsxError) return err.code === 'encrypted' ? 'encrypted' : err.code === 'too_large' ? 'too-large' : 'corrupted'
  return err instanceof RangeError ? 'too-large' : 'corrupted'
}

self.onmessage = (event: MessageEvent<SheetRequest>) => {
  const request = event.data
  const reply = (response: SheetResponse) => (self as unknown as Worker).postMessage(response)
  void (async () => {
    try {
      if (request.type === 'open') reply({ id: request.id, sheets: await open(request) })
      else if (read) reply({ id: request.id, sheet: toSheet(await read(request.index)) })
    } catch (err) {
      reply({ id: request.id, error: errorCode(err) })
    }
  })()
}
