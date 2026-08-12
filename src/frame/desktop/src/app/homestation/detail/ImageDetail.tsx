import { useState } from 'react'
import {
  Bookmark,
  ChevronLeft,
  ChevronRight as ChevronRightIcon,
  Heart,
  ShieldCheck,
} from 'lucide-react'
import type { FeedObject } from '../types'

interface ImageDetailProps {
  feed: FeedObject
  t: (key: string, fallback: string) => string
  onBack: () => void
  onToggleLike: (id: string) => void
  onToggleBookmark: (id: string) => void
}

export function ImageDetail({
  feed,
  t,
  onBack,
  onToggleLike,
  onToggleBookmark,
}: ImageDetailProps) {
  const images = feed.media.filter((m) => m.type === 'image')
  const [currentIdx, setCurrentIdx] = useState(0)

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
          {t('homestation.images', 'Images')}
          {images.length > 1 ? ` (${currentIdx + 1}/${images.length})` : ''}
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

      {/* Image viewer */}
      <div className="relative flex flex-1 items-center justify-center" style={{ background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)' }}>
        <div
          className="flex h-full w-full items-center justify-center"
        >
          <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>
            {images[currentIdx]?.alt ?? `Image ${currentIdx + 1}`}
          </span>
        </div>

        {/* Navigation arrows */}
        {images.length > 1 ? (
          <>
            {currentIdx > 0 ? (
              <button
                type="button"
                onClick={() => setCurrentIdx((p) => p - 1)}
                className="absolute left-3 flex h-10 w-10 items-center justify-center rounded-full"
                style={{ background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)', color: 'var(--cp-text)' }}
              >
                <ChevronLeft size={20} />
              </button>
            ) : null}
            {currentIdx < images.length - 1 ? (
              <button
                type="button"
                onClick={() => setCurrentIdx((p) => p + 1)}
                className="absolute right-3 flex h-10 w-10 items-center justify-center rounded-full"
                style={{ background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)', color: 'var(--cp-text)' }}
              >
                <ChevronRightIcon size={20} />
              </button>
            ) : null}
          </>
        ) : null}

        {/* Dots indicator */}
        {images.length > 1 ? (
          <div className="absolute bottom-3 left-1/2 flex -translate-x-1/2 gap-1.5">
            {images.map((_, idx) => (
              <div
                key={idx}
                className="h-1.5 w-1.5 rounded-full transition-colors"
                style={{
                  background: idx === currentIdx
                    ? 'var(--cp-accent)'
                    : 'color-mix(in srgb, var(--cp-text) 25%, transparent)',
                }}
              />
            ))}
          </div>
        ) : null}
      </div>

      {/* Bottom info */}
      <div className="px-4 py-3" style={{ borderTop: '1px solid var(--cp-border)' }}>
        <div className="flex items-center gap-2">
          <div
            className="flex h-8 w-8 items-center justify-center rounded-full text-xs font-semibold"
            style={{ background: 'color-mix(in srgb, var(--cp-accent) 15%, transparent)', color: 'var(--cp-accent)' }}
          >
            {feed.author.name.charAt(0)}
          </div>
          <span className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            {feed.author.name}
          </span>
          {feed.author.isVerified ? <ShieldCheck size={14} style={{ color: 'var(--cp-success)' }} /> : null}
        </div>
        {feed.text ? (
          <p className="mt-2 text-sm" style={{ color: 'color-mix(in srgb, var(--cp-text) 85%, transparent)' }}>
            {feed.text}
          </p>
        ) : null}
      </div>
    </div>
  )
}
