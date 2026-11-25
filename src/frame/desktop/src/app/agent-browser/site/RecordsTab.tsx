/* What is recorded about the owner's reading, and the owner's say over it: the reading records of the personal site
 * (what its pages report; summed up in `_data/reading.json`) and the pages the owner looked at in the Agent's browser
 * (`browser.visits`). Each can be looked at, switched off and cleared here. Both are for the Agent that maintains the
 * site, to choose what to collect. */

import { useEffect, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { formatMoment } from '../reflow/useReflow'
import { getReflowSource } from '../reflow/source'
import { displayUrl } from '../shell/address'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser } from '../store/context'
import { clearReading, readReading, type ReadingSummary, type ReadingTotal } from './data'
import { clearVisits, readVisits, setVisitsEnabled, type Visit } from './manage'
import { useSite, useSiteController } from './useSite'

const TOP = 5
const VISITS = 8

/** How long something was read, in the unit that fits. */
function duration(ms: number, locale: string): string {
  const minutes = Math.round(ms / 60_000)
  const [value, unit] = minutes >= 90 ? [Math.round(minutes / 6) / 10, 'hour'] : minutes >= 1 ? [minutes, 'minute'] : [Math.round(ms / 1000), 'second']
  return new Intl.NumberFormat(locale, { style: 'unit', unit, unitDisplay: 'short', maximumFractionDigits: 1 }).format(value)
}

function Totals({ title, totals, name, testId }: { title: string; totals: ReadingTotal[]; name: (id: string) => string; testId: string }) {
  const { t, locale } = useI18n()
  if (totals.length === 0) return null
  return (
    <>
      <h4 className="ab-records-sub">{title}</h4>
      <ul className="ab-records-list" data-testid={testId}>
        {totals.slice(0, TOP).map((total) => (
          <li key={total.name}>
            <span className="ab-records-name">{name(total.name)}</span>
            <span className="ab-records-value">{t('agentBrowser.manage.reading.total', '{{time}} · opened {{count}}', { time: duration(total.dwell_ms, locale), count: total.opens })}</span>
          </li>
        ))}
      </ul>
    </>
  )
}

