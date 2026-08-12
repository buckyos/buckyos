import {
  Bookmark,
  ChevronLeft,
  ExternalLink,
  Heart,
  ShieldCheck,
} from 'lucide-react'
import type { FeedObject } from '../types'

interface ArticleDetailProps {
  feed: FeedObject
  t: (key: string, fallback: string) => string
  onBack: () => void
  onToggleLike: (id: string) => void
  onToggleBookmark: (id: string) => void
}

function formatDate(ts: number): string {
  return new Date(ts).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function ArticleDetail({
  feed,
  t,
  onBack,
  onToggleLike,
  onToggleBookmark,
}: ArticleDetailProps) {
  const readingTime = feed.body
    ? Math.max(1, Math.ceil(feed.body.split(/\s+/).length / 200))
    : 1

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
          {t('homestation.article', 'Article')}
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

      {/* Content */}
      <div className="desktop-scrollbar flex-1 overflow-y-auto">
        <div className="mx-auto max-w-2xl px-4 py-6">
          {/* Author */}
          <div className="mb-4 flex items-center gap-3">
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
              <div className="flex items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
                <span>{formatDate(feed.createdAt)}</span>
                <span>· {readingTime} min read</span>
              </div>
            </div>
          </div>

          {/* Title */}
          {feed.title ? (
            <h1 className="mb-4 text-2xl font-bold leading-tight" style={{ color: 'var(--cp-text)' }}>
              {feed.title}
            </h1>
          ) : null}

          {/* Hero image */}
          {feed.media.length > 0 && feed.media[0].type === 'image' ? (
            <div
              className="mb-6 w-full overflow-hidden rounded-2xl"
              style={{ background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)', aspectRatio: '16/9' }}
            />
          ) : null}

          {/* Body */}
          <div
            className="whitespace-pre-wrap text-[15px] leading-relaxed"
            style={{ color: 'color-mix(in srgb, var(--cp-text) 88%, transparent)' }}
          >
            {feed.body ?? feed.text}
          </div>

          {/* Original link */}
          {feed.originalUrl ? (
            <div className="mt-6 rounded-xl p-3" style={{ background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)' }}>
              <div className="flex items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
                <ExternalLink size={14} />
                <span className="truncate">{feed.originalUrl}</span>
              </div>
            </div>
          ) : null}

          {/* Recommendation reason */}
          {feed.recommendReason ? (
            <p className="mt-4 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {feed.recommendReason}
            </p>
          ) : null}
        </div>
      </div>
    </div>
  )
}
