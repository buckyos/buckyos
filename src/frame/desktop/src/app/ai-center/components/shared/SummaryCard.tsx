import type { ReactNode } from 'react'

interface SummaryCardProps {
  icon: ReactNode
  title: string
  value: string | number
  subtitle?: string
  variant?: 'default' | 'warning' | 'error'
  action?: { label: string; onClick: () => void }
  onClick?: () => void
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
  onClick,
}: SummaryCardProps) {
  return (
    <div
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={(event) => {
        if (!onClick) return
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onClick()
        }
      }}
      className={`rounded-xl p-4 min-h-[124px] flex flex-col gap-1 text-left outline-none transition ${onClick ? 'cursor-pointer hover:shadow-sm focus-visible:ring-2 focus-visible:ring-[color:var(--cp-accent)]' : ''}`}
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
        borderLeft: variant !== 'default'
          ? `4px solid ${variantBorderColor[variant]}`
          : '1px solid var(--cp-border)',
      }}
    >
      <div className="flex items-start gap-2 min-h-10 mb-1">
        <span className="shrink-0" style={{ color: 'var(--cp-accent)' }}>{icon}</span>
        <span className="text-xs font-medium leading-5" style={{ color: 'var(--cp-muted)' }}>
          {title}
        </span>
      </div>
      <div className="text-lg font-semibold leading-7 min-h-7 flex items-center break-words" style={{ color: 'var(--cp-text)' }}>
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
