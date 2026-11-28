import { useEffect, useMemo, useState } from 'react'
import { Users } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { PolicyApi } from '../../../../api/aicc_policy'
import { formatRate, type PolicyEffectiveResponse } from '../../datamodel/policy'
import { CreditAmount, CreditPriceBadge, EffectiveLimitRow, ExactModelLabel, Field } from './PolicyWidgets'
import { errorText, fieldStyle, panelStyle } from './policy-labels'

interface EffectivePreviewProps {
  api: PolicyApi
  /** Administrators pick any user; others always see themselves. */
  users?: string[]
  apps: string[]
  initialUser?: string
  initialApp?: string
  subjectLabel: (subject: string) => string
  /** Changes when a policy was saved, to refresh the preview. */
  generation?: number
}

/** What one user (calling through one App) may use, at what price and within which limits — and which
 * policy each value comes from (`policy.effective`). */
export function EffectivePreview({ api, users, apps, initialUser, initialApp, subjectLabel, generation = 0 }: EffectivePreviewProps) {
  const { t, locale } = useI18n()
  const [user, setUser] = useState(initialUser ?? '')
  const [app, setApp] = useState(initialApp ?? '')
  const [result, setResult] = useState<{ key: string; value: PolicyEffectiveResponse } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [onlyUnusable, setOnlyUnusable] = useState(false)
  const pickUser = users !== undefined
  const key = `${user}\u0000${app.trim()}\u0000${generation}`

  useEffect(() => {
    if (pickUser && !user) return
    let live = true
    const timer = setTimeout(() => {
      api.effective({ userId: user || undefined, appId: app.trim() || undefined })
        .then((value) => { if (live) { setResult({ key, value }); setError(null) } })
        .catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    }, 200)
    return () => { live = false; clearTimeout(timer) }
  }, [api, app, key, pickUser, user])

  const effective = result?.key === key ? result.value : null
  const byProvider = useMemo(() => {
    const groups = new Map<string, PolicyEffectiveResponse['models']>()
    for (const model of effective?.models ?? []) {
      if (onlyUnusable && model.usable) continue
      groups.set(model.provider_instance_name, [...(groups.get(model.provider_instance_name) ?? []), model])
    }
    return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [effective, onlyUnusable])

  return (
    <section className="flex flex-col gap-3 rounded-xl p-4" data-testid="policy-effective" style={panelStyle}>
      <h2 className="flex items-center gap-2 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}><Users size={16} />{t('aiCenter.policy.effectiveTitle', 'What a user gets')}</h2>
      <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.effectiveHint', 'The result of every policy that applies, and where each value comes from.')}</p>
      <div className="flex flex-col gap-2 sm:flex-row">
        {pickUser && (
          <select data-testid="policy-effective-user" aria-label={t('aiCenter.policy.pickUser', 'Pick a user')} value={user} onChange={(event) => setUser(event.target.value)} className="min-h-10 flex-1 rounded-lg px-3 text-sm" style={fieldStyle}>
            <option value="">{t('aiCenter.policy.pickUser', 'Pick a user')}</option>
            {users.map((item) => <option key={item} value={item}>{item}</option>)}
          </select>
        )}
        <input
          data-testid="policy-effective-app"
          aria-label={t('aiCenter.policy.throughApp', 'Through App (optional)')}
          placeholder={t('aiCenter.policy.throughApp', 'Through App (optional)')}
          list="policy-effective-apps"
          value={app}
          onChange={(event) => setApp(event.target.value)}
          className="min-h-10 flex-1 rounded-lg px-3 text-sm"
          style={fieldStyle}
        />
        <datalist id="policy-effective-apps">{apps.map((item) => <option key={item} value={item} />)}</datalist>
      </div>
      {error && <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      {effective && (
        <div className="flex flex-col gap-3" data-testid="policy-effective-result">
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Field label={t('aiCenter.credit.balance', 'AI Credit balance')}><CreditAmount value={effective.balance} strong /></Field>
            <Field label={t('aiCenter.policy.visibleModels', 'Visible models')}>{effective.models.length.toLocaleString(locale)}</Field>
            <Field label={t('aiCenter.policy.usableModels', 'Usable')}>{effective.models.filter((model) => model.usable).length.toLocaleString(locale)}</Field>
            <Field label={t('aiCenter.policy.unpricedModels', 'Unpriced')}>{(effective.unpriced_models?.length ?? 0).toLocaleString(locale)}</Field>
          </div>
          {effective.models.length === 0 && <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.userNothing', 'This user cannot see any model.')}</p>}
          {effective.models.length > 0 && (
            <label className="flex items-center gap-2 text-xs" style={{ color: 'var(--cp-text)' }}>
              <input type="checkbox" checked={onlyUnusable} onChange={(event) => setOnlyUnusable(event.target.checked)} />
              {t('aiCenter.policy.onlyUnusable', 'Only models this user cannot use')}
            </label>
          )}
          {byProvider.map(([provider, models]) => (
            <div key={provider} className="flex flex-col">
              <span className="pb-1 text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>{provider}</span>
              {models.map((model) => (
                <div key={model.exact_model} data-testid="policy-effective-model" data-model={model.exact_model} data-usable={model.usable} className="flex flex-wrap items-start gap-x-4 gap-y-1 py-1.5 text-xs" style={{ borderTop: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
                  <span className="min-w-[12rem] flex-[1.4]"><ExactModelLabel exactModel={model.exact_model} /></span>
                  <span className="min-w-[9rem] flex-1" style={{ color: 'var(--cp-muted)' }} title={model.visible_from.join(', ')}>
                    {t('aiCenter.policy.visibleFrom', 'Visible via {{subjects}}', { subjects: model.visible_from.map(subjectLabel).join(', ') || '—' })}
                  </span>
                  <span className="flex min-w-[12rem] flex-[1.2] flex-col gap-0.5">
                    <CreditPriceBadge price={model.price} />
                    {(model.coefficient_from?.length ?? 0) > 0 && (
                      <span style={{ color: 'var(--cp-muted)' }}>
                        {t('aiCenter.policy.coefficientFrom', 'Coefficient ×{{value}} from {{subjects}}', { value: formatRate(model.price.provider_coefficient * model.price.model_coefficient, locale), subjects: model.coefficient_from!.map(subjectLabel).join(', ') })}
                      </span>
                    )}
                  </span>
                </div>
              ))}
            </div>
          ))}
          {effective.limits.length > 0 && (
            <div className="flex flex-col gap-2">
              <span className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.limitsTitle', 'Limits')}</span>
              {effective.limits.map((limit) => (
                <EffectiveLimitRow key={`${limit.policy_id}/${limit.counter.type}/${JSON.stringify(limit.resource)}/${limit.period}/${limit.unit}`} limit={limit} subjectLabel={subjectLabel} />
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  )
}
