import { useState, useSyncExternalStore } from 'react'
import useSWR from 'swr'
import { AlertTriangle, Bell, CheckCircle2, Info, XCircle } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore } from '../../hooks/use-aicc-store'
import { CommandLabel } from '../routing/RoutingKind'
import type { AiccEvent } from '../../datamodel/routing'

const surface = { background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }
const muted = { color: 'var(--cp-muted)' }
const RECENT_LIMIT = 6

export function AiccEventsCard({ onOpenRouting }: { onOpenRouting?: () => void }) {
  const { t, locale } = useI18n()
  const store = useAICCStore()
  const version = useSyncExternalStore(store.subscribe, store.getSnapshotVersion)
  const [expanded, setExpanded] = useState(false)
  const { data, error } = useSWR(['aicc-events', store, version], () => store.listEvents(50), { refreshInterval: 30000, keepPreviousData: true })
  const active = data?.activeWarnings ?? []
  const events = data?.events ?? []
  const visible = expanded ? events : events.slice(0, RECENT_LIMIT)

  return (
    <section className="rounded-xl p-4" style={surface} aria-label={t('aiCenter.events.title', 'System events')} data-testid="aicc-events">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="flex items-center gap-2 text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
          <Bell size={16} style={{ color: active.length ? 'var(--cp-warning)' : 'var(--cp-accent)' }} />
          {t('aiCenter.events.title', 'System events')}
          {active.length > 0 && (
            <span className="rounded-md px-2 py-0.5 text-xs" style={{ color: 'var(--cp-warning)', background: 'color-mix(in srgb, var(--cp-warning) 12%, transparent)' }}>
              {t('aiCenter.events.activeCount', '{{count}} need attention', { count: active.length })}
            </span>
          )}
        </h3>
        {active.length > 0 && onOpenRouting && (
          <button type="button" onClick={onOpenRouting} className="min-h-8 px-2 text-xs" style={{ color: 'var(--cp-accent)' }}>
            {t('aiCenter.events.openRouting', 'Open routing')}
          </button>
        )}
      </div>
      {error && !data && <p className="text-sm" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.events.loadFailed', 'Could not load AICC events.')}</p>}
      {active.length > 0 && (
        <div className="mb-3 flex flex-col gap-1.5">
          {active.map((event) => <EventRow key={`active-${event.id}`} event={event} locale={locale} highlight />)}
        </div>
      )}
      {data && events.length === 0 && active.length === 0 && (
        <p className="text-sm" style={muted}>{t('aiCenter.events.empty', 'No notable events. Routing adjustments and registry problems will show up here.')}</p>
      )}
      {visible.length > 0 && (
        <div className="flex flex-col gap-1">
          {visible.map((event) => <EventRow key={event.id} event={event} locale={locale} />)}
        </div>
      )}
      {events.length > RECENT_LIMIT && (
        <button type="button" onClick={() => setExpanded(!expanded)} className="mt-2 min-h-8 text-xs" style={{ color: 'var(--cp-accent)' }}>
          {expanded ? t('aiCenter.events.showLess', 'Show less') : t('aiCenter.events.showAll', 'Show all ({{count}})', { count: events.length })}
        </button>
      )}
    </section>
  )
}

function EventRow({ event, locale, highlight = false }: { event: AiccEvent; locale: string; highlight?: boolean }) {
  const { t } = useI18n()
  const color = event.level === 'error' ? 'var(--cp-danger)' : event.level === 'warning' ? 'var(--cp-warning)' : event.kind.endsWith('recovered') ? 'var(--cp-success)' : 'var(--cp-muted)'
  const Icon = event.level === 'error' ? XCircle : event.level === 'warning' ? AlertTriangle : event.kind.endsWith('recovered') ? CheckCircle2 : Info
  const reason = typeof event.details.reason === 'string' ? event.details.reason : typeof event.details.error === 'string' ? event.details.error : undefined
  return (
    <div
      className="flex items-start gap-2 rounded-lg px-2 py-1.5 text-xs"
      data-testid="aicc-event"
      data-kind={event.kind}
      style={{ background: highlight ? 'color-mix(in srgb, var(--cp-warning) 8%, var(--cp-bg))' : 'var(--cp-bg)' }}
    >
      <Icon size={14} className="mt-0.5 shrink-0" style={{ color }} />
      <div className="min-w-0 flex-1" style={{ color: 'var(--cp-text)' }}>
        <p className="font-medium">{eventTitle(event, t)}</p>
        {event.command && <div style={muted}><CommandLabel command={event.command} /></div>}
        {reason && <p className="break-words" style={muted}>{reason}</p>}
      </div>
      <time className="shrink-0 tabular-nums" style={muted} dateTime={new Date(event.createdAtMs).toISOString()}>
        {new Date(event.createdAtMs).toLocaleString(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}
      </time>
    </div>
  )
}

function eventTitle(event: AiccEvent, t: ReturnType<typeof useI18n>['t']): string {
  switch (event.kind) {
    case 'routing_command_stale':
      return t('aiCenter.events.routingCommandStale', 'A routing adjustment no longer applies')
    case 'routing_command_recovered':
      return t('aiCenter.events.routingCommandRecovered', 'A routing adjustment applies again')
    case 'routing_commands_updated':
      return t('aiCenter.events.routingCommandsUpdated', 'Routing adjustments saved ({{count}} active)', { count: Number(event.details.count ?? 0) })
    case 'registry_build_failed':
      return t('aiCenter.events.registryBuildFailed', 'Model registry rebuild failed')
    case 'registry_recovered':
      return t('aiCenter.events.registryRecovered', 'Model registry rebuilt successfully')
    default:
      return event.message
  }
}
