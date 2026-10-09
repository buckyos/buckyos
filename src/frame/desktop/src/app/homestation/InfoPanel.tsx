import { Activity, EyeOff, Hash, PenSquare, SlidersHorizontal, TrendingUp, X } from 'lucide-react'
import { useCallback } from 'react'
import { useI18n } from '../../i18n/provider'
import { formatTimeAgo } from './datamodel/format'
import type { FilterRule, ReadingQuery } from './datamodel/types'
import type { HomeStationStore } from './store/types'
import { useHsNav } from './navContext'
import { PublishComposer } from './publish/PublishComposer'
import { useHomeStationStore, useNow, useStoreSelector } from './store/context'
import type { ReadingMode } from './types'

const selectRules = (store: HomeStationStore) => store.peekFilterRules()
const selectMutes = (store: HomeStationStore) => store.peekMuteRules()
const selectSync = (store: HomeStationStore) => store.peekSyncStatus()
const selectTopics = (store: HomeStationStore) => store.peekTopics()

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

  return (
    <aside className="desktop-scrollbar flex h-full flex-col overflow-y-auto" aria-label={t('homestation.info.title', 'Context')} data-testid="hs-info-panel">
      <div className="flex items-center justify-between px-4 py-3">
        <span className="text-sm font-semibold">{t('homestation.info.title', 'Context')}</span>
        <button type="button" className="hs-icon-btn" onClick={onClose} aria-label={t('homestation.info.hide', 'Hide panel')}><X size={14} /></button>
      </div>

      <section className="px-4 py-2">
        <h3 className="hs-section-title mb-2 flex items-center gap-1.5"><SlidersHorizontal size={12} />{t('homestation.info.currentView', 'Current view')}</h3>
        <div className="flex flex-wrap gap-1.5">
          <span className="hs-badge is-accent">{t(`homestation.filter.${query.filter}`, query.filter)}</span>
          <span className="hs-badge">{t(`homestation.mode.${readingMode}`, readingMode)}</span>
          {activeTopic ? <span className="hs-badge is-warning"># {activeTopic.name}</span> : null}
          {query.search.trim() ? <span className="hs-badge">“{query.search.trim()}”</span> : null}
        </div>
        <p className="mt-1.5 text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.modeNote', 'Reading modes only change presentation; the view’s filter, topic and search stay.')}</p>
      </section>

      <section className="px-4 py-3" data-testid="hs-info-rules">
        <h3 className="hs-section-title mb-2 flex items-center gap-1.5"><EyeOff size={12} />{t('homestation.info.exclusions', 'Exclusions in effect')}</h3>
        <ul className="space-y-1.5 text-xs">
          {rules.map(rule => (
            <li key={rule.id} className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={rule.enabled}
                aria-label={ruleLabel(rule)}
                data-testid={`hs-info-rule-${rule.id}`}
                onChange={() => void store.setFilterRule({ ...rule, enabled: !rule.enabled })}
              />
              <span className="flex-1">
                {t('homestation.info.ruleLine', 'Hide: {{conditions}}', { conditions: ruleLabel(rule) })}
                <span style={{ color: 'var(--cp-muted)' }}>{rule.acceptInferred ? t('homestation.info.ruleInferred', ' · inferences ≥ {{n}}', { n: rule.minConfidence.toFixed(2) }) : t('homestation.info.ruleDeclared', ' · declared only')}</span>
              </span>
            </li>
          ))}
          {mutes.length ? <li style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.mutes', 'Not showing: {{names}}', { names: mutes.map(rule => rule.name).join(', ') })}</li> : null}
        </ul>
        <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[11px]">
          <span className="hs-badge is-warning" data-testid="hs-info-hidden">{t('homestation.filters.hiddenCount', '{{n}} hidden by your filters', { n: summary.hiddenByRules })}</span>
          {summary.hiddenByMute ? <span className="hs-badge">{t('homestation.filters.mutedShort', '{{n}} from hidden people', { n: summary.hiddenByMute })}</span> : null}
          {summary.hiddenByRules > 0 || query.showFiltered ? (
            <button type="button" className="hs-badge is-accent" onClick={() => onQueryChange({ showFiltered: !query.showFiltered })}>{query.showFiltered ? t('homestation.filters.hideAgain', 'Hide them again') : t('homestation.filters.show', 'Show them')}</button>
          ) : null}
          <button type="button" className="hs-badge" onClick={() => nav.navigate({ name: 'prefs' }, { reset: true })}>{t('homestation.info.editRules', 'Edit rules')}</button>
        </div>
      </section>

      <section className="px-4 py-3" data-testid="hs-info-sync">
        <h3 className="hs-section-title mb-2 flex items-center gap-1.5"><Activity size={12} />{t('homestation.info.sync', 'Sync status')}</h3>
        <dl className="grid grid-cols-[1fr_auto] gap-y-1 text-xs">
          <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.candidates', 'Followed, not in feed')}</dt>
          <dd><button type="button" className="underline" onClick={() => nav.navigate({ name: 'candidates' })}>{sync.candidates}</button></dd>
          <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.preparing', 'Preparing resources')}</dt>
          <dd>{sync.preparing}</dd>
          <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.lastFetch', 'Last fetch')}</dt>
          <dd>{t('homestation.info.ago', '{{time}} ago', { time: formatTimeAgo(t, sync.lastFetchAt, now) })}</dd>
          <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.info.failing', 'Sources with errors')}</dt>
          <dd><button type="button" className="underline" onClick={() => nav.navigate({ name: 'sources' }, { reset: true })}>{sync.failingSources}/{sync.sources}</button></dd>
        </dl>
      </section>

      <section className="px-4 py-3">
        <h3 className="hs-section-title flex items-center gap-1.5"><TrendingUp size={12} />{t('homestation.info.trending', 'Trending topics')}</h3>
        <p className="mb-2 text-[10px]" style={{ color: 'var(--cp-muted)' }} data-testid="hs-trending-scope">{t('homestation.topics.scope', 'Counts: your feed · last 7 days')}</p>
        <div className="flex flex-col gap-0.5">
          {trending.map((topic, index) => (
            <button key={topic.id} type="button" className="hs-nav-item py-1.5" aria-current={query.topicId === topic.id ? 'page' : undefined} onClick={() => { onQueryChange({ topicId: topic.id }); nav.navigate({ name: 'feed' }, { reset: true }) }}>
              <span className="w-4 text-center text-[11px] font-bold" style={{ color: 'var(--cp-muted)' }}>{index + 1}</span>
              <Hash size={12} style={{ color: 'var(--cp-accent)' }} />
              <span className="flex-1 truncate text-xs">{topic.name}</span>
              <span className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>{topic.recentCount}</span>
            </button>
          ))}
        </div>
      </section>

      <section className="px-4 py-3">
        <h3 className="hs-section-title mb-2 flex items-center gap-1.5"><PenSquare size={12} />{t('homestation.info.quickPublish', 'Quick publish')}</h3>
        <PublishComposer variant="compact" />
      </section>
    </aside>
  )
}
