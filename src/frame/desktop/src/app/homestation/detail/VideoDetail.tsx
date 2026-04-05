import {
  Bookmark,
  ChevronLeft,
  Heart,
  Play,
  ShieldCheck,
} from 'lucide-react'
import type { FeedObject } from '../types'

interface VideoDetailProps {
  feed: FeedObject
  t: (key: string, fallback: string) => string
  onBack: () => void
  onToggleLike: (id: string) => void
  onToggleBookmark: (id: string) => void
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${seconds.toString().padStart(2, '0')}`
}

export function VideoDetail({
  feed,
  t,
  onBack,
  onToggleLike,
  onToggleBookmark,
}: VideoDetailProps) {
  const video = feed.media.find((m) => m.type === 'video')

  return (
    <div className="flex h-full flex-col" style={{ background: 'var(--cp-bg)' }}>
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-3" style={{ borderBottom: '1px solid var(--cp-border)' }}>
        <button
          type="button"
          onClick={onBack}
          className="flex h-9 w-9 items-center justify-center rounded-xl"
          style={{ color: 'var(--cp-muted)', background: 'color-mix(in srgb, var(--cp-text) 7%, transparent)' }}
        >
          <ChevronLeft size={18} />
        </button>
        <span className="flex-1 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('homestation.video', 'Video')}
        </span>
        <button
          type="button"
          onClick={() => onToggleLike(feed.id)}
          className="flex h-9 w-9 items-center justify-center rounded-xl"
          style={{ color: feed.interactions.isLiked ? 'var(--cp-danger)' : 'var(--cp-muted)' }}
        >
          <Heart size={18} fill={feed.interactions.isLiked ? 'currentColor' : 'none'} />
        </button>
        <button
          type="button"
          onClick={() => onToggleBookmark(feed.id)}
          className="flex h-9 w-9 items-center justify-center rounded-xl"
          style={{ color: feed.interactions.isBookmarked ? 'var(--cp-warning)' : 'var(--cp-muted)' }}
        >
          <Bookmark size={18} fill={feed.interactions.isBookmarked ? 'currentColor' : 'none'} />
        </button>
      </div>

      {/* Video player area */}
      <div
        className="relative flex items-center justify-center"
        style={{ background: '#000', aspectRatio: '16/9', maxHeight: '60vh' }}
      >
        <div
          className="flex h-16 w-16 items-center justify-center rounded-full"
          style={{ background: 'rgba(255,255,255,0.2)', color: 'white' }}
        >
          <Play size={28} fill="white" />
        </div>
        {video?.durationMs ? (
          <span
            className="absolute bottom-3 right-3 rounded px-2 py-0.5 text-xs font-medium"
            style={{ background: 'rgba(0,0,0,0.7)', color: 'white' }}
          >
            {formatDuration(video.durationMs)}
          </span>
        ) : null}
      </div>

      {/* Info */}
      <div className="desktop-scrollbar flex-1 overflow-y-auto px-4 py-4">
        {feed.title ? (
          <h2 className="mb-2 text-lg font-bold" style={{ color: 'var(--cp-text)' }}>
            {feed.title}
          </h2>
        ) : null}

        <div className="mb-3 flex items-center gap-3">
          <div
            className="flex h-10 w-10 items-center justify-center rounded-full text-sm font-semibold"
            style={{ background: 'color-mix(in srgb, var(--cp-accent) 15%, transparent)', color: 'var(--cp-accent)' }}
          >
            {feed.author.name.charAt(0)}
          </div>
          <div>
            <div className="flex items-center gap-1">
              <span className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{feed.author.name}</span>
              {feed.author.isVerified ? <ShieldCheck size={14} style={{ color: 'var(--cp-success)' }} /> : null}
            </div>
            <span className="text-xs capitalize" style={{ color: 'var(--cp-muted)' }}>{feed.author.sourceType}</span>
          </div>
        </div>

        {feed.text ? (
          <p className="text-sm leading-relaxed" style={{ color: 'color-mix(in srgb, var(--cp-text) 85%, transparent)' }}>
            {feed.text}
          </p>
        ) : null}

        {feed.recommendReason ? (
          <p className="mt-3 text-xs" style={{ color: 'var(--cp-muted)' }}>
            {feed.recommendReason}
          </p>
        ) : null}
      </div>
    </div>
  )
}
