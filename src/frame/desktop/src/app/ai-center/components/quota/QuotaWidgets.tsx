import { useEffect, useState, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import {
  formatTokens,
  formatTokensExact,
  parseTokens,
  quotaRefusalOf,
  quotaTone,
  splitExactModel,
  usageRatio,
  type QuotaKind,
  type QuotaPeriod,
  type QuotaResource,
  type QuotaRuleSource,
  type QuotaStatus,
  type QuotaUsageState,
} from '../../datamodel/quota'
import { formatLocal, remaining, timeZoneName, useNow, usePeriodLabel } from './quota-labels'

const toneColor = {
  ok: 'var(--cp-success)',
  warning: 'var(--cp-warning)',
  error: 'var(--cp-danger)',
  unknown: 'var(--cp-muted)',
} as const

/** Abbreviated amount with the exact integer as its title (§13: both forms). */
export function Tokens({ value, strong = false }: { value: number; strong?: boolean }) {
  return (
    <span title={formatTokensExact(value)} className={strong ? 'font-semibold' : undefined} style={{ color: 'var(--cp-text)' }}>
      {formatTokens(value)}
    </span>
  )
}

export function StatusChip({ status }: { status: QuotaStatus }) {
  const { t } = useI18n()
  const labels: Record<QuotaStatus, string> = {
    normal: t('aiCenter.quota.status.normal', 'Normal'),
    near_limit: t('aiCenter.quota.status.nearLimit', 'Near limit'),
    exhausted: t('aiCenter.quota.status.exhausted', 'Exhausted'),
    forbidden: t('aiCenter.quota.status.forbidden', 'Use forbidden'),
    disabled: t('aiCenter.quota.status.disabled', 'Disabled'),
    resource_unavailable: t('aiCenter.quota.status.resourceUnavailable', 'Resource unavailable'),
  }
  const color = toneColor[quotaTone(status)]
  return (
    <span
      data-testid="quota-status"
      data-status={status}
      className="inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs whitespace-nowrap"
      style={{ color, background: `color-mix(in srgb, ${color} 12%, transparent)` }}
    >
      <span className="h-1.5 w-1.5 rounded-full" style={{ background: color }} />
      {labels[status]}
    </span>
  )
}

export function UsageBar({ state }: { state: QuotaUsageState }) {
  const cap = state.limit_tokens ?? state.granted_tokens ?? 0
  const settled = cap > 0 ? Math.min(1, state.settled_tokens / cap) : usageRatio(state)
  const reserved = cap > 0 ? Math.min(1 - settled, state.reserved_tokens / cap) : 0
  const color = toneColor[quotaTone(state.status)]
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full" style={{ background: 'var(--cp-surface-2)' }} aria-hidden>
      <div className="flex h-full">
        <div style={{ width: `${settled * 100}%`, background: color }} />
        <div style={{ width: `${reserved * 100}%`, background: `color-mix(in srgb, ${color} 45%, transparent)` }} />
      </div>
    </div>
  )
}

export function ResourceLabel({ resource }: { resource: QuotaResource }) {
  const { t } = useI18n()
  if (resource.type === 'all') return <span>{t('aiCenter.quota.resource.all', 'All token models')}</span>
  if (resource.type === 'hidden') return <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.resource.hidden', 'No longer accessible')}</span>
  if (resource.type === 'provider') return <span>{t('aiCenter.quota.resource.provider', 'Provider {{provider}}', { provider: resource.provider_instance_name })}</span>
  const parts = splitExactModel(resource.exact_model)
  return (
    <span title={resource.exact_model}>
      {parts.model}
      {parts.variant ? <span style={{ color: 'var(--cp-muted)' }}>:{parts.variant}</span> : null}
      <span style={{ color: 'var(--cp-muted)' }}>@{parts.provider}</span>
    </span>
  )
}

export function KindBadge({ kind, period }: { kind: QuotaKind; period?: QuotaPeriod }) {
  const { t } = useI18n()
  const periodLabel = usePeriodLabel()
  return (
    <span className="rounded px-1.5 py-0.5 text-xs whitespace-nowrap" style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-muted)' }}>
      {kind === 'total' ? t('aiCenter.quota.kind.total', 'Total') : periodLabel(period)}
    </span>
  )
}

export function SourceBadge({ source }: { source: QuotaRuleSource }) {
  const { t } = useI18n()
  return (
    <span className="rounded px-1.5 py-0.5 text-xs whitespace-nowrap" style={{ border: '1px solid var(--cp-border)', color: 'var(--cp-muted)' }}>
      {source === 'admin' ? t('aiCenter.quota.source.admin', 'Administrator') : t('aiCenter.quota.source.user', 'Own limit')}
    </span>
  )
}

/** Next reset in local time with its time zone and a countdown; totals never reset by themselves. */
export function ResetLabel({ state }: { state: QuotaUsageState }) {
  const { t } = useI18n()
  const now = useNow()
  if (state.reset_at_ms === null) {
    return <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.noAutoReset', 'No automatic reset')}</span>
  }
  if (state.status === 'forbidden') {
    return <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.quota.forbiddenNoReset', 'Forbidden until the limit is raised')}</span>
  }
  return (
    <span style={{ color: 'var(--cp-muted)' }} title={`${new Date(state.reset_at_ms).toISOString()} UTC`}>
      {t('aiCenter.quota.resetsAt', 'Resets {{time}} ({{zone}}), in {{left}}', {
        time: formatLocal(state.reset_at_ms),
        zone: timeZoneName(),
        left: remaining(state.reset_at_ms - now),
      })}
    </span>
  )
}

