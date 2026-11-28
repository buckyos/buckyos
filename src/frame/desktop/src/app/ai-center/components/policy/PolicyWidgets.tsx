import { useEffect, useState, type ReactNode } from 'react'
import { useMediaQuery } from '@mui/material'
import { Plus, Trash2, X } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useMobileBackHandler, useMobileTitleOverride } from '../../../../desktop/windows/MobileNavContext'
import {
  LIMIT_PERIODS,
  LIMIT_UNITS,
  exactCredit,
  formatCredit,
  formatTokens,
  formatTokensExact,
  isNegativeCredit,
  isUnpricedRefusal,
  limitRatio,
  limitTone,
  parseLimitAmount,
  policyRefusalOf,
  splitExactModel,
  type EffectiveLimit,
  type LimitResource,
  type LimitStatus,
  type ModelCreditPrice,
  type PolicyLimit,
} from '../../datamodel/policy'
import { fieldStyle, formatLocal, remaining, timeZoneName, toneColor, useCounterLabel, useLimitStatusLabel, useNow, usePeriodLabel, usePriceText, useUnitLabel } from './policy-labels'

/** Dot + label status, the AI Center's status idiom. */
export function StatusDot({ tone, label, testId, status }: { tone: keyof typeof toneColor; label: string; testId?: string; status?: string }) {
  const color = toneColor[tone]
  return (
    <span data-testid={testId} data-status={status} className="inline-flex items-center gap-1.5 whitespace-nowrap text-xs" style={{ color: tone === 'muted' ? 'var(--cp-muted)' : 'var(--cp-text)' }}>
      <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: color }} />
      {label}
    </span>
  )
}

export function LimitStatusDot({ status }: { status: LimitStatus }) {
  const label = useLimitStatusLabel()
  return <StatusDot testId="limit-status" status={status} tone={limitTone(status)} label={label(status)} />
}

/** Two decimals with the exact amount as its title; negative balances in the danger color. */
export function CreditAmount({ value, strong = false, testId }: { value: string | undefined; strong?: boolean; testId?: string }) {
  const { locale } = useI18n()
  return (
    <span data-testid={testId} title={exactCredit(value)} className={`tabular-nums ${strong ? 'font-semibold' : ''}`} style={{ color: isNegativeCredit(value) ? 'var(--cp-danger)' : 'var(--cp-text)' }}>
      {formatCredit(value, locale)}
    </span>
  )
}

/** Used / amount of a limit in its unit. */
export function LimitAmount({ unit, value }: { unit: PolicyLimit['unit']; value: string }) {
  const { locale } = useI18n()
  if (unit === 'credit') return <CreditAmount value={value} />
  const tokens = Number(value)
  return <span className="tabular-nums" title={formatTokensExact(tokens, locale)}>{formatTokens(tokens, locale)}</span>
}

export function LimitBar({ limit }: { limit: EffectiveLimit }) {
  const ratio = Math.min(1, limitRatio(limit))
  const color = toneColor[limitTone(limit.status)]
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full" style={{ background: 'var(--cp-surface-2)' }} aria-hidden>
      <div className="h-full" style={{ width: `${ratio * 100}%`, background: color }} />
    </div>
  )
}

/** When the window resets: its end in local time and a countdown; no window until the next use. */
export function ResetLabel({ limit }: { limit: Pick<EffectiveLimit, 'status' | 'reset_at_ms'> }) {
  const { t, locale } = useI18n()
  const now = useNow()
  if (limit.status === 'forbidden') return <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.forbiddenNoReset', 'Forbidden until the limit is raised')}</span>
  if (limit.reset_at_ms === undefined || limit.reset_at_ms <= now) {
    return <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.windowIdle', 'The window starts at the next use')}</span>
  }
  return (
    <span style={{ color: 'var(--cp-muted)' }} title={`${new Date(limit.reset_at_ms).toISOString()} UTC`}>
      {t('aiCenter.policy.resetsAt', 'Resets {{time}} ({{zone}}), in {{left}}', {
        time: formatLocal(limit.reset_at_ms, locale),
        zone: timeZoneName(),
        left: remaining(limit.reset_at_ms - now, locale),
      })}
    </span>
  )
}

export function ExactModelLabel({ exactModel }: { exactModel: string }) {
  const parts = splitExactModel(exactModel)
  return (
    <span title={exactModel} className="break-all">
      {parts.model}
      {parts.variant ? <span style={{ color: 'var(--cp-muted)' }}>:{parts.variant}</span> : null}
      <span style={{ color: 'var(--cp-muted)' }}>@{parts.provider}</span>
    </span>
  )
}

