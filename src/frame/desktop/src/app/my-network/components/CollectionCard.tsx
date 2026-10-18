/* ── Navigation entry of one view or Collection ── */

import type { ReactNode } from 'react'

export function NavItem({
  icon,
  label,
  hint,
  count,
  badge,
  active,
  onClick,
  testId,
}: {
  icon: ReactNode
  label: string
  hint?: string
  count?: number
  /** A count that asks for attention (Requests). */
  badge?: number
  active: boolean
  onClick: () => void
  testId?: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      data-testid={testId}
      className="flex w-full items-center gap-2.5 rounded-xl border px-2.5 py-2 text-left"
      style={active
        ? { background: 'color-mix(in srgb, var(--cp-accent-soft) 20%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))' }
        : { borderColor: 'transparent' }}
    >
      <span
        className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full"
        style={{ background: 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))', color: 'var(--cp-accent)' }}
      >
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-[color:var(--cp-text)]">{label}</span>
        {hint ? <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">{hint}</span> : null}
      </span>
      {badge ? (
        <span className="shrink-0 rounded-full px-1.5 text-[11px] font-semibold tabular-nums leading-5" style={{ background: 'var(--cp-danger)', color: 'var(--cp-bg)' }}>{badge}</span>
      ) : count !== undefined ? (
        <span className="shrink-0 text-[11px] font-medium tabular-nums text-[color:var(--cp-muted)]">{count}</span>
      ) : null}
    </button>
  )
}
