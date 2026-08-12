import { Inbox } from 'lucide-react'
import { FeedCard } from './FeedCard'
import type { FeedObject, ReadingMode } from './types'

interface FeedListProps {
  feeds: FeedObject[]
  readingMode: ReadingMode
  t: (key: string, fallback: string) => string
  onSelectFeed: (id: string) => void
  onToggleLike: (id: string) => void
  onToggleBookmark: (id: string) => void
  onRepost: (id: string) => void
  scrollable?: boolean
}

export function FeedList({
  feeds,
  readingMode,
  t,
  onSelectFeed,
  onToggleLike,
  onToggleBookmark,
  onRepost,
  scrollable = true,
}: FeedListProps) {
  if (feeds.length === 0) {
    return (
      <div
        className="flex h-64 flex-col items-center justify-center gap-3"
        style={{ color: 'var(--cp-muted)' }}
      >
        <Inbox size={40} strokeWidth={1.2} />
        <p className="text-sm">{t('homestation.emptyFeed', 'No content matches your filters')}</p>
      </div>
    )
  }

  return (
    <div className={scrollable ? 'desktop-scrollbar h-full overflow-y-auto' : ''}>
      {feeds.map((feed) => (
        <FeedCard
          key={feed.id}
          feed={feed}
          readingMode={readingMode}
          t={t}
          onSelect={onSelectFeed}
          onToggleLike={onToggleLike}
          onToggleBookmark={onToggleBookmark}
          onRepost={onRepost}
        />
      ))}
    </div>
  )
}
