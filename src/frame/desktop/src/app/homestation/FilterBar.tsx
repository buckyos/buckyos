import { FileText, Image, LayoutList, MonitorPlay } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { FeedFilter, ReadingQuery, TopicView } from './datamodel/types'
import type { ReadingMode } from './types'
import { Menu } from './ui/Popover'

const filterOptions: { id: FeedFilter; labelKey: string; fallback: string }[] = [
  { id: 'all', labelKey: 'homestation.filter.all', fallback: 'All' },
  { id: 'following', labelKey: 'homestation.filter.following', fallback: 'Following' },
  { id: 'news', labelKey: 'homestation.filter.news', fallback: 'News' },
  { id: 'images', labelKey: 'homestation.filter.images', fallback: 'Images' },
  { id: 'videos', labelKey: 'homestation.filter.videos', fallback: 'Videos' },
  { id: 'longform', labelKey: 'homestation.filter.longform', fallback: 'Long-form' },
]

const readingModeOptions: { id: ReadingMode; labelKey: string; fallback: string; icon: (size: number) => React.ReactNode }[] = [
  { id: 'standard', labelKey: 'homestation.mode.standard', fallback: 'Standard', icon: size => <LayoutList size={size} /> },
  { id: 'image', labelKey: 'homestation.mode.image', fallback: 'Images first', icon: size => <Image size={size} /> },
  { id: 'longform', labelKey: 'homestation.mode.longform', fallback: 'Long-form', icon: size => <FileText size={size} /> },
  { id: 'immersive', labelKey: 'homestation.mode.immersive', fallback: 'Immersive', icon: size => <MonitorPlay size={size} /> },
]

interface FilterBarProps {
  query: ReadingQuery
  readingMode: ReadingMode
  topics: TopicView[]
  onQueryChange: (patch: Partial<ReadingQuery>) => void
  onReadingModeChange: (mode: ReadingMode) => void
  isMobile?: boolean
}

/** The view switch on top of the reading column: what kind of item, which topic, and how to read it. */
export function FilterBar({ query, readingMode, topics, onQueryChange, onReadingModeChange, isMobile = false }: FilterBarProps) {
  const { t } = useI18n()
  const ordered = [...topics.filter(topic => topic.subscribed), ...topics.filter(topic => !topic.subscribed)]
  return (
    <>
      <div className="hs-tabs items-stretch" role="toolbar" aria-label={t('homestation.filter.label', 'Feed view')}>
        <div className={isMobile ? 'hs-scroll-x flex min-w-0 flex-1 overflow-x-auto px-1' : 'hs-scroll-x flex min-w-0 flex-1 overflow-x-auto px-3'}>
          {filterOptions.map(option => (
            <button
              key={option.id}
              type="button"
              className="hs-tab"
              aria-pressed={query.filter === option.id}
              data-testid={`hs-filter-${option.id}`}
              onClick={() => onQueryChange({ filter: option.id })}
            >
              {t(option.labelKey, option.fallback)}
            </button>
          ))}
        </div>
        {!isMobile ? (
          <div className="flex flex-shrink-0 items-center gap-1 pl-2 pr-4" role="radiogroup" aria-label={t('homestation.mode.label', 'Reading mode')}>
            {readingModeOptions.map(mode => (
              <button
                key={mode.id}
                type="button"
                role="radio"
                aria-checked={readingMode === mode.id}
                aria-label={t(mode.labelKey, mode.fallback)}
                title={t(mode.labelKey, mode.fallback)}
                className="hs-icon-btn"
                style={{ minWidth: 32, height: 32, padding: 0, ...(readingMode === mode.id ? { color: 'var(--cp-text)', background: 'var(--hs-press-bg)' } : null) }}
                data-testid={`hs-mode-${mode.id}`}
                onClick={() => onReadingModeChange(mode.id)}
              >
                {mode.icon(18)}
              </button>
            ))}
          </div>
        ) : null}
      </div>
      {ordered.length ? (
        <div
          className={isMobile ? 'hs-scroll-x flex gap-2 overflow-x-auto px-4 py-3' : 'hs-scroll-x flex gap-2 overflow-x-auto px-6 py-3'}
          role="group"
          aria-label={t('homestation.topics.title', 'Topics')}
          title={t('homestation.topics.scope', 'Counts: your feed · last 7 days')}
          style={{ borderBottom: '1px solid var(--hs-line)' }}
        >
          {ordered.map(topic => (
            <button
              key={topic.id}
              type="button"
              className="hs-chip"
              aria-pressed={query.topicId === topic.id}
              data-testid={`hs-topic-${topic.id}`}
              onClick={() => onQueryChange({ topicId: query.topicId === topic.id ? null : topic.id })}
            >
              {topic.name}
              {topic.recentCount ? <span className="hs-chip-count">{topic.recentCount}</span> : null}
            </button>
          ))}
        </div>
      ) : null}
    </>
  )
}

/** Phone: the reading mode sits in the top bar as one button with a menu. */
export function ReadingModeMenu({ readingMode, onChange }: { readingMode: ReadingMode; onChange: (mode: ReadingMode) => void }) {
  const { t } = useI18n()
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const close = useCallback(() => setAnchor(null), [])
  const active = readingModeOptions.find(mode => mode.id === readingMode) ?? readingModeOptions[0]
  return (
    <>
      <button
        type="button"
        className="hs-icon-btn"
        aria-haspopup="menu"
        aria-expanded={!!anchor}
        aria-label={`${t('homestation.mode.label', 'Reading mode')}: ${t(active.labelKey, active.fallback)}`}
        data-testid="hs-mode-menu"
        onClick={event => setAnchor(anchor ? null : event.currentTarget)}
      >
        {active.icon(20)}
      </button>
      <Menu
        anchor={anchor}
        open={!!anchor}
        onClose={close}
        label={t('homestation.mode.label', 'Reading mode')}
        items={readingModeOptions.map(mode => ({ id: mode.id, label: t(mode.labelKey, mode.fallback), icon: mode.icon(16), current: mode.id === readingMode, onSelect: () => onChange(mode.id) }))}
      />
    </>
  )
}
