import { X } from 'lucide-react'
import { useCallback } from 'react'
import { useI18n } from '../../i18n/provider'
import { formatTimeAgo } from './datamodel/format'
import type { FilterRule, ReadingQuery } from './datamodel/types'
import type { HomeStationStore } from './store/types'
import { useHsNav } from './navContext'
import { PublishComposer } from './publish/PublishComposer'
import { useHomeStationStore, useNow, useStoreSelector } from './store/context'
import type { ReadingMode } from './types'
import { ColumnBody, PageHeader } from './ui/primitives'

const selectRules = (store: HomeStationStore) => store.peekFilterRules()
const selectMutes = (store: HomeStationStore) => store.peekMuteRules()
const selectSync = (store: HomeStationStore) => store.peekSyncStatus()
const selectTopics = (store: HomeStationStore) => store.peekTopics()

function Section({ title, note, children, testId }: { title: string; note?: string; children: React.ReactNode; testId?: string }) {
  return (
    <section className="px-6 py-5" style={{ borderBottom: '1px solid var(--hs-line)' }} data-testid={testId}>
      <h3 className="hs-section-title">{title}</h3>
      {note ? <p className="hs-section-note">{note}</p> : null}
      <div className="mt-3">{children}</div>
    </section>
  )
}

/** The second column next to the feed: what shapes this view, how fresh it is, and a quick way to post. */
export function InfoPanel({ query, readingMode, onQueryChange, onClose }: { query: ReadingQuery; readingMode: ReadingMode; onQueryChange: (patch: Partial<ReadingQuery>) => void; onClose: () => void }) {
  const { t } = useI18n()
  const now = useNow()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const rules = useStoreSelector(selectRules)
  const mutes = useStoreSelector(selectMutes)
  const sync = useStoreSelector(selectSync)
  const topics = useStoreSelector(selectTopics)
  const selectSummary = useCallback((current: HomeStationStore) => current.peekReadingSummary(query), [query])
  const summary = useStoreSelector(selectSummary)
  const trending = [...topics].sort((left, right) => right.recentCount - left.recentCount).slice(0, 5)
  const activeTopic = topics.find(topic => topic.id === query.topicId)
  const ruleLabel = (rule: FilterRule) => rule.conditions.map(condition => ({ ai_full: t('homestation.filters.condAiFull', 'Fully AI-generated'), ai_assisted: t('homestation.filters.condAiAssisted', 'AI-assisted'), low_quality: t('homestation.filters.condLowQuality', 'Low quality') })[condition]).join(' + ')
  const viewParts = [
    t(`homestation.filter.${query.filter}`, query.filter),
    t(`homestation.mode.${readingMode}`, readingMode),
    activeTopic ? t('homestation.topics.active', 'Topic: {{name}}', { name: activeTopic.name }) : null,
    query.search.trim() ? `“${query.search.trim()}”` : null,
  ].filter(Boolean)

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-info-panel">
      <PageHeader
        title={t('homestation.info.title', 'Context')}
        actions={<button type="button" className="hs-icon-btn" onClick={onClose} aria-label={t('homestation.info.hide', 'Hide panel')} title={t('homestation.info.hide', 'Hide panel')}><X size={18} /></button>}
      />
      <ColumnBody>
        <Section title={t('homestation.info.currentView', 'Current view')} note={t('homestation.info.modeNote', 'Reading modes only change presentation; the view’s filter, topic and search stay.')}>
          <p className="text-[15px] leading-[22px]">{viewParts.join(' · ')}</p>
        </Section>

        <Section title={t('homestation.info.exclusions', 'Exclusions in effect')} testId="hs-info-rules">
          <ul className="space-y-2 text-[14px] leading-5">
            {rules.map(rule => (
              <li key={rule.id}>
                <label className="flex items-start gap-3">
                  <input
                    type="checkbox"
                    className="mt-[3px] h-4 w-4 flex-shrink-0"
                    checked={rule.enabled}
                    aria-label={ruleLabel(rule)}
                    data-testid={`hs-info-rule-${rule.id}`}
                    onChange={() => void store.setFilterRule({ ...rule, enabled: !rule.enabled })}
                  />
                  <span className="min-w-0 flex-1">
                    {t('homestation.info.ruleLine', 'Hide: {{conditions}}', { conditions: ruleLabel(rule) })}
                    <span className="block text-[13px]" style={{ color: 'var(--cp-muted)' }}>{rule.acceptInferred ? t('homestation.info.ruleInferred', 'Includes inferences ≥ {{n}}', { n: rule.minConfidence.toFixed(2) }) : t('homestation.info.ruleDeclared', 'Declared tags only')}</span>
                  </span>
                </label>
              </li>
            ))}
            {mutes.length ? <li className="text-[13px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.mutes', 'Not showing: {{names}}', { names: mutes.map(rule => rule.name).join(', ') })}</li> : null}
          </ul>
          <div className="hs-meta mt-3">
            <span className="tabular-nums" data-testid="hs-info-hidden">{t('homestation.filters.hiddenCount', '{{n}} hidden by your filters', { n: summary.hiddenByRules })}</span>
            {summary.hiddenByMute ? <span className="tabular-nums">{t('homestation.filters.mutedShort', '{{n}} from hidden people', { n: summary.hiddenByMute })}</span> : null}
            {summary.hiddenByRules > 0 || query.showFiltered ? (
              <button type="button" className="hs-link" onClick={() => onQueryChange({ showFiltered: !query.showFiltered })}>{query.showFiltered ? t('homestation.filters.hideAgain', 'Hide them again') : t('homestation.filters.show', 'Show them')}</button>
            ) : null}
            <button type="button" className="hs-link" onClick={() => nav.navigate({ name: 'prefs' }, { reset: true })}>{t('homestation.info.editRules', 'Edit rules')}</button>
          </div>
        </Section>

        <Section title={t('homestation.info.sync', 'Sync status')} testId="hs-info-sync">
          <dl className="grid grid-cols-[1fr_auto] gap-x-4 gap-y-2 text-[14px] leading-5 tabular-nums">
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.candidates', 'Followed, not in feed')}</dt>
            <dd className="text-right"><button type="button" className="hs-link" onClick={() => nav.navigate({ name: 'candidates' })}>{sync.candidates}</button></dd>
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.preparing', 'Preparing resources')}</dt>
            <dd className="text-right">{sync.preparing}</dd>
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.lastFetch', 'Last fetch')}</dt>
            <dd className="text-right">{t('homestation.info.ago', '{{time}} ago', { time: formatTimeAgo(t, sync.lastFetchAt, now) })}</dd>
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.failing', 'Sources with errors')}</dt>
            <dd className="text-right"><button type="button" className="hs-link" onClick={() => nav.navigate({ name: 'sources' }, { reset: true })}>{sync.failingSources}/{sync.sources}</button></dd>
          </dl>
        </Section>

        <Section title={t('homestation.info.trending', 'Trending topics')} note={t('homestation.topics.scope', 'Counts: your feed · last 7 days')}>
          <ol className="-mx-3" data-testid="hs-trending-scope">
            {trending.map((topic, index) => (
              <li key={topic.id}>
                <button type="button" className="hs-nav-item tabular-nums" aria-current={query.topicId === topic.id ? 'page' : undefined} onClick={() => { onQueryChange({ topicId: topic.id }); nav.navigate({ name: 'feed' }, { reset: true }) }}>
                  <span className="w-4 text-right text-[13px]" style={{ color: 'var(--cp-muted)' }}>{index + 1}</span>
                  <span className="min-w-0 flex-1 truncate">{topic.name}</span>
                  <span className="text-[13px]" style={{ color: 'var(--cp-muted)' }}>{topic.recentCount}</span>
                </button>
              </li>
            ))}
          </ol>
        </Section>

        <section className="px-6 py-5">
          <h3 className="hs-section-title">{t('homestation.info.quickPublish', 'Quick publish')}</h3>
          <div className="mt-3"><PublishComposer variant="compact" /></div>
        </section>
      </ColumnBody>
    </div>
  )
}
