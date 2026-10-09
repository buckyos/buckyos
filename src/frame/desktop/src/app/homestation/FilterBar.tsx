import { ChevronDown, FileText, Globe, Hash, Image, LayoutGrid, MonitorPlay, Newspaper, Users, Video } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { FeedFilter, ReadingQuery, TopicView } from './datamodel/types'
import type { ReadingMode } from './types'
import { Menu } from './ui/Popover'

const filterOptions: { id: FeedFilter; labelKey: string; fallback: string; icon: React.ReactNode }[] = [
  { id: 'all', labelKey: 'homestation.filter.all', fallback: 'All', icon: <Globe size={14} /> },
  { id: 'following', labelKey: 'homestation.filter.following', fallback: 'Following', icon: <Users size={14} /> },
  { id: 'news', labelKey: 'homestation.filter.news', fallback: 'News', icon: <Newspaper size={14} /> },
  { id: 'images', labelKey: 'homestation.filter.images', fallback: 'Images', icon: <Image size={14} /> },
  { id: 'videos', labelKey: 'homestation.filter.videos', fallback: 'Videos', icon: <Video size={14} /> },
  { id: 'longform', labelKey: 'homestation.filter.longform', fallback: 'Long-form', icon: <FileText size={14} /> },
]

const readingModeOptions: { id: ReadingMode; labelKey: string; fallback: string; icon: React.ReactNode }[] = [
  { id: 'standard', labelKey: 'homestation.mode.standard', fallback: 'Standard', icon: <LayoutGrid size={14} /> },
  { id: 'image', labelKey: 'homestation.mode.image', fallback: 'Images first', icon: <Image size={14} /> },
  { id: 'longform', labelKey: 'homestation.mode.longform', fallback: 'Long-form', icon: <FileText size={14} /> },
  { id: 'immersive', labelKey: 'homestation.mode.immersive', fallback: 'Immersive', icon: <MonitorPlay size={14} /> },
]

interface FilterBarProps {
  query: ReadingQuery
  readingMode: ReadingMode
  topics: TopicView[]
  onQueryChange: (patch: Partial<ReadingQuery>) => void
  onReadingModeChange: (mode: ReadingMode) => void
  isMobile?: boolean
}

export function FilterBar({ query, readingMode, topics, onQueryChange, onReadingModeChange, isMobile = false }: FilterBarProps) {
  const { t } = useI18n()
  const subscribed = topics.filter(topic => topic.subscribed)
  return (
    <div className="hs-scroll-x flex items-center gap-2 overflow-x-auto px-4 py-2" role="toolbar" aria-label={t('homestation.filter.label', 'Feed view')}>
      {isMobile ? <ReadingModeDropdown readingMode={readingMode} onChange={onReadingModeChange} /> : null}
      {filterOptions.map(option => (
        <button
          key={option.id}
          type="button"
          className="hs-chip"
          aria-pressed={query.filter === option.id}
          data-testid={`hs-filter-${option.id}`}
          onClick={() => onQueryChange({ filter: option.id })}
        >
          {option.icon}
          {t(option.labelKey, option.fallback)}
        </button>
      ))}
      {!isMobile ? (
        <>
          <span className="mx-1 h-5 w-px flex-shrink-0" style={{ background: 'var(--cp-border)' }} />
          {subscribed.map(topic => (
            <button
              key={topic.id}
              type="button"
              className="hs-chip"
              aria-pressed={query.topicId === topic.id}
              data-testid={`hs-topic-${topic.id}`}
              onClick={() => onQueryChange({ topicId: query.topicId === topic.id ? null : topic.id })}
            >
              <Hash size={12} />
              {topic.name}
            </button>
          ))}
          <span className="mx-1 h-5 w-px flex-shrink-0" style={{ background: 'var(--cp-border)' }} />
          <div className="flex flex-shrink-0 items-center gap-0.5 rounded-full p-0.5" style={{ background: 'var(--hs-chip-bg)' }} role="radiogroup" aria-label={t('homestation.mode.label', 'Reading mode')}>
            {readingModeOptions.map(mode => (
              <button
                key={mode.id}
                type="button"
                role="radio"
                aria-checked={readingMode === mode.id}
                aria-label={t(mode.labelKey, mode.fallback)}
                title={t(mode.labelKey, mode.fallback)}
                className="flex h-7 w-7 items-center justify-center rounded-full transition-colors"
                style={{ background: readingMode === mode.id ? 'var(--hs-selected-bg)' : 'transparent', color: readingMode === mode.id ? 'var(--cp-accent)' : 'var(--cp-muted)' }}
                data-testid={`hs-mode-${mode.id}`}
                onClick={() => onReadingModeChange(mode.id)}
              >
                {mode.icon}
              </button>
            ))}
          </div>
        </>
      ) : null}
    </div>
  )
}

function ReadingModeDropdown({ readingMode, onChange }: { readingMode: ReadingMode; onChange: (mode: ReadingMode) => void }) {
  const { t } = useI18n()
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const close = useCallback(() => setAnchor(null), [])
  const active = readingModeOptions.find(mode => mode.id === readingMode) ?? readingModeOptions[0]
  return (
    <>
      <button
        type="button"
        className="hs-chip"
        aria-haspopup="menu"
        aria-expanded={!!anchor}
        aria-label={t('homestation.mode.label', 'Reading mode')}
        data-testid="hs-mode-menu"
        style={{ background: 'var(--hs-pressed-bg)', color: 'var(--cp-accent)' }}
        onClick={event => setAnchor(anchor ? null : event.currentTarget)}
      >
        {active.icon}
        {t(active.labelKey, active.fallback)}
        <ChevronDown size={12} />
      </button>
      <Menu
        anchor={anchor}
        open={!!anchor}
        onClose={close}
        label={t('homestation.mode.label', 'Reading mode')}
        items={readingModeOptions.map(mode => ({ id: mode.id, label: t(mode.labelKey, mode.fallback), icon: mode.icon, onSelect: () => onChange(mode.id) }))}
      />
    </>
  )
}
