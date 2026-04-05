import { useState, useRef, useEffect } from 'react'
import {
  ChevronDown,
  FileText,
  Globe,
  Image,
  LayoutGrid,
  MonitorPlay,
  Newspaper,
  Users,
  Video,
} from 'lucide-react'
import type { FeedFilter, ReadingMode, Topic } from './types'

const filterOptions: { id: FeedFilter; labelKey: string; fallback: string; icon: React.ReactNode }[] = [
  { id: 'all', labelKey: 'homestation.filterAll', fallback: 'All', icon: <Globe size={14} /> },
  { id: 'following', labelKey: 'homestation.filterFollowing', fallback: 'Following', icon: <Users size={14} /> },
  { id: 'news', labelKey: 'homestation.filterNews', fallback: 'News', icon: <Newspaper size={14} /> },
  { id: 'images', labelKey: 'homestation.filterImages', fallback: 'Images', icon: <Image size={14} /> },
  { id: 'videos', labelKey: 'homestation.filterVideos', fallback: 'Videos', icon: <Video size={14} /> },
  { id: 'longform', labelKey: 'homestation.filterLongform', fallback: 'Long-form', icon: <FileText size={14} /> },
]

const readingModeOptions: { id: ReadingMode; labelKey: string; fallback: string; icon: React.ReactNode }[] = [
  { id: 'standard', labelKey: 'homestation.modeStandard', fallback: 'Standard', icon: <LayoutGrid size={14} /> },
  { id: 'image', labelKey: 'homestation.modeImage', fallback: 'Image', icon: <Image size={14} /> },
  { id: 'longform', labelKey: 'homestation.modeLongform', fallback: 'Longform', icon: <FileText size={14} /> },
  { id: 'immersive-video', labelKey: 'homestation.modeVideo', fallback: 'Video', icon: <MonitorPlay size={14} /> },
]

interface FilterBarProps {
  activeFilter: FeedFilter
  activeTopicId: string | null
  readingMode: ReadingMode
  topics: Topic[]
  t: (key: string, fallback: string) => string
  onFilterChange: (filter: FeedFilter) => void
  onTopicSelect: (topicId: string | null) => void
  onReadingModeChange: (mode: ReadingMode) => void
  isMobile?: boolean
}

export function FilterBar({
  activeFilter,
  activeTopicId,
  readingMode,
  topics,
  t,
  onFilterChange,
  onTopicSelect,
  onReadingModeChange,
  isMobile = false,
}: FilterBarProps) {
  const subscribedTopics = topics.filter((tp) => tp.isSubscribed)

  return (
    <div
      className="flex items-center gap-2 overflow-x-auto px-4 py-2"
      style={{ scrollbarWidth: 'none' }}
    >
      {/* Mobile: Reading mode dropdown at the start */}
      {isMobile ? (
        <ReadingModeDropdown
          readingMode={readingMode}
          t={t}
          onReadingModeChange={onReadingModeChange}
        />
      ) : null}

      {/* Filter chips */}
      {filterOptions.map((opt) => (
        <button
          key={opt.id}
          type="button"
          onClick={() => {
            onFilterChange(opt.id)
            onTopicSelect(null)
          }}
          className="flex flex-shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-colors"
          style={{
            background: activeFilter === opt.id && !activeTopicId
              ? 'var(--cp-accent)'
              : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
            color: activeFilter === opt.id && !activeTopicId
              ? 'white'
              : 'var(--cp-text)',
          }}
        >
          {opt.icon}
          {t(opt.labelKey, opt.fallback)}
        </button>
      ))}

      {/* Topic chips - desktop only */}
      {!isMobile ? subscribedTopics.map((topic) => (
        <button
          key={topic.id}
          type="button"
          onClick={() => {
            onTopicSelect(activeTopicId === topic.id ? null : topic.id)
            onFilterChange('all')
          }}
          className="flex flex-shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-colors"
          style={{
            background: activeTopicId === topic.id
              ? 'var(--cp-accent)'
              : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
            color: activeTopicId === topic.id
              ? 'white'
              : 'var(--cp-text)',
          }}
        >
          {topic.name}
        </button>
      )) : null}

      {/* Desktop: Separator + Reading mode toggle */}
      {!isMobile ? (
        <>
          <div
            className="mx-1 h-5 w-px flex-shrink-0"
            style={{ background: 'var(--cp-border)' }}
          />
          <div className="flex flex-shrink-0 items-center gap-1 rounded-full p-0.5" style={{ background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)' }}>
            {readingModeOptions.map((mode) => (
              <button
                key={mode.id}
                type="button"
                onClick={() => onReadingModeChange(mode.id)}
                className="flex items-center justify-center rounded-full p-1.5 transition-colors"
                style={{
                  background: readingMode === mode.id
                    ? 'color-mix(in srgb, var(--cp-accent) 18%, transparent)'
                    : 'transparent',
                  color: readingMode === mode.id
                    ? 'var(--cp-accent)'
                    : 'var(--cp-muted)',
                }}
                title={t(mode.labelKey, mode.fallback)}
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

/* ── Mobile Reading Mode Dropdown ── */

function ReadingModeDropdown({
  readingMode,
  t,
  onReadingModeChange,
}: {
  readingMode: ReadingMode
  t: (key: string, fallback: string) => string
  onReadingModeChange: (mode: ReadingMode) => void
}) {
  const [open, setOpen] = useState(false)
  const btnRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [menuPos, setMenuPos] = useState({ top: 0, left: 0 })

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (
        btnRef.current?.contains(e.target as Node) ||
        menuRef.current?.contains(e.target as Node)
      ) return
      setOpen(false)
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open])

  const handleToggle = () => {
    if (!open && btnRef.current) {
      const rect = btnRef.current.getBoundingClientRect()
      setMenuPos({ top: rect.bottom + 4, left: rect.left })
    }
    setOpen((v) => !v)
  }

  const active = readingModeOptions.find((m) => m.id === readingMode) ?? readingModeOptions[0]

  return (
    <div className="flex-shrink-0">
      <button
        ref={btnRef}
        type="button"
        onClick={handleToggle}
        className="flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-colors"
        style={{
          background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)',
          color: 'var(--cp-accent)',
        }}
      >
        {active.icon}
        {t(active.labelKey, active.fallback)}
        <ChevronDown size={12} />
      </button>

      {open ? (
        <div
          ref={menuRef}
          className="fixed z-[9999] min-w-[140px] rounded-xl py-1 shadow-lg"
          style={{
            top: menuPos.top,
            left: menuPos.left,
            background: 'var(--cp-surface)',
            border: '1px solid var(--cp-border)',
          }}
        >
          {readingModeOptions.map((mode) => (
            <button
              key={mode.id}
              type="button"
              onClick={() => {
                onReadingModeChange(mode.id)
                setOpen(false)
              }}
              className="flex w-full items-center gap-2 px-3 py-2 text-xs transition-colors"
              style={{
                background: readingMode === mode.id
                  ? 'color-mix(in srgb, var(--cp-accent) 10%, transparent)'
                  : 'transparent',
                color: readingMode === mode.id
                  ? 'var(--cp-accent)'
                  : 'var(--cp-text)',
              }}
            >
              {mode.icon}
              {t(mode.labelKey, mode.fallback)}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}
