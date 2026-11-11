/* The wire between the sheet renderer and its Worker (sheet.worker.ts): workbooks are parsed off the main thread. */

export type SheetKind = 'xlsx' | 'csv'
export type SheetErrorCode = 'encrypted' | 'too-large' | 'corrupted'

export interface SheetData {
  /** Display text of each cell; rows are padded to `columns`. */
  rows: string[][]
  columns: number
  /** Column widths in pixels at scale 1. */
  widths: number[]
  /** Merged ranges inside the shown rows and columns: [first row, first column, last row, last column]. */
  merges: [number, number, number, number][]
  /** The sheet has more rows or columns than are shown. */
  truncated: boolean
}

export type SheetRequest =
  | { id: number; type: 'open'; bytes: ArrayBuffer; kind: SheetKind; name: string; rowLimit: number }
  | { id: number; type: 'read'; index: number }

export type SheetResponse =
  | { id: number; sheets: string[] }
  | { id: number; sheet: SheetData }
  | { id: number; error: SheetErrorCode }
