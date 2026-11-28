import type { KeyboardEvent } from 'react'
import { useMediaQuery } from '@mui/material'
import { AlertTriangle, Lock } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { skippedLayers, type UsecaseRow } from '../../datamodel/usecases'
import { ExactModelLabel, StatusDot } from '../policy/PolicyWidgets'
import { panelStyle } from '../policy/policy-labels'
import { useCoverageStatus, useFallbackText, useModelStateLabel, useSourceLabel, useUsecaseName } from './usecase-labels'

const muted = { color: 'var(--cp-muted)' }

function LockMark() {
  const { t } = useI18n()
  const label = t('aiCenter.usecases.locked', 'Locked by the administrator')
  return <span role="img" aria-label={label} title={label} data-testid="usecase-lock" className="inline-flex shrink-0" style={muted}><Lock size={12} aria-hidden /></span>
}

function ModelCell({ row, groupNames }: { row: UsecaseRow; groupNames: Record<string, string> }) {
  const { t } = useI18n()
  const sourceLabel = useSourceLabel(groupNames)
  const stateLabel = useModelStateLabel()
  const status = useCoverageStatus()(row)
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="inline-flex min-w-0 items-center gap-1" data-testid="usecase-model">
        {row.model ? <ExactModelLabel exactModel={row.model} /> : <span className="leading-5" data-testid="usecase-reason" style={muted}>{status.detail ?? '—'}</span>}
        {row.model_locked && <LockMark />}
      </span>
      {skippedLayers(row).map((layer) => (
        <span key={layer.layer} data-testid="usecase-skipped" data-state={layer.model_state} className="flex items-start gap-1 text-xs" style={{ color: 'var(--cp-warning)' }}>
          <AlertTriangle size={12} className="mt-0.5 shrink-0" />
          <span className="min-w-0 break-words">
            {t('aiCenter.usecases.skipped', '{{layer}} chose {{model}}, which {{state}}; the next layer applies.', {
              layer: sourceLabel(layer.layer),
              model: layer.model ?? '',
              state: stateLabel(layer.model_state ?? 'missing'),
            })}
          </span>
        </span>
      ))}
    </div>
  )
}

function PolicyCell({ row }: { row: UsecaseRow }) {
  const text = useFallbackText()(row)
  if (!row.model && row.on_unavailable === 'none') return <span data-testid="usecase-policy" data-policy="none" style={muted}>—</span>
  return (
    <div className="flex min-w-0 flex-col gap-0.5" data-testid="usecase-policy" data-policy={row.on_unavailable}>
      <span className="inline-flex items-center gap-1">{text.policy}{row.on_unavailable_locked && <LockMark />}</span>
      {text.outcome && <span className="break-words text-xs" style={{ color: text.warning ? 'var(--cp-warning)' : 'var(--cp-muted)' }}>{text.outcome}</span>}
    </div>
  )
}

function StatusCell({ row }: { row: UsecaseRow }) {
  const status = useCoverageStatus()(row)
  return (
    <span title={status.detail} className="inline-flex">
      <StatusDot testId="usecase-status" status={row.enabled !== undefined ? (row.enabled ? 'on' : 'off') : row.coverage} tone={status.tone} label={status.label} />
    </span>
  )
}

/** The usecase table of one view: a table on desktop, one card per usecase on phones. Rows open `onOpen`. */
export function UsecaseTable({ rows, groupNames, onOpen }: { rows: UsecaseRow[]; groupNames: Record<string, string>; onOpen: (row: UsecaseRow) => void }) {
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const name = useUsecaseName()
  const sourceLabel = useSourceLabel(groupNames)
  const onKey = (row: UsecaseRow) => (event: KeyboardEvent) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onOpen(row)
    }
  }

  if (isMobile) {
    return (
      <div className="flex flex-col gap-2" data-testid="usecase-table">
        {rows.map((row) => (
          <article
            key={row.usecase}
            role="button"
            tabIndex={0}
            data-testid="usecase-row"
            data-usecase={row.usecase}
            data-coverage={row.coverage}
            onClick={() => onOpen(row)}
            onKeyDown={onKey(row)}
            className="flex min-w-0 flex-col gap-2 rounded-xl p-3 text-left"
            style={panelStyle}
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{name(row.usecase)}</div>
                <div className="truncate font-mono text-xs" style={muted}>{row.usecase}</div>
              </div>
              <StatusCell row={row} />
            </div>
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-xs" style={{ color: 'var(--cp-text)' }}>
              <dt style={muted}>{t('aiCenter.usecases.col.model', 'Model')}</dt>
              <dd className="min-w-0"><ModelCell row={row} groupNames={groupNames} /></dd>
              <dt style={muted}>{t('aiCenter.usecases.col.source', 'Source')}</dt>
              <dd data-testid="usecase-source">{row.model ? sourceLabel(row.model_source) : '—'}</dd>
              <dt style={muted}>{t('aiCenter.usecases.col.unavailable', 'When unavailable')}</dt>
              <dd className="min-w-0"><PolicyCell row={row} /></dd>
            </dl>
          </article>
        ))}
      </div>
    )
  }

  return (
    <div className="overflow-x-auto rounded-xl" style={panelStyle} data-testid="usecase-table">
      <table className="w-full min-w-[640px] table-fixed text-sm" style={{ color: 'var(--cp-text)' }}>
        <colgroup>
          <col className="w-[22%]" />
          <col className="w-[30%]" />
          <col className="w-[13%]" />
          <col className="w-[21%]" />
          <col className="w-[14%]" />
        </colgroup>
        <thead>
          <tr className="text-left text-xs" style={{ ...muted, borderBottom: '1px solid var(--cp-border)' }}>
            <th className="px-3 py-2 font-medium">{t('aiCenter.usecases.col.usecase', 'Usecase')}</th>
            <th className="px-3 py-2 font-medium">{t('aiCenter.usecases.col.model', 'Model')}</th>
            <th className="px-3 py-2 font-medium">{t('aiCenter.usecases.col.source', 'Source')}</th>
            <th className="px-3 py-2 font-medium">{t('aiCenter.usecases.col.unavailable', 'When unavailable')}</th>
            <th className="px-3 py-2 font-medium">{t('aiCenter.usecases.col.status', 'Status')}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={row.usecase}
              tabIndex={0}
              data-testid="usecase-row"
              data-usecase={row.usecase}
              data-coverage={row.coverage}
              onClick={() => onOpen(row)}
              onKeyDown={onKey(row)}
              className="cursor-pointer align-top outline-none hover:bg-[color:var(--cp-surface-2)] focus-visible:bg-[color:var(--cp-surface-2)]"
              style={{ borderTop: '1px solid var(--cp-border)' }}
            >
              <td className="px-3 py-2.5">
                <div className="font-medium">{name(row.usecase)}</div>
                <div className="truncate font-mono text-xs" style={muted}>{row.usecase}</div>
              </td>
              <td className="px-3 py-2.5 text-xs"><ModelCell row={row} groupNames={groupNames} /></td>
              <td className="px-3 py-2.5 text-xs" data-testid="usecase-source">{row.model ? sourceLabel(row.model_source) : '—'}</td>
              <td className="px-3 py-2.5 text-xs"><PolicyCell row={row} /></td>
              <td className="px-3 py-2.5"><StatusCell row={row} /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
