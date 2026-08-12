import {
  Bookmark,
  Eye,
  Heart,
  MessageCircle,
  MoreHorizontal,
  Repeat2,
  ExternalLink,
  ShieldCheck,
  Play,
} from 'lucide-react'
import type { FeedObject, ReadingMode } from './types'

interface FeedCardProps {
  feed: FeedObject
  readingMode: ReadingMode
  t: (key: string, fallback: string) => string
  onSelect: (id: string) => void
  onToggleLike: (id: string) => void
  onToggleBookmark: (id: string) => void
  onRepost: (id: string) => void
}

function formatTimeAgo(timestamp: number): string {
  const diff = Date.now() - timestamp
  const minutes = Math.floor(diff / 60_000)

  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${minutes}m`

  const hours = Math.floor(minutes / 60)

  if (hours < 24) return `${hours}h`

  const days = Math.floor(hours / 24)

  return `${days}d`
}

function formatCount(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`

  return String(count)
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60

  return `${minutes}:${seconds.toString().padStart(2, '0')}`
}

function SourceBadge({ sourceType }: { sourceType: string }) {
  if (sourceType === 'did') {
    return (
      <span
        className="flex items-center gap-0.5 text-[10px]"
        style={{ color: 'var(--cp-success)' }}
        title="Verified DID"
      >
        <ShieldCheck size={11} />
      </span>
    )
  }

  return null
}

