import Icon from '../../icons'
import StatusPill from '../components/StatusPill'
import JsonViewer from '../components/JsonViewer'

type WorkLogDetailProps = {
  log: WsWorkLog
}

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

const formatTime = (iso: string): string => {
  if (!iso) return '—'
  const d = new Date(iso)
  return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

const WorkLogDetail = ({ log }: WorkLogDetailProps) => {
  const iconName = typeIcons[log.type] ?? 'chart'

  return (
    <div className="space-y-4">
      {/* Header */}
      <div>
        <div className="flex items-center gap-2">
          <span className="inline-flex size-7 items-center justify-center rounded-lg bg-[var(--cp-surface-muted)]">
            <Icon name={iconName} className="size-3.5 text-[var(--cp-muted)]" />
          </span>
          <span className="text-sm font-semibold text-[var(--cp-ink)]">
            {log.type.replace(/_/g, ' ')}
          </span>
          <StatusPill status={log.status} />
        </div>
      </div>

      {/* Summary */}
      <div className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs leading-5 text-[var(--cp-ink)]">
        {log.summary}
      </div>

      {/* Metadata */}
      <div className="grid grid-cols-2 gap-2 text-xs">
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Time</p>
          <p className="font-medium text-[var(--cp-ink)]">{formatTime(log.timestamp)}</p>
        </div>
        {log.duration != null && (
          <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[var(--cp-muted)]">Duration</p>
            <p className="font-medium text-[var(--cp-ink)]">{log.duration}s</p>
          </div>
        )}
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Agent</p>
          <p className="font-medium text-[var(--cp-ink)]">{log.agent_id}</p>
        </div>
        {log.related_agent_id && (
          <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[var(--cp-muted)]">Related Agent</p>
            <p className="font-medium text-[var(--cp-ink)]">{log.related_agent_id}</p>
          </div>
        )}
        {log.step_id && (
          <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[var(--cp-muted)]">Step</p>
            <p className="font-medium text-[var(--cp-ink)]">{log.step_id}</p>
          </div>
        )}
      </div>

      {/* Type-specific rendering */}
      {log.type === 'function_call' && log.payload != null && (
        <div className="space-y-2">
          {log.payload.function != null && (
            <div className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs">
              <span className="font-mono font-medium text-[var(--cp-primary-strong)]">
                {String(log.payload.function)}()
              </span>
            </div>
          )}
          {log.payload.input != null && <JsonViewer label="Input" data={log.payload.input} />}
          {log.payload.output != null && <JsonViewer label="Output" data={log.payload.output} />}
          {log.payload.error != null && (
            <div className="rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-700">
              {String(log.payload.error)}
            </div>
          )}
        </div>
      )}

      {log.type === 'action' && log.payload?.batch != null && (
        <div>
          <h4 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            Action Batch
          </h4>
          <div className="space-y-1">
            {(log.payload.batch as Array<{ name: string; status: string; reason?: string }>).map(
              (item, i) => (
                <div
                  key={`${item.name}-${i}`}
                  className={`flex items-center gap-2 rounded-lg border px-3 py-2 text-xs ${
                    item.status === 'success'
                      ? 'border-emerald-200 bg-emerald-50'
                      : item.status === 'skipped'
                        ? 'border-slate-200 bg-slate-50'
                        : 'border-rose-200 bg-rose-50'
                  }`}
                >
                  <StatusPill status={item.status} />
                  <span className="flex-1 text-[var(--cp-ink)]">{item.name}</span>
                  {item.reason && (
                    <span className="text-[var(--cp-muted)]">{item.reason}</span>
                  )}
                </div>
              ),
            )}
          </div>
        </div>
      )}

      {/* Raw payload */}
      {log.payload != null &&
        log.type !== 'function_call' &&
        log.type !== 'action' && (
          <JsonViewer label="Payload" data={log.payload} />
        )}

      <div className="text-[10px] text-[var(--cp-muted)]">ID: {log.log_id}</div>
    </div>
  )
}

export default WorkLogDetail
