import { useState } from 'react'
import { AlertTriangle, RotateCcw, SlidersHorizontal } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore, useRoutingState } from '../../hooks/use-aicc-store'
import {
  commandStatus,
  commandValue,
  findCommand,
  upsertCommand,
  withCommandValue,
  type RoutingCommand,
} from '../../datamodel/routing'

type FactorCommand = Exclude<RoutingCommand, { kind: 'item_weight' }>

export function WeightFactorControl({ subject, label, compact = false }: { subject: FactorCommand; label: string; compact?: boolean }) {
  const { t } = useI18n()
  const store = useAICCStore()
  const { data: state, mutate } = useRoutingState()
  const [draft, setDraft] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const current = state ? findCommand(state.commands, subject) : undefined
  const status = current ? commandStatus(state, current) : undefined
  const factor = current ? commandValue(current) : 1
  const parsed = Number(draft)
  const valid = draft.trim() !== '' && Number.isFinite(parsed) && parsed >= 0 && parsed !== factor

  const save = async (value: number) => {
    if (!state) return
    setSaving(true)
    setError(null)
    try {
      await store.saveRoutingCommands(upsertCommand(state.commands, withCommandValue(subject, value), value === 1), state.settingsRevision)
      setDraft('')
      await mutate()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
      await mutate()
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className={compact ? 'flex flex-wrap items-center gap-1.5 text-xs' : 'flex flex-col gap-2 rounded-lg p-3 text-xs'} style={compact ? undefined : { background: 'var(--cp-bg)' }} data-testid="weight-factor" data-subject={label}>
      <span className="inline-flex items-center gap-1.5" style={{ color: 'var(--cp-muted)' }} title={t('aiCenter.models.factorHint', 'Routing weight factor. 1.0 is the default; raise it to prefer, lower it to avoid. 0 disables.')}>
        <SlidersHorizontal size={13} />
        {compact ? t('aiCenter.models.factorShort', 'Priority') : t('aiCenter.models.factorTitle', 'Routing priority for {{label}}', { label })}
        <strong className="font-mono tabular-nums" style={{ color: factor === 1 ? 'var(--cp-text)' : 'var(--cp-accent)' }} data-testid="weight-factor-value">× {factor.toFixed(2).replace(/0$/, '')}</strong>
      </span>
      {!compact && (
        <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.models.factorHint', 'Routing weight factor. 1.0 is the default; raise it to prefer, lower it to avoid. 0 disables.')}</span>
      )}
      <form
        className="inline-flex items-center gap-1"
        onSubmit={(event) => {
          event.preventDefault()
          if (valid) void save(parsed)
        }}
      >
        <input
          type="number"
          min={0}
          step={0.1}
          value={draft}
          placeholder={factor.toString()}
          onChange={(event) => setDraft(event.target.value)}
          aria-label={t('aiCenter.models.factorInput', 'Weight factor for {{label}}', { label })}
          className="h-8 w-16 rounded-md px-2 outline-none"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
        />
        <button type="submit" disabled={!valid || saving || !state} className="h-8 rounded-md px-2 disabled:opacity-40" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-accent)' }}>
          {t('aiCenter.models.factorApply', 'Apply')}
        </button>
        {current && (
          <button
            type="button"
            disabled={saving}
            onClick={() => void save(1)}
            aria-label={t('aiCenter.models.factorReset', 'Reset to 1.0')}
            title={t('aiCenter.models.factorReset', 'Reset to 1.0')}
            className="flex h-8 w-8 items-center justify-center rounded-md disabled:opacity-40"
            style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-muted)' }}
          >
            <RotateCcw size={13} />
          </button>
        )}
      </form>
      {status?.staleReason && (
        <span className="inline-flex items-center gap-1" style={{ color: 'var(--cp-warning)' }}><AlertTriangle size={12} />{status.staleReason}</span>
      )}
      {error && <span role="alert" style={{ color: 'var(--cp-danger)' }}>{error}</span>}
    </div>
  )
}
