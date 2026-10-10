/* ── Agent setup – shared presentational pieces ── */

import clsx from 'clsx'
import type { ReactNode } from 'react'

export type Tone = 'success' | 'warning' | 'danger' | 'accent' | 'muted'

const toneColor: Record<Tone, string> = {
  success: 'var(--cp-success)',
  warning: 'var(--cp-warning)',
  danger: 'var(--cp-danger)',
  accent: 'var(--cp-accent)',
  muted: 'var(--cp-muted)',
}

const selectedSurface = {
  background: 'color-mix(in srgb, var(--cp-accent-soft) 18%, var(--cp-surface))',
  borderColor: 'color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))',
}

export function Section({
  title,
  description,
  actions,
  children,
  testId,
}: {
  title?: ReactNode
  description?: ReactNode
  actions?: ReactNode
  children?: ReactNode
  testId?: string
}) {
  return (
    <section
      data-testid={testId}
      className="rounded-[18px] border px-4 py-4 sm:px-5"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 36%, var(--cp-surface))',
        borderColor: 'color-mix(in srgb, var(--cp-border) 70%, transparent)',
      }}
    >
      {title || actions ? (
        <div className="mb-3 flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0">
            {title ? (
              <h3 className="font-display text-sm font-semibold text-[color:var(--cp-text)]">{title}</h3>
            ) : null}
            {description ? (
              <p className="mt-1 text-[13px] leading-5 text-[color:var(--cp-muted)]">{description}</p>
            ) : null}
          </div>
          {actions ? <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
        </div>
      ) : null}
      {children}
    </section>
  )
}

export function StatusPill({ tone, children, testId }: { tone: Tone; children: ReactNode; testId?: string }) {
  const color = toneColor[tone]
  return (
    <span
      data-testid={testId}
      className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-semibold"
      style={{
        background: `color-mix(in srgb, ${color} 16%, var(--cp-surface))`,
        color: tone === 'muted' ? 'var(--cp-muted)' : `color-mix(in srgb, ${color} 72%, var(--cp-text))`,
      }}
    >
      {children}
    </span>
  )
}

export function OptionCard({
  selected,
  disabled,
  onSelect,
  title,
  description,
  badge,
  icon,
  children,
  testId,
}: {
  selected: boolean
  disabled?: boolean
  onSelect?: () => void
  title: ReactNode
  description?: ReactNode
  badge?: ReactNode
  icon?: ReactNode
  children?: ReactNode
  testId?: string
}) {
  return (
    <div
      data-testid={testId}
      className={clsx('rounded-[16px] border transition-colors duration-150', disabled ? 'opacity-70' : '')}
      style={selected ? selectedSurface : { background: 'var(--cp-surface)', borderColor: 'var(--cp-border)' }}
    >
      <button
        type="button"
        role="radio"
        aria-checked={selected}
        aria-disabled={disabled || undefined}
        disabled={disabled}
        onClick={onSelect}
        className={clsx(
          'flex w-full items-start gap-3 rounded-[16px] px-4 py-3 text-left',
          disabled ? 'cursor-not-allowed' : 'cursor-pointer',
        )}
      >
        <span
          aria-hidden="true"
          className="mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border"
          style={{ borderColor: selected ? 'var(--cp-accent)' : 'var(--cp-border-opaque)' }}
        >
          {selected ? <span className="size-2 rounded-full" style={{ background: 'var(--cp-accent)' }} /> : null}
        </span>
        {icon ? <span className="shrink-0 text-[color:var(--cp-muted)]">{icon}</span> : null}
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-2 text-sm font-semibold text-[color:var(--cp-text)]">
            {title}
            {badge}
          </span>
          {description ? (
            <span className="mt-0.5 block text-[12px] leading-5 text-[color:var(--cp-muted)]">{description}</span>
          ) : null}
        </span>
      </button>
      {children ? <div className="px-4 pb-4">{children}</div> : null}
    </div>
  )
}

export function SummaryRows({ rows }: { rows: Array<[ReactNode, ReactNode]> }) {
  return (
    <dl className="space-y-1.5">
      {rows.map(([label, value], index) => (
        <div key={index} className="flex flex-col gap-0.5 sm:flex-row sm:items-baseline sm:gap-3">
          <dt className="shrink-0 text-[12px] font-medium text-[color:var(--cp-muted)] sm:w-40">{label}</dt>
          <dd className="min-w-0 break-words text-sm text-[color:var(--cp-text)]">{value}</dd>
        </div>
      ))}
    </dl>
  )
}

export function AgentAvatar({
  name,
  src,
  size = 56,
}: {
  name: string
  src?: string | null
  size?: number
}) {
  const initial = [...name.trim()][0]?.toUpperCase() ?? '?'
  return src ? (
    <img
      src={src}
      alt=""
      className="shrink-0 rounded-full object-cover"
      style={{ width: size, height: size }}
    />
  ) : (
    <span
      aria-hidden="true"
      className="flex shrink-0 items-center justify-center rounded-full font-display font-semibold"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.4),
        background: 'color-mix(in srgb, var(--cp-accent) 16%, var(--cp-surface))',
        color: 'color-mix(in srgb, var(--cp-accent) 80%, var(--cp-text))',
      }}
    >
      {initial}
    </span>
  )
}