export function RecordsTab() {
  const controller = useSiteController()
  const { t, locale } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const site = useSite((snapshot) => snapshot.site)
  const [reading, setReading] = useState<ReadingSummary | null | undefined>(undefined)
  const [visits, setVisits] = useState<{ enabled: boolean; visits: Visit[] } | null | undefined>(undefined)
  const [confirming, setConfirming] = useState<'reading' | 'visits' | null>(null)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const hasSite = !!site
  /** How often the records were changed here: each change is followed by reading them again. */
  const [changes, setChanges] = useState(0)
  useEffect(() => {
    let alive = true
    const summary = hasSite ? getReflowSource().then((source) => source.library()).then(readReading).catch(() => null) : Promise.resolve(null)
    const pages = agent ? readVisits(agent).catch(() => null) : Promise.resolve(null)
    void Promise.all([summary, pages]).then(([nextReading, nextVisits]) => {
      if (!alive) return
      setReading(nextReading)
      setVisits(nextVisits)
      setBusy(false)
    })
    return () => { alive = false }
  }, [agent, hasSite, changes])
  const name = agent?.name ?? ''
  /** One change at a time; what it left is read again. */
  const run = async (change: () => Promise<unknown>) => {
    setBusy(true)
    setFailed(false)
    try {
      await change()
    } catch {
      setFailed(true)
    }
    setChanges((count) => count + 1)
  }
  const clear = (what: 'reading' | 'visits') => {
    setConfirming(null)
    void run(async () => {
      if (what === 'visits') { if (agent) await clearVisits(agent) }
      else await clearReading(await (await getReflowSource()).library())
    })
  }
  return (
    <div className="ab-manage-body" data-testid="ab-manage-records">
      <p className="ab-records-intro" data-testid="ab-records-intro">{t('agentBrowser.manage.records.intro', 'Two things are recorded so that the Agent maintaining your reading site knows what you read. Nobody else uses them. Each is kept for 90 days, and you can look at it, switch it off and clear it here.')}</p>
      {failed ? <p className="ab-manage-note" role="alert" data-testid="ab-records-failed">{t('agentBrowser.manage.records.failed', 'That did not work. Try again.')}</p> : null}

      <section className="ab-records" data-testid="ab-records-reading">
        <button type="button" role="switch" aria-checked={site?.reading === true} disabled={!site || busy} className="ab-list-item" data-testid="ab-reading-switch" onClick={() => void run(() => controller.setReading(!site?.reading))}>
          <span className="ab-list-text">
            <span className="ab-list-title">{t('agentBrowser.manage.reading.title', 'Reading records')}</span>
            <span className="ab-list-detail">{t('agentBrowser.manage.reading.hint', 'For each article of your reading site: when you opened it, how long it stayed in front of you, and how far you read.')}</span>
          </span>
          <span className="ab-switch" aria-hidden />
        </button>
        {reading ? (
          <div data-testid="ab-reading-summary">
            <p className="ab-records-meta">{t('agentBrowser.manage.reading.since', 'Articles read since {{time}}: {{count}}', { time: formatMoment(reading.since, locale), count: reading.articles })}</p>
            <Totals title={t('agentBrowser.manage.reading.sources', 'Most read websites')} totals={reading.sources} name={(id) => site?.sources.find((source) => source.id === id)?.title ?? id} testId="ab-reading-sources" />
            <Totals title={t('agentBrowser.manage.reading.tags', 'Most read topics')} totals={reading.tags} name={(tag) => tag} testId="ab-reading-tags" />
          </div>
        ) : reading === null ? <p className="ab-records-meta" data-testid="ab-reading-empty">{site ? t('agentBrowser.manage.reading.empty', 'Nothing is recorded yet.') : t('agentBrowser.manage.reading.noSite', 'There is no reading site yet.')}</p> : null}
        <div className="ab-manage-foot"><button type="button" className="ab-button" disabled={!site || busy} data-testid="ab-reading-clear" onClick={() => setConfirming('reading')}>{t('agentBrowser.manage.reading.clear', 'Clear reading records')}</button></div>
      </section>

      <section className="ab-records" data-testid="ab-records-visits">
        <button type="button" role="switch" aria-checked={visits?.enabled === true} disabled={!visits || busy} className="ab-list-item" data-testid="ab-visits-switch" onClick={() => { if (agent && visits) void run(() => setVisitsEnabled(agent, !visits.enabled)) }}>
          <span className="ab-list-text">
            <span className="ab-list-title">{t('agentBrowser.manage.visits.title', 'Browsing records')}</span>
            <span className="ab-list-detail">{t('agentBrowser.manage.visits.hint', 'The pages you look at in {{name}}’s browser while you operate it: address (without what follows “?” or “#”), title and how long. Not while you enter a password, and not while {{name}} operates.', { name })}</span>
          </span>
          <span className="ab-switch" aria-hidden />
        </button>
        {visits === null ? <p className="ab-records-meta" data-testid="ab-visits-unavailable">{t('agentBrowser.manage.visits.unavailable', '{{name}} did not answer, so its browsing records cannot be shown.', { name })}</p>
          : visits && visits.visits.length === 0 ? <p className="ab-records-meta" data-testid="ab-visits-empty">{t('agentBrowser.manage.visits.empty', 'Nothing is recorded yet.')}</p>
          : visits ? (
            <>
              <p className="ab-records-meta">{t('agentBrowser.manage.visits.count', 'Pages recorded: {{count}}', { count: visits.visits.length })}</p>
              <ul className="ab-records-list" data-testid="ab-visits-list">
                {visits.visits.slice(0, VISITS).map((visit) => (
                  <li key={visit.url}>
                    <span className="ab-records-name">{visit.title || displayUrl(visit.url)}<span className="ab-records-url">{displayUrl(visit.url)}</span></span>
                    <span className="ab-records-value">{duration(visit.dwell_ms, locale)}</span>
                  </li>
                ))}
              </ul>
            </>
          ) : null}
        <div className="ab-manage-foot"><button type="button" className="ab-button" disabled={!visits || busy} data-testid="ab-visits-clear" onClick={() => setConfirming('visits')}>{t('agentBrowser.manage.visits.clear', 'Clear browsing records')}</button></div>
      </section>

      {confirming ? (
        <Sheet
          testId="ab-records-confirm"
          title={confirming === 'reading' ? t('agentBrowser.manage.reading.clearTitle', 'Clear reading records?') : t('agentBrowser.manage.visits.clearTitle', 'Clear browsing records?')}
          onClose={() => setConfirming(null)}
          footer={<>
            <button type="button" className="ab-button" data-testid="ab-records-cancel" onClick={() => setConfirming(null)}>{t('common.cancel')}</button>
            <button type="button" className="ab-button ab-button-danger" data-testid="ab-records-accept" onClick={() => clear(confirming)}>{t('agentBrowser.clear.accept', 'Clear')}</button>
          </>}
        >
          <p className="ab-dialog-message">
            {confirming === 'reading'
              ? t('agentBrowser.manage.reading.clearBody', 'Everything recorded about what you read in your reading site is deleted. {{name}} then no longer knows which websites and topics you read most. New records are kept while the switch is on.', { name })
              : t('agentBrowser.manage.visits.clearBody', 'The list of pages you looked at in {{name}}’s browser is deleted. New pages are recorded while the switch is on.', { name })}
          </p>
        </Sheet>
      ) : null}
    </div>
  )
}
