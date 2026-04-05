import { useCallback, useRef, useState } from 'react'
import {
  Bookmark,
  ChevronDown,
  ChevronUp,
  Heart,
  MessageCircle,
  Play,
  Repeat2,
  ShieldCheck,
  X,
} from 'lucide-react'
import type { FeedObject } from './types'

interface ImmersiveVideoModeProps {
  feeds: FeedObject[]
  onToggleLike: (id: string) => void
  onToggleBookmark: (id: string) => void
  onRepost: (id: string) => void
  onClose: () => void
}

function formatCount(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`
  return String(count)
}

export function ImmersiveVideoMode({
  feeds,
  onToggleLike,
  onToggleBookmark,
  onRepost,
  onClose,
}: ImmersiveVideoModeProps) {
  const [currentIndex, setCurrentIndex] = useState(0)
  const touchStartRef = useRef<{ y: number; time: number } | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const currentFeed = feeds[currentIndex]
  if (!currentFeed) return null

  const goNext = useCallback(() => {
    setCurrentIndex((prev) => Math.min(prev + 1, feeds.length - 1))
  }, [feeds.length])

  const goPrev = useCallback(() => {
    setCurrentIndex((prev) => Math.max(prev - 1, 0))
  }, [])

  const handleTouchStart = useCallback((e: React.TouchEvent) => {
    touchStartRef.current = { y: e.touches[0].clientY, time: Date.now() }
  }, [])

  const handleTouchEnd = useCallback((e: React.TouchEvent) => {
    if (!touchStartRef.current) return
    const deltaY = touchStartRef.current.y - e.changedTouches[0].clientY
    const elapsed = Date.now() - touchStartRef.current.time
    touchStartRef.current = null

    if (Math.abs(deltaY) > 50 || (Math.abs(deltaY) > 20 && elapsed < 300)) {
      if (deltaY > 0) goNext()
      else goPrev()
    }
  }, [goNext, goPrev])

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown' || e.key === ' ') { goNext(); e.preventDefault() }
    else if (e.key === 'ArrowUp') { goPrev(); e.preventDefault() }
    else if (e.key === 'Escape') onClose()
  }, [goNext, goPrev, onClose])

  const hasVideo = currentFeed.media.some((m) => m.type === 'video')

  return (
    <div
      ref={containerRef}
      className="fixed inset-0 z-50 flex flex-col"
      style={{ background: '#000' }}
      onTouchStart={handleTouchStart}
      onTouchEnd={handleTouchEnd}
      onKeyDown={handleKeyDown}
      tabIndex={0}
      autoFocus
    >
      {/* Close button */}
      <button
        type="button"
        onClick={onClose}
        className="absolute left-4 top-4 z-50 flex h-10 w-10 items-center justify-center rounded-full"
        style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }}
      >
        <X size={20} />
      </button>

      {/* Counter */}
      <span
        className="absolute right-4 top-4 z-50 rounded-full px-3 py-1 text-xs font-medium"
        style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }}
      >
        {currentIndex + 1} / {feeds.length}
      </span>

      {/* Content area */}
      <div className="relative flex flex-1 items-center justify-center">
        {hasVideo ? (
          <div className="flex h-full w-full items-center justify-center">
            <div
              className="flex h-16 w-16 items-center justify-center rounded-full"
              style={{ background: 'rgba(255,255,255,0.2)' }}
            >
              <Play size={32} fill="white" color="white" />
            </div>
          </div>
        ) : currentFeed.media.some((m) => m.type === 'image') ? (
          <div
            className="flex h-full w-full items-center justify-center"
            style={{ background: 'color-mix(in srgb, white 8%, black)' }}
          >
            <span className="text-sm" style={{ color: 'rgba(255,255,255,0.5)' }}>
              {currentFeed.media[0]?.alt ?? 'Image'}
            </span>
          </div>
        ) : (
          <div className="flex h-full w-full flex-col items-center justify-center px-8 text-center">
            <p className="text-lg font-semibold leading-relaxed" style={{ color: 'white' }}>
              {currentFeed.title ?? currentFeed.text}
            </p>
          </div>
        )}

        {/* Navigation arrows (desktop) */}
        <div className="absolute right-4 top-1/2 hidden -translate-y-1/2 flex-col gap-2 md:flex">
          <button
            type="button"
            onClick={goPrev}
            disabled={currentIndex === 0}
            className="flex h-10 w-10 items-center justify-center rounded-full disabled:opacity-30"
            style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }}
          >
            <ChevronUp size={20} />
          </button>
          <button
            type="button"
            onClick={goNext}
            disabled={currentIndex === feeds.length - 1}
            className="flex h-10 w-10 items-center justify-center rounded-full disabled:opacity-30"
            style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }}
          >
            <ChevronDown size={20} />
          </button>
        </div>
      </div>

      {/* Right action column */}
      <div className="absolute bottom-24 right-4 flex flex-col items-center gap-5 md:bottom-32">
        <ActionButton
          icon={<Heart size={24} fill={currentFeed.interactions.isLiked ? 'currentColor' : 'none'} />}
          count={currentFeed.interactions.likeCount}
          active={currentFeed.interactions.isLiked}
          activeColor="#ef4444"
          onClick={() => onToggleLike(currentFeed.id)}
        />
        <ActionButton
          icon={<MessageCircle size={24} />}
          count={currentFeed.interactions.commentCount}
          onClick={() => {}}
        />
        <ActionButton
          icon={<Repeat2 size={24} />}
          count={currentFeed.interactions.repostCount}
          active={currentFeed.interactions.isReposted}
          activeColor="#22c55e"
          onClick={() => onRepost(currentFeed.id)}
        />
        <ActionButton
          icon={<Bookmark size={24} fill={currentFeed.interactions.isBookmarked ? 'currentColor' : 'none'} />}
          active={currentFeed.interactions.isBookmarked}
          activeColor="#f59e0b"
          onClick={() => onToggleBookmark(currentFeed.id)}
        />
      </div>

      {/* Bottom info */}
      <div className="px-4 pb-6 pt-2" style={{ background: 'linear-gradient(transparent, rgba(0,0,0,0.8))' }}>
        <div className="flex items-center gap-2">
          <div
            className="flex h-8 w-8 items-center justify-center rounded-full text-xs font-bold"
            style={{ background: 'rgba(255,255,255,0.2)', color: 'white' }}
          >
            {currentFeed.author.name.charAt(0)}
          </div>
          <span className="text-sm font-semibold" style={{ color: 'white' }}>
            {currentFeed.author.name}
          </span>
          {currentFeed.author.isVerified ? (
            <ShieldCheck size={14} style={{ color: '#22c55e' }} />
          ) : null}
        </div>
        {currentFeed.text ? (
          <p className="mt-1 line-clamp-2 text-sm" style={{ color: 'rgba(255,255,255,0.8)' }}>
            {currentFeed.text}
          </p>
        ) : null}
        {currentFeed.recommendReason ? (
          <p className="mt-1 text-[11px]" style={{ color: 'rgba(255,255,255,0.5)' }}>
            {currentFeed.recommendReason}
          </p>
        ) : null}
      </div>
    </div>
  )
}

function ActionButton({
  icon,
  count,
  active,
  activeColor,
  onClick,
}: {
  icon: React.ReactNode
  count?: number
  active?: boolean
  activeColor?: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex flex-col items-center gap-1"
      style={{ color: active ? activeColor : 'white' }}
    >
      {icon}
      {count !== undefined ? (
        <span className="text-[11px] font-medium">{formatCount(count)}</span>
      ) : null}
    </button>
  )
}
