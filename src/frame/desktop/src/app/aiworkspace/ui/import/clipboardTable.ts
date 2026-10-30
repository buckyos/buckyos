/* Whether a paste is a table (表格数据导入 TODO §4): an HTML `<table>` (Excel, Numbers, Google Sheets, Notion, web
 * pages, aiws's own "copy as table"), else tab-separated text of at least two equal rows (or one row of three or more
 * fields). Space-aligned text is never taken for a table. Safari may hand no HTML: the TSV is used then. */

import { looksLikeHtmlTable, looksLikeTsv } from '@table/mod'
import type { TableImportSource } from './tableImportRequest'

export function clipboardTable(data: DataTransfer): (TableImportSource & { kind: 'clipboard' }) | null {
  const html = data.getData('text/html')
  const text = data.getData('text/plain')
  if (html && looksLikeHtmlTable(html)) return { kind: 'clipboard', html, text: text || null }
  if (text && looksLikeTsv(text)) return { kind: 'clipboard', html: null, text }
  return null
}
