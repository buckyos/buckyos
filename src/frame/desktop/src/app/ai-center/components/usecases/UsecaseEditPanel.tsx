import { useMemo, useState, type ReactNode } from 'react'
import { Lock, Search } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { isRevisionConflict, type RoutingApi } from '../../../../api/aicc_routing'
import { useLocalModels, useProviders, useUsecaseRequirements } from '../../hooks/use-aicc-store'
import { vendorNames } from '../../datamodel/model-catalog'
import {
  FALLBACK_LLM,
  ON_UNAVAILABLE,
  modelChoices,
  ownLayer,
  targetOfView,
  type OnUnavailable,
  type RoutingUsecasesResponse,
  type UsecaseField,
  type UsecaseRow,
  type UsecaseSetRequest,
} from '../../datamodel/usecases'
import { ExactModelLabel, SidePanel, StatusDot } from '../policy/PolicyWidgets'
import { buttonStyle, errorText, fieldStyle } from '../policy/policy-labels'
import { useCoverageStatus, useFallbackText, useModelPriceText, usePolicyHint, usePolicyLabel, useSourceLabel, useUsecaseName } from './usecase-labels'

type Choice<T> = 'follow' | T

const muted = { color: 'var(--cp-muted)' }
const SEARCH_FROM = 8

function RadioRow({ name, checked, onSelect, label, hint, testId, value }: {
  name: string
  checked: boolean
  onSelect: () => void
  label: ReactNode
  hint?: ReactNode
  testId?: string
  value: string
}) {
  return (
    <label
      data-testid={testId}
      data-value={value}
      className="flex min-h-11 cursor-pointer items-start gap-3 rounded-lg px-3 py-2"
      style={{
        background: checked ? 'color-mix(in srgb, var(--cp-accent) 8%, var(--cp-surface))' : 'var(--cp-surface)',
        border: `1px solid ${checked ? 'var(--cp-accent)' : 'var(--cp-border)'}`,
      }}
    >
      <input type="radio" name={name} value={value} checked={checked} onChange={onSelect} className="mt-1 shrink-0" style={{ accentColor: 'var(--cp-accent)' }} />
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="min-w-0 break-words text-sm" style={{ color: 'var(--cp-text)' }}>{label}</span>
        {hint && <span className="text-xs leading-5" style={muted}>{hint}</span>}
      </span>
    </label>
  )
}

function LockedNote() {
  const { t } = useI18n()
  return (
    <p className="flex items-center gap-1.5 text-xs" data-testid="usecase-edit-locked" style={muted}>
      <Lock size={12} />{t('aiCenter.usecases.lockedHint', 'Locked by the administrator in the Zone default; it cannot be changed here.')}
    </p>
  )
}

