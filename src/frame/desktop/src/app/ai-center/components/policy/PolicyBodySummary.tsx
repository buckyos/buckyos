import type { ReactNode } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { formatRate, type PolicyBody, type PolicyLimit } from '../../datamodel/policy'
import { ExactModelLabel, LimitAmount } from './PolicyWidgets'
import { usePeriodLabel, useUnitLabel } from './policy-labels'

function Limits({ limits }: { limits?: PolicyLimit[] }) {
  const periodLabel = usePeriodLabel()
  const unitLabel = useUnitLabel()
  if (!limits?.length) return null
  return (
    <span className="flex flex-wrap gap-x-3 gap-y-1">
      {limits.map((limit) => (
        <span key={`${limit.period}/${limit.unit}`} className="whitespace-nowrap">
          {periodLabel(limit.period)}: <LimitAmount unit={limit.unit} value={limit.amount} /> {unitLabel(limit.unit)}
        </span>
      ))}
    </span>
  )
}

/** A read-only account of what one policy says (administrator policies seen by a user, or another user's own). */
export function PolicyBodySummary({ body }: { body: PolicyBody }) {
  const { t, locale } = useI18n()
  const coefficient = (value?: number) => value === undefined ? null : value === 0 ? t('aiCenter.policy.coefficient.free', 'Free') : `×${formatRate(value, locale)}`
  const visibility = (value?: string) => {
    switch (value) {
      case 'all_models': return t('aiCenter.policy.visibility.allModels', 'All models')
      case 'selected_models': return t('aiCenter.policy.visibility.selected', 'Selected models')
      case 'hidden': return t('aiCenter.policy.visibility.hidden', 'Hidden')
      default: return null
    }
  }
  const rows: { key: string; label: ReactNode; parts: ReactNode[] }[] = []
  const top: ReactNode[] = []
  if (body.all_providers) top.push(t('aiCenter.policy.allProviders', 'All Providers (following the list)'))
  if (body.price_coefficient !== undefined) top.push(t('aiCenter.policy.summary.coefficient', 'Coefficient {{value}}', { value: coefficient(body.price_coefficient) ?? '' }))
  if (body.limits?.length) top.push(<Limits key="limits" limits={body.limits} />)
  if (top.length) rows.push({ key: '', label: t('aiCenter.policy.resource.all', 'All models'), parts: top })
  for (const [name, entry] of Object.entries(body.providers ?? {})) {
    rows.push({ key: name, label: name, parts: [visibility(entry.visible), coefficient(entry.price_coefficient), entry.limits?.length ? <Limits key="limits" limits={entry.limits} /> : null].filter(Boolean) })
  }
  for (const [name, entry] of Object.entries(body.models ?? {})) {
    rows.push({ key: name, label: <ExactModelLabel exactModel={name} />, parts: [entry.visible === undefined ? null : entry.visible ? t('aiCenter.policy.visibility.shown', 'Visible') : t('aiCenter.policy.visibility.hidden', 'Hidden'), coefficient(entry.price_coefficient), entry.limits?.length ? <Limits key="limits" limits={entry.limits} /> : null].filter(Boolean) })
  }
  if (rows.length === 0) return <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.summary.empty', 'This policy sets nothing.')}</p>
  return (
    <dl className="flex flex-col gap-1.5 text-xs" data-testid="policy-body-summary">
      {rows.map((row) => (
        <div key={row.key} className="grid grid-cols-1 gap-1 sm:grid-cols-[minmax(8rem,14rem)_1fr]">
          <dt className="min-w-0 break-all font-medium" style={{ color: 'var(--cp-text)' }}>{row.label}</dt>
          <dd className="flex flex-wrap gap-x-3 gap-y-1" style={{ color: 'var(--cp-muted)' }}>{row.parts.map((part, index) => <span key={index}>{part}</span>)}</dd>
        </div>
      ))}
    </dl>
  )
}
