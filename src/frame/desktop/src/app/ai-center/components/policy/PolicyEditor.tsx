import { useState } from 'react'
import { ChevronDown, Trash2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import {
  providerVisibilityChoices,
  withModel,
  withProvider,
  type EditorMode,
  type PolicyBody,
  type ProviderVisibility,
} from '../../datamodel/policy'
import { CoefficientField, ExactModelLabel, LimitsEditor, Segmented } from './PolicyWidgets'
import { fieldStyle, panelStyle } from './policy-labels'

export interface EditorProvider { name: string; enabled: boolean; models: string[] }

interface PolicyEditorProps {
  mode: EditorMode
  body: PolicyBody
  onChange: (body: PolicyBody) => void
  providers: EditorProvider[]
  /** A Provider or model to open (from a Usage row). */
  focus?: { provider?: string; model?: string }
}

type VisibilityChoice = 'unset' | ProviderVisibility

function useVisibilityLabel() {
  const { t } = useI18n()
  return (choice: VisibilityChoice) => {
    switch (choice) {
      case 'unset': return t('aiCenter.policy.visibility.unset', 'No opinion')
      case 'all_models': return t('aiCenter.policy.visibility.allModels', 'All models')
      case 'selected_models': return t('aiCenter.policy.visibility.selected', 'Selected models')
      case 'hidden': return t('aiCenter.policy.visibility.hidden', 'Hidden')
    }
  }
}

function ModelRow({ mode, provider, exactModel, body, onChange, open }: {
  mode: EditorMode
  provider: string
  exactModel: string
  body: PolicyBody
  onChange: (body: PolicyBody) => void
  open?: boolean
}) {
  const { t } = useI18n()
  const entry = body.models?.[exactModel] ?? {}
  const [limitsOpen, setLimitsOpen] = useState(open || (entry.limits?.length ?? 0) > 0)
  const providerVisible = body.providers?.[provider]?.visible
  // A personal or App policy can only list a model as visible inside "selected models" (it never widens).
  const canShow = mode === 'user_side' || providerVisible === 'selected_models'
  const visibility = entry.visible === undefined ? 'unset' : entry.visible ? 'shown' : 'hidden'
  const coefficientText = entry.price_coefficient === undefined ? '' : String(entry.price_coefficient)
  return (
    <div data-testid="policy-model" data-model={exactModel} className="flex flex-col gap-2 py-2" style={{ borderTop: '1px solid var(--cp-border)' }}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-[10rem] flex-1 text-sm" style={{ color: 'var(--cp-text)' }}><ExactModelLabel exactModel={exactModel} /></span>
        <div className="flex flex-wrap items-center gap-2">
          <select
            aria-label={t('aiCenter.policy.modelVisibility', 'Visibility of {{model}}', { model: exactModel })}
            data-testid="policy-model-visibility"
            value={visibility}
            onChange={(event) => onChange(withModel(body, exactModel, { visible: event.target.value === 'unset' ? undefined : event.target.value === 'shown' }))}
            className="min-h-10 rounded-lg px-2 text-sm"
            style={fieldStyle}
          >
            <option value="unset">{t('aiCenter.policy.visibility.unset', 'No opinion')}</option>
            {(canShow || visibility === 'shown') && <option value="shown">{t('aiCenter.policy.visibility.shown', 'Visible')}</option>}
            <option value="hidden">{t('aiCenter.policy.visibility.hidden', 'Hidden')}</option>
          </select>
          {mode === 'user_side' && (
            <label className="flex items-center gap-1 text-sm" style={{ color: 'var(--cp-muted)' }}>
              ×
              <input
                aria-label={t('aiCenter.policy.modelCoefficient', 'Model coefficient of {{model}}', { model: exactModel })}
                data-testid="policy-model-coefficient"
                defaultValue={coefficientText}
                placeholder={t('aiCenter.policy.coefficient.unsetShort', 'not set')}
                inputMode="decimal"
                onChange={(event) => {
                  const text = event.target.value.trim()
                  const value = Number(text)
                  if (!text) onChange(withModel(body, exactModel, { price_coefficient: undefined }))
                  else if (Number.isFinite(value) && value >= 0) onChange(withModel(body, exactModel, { price_coefficient: value }))
                }}
                className="min-h-10 w-20 rounded-lg px-2 text-sm"
                style={fieldStyle}
              />
            </label>
          )}
          <button type="button" onClick={() => setLimitsOpen(!limitsOpen)} aria-expanded={limitsOpen} className="min-h-10 rounded-lg px-2 text-xs" style={{ color: 'var(--cp-accent)' }}>
            {t('aiCenter.policy.limitsCount', 'Limits ({{count}})', { count: entry.limits?.length ?? 0 })}
          </button>
        </div>
      </div>
      {limitsOpen && (
        <div className="pl-0 sm:pl-4">
          <LimitsEditor testId="policy-model-limits" limits={entry.limits} onChange={(limits) => onChange(withModel(body, exactModel, { limits }))} />
        </div>
      )}
    </div>
  )
}

function ProviderCard({ mode, provider, body, onChange, focus }: {
  mode: EditorMode
  provider: EditorProvider
  body: PolicyBody
  onChange: (body: PolicyBody) => void
  focus?: { provider?: string; model?: string }
}) {
  const { t } = useI18n()
  const visibilityLabel = useVisibilityLabel()
  const entry = body.providers?.[provider.name] ?? {}
  const focused = focus?.provider === provider.name || (focus?.model ? provider.models.includes(focus.model) : false)
  const [open, setOpen] = useState(focused || entry.visible === 'selected_models' || entry.price_coefficient !== undefined || (entry.limits?.length ?? 0) > 0)
  const choices = providerVisibilityChoices(mode)
  const value: VisibilityChoice = entry.visible ?? 'unset'
  const shownChoices: VisibilityChoice[] = [...choices.map((choice) => choice ?? 'unset' as const)]
  if (!shownChoices.includes(value)) shownChoices.push(value)
  const hint = value === 'all_models'
    ? t('aiCenter.policy.visibility.allModelsHint', 'Every model of this Provider, including ones added later.')
    : value === 'selected_models'
      ? t('aiCenter.policy.visibility.selectedHint', 'Only models marked Visible below; models found later stay hidden.')
      : value === 'hidden'
        ? t('aiCenter.policy.visibility.hiddenHint', 'This Provider cannot be seen or used here.')
        : mode === 'user_side'
          ? t('aiCenter.policy.visibility.unsetHint', '“All Providers” or a less specific layer decides.')
          : t('aiCenter.policy.visibility.unsetAppHint', 'Whatever the user may use stays available.')
  return (
    <div data-testid="policy-provider" data-provider={provider.name} className="flex flex-col gap-3 rounded-xl p-4" style={panelStyle}>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <button type="button" onClick={() => setOpen(!open)} aria-expanded={open} className="flex min-h-10 min-w-[10rem] flex-1 items-center gap-2 text-left">
          <ChevronDown size={16} className={`shrink-0 transition-transform ${open ? '' : '-rotate-90'}`} style={{ color: 'var(--cp-muted)' }} />
          <span className="min-w-0 truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{provider.name}</span>
          {!provider.enabled && <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.providerDisabled', 'disabled')}</span>}
          <span className="shrink-0 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.modelCount', '{{count}} models', { count: provider.models.length })}</span>
        </button>
        <div className="w-full max-w-[460px]">
          <Segmented
            testId="policy-provider-visibility"
            label={t('aiCenter.policy.providerVisibility', 'Visibility of {{provider}}', { provider: provider.name })}
            value={value}
            onChange={(next) => onChange(withProvider(body, provider.name, { visible: next === 'unset' ? undefined : next }))}
            options={shownChoices.map((choice) => ({ value: choice, label: visibilityLabel(choice) }))}
          />
        </div>
      </div>
      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{hint}</span>
      {open && (
        <div className="flex flex-col gap-4">
          {mode === 'user_side' && (
            <CoefficientField
              testId="policy-provider-coefficient"
              label={t('aiCenter.policy.providerCoefficient', 'Price coefficient for this Provider')}
              value={entry.price_coefficient}
              onChange={(price_coefficient) => onChange(withProvider(body, provider.name, { price_coefficient }))}
              inherited={t('aiCenter.policy.coefficient.providerUnsetHint', 'This layer’s default coefficient applies.')}
            />
          )}
          <div className="flex flex-col gap-1">
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.providerLimits', 'Limits on this Provider')}</span>
            <LimitsEditor testId="policy-provider-limits" limits={entry.limits} onChange={(limits) => onChange(withProvider(body, provider.name, { limits }))} />
          </div>
          <div className="flex flex-col">
            <span className="pb-1 text-sm" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.models', 'Models')}</span>
            {provider.models.length === 0 && <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.noModels', 'No model in the list yet')}</span>}
            {provider.models.map((model) => (
              <ModelRow key={model} mode={mode} provider={provider.name} exactModel={model} body={body} onChange={onChange} open={focus?.model === model} />
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

/** The body of one policy: what this layer (or App) says about visibility, price coefficients and limits. */
export function PolicyEditor({ mode, body, onChange, providers, focus }: PolicyEditorProps) {
  const { t } = useI18n()
  const known = new Set(providers.map((provider) => provider.name))
  const knownModels = new Set(providers.flatMap((provider) => provider.models))
  const missing = [
    ...Object.keys(body.providers ?? {}).filter((name) => !known.has(name)),
    ...Object.keys(body.models ?? {}).filter((name) => !knownModels.has(name)),
  ]
  const removeMissing = (name: string) => {
    const providersLeft = { ...(body.providers ?? {}) }
    const modelsLeft = { ...(body.models ?? {}) }
    delete providersLeft[name]
    delete modelsLeft[name]
    onChange({ ...body, providers: providersLeft, models: modelsLeft })
  }
  return (
    <div className="flex flex-col gap-4" data-testid="policy-editor" data-mode={mode}>
      <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {mode === 'user_side'
          ? t('aiCenter.policy.userSideHint', 'For each setting, the most specific layer that sets it decides: a user over their system groups, groups over their user type. Among several groups the one most favourable to the user wins. Limits all apply.')
          : mode === 'app_side'
            ? t('aiCenter.policy.appSideHint', 'App policies only narrow what users may already use, and add limits. They carry no price coefficient.')
            : t('aiCenter.policy.ownAppHint', 'Your own policy on one of your Apps or Agents: hide resources from it and add limits. Administrator policies still apply.')}
      </p>
      {mode === 'user_side' && (
        <>
          <label className="flex items-start gap-3 rounded-xl p-4 text-sm" style={{ ...panelStyle, color: 'var(--cp-text)' }}>
            <input type="checkbox" data-testid="policy-all-providers" checked={body.all_providers === true} onChange={(event) => onChange({ ...body, all_providers: event.target.checked ? true : undefined })} className="mt-0.5" />
            <span>
              <span className="font-medium">{t('aiCenter.policy.allProviders', 'All Providers (following the list)')}</span>
              <span className="block text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.allProvidersHint', 'Every Provider and model, including ones added later, unless a setting below says otherwise.')}</span>
            </span>
          </label>
          <div className="rounded-xl p-4" style={panelStyle}>
            <CoefficientField
              testId="policy-coefficient"
              label={t('aiCenter.policy.layerCoefficient', 'Default price coefficient')}
              value={body.price_coefficient}
              onChange={(price_coefficient) => onChange({ ...body, price_coefficient })}
            />
          </div>
        </>
      )}
      <div className="flex flex-col gap-2 rounded-xl p-4" style={panelStyle}>
        <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.allLimits', 'Limits on all models')}</span>
        <LimitsEditor
          testId="policy-limits"
          limits={body.limits}
          onChange={(limits) => onChange({ ...body, limits })}
          hint={mode === 'user_side' ? t('aiCenter.policy.limitsUserHint', 'Counted for each user separately.') : mode === 'app_side' ? t('aiCenter.policy.limitsAppHint', 'An App limit is shared by all its users; a user + App limit is that user’s own.') : undefined}
        />
      </div>
      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.providers', 'Providers')}</h3>
        {providers.length === 0 && <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.noProviders', 'No Provider yet.')}</span>}
        {providers.map((provider) => (
          <ProviderCard key={provider.name} mode={mode} provider={provider} body={body} onChange={onChange} focus={focus} />
        ))}
      </section>
      {missing.length > 0 && (
        <section className="flex flex-col gap-2 rounded-xl p-4" data-testid="policy-missing" style={panelStyle}>
          <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.missing', 'Not in the model list')}</span>
          <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.missingHint', 'These settings are kept but apply to nothing until the Provider or model comes back.')}</span>
          {missing.map((name) => (
            <div key={name} className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <span className="min-w-0 flex-1 break-all">{name}</span>
              <button type="button" onClick={() => removeMissing(name)} aria-label={t('aiCenter.policy.removeSetting', 'Remove this setting')} className="min-h-10 min-w-10 rounded-lg p-2" style={{ color: 'var(--cp-muted)' }}>
                <Trash2 size={16} />
              </button>
            </div>
          ))}
        </section>
      )}
    </div>
  )
}
