import {
  Hash,
  Send,
  SlidersHorizontal,
  TrendingUp,
  X,
} from 'lucide-react'
import { useState } from 'react'
import type { FeedFilter, ReadingMode, Topic } from './types'

interface InfoPanelProps {
  activeFilter: FeedFilter
  activeTopicId: string | null
  readingMode: ReadingMode
  topics: Topic[]
  t: (key: string, fallback: string) => string
  onPublish: (text: string) => void
  onSelectTopic: (id: string) => void
  onClose: () => void
}

const filterLabels: Record<FeedFilter, string> = {
  all: 'All',
  following: 'Following',
  images: 'Images',
  videos: 'Videos',
  longform: 'Long-form',
  news: 'News',
}

const modeLabels: Record<ReadingMode, string> = {
  standard: 'Standard',
  image: 'Image First',
  longform: 'Long-form',
  'immersive-video': 'Immersive Video',
}

export function InfoPanel({
  activeFilter,
  activeTopicId,
  readingMode,
  topics,
  t,
  onPublish,
  onSelectTopic,
  onClose,
}: InfoPanelProps) {
  const [quickText, setQuickText] = useState('')

  const trendingTopics = [...topics]
    .sort((a, b) => (b.trendScore ?? 0) - (a.trendScore ?? 0))
    .slice(0, 5)

  const activeTopic = activeTopicId ? topics.find((tp) => tp.id === activeTopicId) : null

  const handleSubmitQuickPublish = () => {
    if (!quickText.trim()) return
    onPublish(quickText.trim())
    setQuickText('')
  }

  return (
    <div
      className="desktop-scrollbar flex h-full flex-col overflow-y-auto"
      style={{
        background: 'var(--cp-bg)',
      }}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3">
        <span className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('homestation.infoPanel', 'Info')}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="flex h-7 w-7 items-center justify-center rounded-lg"
          style={{ color: 'var(--cp-muted)' }}
        >
          <X size={14} />
        </button>
      </div>

      {/* Active filters */}
      <div className="px-4 py-3">
        <div className="mb-2 flex items-center gap-2">
          <SlidersHorizontal size={14} style={{ color: 'var(--cp-muted)' }} />
          <span className="text-xs font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
            {t('homestation.activeFilters', 'Active Filters')}
          </span>
        </div>
        <div className="flex flex-wrap gap-1.5">
          <span
            className="rounded-full px-2.5 py-1 text-[11px] font-medium"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)',
              color: 'var(--cp-accent)',
            }}
          >
            {filterLabels[activeFilter]}
          </span>
          <span
            className="rounded-full px-2.5 py-1 text-[11px] font-medium"
            style={{
              background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
              color: 'var(--cp-text)',
            }}
          >
            {modeLabels[readingMode]}
          </span>
          {activeTopic ? (
            <span
              className="rounded-full px-2.5 py-1 text-[11px] font-medium"
              style={{
                background: 'color-mix(in srgb, var(--cp-warning) 12%, transparent)',
                color: 'var(--cp-warning)',
              }}
            >
              # {activeTopic.name}
            </span>
          ) : null}
        </div>
      </div>

      {/* Trending topics */}
      <div className="px-4 py-3">
        <div className="mb-2 flex items-center gap-2">
          <TrendingUp size={14} style={{ color: 'var(--cp-muted)' }} />
          <span className="text-xs font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
            {t('homestation.trendingTopics', 'Trending Topics')}
          </span>
        </div>
        <div className="flex flex-col gap-1">
          {trendingTopics.map((topic, idx) => (
            <button
              key={topic.id}
              type="button"
              onClick={() => onSelectTopic(topic.id)}
              className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]"
            >
              <span className="w-4 text-center text-[11px] font-bold" style={{ color: 'var(--cp-muted)' }}>
                {idx + 1}
              </span>
              <Hash size={12} style={{ color: 'var(--cp-accent)' }} />
              <span className="flex-1 truncate text-xs font-medium" style={{ color: 'var(--cp-text)' }}>
                {topic.name}
              </span>
              <span className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>
                {topic.feedCount}
              </span>
            </button>
          ))}
        </div>
      </div>

      {/* Quick publish */}
      <div className="px-4 py-3">
        <span className="mb-2 block text-xs font-semibold uppercase tracking-wider" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.quickPublish', 'Quick Publish')}
        </span>
        <div
          className="rounded-xl p-2"
          style={{ background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)' }}
        >
          <textarea
            value={quickText}
            onChange={(e) => setQuickText(e.target.value)}
            placeholder={t('homestation.whatOnYourMind', "What's on your mind?")}
            className="w-full resize-none bg-transparent text-sm outline-none"
            style={{ color: 'var(--cp-text)', minHeight: 60 }}
            rows={3}
          />
          <div className="flex items-center justify-end pt-1">
            <button
              type="button"
              onClick={handleSubmitQuickPublish}
              disabled={!quickText.trim()}
              className="flex items-center gap-1 rounded-lg px-3 py-1.5 text-xs font-medium transition-opacity disabled:opacity-40"
              style={{ background: 'var(--cp-accent)', color: 'white' }}
            >
              <Send size={12} />
              {t('homestation.publish', 'Publish')}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
