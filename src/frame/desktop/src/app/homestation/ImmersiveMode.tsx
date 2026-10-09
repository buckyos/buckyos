import { Bookmark, ChevronDown, ChevronLeft, ChevronRight, ChevronUp, Heart, Loader2, MessageCircle, Play, Repeat2, X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import useSWR from 'swr'
import { useI18n } from '../../i18n/provider'
import { useItemActions } from './actions'
import { reasonLabel } from './card/labels'
import { ResourceOverlay, VerificationBadge } from './card/parts'
import { formatCount } from './datamodel/format'
import type { CardView, ReadingQuery } from './datamodel/types'
import { useReadingList } from './feed/useReadingList'
import { mediaUrl } from './mock/media'
import { useHsNav } from './navContext'
import { useCardView, useHomeStationStore } from './store/context'

const PAGE_CHARS = 520

function paginate(markdown: string) {
  const paragraphs = markdown.split(/\n{2,}/)
  const pages: string[] = []
  let current = ''
  for (const paragraph of paragraphs) {
    if (current && current.length + paragraph.length > PAGE_CHARS) {
      pages.push(current)
      current = ''
    }
    current = current ? `${current}\n\n${paragraph}` : paragraph
  }
  if (current) pages.push(current)
  return pages
}

function ArticlePages({ view, page, onPages }: { view: CardView; page: number; onPages: (count: number) => void }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const body = useSWR(['hs-body', store.id, view.item.objId], () => store.getWrappedBody(view.item.objId), { revalidateOnFocus: false })
  const pages = body.data?.state === 'ready' ? paginate(body.data.markdown.replace(/^#+\s*/gm, '')) : [view.item.object.content?.summary ?? '']
  useEffect(() => onPages(pages.length), [onPages, pages.length])
  return (
    <div className="flex h-full w-full max-w-xl flex-col justify-center px-8 text-left" style={{ color: 'white' }} data-testid="hs-immersive-article">
      <p className="mb-3 text-xs uppercase tracking-wider" style={{ color: 'rgba(255,255,255,0.55)' }}>
        {body.data?.state === 'ready' ? t('homestation.immersive.page', 'Page {{n}} of {{total}}', { n: Math.min(page, pages.length - 1) + 1, total: pages.length }) : body.data?.state === 'preparing' ? t('homestation.article.preparing', 'Preparing the full text…') : body.isLoading ? t('homestation.state.loading', 'Loading…') : t('homestation.immersive.summary', 'Summary')}
      </p>
      <h2 className="mb-4 text-xl font-bold leading-snug">{view.item.object.content?.title}</h2>
      <p className="whitespace-pre-line text-[15px] leading-7" style={{ color: 'rgba(255,255,255,0.86)' }}>{pages[Math.min(page, pages.length - 1)]}</p>
    </div>
  )
}

function ImmersiveItem({ objId, subIndex, onSubCount }: { objId: string; subIndex: number; onSubCount: (count: number) => void }) {
  const { t } = useI18n()
  const view = useCardView(objId)
  const reportImages = view?.item.contentType === 'image' ? view.item.media.length : null
  useEffect(() => {
    if (reportImages !== null) onSubCount(reportImages)
    else if (view?.item.contentType !== 'article') onSubCount(1)
  }, [onSubCount, reportImages, view?.item.contentType])
  if (!view) return null
  const { item } = view
  const resources = view.resources
  if (item.entry?.state === 'withdrawn') {
    return <p className="px-8 text-center text-sm" style={{ color: 'rgba(255,255,255,0.7)' }}>{t('homestation.withdrawn.title', '{{name}} withdrew this post', { name: item.publisher.name })}</p>
  }
  if (item.contentType === 'video') {
    const ready = resources === 'local' || resources === 'reachable'
    return (
      <div className="relative flex h-full w-full items-center justify-center" data-testid="hs-immersive-video" data-resources={resources}>
        {item.cover ? <img src={mediaUrl(item.cover.object, item.cover.file, item.object.content?.title)} alt="" className="max-h-full max-w-full object-contain" style={{ opacity: ready ? 1 : 0.45 }} /> : null}
        <div className="absolute inset-0 flex items-center justify-center">
          {ready ? <span className="flex h-16 w-16 items-center justify-center rounded-full" style={{ background: 'rgba(255,255,255,0.2)' }} aria-label={t('homestation.media.playVideo', 'Play video')}><Play size={30} fill="white" color="white" /></span> : <ResourceOverlay state={resources} objId={item.objId} kind="video" />}
        </div>
      </div>
    )
  }
  if (item.contentType === 'image' && item.media.length > 0) {
    const part = item.media[Math.min(subIndex, item.media.length - 1)]
    return (
      <div className="flex h-full w-full flex-col items-center justify-center" data-testid="hs-immersive-image">
        <img src={mediaUrl(part.object, part.file, part.alt)} alt={part.alt ?? ''} className="max-h-[78%] max-w-full object-contain" />
        {item.media.length > 1 ? <span className="mt-3 text-xs" style={{ color: 'rgba(255,255,255,0.6)' }}>{t('homestation.immersive.imageOf', 'Image {{n}} of {{total}}', { n: Math.min(subIndex, item.media.length - 1) + 1, total: item.media.length })}</span> : null}
      </div>
    )
  }
  if (item.contentType === 'article') return <ArticlePages view={view} page={subIndex} onPages={onSubCount} />
  const content = item.object.content
  const embedded = item.embedded?.item?.object.content
  return (
    <div className="flex h-full w-full max-w-xl flex-col justify-center px-8 text-center" style={{ color: 'white' }} data-testid="hs-immersive-text">
      {content?.title ? <h2 className="mb-3 text-xl font-bold leading-snug">{content.title}</h2> : null}
      <p className="text-lg font-semibold leading-relaxed">{content?.text ?? content?.summary ?? embedded?.text ?? embedded?.title}</p>
    </div>
  )
}

export function ImmersiveMode({ query, onClose }: { query: ReadingQuery; onClose: () => void }) {
  const { t } = useI18n()
  const nav = useHsNav()
  const actions = useItemActions()
  const list = useReadingList(query)
  const { objIds, hasMore, loadMore, isLoadingMore } = list
  const [index, setIndex] = useState(0)
  const [subIndex, setSubIndex] = useState(0)
  const [subCount, setSubCount] = useState(1)
  const containerRef = useRef<HTMLDivElement>(null)
  const touchStart = useRef<{ x: number; y: number } | null>(null)
  const safeIndex = Math.min(index, Math.max(objIds.length - 1, 0))
  const currentId = objIds[safeIndex]
  const view = useCardView(currentId ?? '')

  useEffect(() => {
    if (hasMore && !isLoadingMore && objIds.length > 0 && safeIndex >= objIds.length - 2) loadMore()
  }, [hasMore, isLoadingMore, loadMore, objIds.length, safeIndex])

  useEffect(() => {
    containerRef.current?.focus()
  }, [])

  const go = useCallback((delta: number) => {
    setIndex(current => Math.max(0, Math.min(current + delta, objIds.length - 1)))
    setSubIndex(0)
  }, [objIds.length])
  const goSub = useCallback((delta: number) => setSubIndex(current => Math.max(0, Math.min(current + delta, subCount - 1))), [subCount])

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'ArrowDown' || event.key === ' ') go(1)
    else if (event.key === 'ArrowUp') go(-1)
    else if (event.key === 'ArrowRight') goSub(1)
    else if (event.key === 'ArrowLeft') goSub(-1)
    else if (event.key === 'Escape') onClose()
    else return
    event.preventDefault()
  }

  const scopeLabel = [t(`homestation.filter.${query.filter}`, query.filter), query.topicId ? `#${query.topicId.replace('topic-', '')}` : null, query.search.trim() ? `“${query.search.trim()}”` : null].filter(Boolean).join(' · ')

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      role="dialog"
      aria-label={t('homestation.mode.immersive', 'Immersive')}
      className="absolute inset-0 z-50 flex flex-col outline-none"
      style={{ background: '#05060a' }}
      data-testid="hs-immersive"
      onKeyDown={onKeyDown}
      onTouchStart={event => { touchStart.current = { x: event.touches[0].clientX, y: event.touches[0].clientY } }}
      onTouchEnd={event => {
        const start = touchStart.current
        touchStart.current = null
        if (!start) return
        const dx = start.x - event.changedTouches[0].clientX
        const dy = start.y - event.changedTouches[0].clientY
        if (Math.abs(dy) > Math.abs(dx) && Math.abs(dy) > 40) go(dy > 0 ? 1 : -1)
        else if (Math.abs(dx) > 40) goSub(dx > 0 ? 1 : -1)
      }}
    >
      <div className="absolute left-4 right-4 top-4 z-10 flex items-center gap-2" style={{ paddingTop: 'var(--sat)' }}>
        <button type="button" onClick={onClose} className="flex h-10 w-10 items-center justify-center rounded-full" style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }} aria-label={t('common.close', 'Close')} data-testid="hs-immersive-close">
          <X size={20} />
        </button>
        <span className="min-w-0 flex-1 truncate rounded-full px-3 py-1 text-xs" style={{ background: 'rgba(255,255,255,0.12)', color: 'rgba(255,255,255,0.85)' }} data-testid="hs-immersive-scope">
          {t('homestation.immersive.scope', 'Same view: {{scope}}', { scope: scopeLabel })}
        </span>
        <span className="rounded-full px-3 py-1 text-xs font-medium tabular-nums" style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }} data-testid="hs-immersive-counter">
          {objIds.length ? `${safeIndex + 1} / ${list.meta?.total ?? objIds.length}` : '0 / 0'}
        </span>
      </div>

      <div className="relative flex min-h-0 flex-1 items-center justify-center">
        {list.isLoading ? <Loader2 size={28} className="animate-spin" color="white" /> : null}
        {!list.isLoading && objIds.length === 0 ? (
          <div className="px-8 text-center" style={{ color: 'rgba(255,255,255,0.75)' }} data-testid="hs-immersive-empty">
            <p className="text-sm font-semibold">{t('homestation.immersive.empty', 'Nothing to show in this view')}</p>
            <p className="mt-1 text-xs">{t('homestation.immersive.emptyBody', 'Immersive mode uses the same filter, topic and search as your feed.')}</p>
          </div>
        ) : null}
        {currentId ? <ImmersiveItem key={currentId} objId={currentId} subIndex={subIndex} onSubCount={setSubCount} /> : null}
        {subCount > 1 ? (
          <>
            <button type="button" className="absolute left-3 top-1/2 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-full disabled:opacity-30" style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }} disabled={subIndex === 0} onClick={() => goSub(-1)} aria-label={t('homestation.immersive.prevPage', 'Previous page')}><ChevronLeft size={20} /></button>
            <button type="button" className="absolute right-16 top-1/2 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-full disabled:opacity-30 md:right-20" style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }} disabled={subIndex >= subCount - 1} onClick={() => goSub(1)} aria-label={t('homestation.immersive.nextPage', 'Next page')}><ChevronRight size={20} /></button>
          </>
        ) : null}
        <div className="absolute right-4 top-1/2 hidden -translate-y-1/2 flex-col gap-2 md:flex">
          <button type="button" onClick={() => go(-1)} disabled={safeIndex === 0} className="flex h-10 w-10 items-center justify-center rounded-full disabled:opacity-30" style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }} aria-label={t('homestation.immersive.prev', 'Previous item')}><ChevronUp size={20} /></button>
          <button type="button" onClick={() => go(1)} disabled={safeIndex >= objIds.length - 1} className="flex h-10 w-10 items-center justify-center rounded-full disabled:opacity-30" style={{ background: 'rgba(255,255,255,0.15)', color: 'white' }} aria-label={t('homestation.immersive.next', 'Next item')}><ChevronDown size={20} /></button>
        </div>
      </div>

      {view && view.personal ? (
        <div className="absolute bottom-28 right-3 flex flex-col items-center gap-4 md:bottom-32 md:right-4" style={{ color: 'white' }}>
          <button type="button" className="flex flex-col items-center gap-1" aria-pressed={view.personal.like.on} aria-label={t('homestation.like.like', 'Like')} style={{ color: view.personal.like.on ? '#f87171' : 'white' }} onClick={() => void actions.toggleLike(view)}>
            <Heart size={26} fill={view.personal.like.on ? 'currentColor' : 'none'} />
            <span className="text-[11px]">{formatCount(view.stats?.likes ?? 0)}</span>
          </button>
          <button type="button" className="flex flex-col items-center gap-1" aria-label={t('homestation.comments.open', 'Comments')} onClick={() => { onClose(); nav.openDetail(view.item.objId) }}>
            <MessageCircle size={26} />
            <span className="text-[11px]">{formatCount(view.stats?.textComments ?? 0)}</span>
          </button>
          <button type="button" className="flex flex-col items-center gap-1 disabled:opacity-40" disabled={!view.canRepost} aria-label={t('homestation.repost.label', 'Repost')} style={{ color: view.personal.repost.on ? '#4ade80' : 'white' }} onClick={() => void actions.repost(view)}>
            <Repeat2 size={26} />
            <span className="text-[11px]">{formatCount((view.stats?.reposts ?? 0) + (view.stats?.quotes ?? 0))}</span>
          </button>
          <button type="button" className="flex flex-col items-center gap-1" aria-pressed={view.personal.bookmark.on} aria-label={t('homestation.bookmark.add', 'Bookmark (private)')} style={{ color: view.personal.bookmark.on ? '#fbbf24' : 'white' }} onClick={() => void actions.toggleBookmark(view)}>
            <Bookmark size={26} fill={view.personal.bookmark.on ? 'currentColor' : 'none'} />
          </button>
        </div>
      ) : null}

      {view ? (
        <div className="px-4 pb-6 pt-2" style={{ background: 'linear-gradient(transparent, rgba(0,0,0,0.85))', paddingBottom: 'calc(1.5rem + var(--sab))' }}>
          <div className="flex items-center gap-2 pr-16 text-sm font-semibold" style={{ color: 'white' }}>
            <span className="truncate">{view.item.isPrivateCapture ? view.item.capturedFrom : view.item.publisher.name}</span>
            <VerificationBadge verification={view.item.verification} />
          </div>
          {view.item.object.content?.text && view.item.contentType !== 'text' && view.item.contentType !== 'comment' ? <p className="mt-1 line-clamp-2 pr-16 text-sm" style={{ color: 'rgba(255,255,255,0.8)' }}>{view.item.object.content.text}</p> : null}
          {view.reading ? <p className="mt-1 text-[11px]" style={{ color: 'rgba(255,255,255,0.55)' }}>{reasonLabel(t, view.reading)}</p> : null}
        </div>
      ) : null}
    </div>
  )
}
