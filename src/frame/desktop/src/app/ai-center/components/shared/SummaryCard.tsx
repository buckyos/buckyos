import type { ReactNode } from 'react'

interface SummaryCardProps {
  icon: ReactNode
  title: string
  value: string | number
  subtitle?: string
  variant?: 'default' | 'warning' | 'error'
  action?: { label: string; onClick: () => void }
}

const variantBorderColor: Record<string, string> = {
  default: 'transparent',
  warning: 'var(--cp-warning)',
  error: 'var(--cp-danger)',
}

export function SummaryCard({
  icon,
  title,
  value,
  subtitle,
  variant = 'default',
  action,
}: SummaryCardProps) {
  return (
    <div
      className="rounded-xl p-4 flex flex-col gap-1"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
        borderLeft: variant !== 'default'
          ? `4px solid ${variantBorderColor[variant]}`
          : '1px solid var(--cp-border)',
      }}
    >
      <div className="flex items-center gap-2 mb-1">
        <span style={{ color: 'var(--cp-accent)' }}>{icon}</span>
        <span className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
          {title}
        </span>
      </div>
      <div className="text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
        {value}
      </div>
      {subtitle && (
        <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
          {subtitle}
        </div>
      )}
      {action && (
        <button
          type="button"
          onClick={action.onClick}
          className="self-end text-xs font-medium mt-1 hover:underline"
          style={{ color: 'var(--cp-accent)' }}
        >
          {action.label}
        </button>
      )}
    </div>
  )
}
