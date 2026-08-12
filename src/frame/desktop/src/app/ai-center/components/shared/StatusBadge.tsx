const statusColors: Record<string, string> = {
  ok: 'var(--cp-success)',
  warning: 'var(--cp-warning)',
  error: 'var(--cp-danger)',
  unknown: 'var(--cp-muted)',
}

interface StatusBadgeProps {
  status: 'ok' | 'warning' | 'error' | 'unknown'
  label?: string
}

export function StatusBadge({ status, label }: StatusBadgeProps) {
  const color = statusColors[status] ?? statusColors.unknown

  return (
    <span className="inline-flex items-center gap-1.5 text-xs">
      <span
        className="inline-block w-2 h-2 rounded-full shrink-0"
        style={{ background: color }}
      />
      {label && <span style={{ color: 'var(--cp-text)' }}>{label}</span>}
    </span>
  )
}