export function LimitResourceLabel({ resource }: { resource: LimitResource }) {
  const { t } = useI18n()
  switch (resource.type) {
    case 'all': return <span>{t('aiCenter.policy.resource.all', 'All models')}</span>
    case 'hidden': return <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.resource.hidden', 'A resource you cannot see')}</span>
    case 'provider': return <span>{t('aiCenter.policy.resource.provider', 'Provider {{provider}}', { provider: resource.provider_instance_name })}</span>
    case 'model': return <ExactModelLabel exactModel={resource.exact_model} />
  }
}

/** One limit that applies, with its window state. */
export function EffectiveLimitRow({ limit, subjectLabel }: { limit: EffectiveLimit; subjectLabel: (subject: string) => string }) {
  const { t } = useI18n()
  const periodLabel = usePeriodLabel()
  const unitLabel = useUnitLabel()
  const counterLabel = useCounterLabel()
  return (
    <div data-testid="effective-limit" data-status={limit.status} className="flex flex-col gap-1.5 rounded-lg px-3 py-2" style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs" style={{ color: 'var(--cp-text)' }}>
        <span className="font-medium"><LimitResourceLabel resource={limit.resource} /></span>
        <span style={{ color: 'var(--cp-muted)' }}>· {periodLabel(limit.period)} · {unitLabel(limit.unit)}</span>
        <span className="ml-auto"><LimitStatusDot status={limit.status} /></span>
      </div>
      <LimitBar limit={limit} />
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
        <span>{t('aiCenter.policy.used', 'Used')} <LimitAmount unit={limit.unit} value={limit.used} /> / <LimitAmount unit={limit.unit} value={limit.amount} /></span>
        <span>{t('aiCenter.policy.from', 'From {{subject}}', { subject: subjectLabel(limit.subject) })} · {counterLabel(limit.counter)}</span>
        <span className="md:ml-auto"><ResetLabel limit={limit} /></span>
      </div>
    </div>
  )
}

/** Compact mark of a model's price for the viewer. */
export function CreditPriceBadge({ price, testId = 'credit-price' }: { price: ModelCreditPrice; testId?: string }) {
  const text = usePriceText()
  const tone = price.state === 'free' ? 'ok' : price.state === 'unpriced' ? 'error' : 'muted'
  return <StatusDot testId={testId} status={price.state} tone={tone} label={text(price)} />
}

/** Right-hand drawer on desktop, full screen on phones, where the shell's status bar shows its title and Back. */
export function SidePanel({ title, onClose, children, testId, wide = false }: { title: string; onClose: () => void; children: ReactNode; testId?: string; wide?: boolean }) {
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 767px)')
  useMobileBackHandler(isMobile ? onClose : null)
  useMobileTitleOverride(isMobile ? { title } : null)
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
        className={`relative flex h-full w-full flex-col md:shadow-xl ${wide ? 'md:max-w-[720px]' : 'md:max-w-[560px]'}`}
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

/** A centered dialog on desktop, a bottom sheet on phones. */
export function Modal({ title, onClose, children, testId, busy = false }: { title: string; onClose: () => void; children: ReactNode; testId?: string; busy?: boolean }) {
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center md:items-center" data-testid={testId}>
      <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.4)' }} onClick={busy ? undefined : onClose} />
      <div role="dialog" aria-label={title} className="relative max-h-[calc(100dvh-1rem)] w-full overflow-y-auto rounded-t-xl p-6 shadow-lg md:mx-4 md:max-w-lg md:rounded-xl" style={{ background: 'var(--cp-surface)', paddingBottom: 'calc(1.5rem + env(safe-area-inset-bottom))' }}>
        <h3 className="mb-3 text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{title}</h3>
        {children}
      </div>
    </div>
  )
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="min-w-0 break-words text-sm" style={{ color: 'var(--cp-text)' }}>{children}</span>
    </div>
  )
}

