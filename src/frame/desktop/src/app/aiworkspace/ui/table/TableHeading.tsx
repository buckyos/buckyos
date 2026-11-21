import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Info, LayoutGrid, Rows3 } from 'lucide-react'
import { useAiwsText } from '../../useAiwsText'
import { TablePop, anchorOf } from './TablePop'
import type { GridStyle } from './appearance'
import type { TableLayout } from './model'

const styleLabels = { grid: 'aiworkspace.table.style.grid', striped: 'aiworkspace.table.style.striped', lines: 'aiworkspace.table.style.lines' }

export function TableHeading({ title, layout, setLayout, source, fields, records, reason, showHeader, style, setAppearance, children }: {
  title: ReactNode; layout: TableLayout; setLayout: (layout: TableLayout) => void
  source: string; fields: number; records: number; reason?: string | null; showHeader: boolean; style: GridStyle
  setAppearance?: (change: { show_header?: boolean; grid_style?: GridStyle }) => void
  children?: ReactNode
}) {
  const { t } = useAiwsText()
  const [position, setPosition] = useState<ReturnType<typeof anchorOf> | null>(null)
  const open = () => setPosition(anchorOf(anchor.current))
  const anchor = useRef<HTMLDivElement>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const cancel = () => clearTimeout(timer.current)
  const close = () => { cancel(); setPosition(null) }
  const leave = () => { cancel(); timer.current = setTimeout(close, 300) }
  useEffect(() => () => clearTimeout(timer.current), [])
  const label = t('aiworkspace.table.properties')
  return <div className="aiws-table-heading" ref={anchor}>
    <div className="aiws-table-heading-title" onPointerEnter={(e) => { if (e.pointerType === 'mouse') { cancel(); timer.current = setTimeout(open, 500) } }}
      onPointerLeave={leave}>{title}</div>
    <button type="button" className="aiws-table-tool" title={t(layout === 'grid' ? 'aiworkspace.table.layout.showCards' : 'aiworkspace.table.layout.showGrid')}
      aria-label={t(layout === 'grid' ? 'aiworkspace.table.layout.showCards' : 'aiworkspace.table.layout.showGrid')} data-testid="aiws-table-toggle-layout" onClick={() => setLayout(layout === 'grid' ? 'cards' : 'grid')}>
      {layout === 'grid' ? <LayoutGrid size={16} /> : <Rows3 size={16} />}</button>
    <button type="button" className="aiws-table-tool" title={label} aria-label={label} aria-expanded={Boolean(position)} data-testid="aiws-table-properties"
      onFocus={(e) => { if (e.currentTarget.matches(':focus-visible')) open() }} onClick={() => { cancel(); open() }}><Info size={16} /></button>
    {children}
    {position && <TablePop anchor={position} onClose={close} label={label} testId="aiws-table-properties-panel">
      <div className="aiws-table-properties" onPointerEnter={cancel} onPointerLeave={leave} onFocus={cancel}>
        <strong>{source}</strong><span>{t('aiworkspace.table.dimensions', { fields, records })}</span>{reason && <span>{reason}</span>}
        <label><input type="checkbox" checked={showHeader} disabled={!setAppearance} onChange={(e) => setAppearance?.({ show_header: e.target.checked })} />{t('aiworkspace.table.showHeader')}</label>
        <label>{t('aiworkspace.table.style')} <select value={style} disabled={!setAppearance} onChange={(e) => setAppearance?.({ grid_style: e.target.value as GridStyle })}>
          {(['grid', 'striped', 'lines'] as const).map((item) => <option key={item} value={item}>{t(styleLabels[item])}</option>)}</select></label>
      </div>
    </TablePop>}
  </div>
}
