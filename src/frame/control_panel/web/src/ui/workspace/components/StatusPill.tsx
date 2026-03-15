const statusColors: Record<string, string> = {
  running: 'bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]',
  success: 'bg-emerald-100 text-emerald-700',
  failed: 'bg-rose-100 text-rose-700',
  error: 'bg-rose-100 text-rose-700',
  cancelled: 'bg-slate-100 text-slate-600',
  sleeping: 'bg-amber-100 text-amber-700',
  idle: 'bg-slate-100 text-slate-600',
  offline: 'bg-slate-100 text-slate-500',
  queued: 'bg-sky-100 text-sky-700',
  skipped: 'bg-slate-100 text-slate-500',
  partial: 'bg-amber-100 text-amber-700',
  info: 'bg-sky-100 text-sky-700',
  open: 'bg-sky-100 text-sky-700',
  done: 'bg-emerald-100 text-emerald-700',
}

type StatusPillProps = {
  status: string
  className?: string
}

const StatusPill = ({ status, className = '' }: StatusPillProps) => {
  const colors = statusColors[status] ?? 'bg-slate-100 text-slate-600'
  return (
    <span className={`cp-pill ${colors} ${className}`}>
      {status === 'running' && (
        <span className="inline-flex size-1.5 animate-pulse rounded-full bg-current" />
      )}
      {status}
    </span>
  )
}

export default StatusPill
