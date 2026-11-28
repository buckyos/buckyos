import useSWR from 'swr'
import { ArrowRight, Loader2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useRoutingApi } from '../../hooks/use-routing-api'
import { usecaseImpact } from '../../datamodel/usecases'
import { ExactModelLabel } from '../policy/PolicyWidgets'
import { errorText } from '../policy/policy-labels'
import { useUsecaseName } from '../usecases/usecase-labels'

const muted = { color: 'var(--cp-muted)' }

/** The Zone default's usecases that change when a Provider is deleted or disabled: who takes over, or none. */
export function ProviderImpactList({ provider }: { provider: string }) {
  const { t } = useI18n()
  const api = useRoutingApi()
  const name = useUsecaseName()
  const { data, error } = useSWR(['aicc-provider-impact', api, provider], async () => {
    const [current, without] = await Promise.all([api.usecases({ view: 'zone' }), api.usecases({ view: 'zone', withoutProvider: provider })])
    return usecaseImpact(current.rows, without.rows)
  }, { revalidateOnFocus: false })
  if (error) return <p className="break-words text-xs" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.providers.impactFailed', 'Could not check which usecases change: {{error}}', { error: errorText(error) })}</p>
  if (!data) return <p role="status" className="flex items-center gap-1.5 text-xs" style={muted}><Loader2 size={12} className="animate-spin" />{t('aiCenter.providers.impactLoading', 'Checking the usecases it serves…')}</p>
  if (data.length === 0) return <p className="text-xs" data-testid="provider-impact" data-count={0} style={muted}>{t('aiCenter.providers.impactNone', 'No usecase of the Zone default changes.')}</p>
  return (
    <div className="flex flex-col gap-1.5" data-testid="provider-impact" data-count={data.length}>
      <p className="text-xs font-medium" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.providers.impactTitle', 'Usecases that change: {{count}}', { count: data.length })}</p>
      <ul className="flex max-h-56 flex-col gap-1.5 overflow-y-auto">
        {data.map((entry) => (
          <li key={entry.usecase} className="flex flex-col gap-0.5 rounded-lg px-2 py-1.5 text-xs" data-testid="provider-impact-row" data-usecase={entry.usecase} data-coverage={entry.coverage} style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)' }}>
            <span className="font-medium">{name(entry.usecase)} <span className="font-mono font-normal" style={muted}>{entry.usecase}</span></span>
            <span className="flex min-w-0 flex-wrap items-center gap-1">
              {entry.before ? <ExactModelLabel exactModel={entry.before} /> : <span style={muted}>—</span>}
              <ArrowRight size={12} style={muted} />
              {entry.coverage === 'uncovered'
                ? <span style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.providers.impactUncovered', 'no model (uncovered)')}</span>
                : entry.after
                  ? <>{entry.coverage === 'fallback_llm' && <span style={{ color: 'var(--cp-warning)' }}>{t('aiCenter.usecases.status.fallbackLlm', 'Fallback LLM')}:</span>}<ExactModelLabel exactModel={entry.after} /></>
                  : <span style={muted}>—</span>}
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}