/** Choose a usecase's model and its "when unavailable" policy in the view's own layer, or follow the layer below. */
export function UsecaseEditPanel({ api, row, table, view, groupNames, notice, onSaved, onClose }: {
  api: RoutingApi
  row: UsecaseRow
  table: RoutingUsecasesResponse
  view: string
  groupNames: Record<string, string>
  /** The outcome of the last save of this row; the panel is opened again on the reloaded table. */
  notice: 'saved' | 'conflict' | null
  onSaved: (notice: 'saved' | 'conflict') => void
  onClose: () => void
}) {
  const { t } = useI18n()
  const name = useUsecaseName()
  const sourceLabel = useSourceLabel(groupNames)
  const policyLabel = usePolicyLabel()
  const policyHint = usePolicyHint()
  const fallbackText = useFallbackText()
  const coverage = useCoverageStatus()
  const priceText = useModelPriceText()
  const providers = useProviders()
  const localModels = useLocalModels()
  const requirements = useUsecaseRequirements()
  const isZone = view === 'zone'
  const target = targetOfView(view)
  const editable = table.can_edit && target !== null
  const own = ownLayer(row, view)
  const modelLocked = !isZone && row.model_locked === true
  const policyLocked = !isZone && row.on_unavailable_locked === true
  const [model, setModel] = useState<Choice<string>>(own?.model ?? 'follow')
  const [policy, setPolicy] = useState<Choice<OnUnavailable>>(own?.on_unavailable ?? 'follow')
  const [lockModel, setLockModel] = useState(own?.model_locked === true)
  const [lockPolicy, setLockPolicy] = useState(own?.on_unavailable_locked === true)
  const [query, setQuery] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const families = useMemo(() => modelChoices(
    [...providers.filter((provider) => provider.config.enabled).flatMap((provider) => provider.inventory.models), ...localModels],
    row.api_type,
    row.usecase === FALLBACK_LLM ? undefined : requirements[row.usecase],
  ), [localModels, providers, requirements, row.api_type, row.usecase])
  const choiceCount = families.reduce((count, family) => count + family.models.length, 0)
  const needle = query.trim().toLowerCase()
  const shownFamilies = needle
    ? families.map((family) => ({ ...family, models: family.models.filter((entry) => entry.exact_model.toLowerCase().includes(needle)) })).filter((family) => family.models.length > 0)
    : families
  const ownModelMissing = own?.model !== undefined && !families.some((family) => family.models.some((entry) => entry.exact_model === own.model))
  const status = coverage(row)
  const outcome = fallbackText(row)

  const request = ((): UsecaseSetRequest | null => {
    if (!editable || !target) return null
    const next: UsecaseSetRequest = { target, usecase: row.usecase, revision: table.revision }
    const clear: UsecaseField[] = []
    let changed = false
    if (!modelLocked) {
      if (model === 'follow') {
        if (own?.model) clear.push('model')
      } else if (model !== own?.model || (isZone && lockModel !== (own?.model_locked === true))) {
        next.model = isZone ? { value: model, locked: lockModel } : model
        changed = true
      }
    }
    if (!policyLocked) {
      if (policy === 'follow') {
        if (own?.on_unavailable) clear.push('on_unavailable')
      } else if (policy !== own?.on_unavailable || (isZone && lockPolicy !== (own?.on_unavailable_locked === true))) {
        next.on_unavailable = isZone ? { value: policy, locked: lockPolicy } : policy
        changed = true
      }
    }
    if (clear.length) next.clear = clear
    return changed || clear.length ? next : null
  })()

  const save = async () => {
    if (!request) return
    setBusy(true)
    setError(null)
    try {
      await api.setUsecase(request)
      onSaved('saved')
    } catch (reason) {
      if (isRevisionConflict(reason)) {
        onSaved('conflict')
      } else {
        setError(errorText(reason))
      }
    } finally {
      setBusy(false)
    }
  }

  const followHint = isZone
    ? t('aiCenter.usecases.followHint.zone', 'The built-in default chooses.')
    : view === 'self'
      ? t('aiCenter.usecases.followHint.self', 'The Zone default applies.')
      : t('aiCenter.usecases.followHint.group', 'Your own choice or the Zone default applies.')

  return (
    <SidePanel title={name(row.usecase)} onClose={onClose} testId="usecase-edit">
      <div className="flex flex-col gap-5" data-usecase={row.usecase}>
        <section className="flex flex-col gap-1.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="font-mono text-xs" style={muted}>{row.usecase}</span>
            <StatusDot tone={status.tone} label={status.label} />
          </div>
          <p className="text-sm" style={{ color: 'var(--cp-text)' }} data-testid="usecase-edit-current">
            {row.model
              ? <>{t('aiCenter.usecases.currentModel', 'Uses')} <ExactModelLabel exactModel={row.model} /> · <span style={muted}>{sourceLabel(row.model_source)}</span></>
              : <span style={muted}>{t('aiCenter.usecases.noModel', 'No model')}</span>}
          </p>
          {status.detail && <p className="text-xs leading-5" style={muted}>{status.detail}</p>}
          {row.detail && <p className="break-words font-mono text-xs" style={muted}>{row.detail}</p>}
          {notice === 'saved' && <p role="status" data-testid="usecase-edit-saved" className="text-xs" style={{ color: 'var(--cp-success)' }}>{t('aiCenter.usecases.saved', 'Saved. The table shows the result.')}</p>}
          {notice === 'conflict' && <p role="alert" className="text-xs" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.usecases.conflict', 'This table was changed elsewhere and has been reloaded. Check your choice and save again.')}</p>}
          {!editable && (
            <p className="text-xs leading-5" data-testid="usecase-edit-readonly" style={muted}>
              {view.startsWith('app:')
                ? t('aiCenter.usecases.readonlyApp', 'This is what calls from this App see. To change it, edit the routing group bound to the App.')
                : t('aiCenter.usecases.readonly', 'You cannot change this view.')}
            </p>
          )}
        </section>

        <fieldset className="flex min-w-0 flex-col gap-2" disabled={!editable || modelLocked || busy}>
          <legend className="mb-2 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.usecases.col.model', 'Model')}</legend>
          {modelLocked && <LockedNote />}
          <RadioRow name="usecase-model" value="follow" testId="usecase-model-option" checked={model === 'follow'} onSelect={() => setModel('follow')} label={t('aiCenter.usecases.follow', 'Follow the lower layer')} hint={followHint} />
          {ownModelMissing && own?.model && (
            <RadioRow name="usecase-model" value={own.model} testId="usecase-model-option" checked={model === own.model} onSelect={() => setModel(own.model!)} label={<ExactModelLabel exactModel={own.model} />} hint={t('aiCenter.usecases.ownModelUnavailable', 'Your current choice, which cannot be used now; the next layer applies.')} />
          )}
          {choiceCount >= SEARCH_FROM && (
            <label className="flex min-h-10 items-center gap-2 rounded-lg px-3" style={fieldStyle}>
              <Search size={15} style={muted} />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t('aiCenter.usecases.searchModel', 'Search models')}
                aria-label={t('aiCenter.usecases.searchModel', 'Search models')}
                className="min-w-0 flex-1 bg-transparent py-2 text-sm outline-none"
                style={{ color: 'var(--cp-text)' }}
              />
            </label>
          )}
          {shownFamilies.map((family) => (
            <div key={family.family} className="flex flex-col gap-1.5">
              <span className="px-1 pt-1 text-xs font-medium" style={muted}>{vendorNames[family.family] ?? family.family}</span>
              {family.models.map((entry) => (
                <RadioRow
                  key={entry.exact_model}
                  name="usecase-model"
                  value={entry.exact_model}
                  testId="usecase-model-option"
                  checked={model === entry.exact_model}
                  onSelect={() => setModel(entry.exact_model)}
                  label={<ExactModelLabel exactModel={entry.exact_model} />}
                  hint={priceText(entry) ?? t('aiCenter.usecases.priceUnknown', 'Price unknown')}
                />
              ))}
            </div>
          ))}
          {choiceCount === 0 && <p className="text-xs" style={muted}>{t('aiCenter.usecases.noChoices', 'No model you can use fits this usecase. Add a Provider that offers one.')}</p>}
          {isZone && (
            <label className="flex min-h-10 items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <input type="checkbox" data-testid="usecase-lock-model" checked={lockModel && model !== 'follow'} disabled={model === 'follow'} onChange={(event) => setLockModel(event.target.checked)} style={{ accentColor: 'var(--cp-accent)' }} />
              {t('aiCenter.usecases.lockModel', 'Lock: users and groups cannot change the model')}
            </label>
          )}
        </fieldset>

        <fieldset className="flex min-w-0 flex-col gap-2" disabled={!editable || policyLocked || busy}>
          <legend className="mb-2 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.usecases.unavailableTitle', 'When its Provider is unavailable')}</legend>
          {policyLocked && <LockedNote />}
          <RadioRow name="usecase-policy" value="follow" testId="usecase-policy-option" checked={policy === 'follow'} onSelect={() => setPolicy('follow')} label={t('aiCenter.usecases.follow', 'Follow the lower layer')} hint={followHint} />
          {ON_UNAVAILABLE.map((value) => (
            <RadioRow key={value} name="usecase-policy" value={value} testId="usecase-policy-option" checked={policy === value} onSelect={() => setPolicy(value)} label={policyLabel(value)} hint={policyHint(value)} />
          ))}
          <p className="text-xs leading-5" data-testid="usecase-edit-prediction" style={{ color: outcome.warning ? 'var(--cp-warning)' : 'var(--cp-muted)' }}>
            {t('aiCenter.usecases.now', 'Now: {{policy}}', { policy: outcome.outcome ? `${outcome.policy}, ${outcome.outcome}` : outcome.policy })}
          </p>
          <p className="text-xs leading-5" style={muted}>{t('aiCenter.usecases.trigger', 'It switches at most once per request, and only when the Provider fails after retries (timeout, server error, no connection, model gone, quota used up) or is marked faulty. Errors in the request itself never switch.')}</p>
          {isZone && (
            <label className="flex min-h-10 items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <input type="checkbox" data-testid="usecase-lock-policy" checked={lockPolicy && policy !== 'follow'} disabled={policy === 'follow'} onChange={(event) => setLockPolicy(event.target.checked)} style={{ accentColor: 'var(--cp-accent)' }} />
              {t('aiCenter.usecases.lockPolicy', 'Lock: users and groups cannot change this policy')}
            </label>
          )}
        </fieldset>

        {(row.layers?.length ?? 0) > 0 && (
          <section className="flex flex-col gap-1">
            <h3 className="text-xs font-semibold" style={muted}>{t('aiCenter.usecases.layersTitle', 'Set in')}</h3>
            {row.layers!.map((layer) => (
              <p key={layer.layer} className="break-words text-xs" style={{ color: 'var(--cp-text)' }} data-testid="usecase-layer" data-layer={layer.layer}>
                <span className="font-medium">{sourceLabel(layer.layer)}</span>
                {layer.model && <> · {layer.model}{layer.model_locked ? ` (${t('aiCenter.usecases.lockedShort', 'locked')})` : ''}</>}
                {layer.on_unavailable && <> · {policyLabel(layer.on_unavailable)}{layer.on_unavailable_locked ? ` (${t('aiCenter.usecases.lockedShort', 'locked')})` : ''}</>}
              </p>
            ))}
          </section>
        )}

        {error && <p role="alert" className="break-words text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
        {editable && (
          <div
            className="sticky flex flex-col-reverse gap-2 pt-3 sm:flex-row sm:justify-end"
            style={{ background: 'var(--cp-bg)', borderTop: '1px solid var(--cp-border)', bottom: 'calc(-1rem - env(safe-area-inset-bottom))', paddingBottom: 'calc(1rem + env(safe-area-inset-bottom))' }}
          >
            <button type="button" onClick={onClose} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.close', 'Close')}</button>
            <button type="button" data-testid="usecase-save" disabled={!request || busy} onClick={() => void save()} className="min-h-11 rounded-lg px-5 text-sm font-medium disabled:opacity-40" style={buttonStyle.primary}>
              {busy ? t('common.saving', 'Saving...') : t('common.save', 'Save')}
            </button>
          </div>
        )}
      </div>
    </SidePanel>
  )
}
