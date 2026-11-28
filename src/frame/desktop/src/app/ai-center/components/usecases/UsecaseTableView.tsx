import { useState } from 'react'
import { Loader2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { isRevisionConflict } from '../../../../api/aicc_routing'
import { useInvalidateUsecases, useRoutingApi, useUsecaseTable } from '../../hooks/use-routing-api'
import { FALLBACK_LLM, needsAttention, type RoutingUsecasesResponse, type UsecaseRow } from '../../datamodel/usecases'
import { ExactModelLabel, Segmented, StatusDot } from '../policy/PolicyWidgets'
import { buttonStyle, errorText, panelStyle } from '../policy/policy-labels'
import { Switch } from '../shared/Switch'
import { UsecaseTable } from './UsecaseTable'
import { UsecaseEditPanel } from './UsecaseEditPanel'
import { useSourceLabel } from './usecase-labels'

const muted = { color: 'var(--cp-muted)' }

function FallbackLlmCard({ row, table, isAdmin, groupNames, onOpen, onChanged }: {
  row: UsecaseRow
  table: RoutingUsecasesResponse
  isAdmin: boolean
  groupNames: Record<string, string>
  onOpen: () => void
  onChanged: () => void
}) {
  const { t } = useI18n()
  const api = useRoutingApi()
  const sourceLabel = useSourceLabel(groupNames)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const enabled = row.enabled === true
  const toggle = async (next: boolean) => {
    setBusy(true)
    setError(null)
    try {
      await api.setUsecase({ target: 'zone', usecase: FALLBACK_LLM, enabled: next, revision: table.settings_revision })
    } catch (reason) {
      setError(isRevisionConflict(reason)
        ? t('aiCenter.usecases.conflict', 'This table was changed elsewhere and has been reloaded. Check your choice and save again.')
        : errorText(reason))
    } finally {
      setBusy(false)
      onChanged()
    }
  }
  return (
    <section data-testid="fallback-llm" data-enabled={enabled} className="flex flex-col gap-3 rounded-xl p-4" style={panelStyle}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            {t('aiCenter.usecases.name.llm.fallback', 'Fallback LLM')} <span className="font-mono text-xs font-normal" style={muted}>{FALLBACK_LLM}</span>
          </h3>
          <p className="mt-1 text-xs leading-5" style={muted}>
            {t('aiCenter.usecases.fallback.hint', 'Turning it on means at least one LLM can keep the system running: LLM usecases without a model of their own use it, and sessions using it show a warning. A Provider failure never switches to it.')}
          </p>
        </div>
        {isAdmin
          ? <Switch checked={enabled} disabled={busy} onChange={(next) => void toggle(next)} label={t('aiCenter.usecases.name.llm.fallback', 'Fallback LLM')} testId="fallback-llm-switch" />
          : <StatusDot tone={enabled ? 'ok' : 'muted'} label={enabled ? t('aiCenter.usecases.fallback.on', 'On') : t('aiCenter.usecases.fallback.off', 'Off')} />}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs" style={{ color: 'var(--cp-text)' }}>
        <span className="min-w-0" data-testid="fallback-llm-model">
          {row.model
            ? <><ExactModelLabel exactModel={row.model} /> <span style={muted}>· {sourceLabel(row.model_source)}</span></>
            : <span style={muted}>{t('aiCenter.usecases.noModel', 'No model')}</span>}
        </span>
        <button type="button" onClick={onOpen} className="min-h-10 rounded-lg px-3 text-sm" style={buttonStyle.secondary}>{t('aiCenter.usecases.chooseModel', 'Choose model')}</button>
      </div>
      {!isAdmin && <p className="text-xs" style={muted}>{t('aiCenter.usecases.fallback.adminOnly', 'Only an administrator can turn it on or off.')}</p>}
      {error && <p role="alert" className="break-words text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
    </section>
  )
}

/** The usecase table of one view with its filter, the fallback LLM and the edit panel. */
export function UsecaseTableView({ view, groupNames, isAdmin, onChanged }: {
  view: string
  groupNames: Record<string, string>
  isAdmin: boolean
  /** A layer of the view changed (a group's version moves on). */
  onChanged?: () => void
}) {
  const { t } = useI18n()
  const api = useRoutingApi()
  const { data, error, mutate } = useUsecaseTable(view)
  const invalidate = useInvalidateUsecases()
  const table = data?.view === view ? data : undefined
  const [filter, setFilter] = useState<'all' | 'attention'>('all')
  const [open, setOpen] = useState<string | null>(null)
  const [notice, setNotice] = useState<{ usecase: string; kind: 'saved' | 'conflict' } | null>(null)
  const changed = () => {
    void invalidate()
    onChanged?.()
  }

  if (!table) {
    return error ? (
      <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-xl p-3 text-sm" style={{ ...panelStyle, color: 'var(--cp-danger)' }}>
        <span className="min-w-0 break-words">{t('aiCenter.usecases.loadFailed', 'Could not load the usecase table: {{error}}', { error: errorText(error) })}</span>
        <button type="button" className="min-h-10 px-3 underline" onClick={() => void mutate()}>{t('common.retry', 'Retry')}</button>
      </div>
    ) : (
      <div role="status" className="flex items-center justify-center gap-2 py-12 text-sm" style={muted}>
        <Loader2 size={18} className="animate-spin" />{t('aiCenter.usecases.loading', 'Loading usecases…')}
      </div>
    )
  }

  const rows = table.rows.filter((row) => row.usecase !== FALLBACK_LLM)
  const fallbackRow = table.rows.find((row) => row.usecase === FALLBACK_LLM)
  const attention = rows.filter(needsAttention)
  const visible = filter === 'attention' ? attention : rows
  const openRow = table.rows.find((row) => row.usecase === open)
  const openEditor = (row: UsecaseRow) => {
    setNotice(null)
    setOpen(row.usecase)
  }

  return (
    <div className="flex min-w-0 flex-col gap-3" data-testid="usecase-view" data-view={view} data-can-edit={table.can_edit}>
      <div className="sm:w-96">
        <Segmented
          label={t('aiCenter.usecases.filter', 'Show')}
          value={filter}
          onChange={setFilter}
          testId="usecase-filter"
          options={[
            { value: 'all', label: t('aiCenter.usecases.filterAll', 'All ({{count}})', { count: rows.length }) },
            { value: 'attention', label: t('aiCenter.usecases.filterAttention', 'Needs attention ({{count}})', { count: attention.length }) },
          ]}
        />
      </div>
      {visible.length > 0
        ? <UsecaseTable rows={visible} groupNames={groupNames} onOpen={openEditor} />
        : <p className="rounded-xl p-6 text-center text-sm" style={{ ...panelStyle, ...muted }}>{t('aiCenter.usecases.allGood', 'Every usecase has its own model.')}</p>}
      {fallbackRow && <FallbackLlmCard row={fallbackRow} table={table} isAdmin={isAdmin} groupNames={groupNames} onOpen={() => openEditor(fallbackRow)} onChanged={changed} />}
      {openRow && (
        <UsecaseEditPanel
          key={`${openRow.usecase}:${table.revision}:${table.settings_revision}`}
          api={api}
          row={openRow}
          table={table}
          view={view}
          groupNames={groupNames}
          notice={notice?.usecase === openRow.usecase ? notice.kind : null}
          onSaved={(kind) => {
            setNotice({ usecase: openRow.usecase, kind })
            changed()
          }}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  )
}