/** Token amount input accepting K / M, showing the exact integer it means. */
export function TokenInput({
  label,
  value,
  onChange,
  testId,
  allowZero = true,
}: {
  label: string
  value: number | null
  onChange: (value: number | null) => void
  testId?: string
  allowZero?: boolean
}) {
  const { t } = useI18n()
  // The parent only ever changes `value` through this input, so the text is the source of truth.
  const [text, setText] = useState(value === null ? '' : String(value))
  const parsed = parseTokens(text)
  const invalid = text.trim() !== '' && (parsed === null || (!allowZero && parsed === 0))
  return (
    <label className="flex flex-col gap-1 text-sm">
      <span style={{ color: 'var(--cp-text)' }}>{label}</span>
      <input
        data-testid={testId}
        value={text}
        inputMode="decimal"
        placeholder="50K"
        onChange={(event) => {
          setText(event.target.value)
          const next = parseTokens(event.target.value)
          onChange(next !== null && (allowZero || next > 0) ? next : null)
        }}
        className="min-h-10 rounded-lg px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-accent)]"
        style={{ background: 'var(--cp-surface)', border: `1px solid ${invalid ? 'var(--cp-danger)' : 'var(--cp-border)'}`, color: 'var(--cp-text)' }}
      />
      <span className="text-xs" style={{ color: invalid ? 'var(--cp-danger)' : 'var(--cp-muted)' }}>
        {invalid
          ? t('aiCenter.quota.input.invalid', 'Enter a whole number of tokens, e.g. 50000, 50K or 1.5M')
          : parsed !== null
            ? t('aiCenter.quota.input.exact', '= {{exact}} tokens', { exact: formatTokensExact(parsed) })
            : t('aiCenter.quota.input.hint', 'K and M are accepted')}
      </span>
    </label>
  )
}

/** Right-hand drawer on desktop, full screen on phones. */
export function SidePanel({ title, onClose, children, testId }: { title: string; onClose: () => void; children: ReactNode; testId?: string }) {
  const { t } = useI18n()
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div className="fixed inset-0 z-40 flex justify-end" data-testid={testId}>
      <div className="absolute inset-0 hidden md:block" style={{ background: 'rgba(0,0,0,0.3)' }} onClick={onClose} />
      <section
        role="dialog"
        aria-label={title}
        className="relative flex h-full w-full flex-col md:max-w-[560px] md:shadow-xl"
        style={{ background: 'var(--cp-bg)', borderLeft: '1px solid var(--cp-border)' }}
      >
        <header className="flex items-center justify-between gap-3 px-4 py-3 md:px-6" style={{ borderBottom: '1px solid var(--cp-border)', paddingTop: 'max(0.75rem, env(safe-area-inset-top))' }}>
          <h2 className="truncate text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{title}</h2>
          <button type="button" onClick={onClose} aria-label={t('common.close', 'Close')} className="min-h-10 min-w-10 rounded-lg p-2" style={{ color: 'var(--cp-muted)' }}>
            <X size={18} />
          </button>
        </header>
        <div className="flex-1 overflow-y-auto px-4 py-4 md:px-6" style={{ paddingBottom: 'calc(1rem + env(safe-area-inset-bottom))' }}>{children}</div>
      </section>
    </div>
  )
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="text-sm" style={{ color: 'var(--cp-text)' }}>{children}</span>
    </div>
  )
}

export function Segmented<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string; disabled?: boolean }[]; onChange: (value: T) => void }) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-1 rounded-lg p-1" style={{ background: 'var(--cp-surface-2)' }}>
      {options.map((option) => {
        const active = option.value === value
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={option.disabled}
            onClick={() => onChange(option.value)}
            className="min-h-9 flex-1 whitespace-nowrap rounded-md px-3 text-sm disabled:opacity-40"
            style={{
              background: active ? 'var(--cp-surface)' : 'transparent',
              color: active ? 'var(--cp-text)' : 'var(--cp-muted)',
              boxShadow: active ? '0 1px 2px rgba(0,0,0,0.08)' : undefined,
            }}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}

/** What an AICC refusal means for the caller (§7.4), from its code; shown next to the raw error. */
export function QuotaRefusalNote({ error }: { error: unknown }) {
  const { t } = useI18n()
  const refusal = quotaRefusalOf(error)
  if (!refusal) return null
  const details = refusal.details
  let text: string
  switch (refusal.code) {
    case 'quota_exhausted':
      text = details?.reset_at_ms
        ? t('aiCenter.quota.refusal.exhaustedUntil', 'AI token quota exhausted until {{time}}.', { time: formatLocal(details.reset_at_ms) })
        : t('aiCenter.quota.refusal.exhausted', 'AI token quota exhausted. A total quota needs an administrator to recharge it.')
      break
    case 'quota_budget_exceeded':
      text = t('aiCenter.quota.refusal.budget', 'This request may need more tokens than the quota has left ({{max}} available). Lower the max output tokens or shorten the input.', { max: formatTokensExact(details?.max_allowed_tokens ?? 0) })
      break
    case 'quota_unavailable':
      text = t('aiCenter.quota.refusal.unavailable', 'The quota state is temporarily unavailable, so the call was not started. Try again shortly.')
      break
    case 'resource_not_permitted':
      text = t('aiCenter.quota.refusal.notPermitted', 'This model is not open to you. An administrator can open it under AI Center › Visibility.')
      break
    case 'attribution_unavailable':
      text = t('aiCenter.quota.refusal.attribution', 'The call could not be attributed to a user, so it was not started.')
      break
  }
  return <p data-testid="quota-refusal" data-code={refusal.code} className="text-sm" style={{ color: 'var(--cp-text)' }}>{text}</p>
}
