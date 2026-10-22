import { useEffect, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import type { QuotaObject, QuotaPeriod } from '../../datamodel/quota'

export function usePeriodLabel() {
  const { t } = useI18n()
  return (period?: QuotaPeriod) => {
    switch (period) {
      case '4h': return t('aiCenter.quota.period.4h', 'Every 4 hours')
      case '24h': return t('aiCenter.quota.period.24h', 'Every 24 hours')
      case '72h': return t('aiCenter.quota.period.72h', 'Every 72 hours')
      case 'week': return t('aiCenter.quota.period.week', 'Every week')
      default: return ''
    }
  }
}

export function useObjectLabel() {
  const { t } = useI18n()
  return (object: QuotaObject) => {
    switch (object.type) {
      case 'user': return t('aiCenter.quota.object.user', 'User {{user}}', { user: object.user_id })
      case 'app': return t('aiCenter.quota.object.app', '{{app}} · all users share', { app: object.app_id })
      case 'user_app': return t('aiCenter.quota.object.userApp', '{{app}} of {{user}}', { app: object.app_id, user: object.user_id })
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

export function formatLocal(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

export function remaining(ms: number): string {
  const minutes = Math.max(0, Math.round(ms / 60_000))
  const days = Math.floor(minutes / 1440)
  const hours = Math.floor((minutes % 1440) / 60)
  const mins = minutes % 60
  return days > 0 ? `${days}d ${hours}h` : hours > 0 ? `${hours}h ${mins}m` : `${mins}m`
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

export const buttonStyle = {
  primary: { background: 'var(--cp-accent)', color: '#fff' },
  secondary: { background: 'var(--cp-surface)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' },
  danger: { background: 'var(--cp-surface)', color: 'var(--cp-danger)', border: '1px solid var(--cp-border)' },
} as const