function MediaGrid({ feed, onSelect }: { feed: FeedObject; onSelect: () => void }) {
  const images = feed.media.filter((m) => m.type === 'image')
  const videos = feed.media.filter((m) => m.type === 'video')

  if (videos.length > 0) {
    const video = videos[0]

    return (
      <button
        type="button"
        onClick={onSelect}
        className="relative mt-2 w-full overflow-hidden rounded-xl"
        style={{ background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)', aspectRatio: '16/9' }}
      >
        <div className="flex h-full w-full items-center justify-center">
          <div
            className="flex h-12 w-12 items-center justify-center rounded-full"
            style={{ background: 'rgba(0,0,0,0.6)', color: 'white' }}
          >
            <Play size={20} fill="white" />
          </div>
        </div>
        {video.durationMs ? (
          <span
            className="absolute bottom-2 right-2 rounded px-1.5 py-0.5 text-[11px] font-medium"
            style={{ background: 'rgba(0,0,0,0.7)', color: 'white' }}
          >
            {formatDuration(video.durationMs)}
          </span>
        ) : null}
      </button>
    )
  }

  if (images.length === 0) return null

  if (images.length === 1) {
    return (
      <button
        type="button"
        onClick={onSelect}
        className="mt-2 w-full overflow-hidden rounded-xl"
        style={{ background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)', aspectRatio: '16/10' }}
      />
    )
  }

  if (images.length === 2) {
    return (
      <button
        type="button"
        onClick={onSelect}
        className="mt-2 grid w-full grid-cols-2 gap-1 overflow-hidden rounded-xl"
      >
        <div style={{ background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)', aspectRatio: '1' }} className="rounded-l-xl" />
        <div style={{ background: 'color-mix(in srgb, var(--cp-text) 10%, transparent)', aspectRatio: '1' }} className="rounded-r-xl" />
      </button>
    )
  }

  if (images.length === 3) {
    return (
      <button
        type="button"
        onClick={onSelect}
        className="mt-2 grid w-full grid-cols-2 gap-1 overflow-hidden rounded-xl"
        style={{ gridTemplateRows: 'auto auto' }}
      >
        <div style={{ background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)', aspectRatio: '1' }} className="row-span-2 rounded-l-xl" />
        <div style={{ background: 'color-mix(in srgb, var(--cp-text) 10%, transparent)', aspectRatio: '2/1' }} className="rounded-tr-xl" />
        <div style={{ background: 'color-mix(in srgb, var(--cp-text) 12%, transparent)', aspectRatio: '2/1' }} className="rounded-br-xl" />
      </button>
    )
  }

  return (
    <button
      type="button"
      onClick={onSelect}
      className="mt-2 grid w-full grid-cols-2 gap-1 overflow-hidden rounded-xl"
    >
      {images.slice(0, 4).map((_, idx) => (
        <div
          key={idx}
          style={{
            background: `color-mix(in srgb, var(--cp-text) ${8 + idx * 2}%, transparent)`,
            aspectRatio: '1',
          }}
          className={
            idx === 0 ? 'rounded-tl-xl'
              : idx === 1 ? 'rounded-tr-xl'
                : idx === 2 ? 'rounded-bl-xl'
                  : 'rounded-br-xl'
          }
        />
      ))}
    </button>
  )
}

export function FeedCard({
  feed,
  readingMode,
  t,
  onSelect,
  onToggleLike,
  onToggleBookmark,
  onRepost,
}: FeedCardProps) {
  const isImageMode = readingMode === 'image'
  const isLongformMode = readingMode === 'longform'
  const textClamp = isLongformMode ? 'line-clamp-6' : isImageMode ? 'line-clamp-1' : 'line-clamp-3'

  return (
    <article
      className="border-b px-4 py-3 transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_3%,transparent)]"
      style={{ borderColor: 'color-mix(in srgb, var(--cp-border) 50%, transparent)' }}
    >
      {/* Author row */}
      <div className="flex items-center gap-2">
        <div
          className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full text-sm font-semibold"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent) 15%, transparent)',
            color: 'var(--cp-accent)',
          }}
        >
          {feed.author.name.charAt(0)}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1">
            <span className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              {feed.author.name}
            </span>
            <SourceBadge sourceType={feed.author.sourceType} />
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              · {formatTimeAgo(feed.createdAt)}
            </span>
          </div>
          <span
            className="text-[11px] capitalize"
            style={{ color: 'var(--cp-muted)' }}
          >
            {feed.author.sourceType}
          </span>
        </div>
        {feed.originalUrl ? (
          <button
            type="button"
            className="flex h-7 w-7 items-center justify-center rounded-lg"
            style={{ color: 'var(--cp-muted)' }}
            title={t('homestation.viewOriginal', 'View original')}
          >
            <ExternalLink size={14} />
          </button>
        ) : null}
        <button
          type="button"
          className="flex h-7 w-7 items-center justify-center rounded-lg"
          style={{ color: 'var(--cp-muted)' }}
        >
          <MoreHorizontal size={14} />
        </button>
      </div>

      {/* Title (for articles/products/links) */}
      {feed.title ? (
        <h3
          className="mt-2 cursor-pointer text-[15px] font-semibold leading-snug"
          style={{ color: 'var(--cp-text)' }}
          onClick={() => onSelect(feed.id)}
        >
          {feed.title}
        </h3>
      ) : null}

      {/* Text content */}
      {feed.text ? (
        <p
          className={`mt-1.5 cursor-pointer text-sm leading-relaxed ${textClamp}`}
          style={{ color: 'color-mix(in srgb, var(--cp-text) 85%, transparent)' }}
          onClick={() => onSelect(feed.id)}
        >
          {feed.text}
        </p>
      ) : null}

      {/* Media */}
      {!isLongformMode && feed.media.length > 0 ? (
        <MediaGrid feed={feed} onSelect={() => onSelect(feed.id)} />
      ) : null}

      {/* Recommendation reason */}
      {feed.recommendReason ? (
        <div
          className="mt-2 flex items-center gap-1 text-[11px]"
          style={{ color: 'var(--cp-muted)' }}
        >
          <Eye size={11} />
          <span>{feed.recommendReason}</span>
        </div>
      ) : null}

      {/* Interaction bar */}
      <div className="mt-2 flex items-center gap-1">
        <button
          type="button"
          onClick={() => onToggleLike(feed.id)}
          className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs transition-colors"
          style={{
            color: feed.interactions.isLiked ? 'var(--cp-danger)' : 'var(--cp-muted)',
          }}
        >
          <Heart size={15} fill={feed.interactions.isLiked ? 'currentColor' : 'none'} />
          {feed.interactions.likeCount > 0 ? formatCount(feed.interactions.likeCount) : null}
        </button>

        <button
          type="button"
          onClick={() => onSelect(feed.id)}
          className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs transition-colors"
          style={{ color: 'var(--cp-muted)' }}
        >
          <MessageCircle size={15} />
          {feed.interactions.commentCount > 0 ? formatCount(feed.interactions.commentCount) : null}
        </button>

        <button
          type="button"
          onClick={() => onRepost(feed.id)}
          className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs transition-colors"
          style={{
            color: feed.interactions.isReposted ? 'var(--cp-success)' : 'var(--cp-muted)',
          }}
        >
          <Repeat2 size={15} />
          {feed.interactions.repostCount > 0 ? formatCount(feed.interactions.repostCount) : null}
        </button>

        <button
          type="button"
          onClick={() => onToggleBookmark(feed.id)}
          className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs transition-colors"
          style={{
            color: feed.interactions.isBookmarked ? 'var(--cp-warning)' : 'var(--cp-muted)',
          }}
        >
          <Bookmark size={15} fill={feed.interactions.isBookmarked ? 'currentColor' : 'none'} />
        </button>
      </div>
    </article>
  )
}
