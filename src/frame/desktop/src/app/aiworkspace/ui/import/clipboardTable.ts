/* Whether a paste is a table (表格数据导入 TODO §4): an HTML `<table>` (Excel, Numbers, Google Sheets, Notion, web
 * pages, a BuckyOS typed table such as aiws's own "copy as table"), else tab-separated text of at least two equal rows
 * (or one row of three or more fields). Space-aligned text is never taken for a table. Safari may hand no HTML: the
 * TSV is used then. */

import { looksLikeHtmlTable, looksLikeTsv } from '@table/mod'
import type { TableImportSource } from './tableImportRequest'

/** `data`: what a paste carried (a `buckyos/transfer` reader, or its HTML and text). */
export function clipboardTable(data: { html?: string; text?: string }): (TableImportSource & { kind: 'clipboard' }) | null {
  const { html, text } = data
  if (html && looksLikeHtmlTable(html)) return { kind: 'clipboard', html, text: text || null }
  if (text && looksLikeTsv(text)) return { kind: 'clipboard', html: null, text }
  return null
}
