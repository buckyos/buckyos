import { useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { formatRate, parseSubject, priceSummary, type LimitCounter, type LimitPeriod, type LimitStatus, type LimitUnit, type ModelCreditPrice } from '../../datamodel/policy'

export function usePeriodLabel() {
  const { t } = useI18n()
  return (period: LimitPeriod) => {
    switch (period) {
      case '5h': return t('aiCenter.policy.period.5h', '5 hours')
      case '24h': return t('aiCenter.policy.period.24h', '24 hours')
      case '7d': return t('aiCenter.policy.period.7d', '7 days')
    }
  }
}

export function useUnitLabel() {
  const { t } = useI18n()
  return (unit: LimitUnit) => unit === 'credit' ? t('aiCenter.policy.unit.credit', 'AI Credit') : t('aiCenter.policy.unit.token', 'Tokens')
}

export function useLimitStatusLabel() {
  const { t } = useI18n()
  return (status: LimitStatus) => {
    switch (status) {
      case 'normal': return t('aiCenter.policy.status.normal', 'Normal')
      case 'near_limit': return t('aiCenter.policy.status.nearLimit', 'Near limit')
      case 'exhausted': return t('aiCenter.policy.status.exhausted', 'Used up')
      case 'forbidden': return t('aiCenter.policy.status.forbidden', 'Use forbidden')
    }
  }
}

/** A readable name for a policy subject; group names come from Users & Agents when known. */
export function useSubjectLabel(groupNames: Record<string, string> = {}) {
  const { t } = useI18n()
  return (subject: string) => {
    if (subject === 'type:admin') return t('aiCenter.policy.type.admin', 'Administrators')
    if (subject === 'type:user') return t('aiCenter.policy.type.user', 'Users')
    if (subject === 'type:limited') return t('aiCenter.policy.type.limited', 'Limited users')
    const parts = parseSubject(subject)
    if (!parts) return subject
    switch (parts.kind) {
      case 'group': return groupNames[parts.id] || parts.id
      case 'user': return t('aiCenter.policy.subject.user', 'User {{user}}', { user: parts.id })
      case 'app': return t('aiCenter.policy.subject.app', '{{app}} · all users', { app: parts.id })
      case 'user_app': return t('aiCenter.policy.subject.userApp', '{{app}} of {{user}}', { app: parts.appId ?? '', user: parts.userId ?? '' })
      default: return subject
    }
  }
}

export function useCounterLabel() {
  const { t } = useI18n()
  return (counter: LimitCounter) => {
    switch (counter.type) {
      case 'user': return t('aiCenter.policy.counter.user', 'Counted for {{user}}', { user: counter.user_id })
      case 'app': return t('aiCenter.policy.counter.app', 'Shared by every user of {{app}}', { app: counter.app_id })
      case 'user_app': return t('aiCenter.policy.counter.userApp', 'Counted for {{user}} in {{app}}', { user: counter.user_id, app: counter.app_id })
    }
  }
}

export function timeZoneName(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone
  } catch {
    return 'local'
  }
}

export function formatLocal(ms: number, locale: string): string {
  return new Date(ms).toLocaleString(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

export function remaining(ms: number, locale: string): string {
  const minutes = Math.max(0, Math.round(ms / 60_000))
  const days = Math.floor(minutes / 1440)
  const hours = Math.floor((minutes % 1440) / 60)
  const mins = minutes % 60
  const unit = (value: number, unit: 'day' | 'hour' | 'minute') =>
    value.toLocaleString(locale, { style: 'unit', unit, unitDisplay: 'narrow' })
  return days > 0 ? `${unit(days, 'day')} ${unit(hours, 'hour')}` : hours > 0 ? `${unit(hours, 'hour')} ${unit(mins, 'minute')}` : unit(mins, 'minute')
}

/** Wall clock refreshed every 30 s, for countdowns and current windows. */
export function useNow(): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(timer)
  }, [])
  return now
}

export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export const buttonStyle = {
  primary: { background: 'var(--cp-accent)', color: '#fff' },
  secondary: { background: 'var(--cp-surface)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' },
  danger: { background: 'var(--cp-surface)', color: 'var(--cp-danger)', border: '1px solid var(--cp-border)' },
} as const

export const fieldStyle = { background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' } as const
export const panelStyle = { background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' } as const
export const toneColor = { ok: 'var(--cp-success)', warning: 'var(--cp-warning)', error: 'var(--cp-danger)', muted: 'var(--cp-muted)' } as const

export function usePriceUnitLabel() {
  const { t } = useI18n()
  return (unit: string) => {
    switch (unit) {
      case 'request': return t('aiCenter.prices.unit.request', 'per request')
      case 'image': return t('aiCenter.prices.unit.image', 'per image')
      case 'second': return t('aiCenter.prices.unit.second', 'per second')
      case 'video_second': return t('aiCenter.prices.unit.videoSecond', 'per video second')
      case 'audio_second': return t('aiCenter.prices.unit.audioSecond', 'per audio second')
      case 'character': return t('aiCenter.prices.unit.character', 'per character')
      case 'megapixel': return t('aiCenter.prices.unit.megapixel', 'per megapixel')
      case 'reference_image': return t('aiCenter.prices.unit.referenceImage', 'per reference image')
      default: return unit
    }
  }
}

/** The text of what a model costs a person: free, a Credit price (coefficients applied) or unpriced. */
export function usePriceText() {
  const { t, locale } = useI18n()
  const unitLabel = usePriceUnitLabel()
  return (price: ModelCreditPrice): string => {
    const summary = priceSummary(price)
    if (summary.state === 'free') return t('aiCenter.prices.free', 'Free')
    if (summary.state === 'unpriced') return t('aiCenter.prices.unpriced', 'Unpriced (not usable)')
    if (summary.currency) return t('aiCenter.prices.providerCostRate', 'Provider cost: 1 {{currency}} = {{rate}} AI Credit', { currency: summary.currency, rate: formatRate(summary.rate ?? 0, locale) })
    if (summary.unit) return t('aiCenter.prices.perUnit', '{{amount}} AI Credit {{unit}}', { amount: formatRate(summary.amount ?? 0, locale), unit: unitLabel(summary.unit) })
    if (summary.input !== undefined || summary.output !== undefined) {
      return t('aiCenter.prices.perMillion', 'In {{input}} · Out {{output}} AI Credit / M tokens', {
        input: summary.input === undefined ? '—' : formatRate(summary.input, locale),
        output: summary.output === undefined ? '—' : formatRate(summary.output, locale),
      })
    }
    return t('aiCenter.prices.customPrice', 'Custom price')
  }
}
