/* Pasted plain text (表格数据导入 TODO §4): spreadsheets put the selection on the clipboard as TSV (cells with line
 * breaks quoted, as in CSV). It is taken for a table when it has at least two rows of the same number (> 1) of
 * tab-separated fields, or one row of three or more; space-aligned text is never guessed (it would catch prose),
 * but the wizard can split a paste on runs of spaces when asked. */

import { parseDelimited } from './csv.ts'
import { textGrid, type Grid } from './grid.ts'

function tsvRows(text: string): string[][] {
  const rows = parseDelimited(text.replace(/\r?\n$/, ''), '\t')
  // a trailing line break leaves no empty record; a lone empty line in between does not count as a row shape
  return rows.filter((r) => r.length > 1 || r[0] !== '')
}

export function looksLikeTsv(text: string): boolean {
  if (!text.includes('\t')) return false
  const rows = tsvRows(text.slice(0, 512 * 1024))
  if (rows.length === 0) return false
  if (rows.length === 1) return rows[0].length >= 3
  const width = rows[0].length
  return width >= 2 && rows.every((r) => r.length === width)
}

export type TextSplit = 'tab' | 'spaces'

/** Pasted text as a grid: tab-separated, or split on runs of two or more spaces (and tabs). */
export function readPastedText(name: string, text: string, split: TextSplit = 'tab'): Grid {
  if (split === 'tab') return textGrid(name, tsvRows(text))
  const rows = text.split(/\r\n|\r|\n/).filter((l) => l.trim() !== '').map((l) => l.trim().split(/\t| {2,}/))
  return textGrid(name, rows)
}
