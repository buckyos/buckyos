import { Bookmark, Clock, ClockCheck, Heart, MessageCircle, Quote, Repeat2, ThumbsDown, Undo2 } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useItemActions } from '../actions'
import { formatCount } from '../datamodel/format'
import type { CardView } from '../datamodel/types'
import { Menu } from '../ui/Popover'
import { CountScope } from './parts'

function repostBlockedText(t: (key: string, fallback: string) => string, reason: CardView['repostBlockedReason']) {
  switch (reason) {
    case 'restricted':
      return t('homestation.repost.blockedRestricted', 'Posts with a restricted audience can’t be reposted or quoted.')
    case 'private_capture':
      return t('homestation.repost.blockedCapture', 'This is your private capture. Use “Share to my homepage” instead.')
    case 'withdrawn':
      return t('homestation.repost.blockedWithdrawn', 'The author withdrew this post.')
    default:
      return t('homestation.repost.blocked', 'This item can’t be reposted.')
  }
}

export function ActionBar({ view, onComment, showScope = true }: { view: CardView; onComment: () => void; showScope?: boolean }) {
  const { t } = useI18n()
  const actions = useItemActions()
  const [repostAnchor, setRepostAnchor] = useState<HTMLElement | null>(null)
  const personal = view.personal
  const stats = view.stats
  if (!personal) return <ReadOnlyCounts view={view} />
  const liked = personal.like.on
  const reposted = personal.repost.on
  const repostCount = (stats?.reposts ?? 0) + (stats?.quotes ?? 0)
  const blockedText = view.canRepost ? undefined : repostBlockedText(t, view.repostBlockedReason)
  const stop = (event: React.MouseEvent) => event.stopPropagation()

  return (
    <div className="-ml-2 mt-2 flex flex-wrap items-center gap-1" onClick={stop} data-testid="hs-action-bar">
      <button
        type="button"
        className="hs-icon-btn"
        aria-pressed={liked}
        aria-label={liked ? t('homestation.like.unlike', 'Unlike') : t('homestation.like.like', 'Like')}
        title={view.item.audience.restricted ? t('homestation.like.restrictedHint', 'Your like goes only to the author') : t('homestation.like.publicHint', 'Likes are public')}
        style={liked ? { color: 'var(--hs-like)' } : undefined}
        onClick={() => void actions.toggleLike(view)}
        data-testid="hs-like"
      >
        <Heart size={18} fill={liked ? 'currentColor' : 'none'} />
        {stats && stats.likes > 0 ? <span data-testid="hs-like-count">{formatCount(stats.likes)}</span> : null}
      </button>
      <button type="button" className="hs-icon-btn" aria-label={t('homestation.comments.open', 'Comments')} title={t('homestation.comments.countHint', 'Text comments only, from your local merged view')} onClick={onComment} data-testid="hs-comment">
        <MessageCircle size={18} />
        {stats && stats.textComments > 0 ? formatCount(stats.textComments) : null}
      </button>
      <span title={blockedText}>
        <button
          type="button"
          className="hs-icon-btn"
          aria-pressed={reposted}
          aria-label={reposted ? t('homestation.repost.reposted', 'Reposted') : t('homestation.repost.label', 'Repost')}
          disabled={!view.canRepost && !reposted}
          onClick={event => setRepostAnchor(repostAnchor ? null : event.currentTarget)}
          data-testid="hs-repost"
        >
          <Repeat2 size={18} />
          {repostCount > 0 ? formatCount(repostCount) : null}
        </button>
      </span>
      <button
        type="button"
        className="hs-icon-btn"
        aria-pressed={personal.bookmark.on}
        aria-label={personal.bookmark.on ? t('homestation.bookmark.remove', 'Remove bookmark') : t('homestation.bookmark.add', 'Bookmark (private)')}
        title={personal.bookmark.on && personal.bookmark.visibility !== 'private' ? t('homestation.bookmark.isPublic', 'Public bookmark') : t('homestation.bookmark.isPrivate', 'Bookmarks are private by default')}
        onClick={() => void actions.toggleBookmark(view)}
        data-testid="hs-bookmark"
      >
        <Bookmark size={18} fill={personal.bookmark.on ? 'currentColor' : 'none'} />
      </button>
      <button
        type="button"
        className="hs-icon-btn"
        aria-pressed={personal.readLater}
        aria-label={personal.readLater ? t('homestation.readLater.remove', 'Remove from Read later') : t('homestation.readLater.add', 'Read later')}
        title={personal.readLater ? t('homestation.readLater.remove', 'Remove from Read later') : t('homestation.readLater.add', 'Read later')}
        onClick={() => void actions.toggleReadLater(view)}
        data-testid="hs-read-later"
      >
        {personal.readLater ? <ClockCheck size={18} /> : <Clock size={18} />}
      </button>
      <button
        type="button"
        className="hs-icon-btn"
        aria-pressed={personal.dislike}
        aria-label={t('homestation.dislike.label', 'Dislike (local only)')}
        title={t('homestation.dislike.hint', 'Only tunes your local recommendations; never published')}
        onClick={() => void actions.toggleDislike(view)}
      >
        <ThumbsDown size={18} fill={personal.dislike ? 'currentColor' : 'none'} />
      </button>
      {showScope ? <CountScope view={view} /> : null}
      <Menu
        anchor={repostAnchor}
        open={!!repostAnchor}
        onClose={() => setRepostAnchor(null)}
        label={t('homestation.repost.menu', 'Repost options')}
        testId="hs-repost-menu"
        align="start"
        items={reposted
          ? [{ id: 'undo', label: t('homestation.repost.undo', 'Undo repost'), hint: t('homestation.repost.undoHint', 'Withdraws your repost entry; the original stays'), icon: <Undo2 size={16} />, onSelect: () => void actions.undoRepost(view) }, { id: 'quote', label: t('homestation.quote.label', 'Quote'), icon: <Quote size={16} />, disabled: !view.canRepost, onSelect: () => void actions.quote(view) }]
          : [{ id: 'repost', label: t('homestation.repost.label', 'Repost'), hint: t('homestation.repost.hint', 'Wraps this exact version in a post signed by you'), icon: <Repeat2 size={16} />, onSelect: () => void actions.repost(view) }, { id: 'quote', label: t('homestation.quote.label', 'Quote'), hint: t('homestation.quote.hint', 'Repost with your own words'), icon: <Quote size={16} />, onSelect: () => void actions.quote(view) }]}
      />
    </div>
  )
}

export function ReadOnlyCounts({ view }: { view: CardView }) {
  const { t } = useI18n()
  const stats = view.stats
  if (!stats) return null
  return (
    <p className="hs-meta mt-2 tabular-nums" data-testid="hs-readonly-counts">
      <span>{t('homestation.stats.likes', '{{n}} likes', { n: formatCount(stats.likes) })}</span>
      <span>{t('homestation.stats.comments', '{{n}} comments', { n: formatCount(stats.textComments) })}</span>
      <span>{t('homestation.stats.reposts', '{{n}} reposts', { n: formatCount(stats.reposts + stats.quotes) })}</span>
      <span title={t('homestation.stats.scopeHint', 'Counts come from interaction records your HomeStation verified. They are not a network-wide total.')}>{t('homestation.stats.localVerified', 'Verified locally')}</span>
    </p>
  )
}
