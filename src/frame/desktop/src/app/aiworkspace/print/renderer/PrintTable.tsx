/* A table view on a printed page (打印 TODO T9): every row of the view (its filter and order, a Block's row limit), the
 * columns it shows at their widths, one static HTML table. On screen the table is a virtual scroller; here it is laid out
 * whole, and the flow pagination breaks it between rows with its header repeated (`PrintFragmentContext.rows`). On a free
 * canvas the Block's box cuts it off where the screen does. */

import { useCallback, useContext } from 'react'
import { Check } from 'lucide-react'
import type { CellPayload, FieldDef, KeyedContent, QueryRow, TableSourceContent } from '../../api/types'
import { useLoad, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { asFormula, formulaResult } from '../../ui/formula'
import { blockColumns, toneOf, type Column } from '../../ui/table/model'
import { MIN_ROW_HEIGHT, ROW_HEIGHT, tableAppearance } from '../../ui/table/appearance'
import { ReferenceDisplay } from '../../ui/table/TableReference'
import { formatValue } from '../../ui/values'
import { PrintFragmentContext } from './context'

const PAGE = 1000
/** Rows beyond this are not printed (a table this long is a data export, not a page). */
const MAX_ROWS = 20_000

interface TableData { appearance: ReturnType<typeof tableAppearance>; title: string; columns: Column[]; rows: QueryRow[]; fields: FieldDef[] }

function Value({ field, row, fields }: { field: FieldDef; row: QueryRow; fields: FieldDef[] }) {
  const value = row.values[field.field_id]
  const formula = asFormula(value)
  if (formula) {
    const result = formulaResult(formula, row.values, fields)
    return <>{result === null ? `= ${formula.$formula.expr}` : String(result)}</>
  }
  if (field.ref_purpose) return <ReferenceDisplay field={field} value={value} print />
  if (field.type === 'boolean') return value === true ? <Check size={14} aria-label={formatValue(field, value)} /> : null
  if ((field.type === 'select' || field.type === 'multi_select') && value !== undefined && value !== null) {
    const ids = (Array.isArray(value) ? value : [value]).map(String)
    return (
      <span className="aiws-pills">
        {ids.map((id) => <span key={id} className="aiws-pill" data-tone={toneOf(id)}>{field.options?.find((o) => o.option_id === id)?.label ?? id}</span>)}
      </span>
    )
  }
  return <>{formatValue(field, value)}</>
}

export function PrintTable({ cellId, width, height }: { cellId: string; width: number; height: number | null }) {
  const store = useStore()
  const { t } = useAiwsText()
  const fragment = useContext(PrintFragmentContext)
  // a box of fixed height (a free canvas) shows the rows that fit; a flow page prints them all
  const wanted = height === null ? MAX_ROWS : Math.max(1, Math.ceil(height / MIN_ROW_HEIGHT) + 1)
  const load = useCallback(async (): Promise<TableData | null> => {
    const cell = await store.session.read<KeyedContent<CellPayload>>(cellId)
    const payload = cell.content.payload
    const sourceId = payload.source_ref?.entity_id
    if (!sourceId) return null
    const source = await store.session.read<TableSourceContent>(sourceId)
    const fields = source.content.fields
    const columns = blockColumns(fields, payload.fields).filter((c) => c.shown).map(({ field, width: w }) => ({ field, width: w }))
    const rowLimit = (payload as { row_limit?: unknown }).row_limit
    const limit = Math.min(wanted, typeof rowLimit === 'number' ? rowLimit : MAX_ROWS)
    const rows: QueryRow[] = []
    for (let cursor: string | null = null; rows.length < limit;) {
      const page = await store.session.query({ view_id: cellId, limit: Math.min(PAGE, limit - rows.length), ...(cursor ? { cursor } : {}), consistency: 'snapshot' })
      rows.push(...page.rows)
      cursor = page.next_cursor ?? null
      if (!cursor || page.rows.length === 0) break
    }
    return { columns, rows, fields, appearance: tableAppearance(payload.config), title: payload.title ?? source.title ?? source.name ?? '' }
  }, [store, cellId, wanted])
  const data = useLoad(load)
  if (data.error) return <div className="aiws-print-blank" />
  if (!data.data) return <div className="aiws-block-loading" data-testid={`aiws-block-loading-${cellId}`}>{t('aiworkspace.blocks.loading')}</div>
  const { columns, rows, fields, appearance, title } = data.data
  // the view's widths, narrowed together when they do not fit the page
  const total = columns.reduce((sum, c) => sum + c.width, 0)
  const scale = total > width ? width / total : 1
  const [from, to] = fragment?.rows ?? [0, rows.length]
  return (
    <table className="aiws-print-table" data-grid-style={appearance.style} style={{ width: total * scale }} data-testid={`aiws-print-table-${cellId}`} data-rows={rows.length}>
      {!fragment && title && <caption>{title}</caption>}
      <colgroup>{columns.map((c) => <col key={c.field.field_id} style={{ width: c.width * scale }} />)}</colgroup>
      {appearance.showHeader && <thead data-print-header="">
        <tr>{columns.map((c) => <th key={c.field.field_id}>{c.field.name}</th>)}</tr>
      </thead>}
      <tbody>
        {rows.slice(from, to).map((row, i) => (
          <tr key={row.record_id} data-print-row={from + i} data-stripe={(from + i) % 2} style={{ height: appearance.rowHeights[row.record_id] ?? ROW_HEIGHT }}>
            {columns.map((c) => <td key={c.field.field_id} data-type={c.field.type}><Value field={c.field} row={row} fields={fields} /></td>)}
          </tr>
        ))}
      </tbody>
    </table>
  )
}
