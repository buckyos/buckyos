import Icon from '../../icons'
import StatusPill from './StatusPill'

const typeIcons: Record<WorkLogType, IconName> = {
  message_sent: 'message',
  message_reply: 'message',
  function_call: 'function',
  action: 'action',
  sub_agent_created: 'branch',
  sub_agent_sleep: 'pause',
  sub_agent_wake: 'play',
  sub_agent_destroyed: 'close',
}

const typeLabels: Record<WorkLogType, string> = {
  message_sent: 'Message Sent',
  message_reply: 'Message Reply',
  function_call: 'Function Call',
  action: 'Action',
  sub_agent_created: 'Sub-Agent Created',
  sub_agent_sleep: 'Sub-Agent Sleep',
  sub_agent_wake: 'Sub-Agent Wake',
  sub_agent_destroyed: 'Sub-Agent Destroyed',
}

type WorkLogRowProps = {
  log: WsWorkLog
  onClick: () => void
}

const formatTime = (iso: string): string => {
  if (!iso) return '—'
  return new Date(iso).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

const WorkLogRow = ({ log, onClick }: WorkLogRowProps) => {
  const iconName = typeIcons[log.type] ?? 'chart'
  const isFailed = log.status === 'failed'

  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition hover:bg-[var(--cp-surface-muted)] ${
        isFailed ? 'bg-rose-50/50' : ''
      }`}
    >
      {/* Type icon */}
      <span
        className={`inline-flex size-7 flex-none items-center justify-center rounded-lg ${
          isFailed ? 'bg-rose-100 text-rose-600' : 'bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]'
        }`}
      >
        <Icon name={iconName} className="size-3.5" />
      </span>

      {/* Summary */}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[10px] font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            {typeLabels[log.type]}
          </span>
          <StatusPill status={log.status} />
          {log.step_id && (
            <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">
              {log.step_id.slice(-6)}
            </span>
          )}
        </div>
        <p className="mt-0.5 truncate text-xs text-[var(--cp-ink)]">{log.summary}</p>
      </div>

      {/* Timestamp + duration */}
      <div className="flex flex-none flex-col items-end text-[10px] text-[var(--cp-muted)]">
        <span>{formatTime(log.timestamp)}</span>
        {log.duration != null && <span>{log.duration}s</span>}
      </div>
    </button>
  )
}

export default WorkLogRow
