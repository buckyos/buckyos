import { useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useI18n } from '../../i18n/provider'

export interface DataTableColumn<T> {
  key: string
  title: ReactNode
  render: (row: T) => ReactNode
  className?: string
}

export function DataTable<T>({
  rows,
  columns,
  rowKey,
  onSelect,
  actions,
  pageSize = 10,
}: {
  rows: T[]
  columns: Array<DataTableColumn<T>>
  rowKey: (row: T) => string
  onSelect?: (row: T) => void
  actions?: (row: T) => ReactNode
  pageSize?: number
}) {
  const { t } = useI18n()
  const [page, setPage] = useState(1)
  const pageCount = Math.max(1, Math.ceil(rows.length / pageSize))
  const pagedRows = useMemo(() => rows.slice((page - 1) * pageSize, page * pageSize), [page, pageSize, rows])

  useEffect(() => {
    setPage((current) => Math.min(current, pageCount))
  }, [pageCount])

  return (
    <div className="overflow-hidden rounded-lg border border-[color:var(--cp-border)]">
      <div className="overflow-x-auto shell-scrollbar">
        <table className="min-w-full border-collapse text-left text-sm">
          <thead className="bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)] text-xs uppercase text-[color:var(--cp-muted)]">
            <tr>
              {columns.map((column) => (
                <th className={`whitespace-nowrap px-3 py-2 font-semibold ${column.className ?? ''}`} key={column.key}>
                  {column.title}
                </th>
              ))}
              {actions && <th className="w-1 whitespace-nowrap px-3 py-2 font-semibold">{t('table.actions', 'Actions')}</th>}
            </tr>
          </thead>
          <tbody>
            {pagedRows.map((row, rowIndex) => (
              <tr
                className="cursor-pointer border-t border-[color:var(--cp-border)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_10%,transparent)]"
                key={`${rowKey(row)}-${(page - 1) * pageSize + rowIndex}`}
                onClick={() => onSelect?.(row)}
              >
                {columns.map((column) => (
                  <td className={`px-3 py-2 align-top ${column.className ?? ''}`} key={column.key}>
                    {column.render(row)}
                  </td>
                ))}
                {actions && <td className="whitespace-nowrap px-3 py-2 align-top" onClick={(event) => event.stopPropagation()}>{actions(row)}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {rows.length > pageSize && (
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[color:var(--cp-border)] px-3 py-2 text-xs text-[color:var(--cp-muted)]">
          <span>
            {t('table.pagination', 'Page')} {page}/{pageCount} · {rows.length} {t('table.records', 'records')}
          </span>
          <div className="flex items-center gap-2">
            <button
              className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[color:var(--cp-border)] disabled:opacity-40"
              disabled={page <= 1}
              onClick={() => setPage((value) => Math.max(1, value - 1))}
              type="button"
            >
              <ChevronLeft size={15} />
            </button>
            <button
              className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[color:var(--cp-border)] disabled:opacity-40"
              disabled={page >= pageCount}
              onClick={() => setPage((value) => Math.min(pageCount, value + 1))}
              type="button"
            >
              <ChevronRight size={15} />
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
