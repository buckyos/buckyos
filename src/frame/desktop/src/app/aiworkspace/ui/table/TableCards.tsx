/* The cards of the table editor (UI 框架 PRD §7.3): one card per record — the first field as its title and at most
 * three more fields — virtualised like the grid and loaded page by page as it scrolls. A tap opens the record's form;
 * a long press starts selecting (then taps add and remove). Side by side when the editor is wide. */

import { useEffect, useState, type RefObject } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { MessageSquare } from 'lucide-react'
import type { FieldDef, QueryRow } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { asFormula, formulaResult } from '../formula'
import type { PagerSnapshot, TablePager } from '../tablePager'
import { formatValue } from '../values'
import type { Column } from './model'
import { CellDisplay } from './RecordForm'
import { usePress } from './usePress'

const CARD_MIN_WIDTH = 260
const GAP = 8
const TITLE_HEIGHT = 24
const LINE_HEIGHT = 24
const PADDING = 22

export interface TableCardsProps {
  tableId: string
  fields: FieldDef[]
  columns: Column[]
  titleField: FieldDef | undefined
  rows: PagerSnapshot
  pager: TablePager
  scrollRef: RefObject<HTMLDivElement | null>
  maxHeight: number | null
  selection: ReadonlySet<string> | null
  onToggle: (recordId: string, on: boolean) => void
  onStartSelect: (recordId: string) => void
  onOpen: (recordId: string) => void
  annotatedRecords: ReadonlySet<string>
}

export function TableCards(props: TableCardsProps) {
  const { t } = useAiwsText()
  const { tableId, columns, titleField, rows, pager, scrollRef } = props
  const lines = columns.filter((column) => column.field.field_id !== titleField?.field_id).slice(0, 3)
  const cardHeight = PADDING + TITLE_HEIGHT + lines.length * LINE_HEIGHT
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    const observer = new ResizeObserver(() => setWidth(el.clientWidth))
    observer.observe(el)
    return () => observer.disconnect()
  }, [scrollRef])
  const lanes = Math.max(1, Math.floor(width / (CARD_MIN_WIDTH + GAP)))
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns unstable functions by design; nothing here is memoised on them.
  const virtualizer = useVirtualizer({ count: rows.total, getScrollElement: () => scrollRef.current, estimateSize: () => cardHeight + GAP, overscan: 6, lanes, paddingStart: GAP, paddingEnd: GAP })
  const items = virtualizer.getVirtualItems()
  const lastIndex = items.length > 0 ? items[items.length - 1].index : 0
  useEffect(() => { pager.ensure(lastIndex) }, [pager, lastIndex])
  const laneWidth = lanes > 1 ? (width - GAP * (lanes + 1)) / lanes : null
  return (
    <div ref={scrollRef} className="aiws-table-scroll aiws-table-cards" style={props.maxHeight === null ? undefined : { maxHeight: props.maxHeight }} data-testid={`aiws-table-scroll-${tableId}`} role="list" aria-label={t('aiworkspace.table.layout.cards')}>
      <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
        {items.map((item) => {
          const row = rows.rows[item.index]
          const style = laneWidth === null
            ? { transform: `translateY(${item.start}px)`, height: cardHeight, left: GAP, right: GAP }
            : { transform: `translateY(${item.start}px)`, height: cardHeight, left: GAP + item.lane * (laneWidth + GAP), width: laneWidth }
          return row
            ? <Card key={item.key} {...props} row={row} lines={lines} style={style} />
            : <div key={item.key} className="aiws-table-card is-loading" style={style} aria-hidden="true">…</div>
        })}
      </div>
    </div>
  )
}

function Card({ row, lines, style, fields, titleField, selection, onToggle, onStartSelect, onOpen, annotatedRecords }: TableCardsProps & { row: QueryRow; lines: Column[]; style: React.CSSProperties }) {
  const { t } = useAiwsText()
  const selected = Boolean(selection?.has(row.record_id))
  const press = usePress({ onTap: () => (selection ? onToggle(row.record_id, !selected) : onOpen(row.record_id)), onLong: () => onStartSelect(row.record_id) })
  const title = titleField ? formatValue(titleField, row.values[titleField.field_id]) : ''
  return (
    <div role="listitem" className="aiws-table-card-item" style={style}>
      <button type="button" className={`aiws-table-card${selected ? ' is-selected' : ''}`} data-testid="aiws-table-card" data-record-id={row.record_id}
        aria-pressed={selection ? selected : undefined} {...press}>
        <span className="aiws-table-card-title">
          {selection && <span className="aiws-table-card-check" aria-hidden="true" data-checked={selected ? 'true' : 'false'} />}
          <span className={title ? '' : 'aiws-muted'}>{title || t('aiworkspace.table.record.untitled')}</span>
          {annotatedRecords.has(row.record_id) && <MessageSquare size={13} className="aiws-table-card-note" aria-label={t('aiworkspace.table.record.hasComments')} />}
        </span>
        {lines.map(({ field }) => {
          const value = row.values[field.field_id]
          const formula = asFormula(value)
          const result = formula ? formulaResult(formula, row.values, fields) : null
          const shown = formula ? (result === null ? `= ${formula.$formula.expr}` : String(result)) : formatValue(field, value)
          return (
            <span key={field.field_id} className="aiws-table-card-line" data-field-id={field.field_id}>
              <span className="aiws-table-card-label">{field.name}</span>
              <span className="aiws-table-card-value"><CellDisplay def={field} value={formula ? undefined : value} shown={shown} /></span>
            </span>
          )
        })}
      </button>
    </div>
  )
}
