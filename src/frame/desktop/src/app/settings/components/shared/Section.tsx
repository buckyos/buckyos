import type { ReactNode } from 'react'

interface SectionProps {
  title: string
  description?: string
  children: ReactNode
  /** Controls at the right of the title (refresh, add, …). */
  actions?: ReactNode
  testId?: string
}

export function Section({ title, description, children, actions, testId }: SectionProps) {
  return (
    <section className="shell-subtle-panel p-4 sm:p-5" data-testid={testId}>
      <div className="flex items-center justify-between gap-3">
        <h3
          className="font-display text-base font-semibold"
          style={{ color: 'var(--cp-text)' }}
        >
          {title}
        </h3>
        {actions ? <div className="flex shrink-0 items-center gap-1">{actions}</div> : null}
      </div>
      {description && (
        <p className="mt-1 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
          {description}
        </p>
      )}
      <div className="mt-3">{children}</div>
    </section>
  )
}

interface CollapsibleSectionProps {
  title: string
  description?: string
  children: ReactNode
  defaultCollapsed?: boolean
}

export function CollapsibleSection({
  title,
  description,
  children,
  defaultCollapsed = true,
}: CollapsibleSectionProps) {
  return (
    <details className="shell-subtle-panel p-4 sm:p-5 group" open={!defaultCollapsed}>
      <summary
        className="cursor-pointer select-none list-none flex items-center justify-between"
      >
        <div>
          <h3
            className="font-display text-base font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            {title}
          </h3>
          {description && (
            <p className="mt-1 text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
              {description}
            </p>
          )}
        </div>
        <span
          className="text-xs transition-transform group-open:rotate-90"
          style={{ color: 'var(--cp-muted)' }}
        >
          ▶
        </span>
      </summary>
      <div className="mt-3">{children}</div>
    </details>
  )
}

interface InfoRowProps {
  label: string
  value: ReactNode
}

export function InfoRow({ label, value }: InfoRowProps) {
  return (
    <div className="flex flex-col gap-1.5 py-2 text-sm sm:flex-row sm:items-center sm:justify-between sm:gap-4">
      <span style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span
        className="w-full break-words font-medium text-left sm:w-auto sm:text-right"
        style={{ color: 'var(--cp-text)' }}
      >
        {value}
      </span>
    </div>
  )
}

interface StatusBadgeProps {
  status: 'pass' | 'warn' | 'fail' | 'info' | 'neutral'
  label: string
}

export function StatusBadge({ status, label }: StatusBadgeProps) {
  const colorMap: Record<StatusBadgeProps['status'], string> = {
    pass: 'var(--cp-success)',
    warn: 'var(--cp-warning)',
    fail: 'var(--cp-danger)',
    info: 'var(--cp-accent)',
    neutral: 'var(--cp-muted)',
  }

  const color = colorMap[status] ?? 'var(--cp-muted)'

  return (
    <span
      className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium"
      style={{
        color,
        background: `color-mix(in srgb, ${color} 14%, transparent)`,
      }}
    >
      <span
        className="w-1.5 h-1.5 rounded-full"
        style={{ background: color }}
      />
      {label}
    </span>
  )
}

/** A muted one-line state inside a section (loading, empty, failed). */
export function SectionNote({ tone = 'muted', children }: { tone?: 'muted' | 'danger' | 'warning'; children: ReactNode }) {
  const color = tone === 'danger' ? 'var(--cp-danger)' : tone === 'warning' ? 'var(--cp-warning)' : 'var(--cp-muted)'
  return (
    <p className="text-sm leading-6" style={{ color }} role={tone === 'danger' ? 'alert' : undefined}>
      {children}
    </p>
  )
}

/** A rounded row inside a section's list. */
export function ListCard({ children, testId }: { children: ReactNode; testId?: string }) {
  return (
    <div
      className="rounded-lg p-3"
      style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
      data-testid={testId}
    >
      {children}
    </div>
  )
}
