import type { ReactNode } from 'react'

const toneStyles = {
  neutral: 'border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] text-[color:var(--cp-muted)]',
  success: 'border-[color:color-mix(in_srgb,var(--cp-success)_42%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-success)_12%,transparent)] text-[color:var(--cp-success)]',
  warning: 'border-[color:color-mix(in_srgb,var(--cp-warning)_48%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-warning)_13%,transparent)] text-[color:var(--cp-warning)]',
  danger: 'border-[color:color-mix(in_srgb,var(--cp-danger)_44%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-danger)_12%,transparent)] text-[color:var(--cp-danger)]',
  accent: 'border-[color:color-mix(in_srgb,var(--cp-accent)_38%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-accent)_10%,transparent)] text-[color:var(--cp-accent)]',
}

export function StatusBadge({
  children,
  tone = 'neutral',
}: {
  children: ReactNode
  tone?: keyof typeof toneStyles
}) {
  return (
    <span className={`inline-flex items-center gap-1 rounded-full border px-2.5 py-1 text-xs font-semibold ${toneStyles[tone]}`}>
      {children}
    </span>
  )
}
