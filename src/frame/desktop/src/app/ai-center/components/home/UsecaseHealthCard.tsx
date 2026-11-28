import { useSyncExternalStore, type ReactNode } from 'react'
import useSWR from 'swr'
import { ChevronRight, ListChecks, Plug, Shuffle } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore, useProviders } from '../../hooks/use-aicc-store'
import { useUsecaseTable } from '../../hooks/use-routing-api'
import { FALLBACK_LLM } from '../../datamodel/usecases'
import { panelStyle, toneColor } from '../policy/policy-labels'

const muted = { color: 'var(--cp-muted)' }

function Row({ icon, tone, title, detail, onClick, testId, count, first = false }: {
  icon: ReactNode
  tone: keyof typeof toneColor
  title: string
  detail?: string
  onClick: () => void
  testId: string
  count: number
  first?: boolean
}) {
  return (
    <button type="button" onClick={onClick} data-testid={testId} data-count={count} className="flex min-h-12 w-full items-center gap-3 px-4 py-2.5 text-left" style={first ? undefined : { borderTop: '1px solid var(--cp-border)' }}>
      <span className="shrink-0" style={muted}>{icon}</span>
      <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: toneColor[tone] }} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="text-sm" style={{ color: 'var(--cp-text)' }}>{title}</span>
        {detail && <span className="truncate text-xs" style={muted}>{detail}</span>}
      </span>
      <ChevronRight size={16} className="shrink-0" style={muted} />
    </button>
  )
}

/** Uncovered usecases, Providers marked faulty and recent policy switches, each opening its page. */
export function UsecaseHealthCard({ onOpenUsecases, onOpenProviders, onOpenSwitches }: {
  onOpenUsecases: () => void
  onOpenProviders: () => void
  onOpenSwitches: () => void
}) {
  const { t } = useI18n()
  const store = useAICCStore()
  const version = useSyncExternalStore(store.subscribe, store.getSnapshotVersion)
  const providers = useProviders()
  const { data: table } = useUsecaseTable('self')
  const { data: events } = useSWR(['aicc-events', store, version], () => store.listEvents(50), { refreshInterval: 30000, keepPreviousData: true })
  const rows = (table?.rows ?? []).filter((row) => row.usecase !== FALLBACK_LLM)
  const uncovered = rows.filter((row) => row.coverage !== 'covered')
  const servedByFallback = uncovered.filter((row) => row.coverage === 'fallback_llm').length
  const faulty = providers.filter((provider) => provider.config.marked_faulty)
  const switches = (events?.events ?? []).filter((event) => event.kind === 'unavailable_fallback')
  const latest = switches[0]
  const latestText = latest && typeof latest.details.from === 'string' && typeof latest.details.to === 'string'
    ? t('aiCenter.home.switchLatest', 'Latest: {{from}} → {{to}}', { from: latest.details.from, to: latest.details.to })
    : undefined

  return (
    <section className="overflow-hidden rounded-xl" style={panelStyle} aria-label={t('aiCenter.home.usecaseHealth', 'Usecases and Providers')} data-testid="usecase-health">
      <div className="flex flex-col">
        <Row
          first
          testId="home-uncovered"
          count={uncovered.length}
          icon={<ListChecks size={16} />}
          tone={!table ? 'muted' : uncovered.length > servedByFallback ? 'error' : uncovered.length > 0 ? 'warning' : 'ok'}
          title={!table
            ? t('aiCenter.home.uncoveredLoading', 'Checking usecases…')
            : uncovered.length === 0
              ? t('aiCenter.home.uncoveredNone', 'Every usecase has a model')
              : t('aiCenter.home.uncovered', 'Usecases without a model: {{count}}', { count: uncovered.length })}
          detail={uncovered.length === 0
            ? undefined
            : table?.fallback_llm_enabled
              ? t('aiCenter.home.uncoveredFallback', 'Served by the fallback LLM: {{count}}', { count: servedByFallback })
              : t('aiCenter.home.fallbackOff', 'The fallback LLM is off')}
          onClick={onOpenUsecases}
        />
        <Row
          testId="home-faulty"
          count={faulty.length}
          icon={<Plug size={16} />}
          tone={faulty.length > 0 ? 'warning' : 'ok'}
          title={faulty.length === 0
            ? t('aiCenter.home.faultyNone', 'No Provider is marked faulty')
            : t('aiCenter.home.faulty', 'Providers marked faulty: {{count}}', { count: faulty.length })}
          detail={faulty.length > 0 ? faulty.map((provider) => provider.config.name).join(', ') : undefined}
          onClick={onOpenProviders}
        />
        <Row
          testId="home-switches"
          count={switches.length}
          icon={<Shuffle size={16} />}
          tone={switches.length > 0 ? 'warning' : 'ok'}
          title={switches.length === 0
            ? t('aiCenter.home.switchesNone', 'No recent switch by an unavailable policy')
            : t('aiCenter.home.switches', 'Recent switches by an unavailable policy: {{count}}', { count: switches.length })}
          detail={latestText}
          onClick={onOpenSwitches}
        />
      </div>
    </section>
  )
}