export function Segmented<T extends string>({ label, value, options, onChange, testId }: { label: string; value: T; options: { value: T; label: string; disabled?: boolean }[]; onChange: (value: T) => void; testId?: string }) {
  return (
    <div role="radiogroup" aria-label={label} data-testid={testId} className="flex flex-wrap gap-1 rounded-lg p-1" style={{ background: 'var(--cp-surface-2)' }}>
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

/** A price coefficient: no opinion, free (0) or a multiplier of the price list. */
export function CoefficientField({ label, value, onChange, testId, inherited }: { label: string; value: number | undefined; onChange: (value: number | undefined) => void; testId?: string; inherited?: string }) {
  const { t } = useI18n()
  const mode = value === undefined ? 'unset' : value === 0 ? 'free' : 'charged'
  const [text, setText] = useState(value !== undefined && value !== 0 ? String(value) : '1')
  const parsed = Number(text)
  const invalid = mode === 'charged' && !(text.trim() !== '' && Number.isFinite(parsed) && parsed > 0)
  return (
    <div className="flex flex-col gap-1.5" data-testid={testId}>
      <span className="text-sm" style={{ color: 'var(--cp-text)' }}>{label}</span>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <div className="sm:w-80">
          <Segmented
            label={label}
            value={mode}
            onChange={(next) => onChange(next === 'unset' ? undefined : next === 'free' ? 0 : (Number.isFinite(parsed) && parsed > 0 ? parsed : 1))}
            options={[
              { value: 'unset', label: t('aiCenter.policy.coefficient.unset', 'Not set') },
              { value: 'free', label: t('aiCenter.policy.coefficient.free', 'Free') },
              { value: 'charged', label: t('aiCenter.policy.coefficient.charged', 'Charged') },
            ]}
          />
        </div>
        {mode === 'charged' && (
          <label className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-muted)' }}>
            ×
            <input
              data-testid={testId ? `${testId}-value` : undefined}
              aria-label={t('aiCenter.policy.coefficient.value', 'Multiplier of the price list')}
              value={text}
              inputMode="decimal"
              onChange={(event) => {
                setText(event.target.value)
                const next = Number(event.target.value)
                if (event.target.value.trim() !== '' && Number.isFinite(next) && next > 0) onChange(next)
              }}
              className="min-h-10 w-24 rounded-lg px-3 text-sm"
              style={{ ...fieldStyle, borderColor: invalid ? 'var(--cp-danger)' : 'var(--cp-border)' }}
            />
          </label>
        )}
      </div>
      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {mode === 'unset'
          ? inherited ?? t('aiCenter.policy.coefficient.unsetHint', 'A less specific layer decides; with none, the price list applies as is (×1).')
          : mode === 'free'
            ? t('aiCenter.policy.coefficient.freeHint', 'Calls cost no AI Credit, priced or not.')
            : t('aiCenter.policy.coefficient.chargedHint', 'The price list × this multiplier. Models without a price cannot be used.')}
      </span>
    </div>
  )
}

function LimitRow({ limit, onChange, onRemove, testId }: { limit: PolicyLimit; onChange: (limit: PolicyLimit) => void; onRemove: () => void; testId?: string }) {
  const { t } = useI18n()
  const periodLabel = usePeriodLabel()
  const unitLabel = useUnitLabel()
  const [text, setText] = useState(limit.amount)
  const invalid = parseLimitAmount(limit.unit, text) === null
  return (
    <div data-testid={testId} className="flex flex-wrap items-center gap-2">
      <select aria-label={t('aiCenter.policy.limit.period', 'Window')} data-testid="limit-period" value={limit.period} onChange={(event) => onChange({ ...limit, period: event.target.value as PolicyLimit['period'] })} className="min-h-10 rounded-lg px-2 text-sm" style={fieldStyle}>
        {LIMIT_PERIODS.map((period) => <option key={period} value={period}>{periodLabel(period)}</option>)}
      </select>
      <input
        aria-label={t('aiCenter.policy.limit.amount', 'Amount')}
        data-testid="limit-amount"
        value={text}
        inputMode="decimal"
        placeholder={limit.unit === 'credit' ? '10' : '200K'}
        onChange={(event) => {
          setText(event.target.value)
          const amount = parseLimitAmount(limit.unit, event.target.value)
          onChange({ ...limit, amount: amount ?? event.target.value })
        }}
        className="min-h-10 w-28 min-w-0 flex-1 rounded-lg px-3 text-sm sm:flex-none"
        style={{ ...fieldStyle, borderColor: invalid ? 'var(--cp-danger)' : 'var(--cp-border)' }}
      />
      <select aria-label={t('aiCenter.policy.limit.unit', 'Unit')} data-testid="limit-unit" value={limit.unit} onChange={(event) => {
        const unit = event.target.value as PolicyLimit['unit']
        onChange({ ...limit, unit, amount: parseLimitAmount(unit, text) ?? text })
      }} className="min-h-10 rounded-lg px-2 text-sm" style={fieldStyle}>
        {LIMIT_UNITS.map((unit) => <option key={unit} value={unit}>{unitLabel(unit)}</option>)}
      </select>
      <button type="button" onClick={onRemove} aria-label={t('aiCenter.policy.limit.remove', 'Remove limit')} className="min-h-10 min-w-10 rounded-lg p-2" style={{ color: 'var(--cp-muted)' }}>
        <Trash2 size={16} />
      </button>
    </div>
  )
}

/** Time window limits of one place of a policy; every limit applies (one per window and unit). */
export function LimitsEditor({ limits, onChange, testId, hint }: { limits: PolicyLimit[] | undefined; onChange: (limits: PolicyLimit[]) => void; testId?: string; hint?: string }) {
  const { t } = useI18n()
  const list = limits ?? []
  const [keys, setKeys] = useState(() => list.map((_, index) => index))
  const [next, setNext] = useState(list.length)
  const duplicate = new Set<string>()
  const seen = new Set<string>()
  list.forEach((limit) => {
    const key = `${limit.period}/${limit.unit}`
    if (seen.has(key)) duplicate.add(key)
    seen.add(key)
  })
  const add = () => {
    const used = new Set(list.map((limit) => `${limit.period}/${limit.unit}`))
    const period = LIMIT_PERIODS.find((item) => !used.has(`${item}/credit`)) ?? '24h'
    setKeys([...keys, next])
    setNext(next + 1)
    onChange([...list, { period, unit: 'credit', amount: '10' }])
  }
  return (
    <div className="flex flex-col gap-2" data-testid={testId}>
      {list.map((limit, index) => (
        <LimitRow
          key={keys[index] ?? `extra-${index}`}
          testId="limit-row"
          limit={limit}
          onChange={(changed) => onChange(list.map((item, position) => position === index ? changed : item))}
          onRemove={() => { setKeys(keys.filter((_, position) => position !== index)); onChange(list.filter((_, position) => position !== index)) }}
        />
      ))}
      {duplicate.size > 0 && <span className="text-xs" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.policy.limit.duplicate', 'One limit per window and unit.')}</span>}
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" data-testid={testId ? `${testId}-add` : undefined} onClick={add} className="inline-flex min-h-9 items-center gap-1 rounded-lg px-2 text-sm" style={{ color: 'var(--cp-accent)' }}>
          <Plus size={14} />{t('aiCenter.policy.limit.add', 'Add a limit')}
        </button>
        {hint && <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{hint}</span>}
      </div>
    </div>
  )
}

/** What an AICC refusal means for the caller (§3.7), from its code; shown next to the raw error. */
export function PolicyRefusalNote({ error }: { error: unknown }) {
  const { t, locale } = useI18n()
  const refusal = policyRefusalOf(error)
  if (!refusal) return null
  let text: string
  switch (refusal.code) {
    case 'quota_exhausted': {
      const reset = refusal.details?.reset_at_ms
      text = reset
        ? t('aiCenter.policy.refusal.exhaustedUntil', 'An AI usage limit is used up until {{time}}.', { time: formatLocal(reset, locale) })
        : t('aiCenter.policy.refusal.exhausted', 'An AI usage limit is used up or set to zero. An administrator can raise it under AI Center › Policies.')
      break
    }
    case 'credit_insufficient':
      text = t('aiCenter.policy.refusal.credit', 'Your AI Credit balance is {{balance}}, so models that cost AI Credit are refused. Ask an administrator to recharge it. Free models still work. (This is not the Provider account balance.)', { balance: formatCredit(refusal.details?.balance, locale) })
      break
    case 'quota_unavailable':
      text = t('aiCenter.policy.refusal.unavailable', 'The policy or balance is temporarily unavailable, so the call was not started. Try again shortly.')
      break
    case 'resource_not_permitted':
      text = isUnpricedRefusal(refusal)
        ? t('aiCenter.policy.refusal.unpriced', 'This model has no price for you, so it cannot be used. Ask an administrator to price it under AI Center › Price list, or make it free.')
        : t('aiCenter.policy.refusal.notPermitted', 'This model is not open to you. An administrator can open it under AI Center › Policies.')
      break
    case 'attribution_unavailable':
      text = t('aiCenter.policy.refusal.attribution', 'The call could not be attributed to a user, so it was not started.')
      break
  }
  return <p data-testid="policy-refusal" data-code={refusal.code} className="text-sm" style={{ color: 'var(--cp-text)' }}>{text}</p>
}
