import { Bookmark, CheckCircle2, CircleAlert, Loader2, RotateCw, Trash2 } from 'lucide-react'
import { useRef } from 'react'
import { useI18n } from '../../../i18n/provider'
import { formatCount, formatTimeAgo } from '../datamodel/format'
import type { CardView, PublishTask } from '../datamodel/types'
import { useHsNav } from '../navContext'
import { useCardView, useNow } from '../store/context'
import type { CardVariant, ReadingMode } from '../types'
import { ActionBar, ReadOnlyCounts } from './ActionBar'
import { ItemMenu, type ItemMenuHandle } from './ItemMenu'
import { CardHeader, ContentBody, EmbeddedCard, ReasonLine, TagRow } from './parts'

function WithdrawnCard({ view, variant, footer }: { view: CardView; variant: CardVariant; footer?: React.ReactNode }) {
  const { t } = useI18n()
  const now = useNow()
  return (
    <article className="hs-card flex items-start gap-3" data-testid="hs-card" data-objid={view.item.objId} data-state="withdrawn">
      <span className="mt-0.5 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full" style={{ background: 'var(--hs-chip-bg)', color: 'var(--cp-muted)' }}><Trash2 size={15} /></span>
      <div className="min-w-0 flex-1 text-[13px] leading-5">
        <p className="font-medium">{t('homestation.withdrawn.title', '{{name}} withdrew this post', { name: view.item.publisher.name })}</p>
        <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.withdrawn.body', 'It is no longer recommended as current content. Comments and interactions by others keep their own identity.')}
          {' · '}
          {formatTimeAgo(t, view.item.createdAt, now)}
        </p>
        {variant === 'saved' && view.personal?.bookmark.on ? (
          <p className="mt-1 flex items-center gap-1 text-xs" style={{ color: 'color-mix(in srgb, var(--cp-warning) 50%, var(--cp-text))' }}><Bookmark size={12} />{t('homestation.withdrawn.bookmarkKept', 'Your bookmark is kept.')}</p>
        ) : null}
        {footer}
      </div>
    </article>
  )
}

export function PublishStatus({ task, onRetry }: { task?: PublishTask; onRetry?: () => void }) {
  const { t } = useI18n()
  if (!task) return null
  if (task.stage === 'uploading') {
    return <span className="hs-badge" data-testid="hs-publish-status" data-stage="uploading"><Loader2 size={11} className="animate-spin" />{t('homestation.publish.uploading', 'Uploading…')}</span>
  }
  if (task.stage === 'failed') {
    return (
      <span className="inline-flex items-center gap-1.5" data-testid="hs-publish-status" data-stage="failed">
        <span className="hs-badge is-danger"><CircleAlert size={11} />{t('homestation.publish.failed', 'Not published')}</span>
        {onRetry ? <button type="button" className="hs-badge is-accent" onClick={onRetry}><RotateCw size={11} />{t('common.retry', 'Retry')}</button> : null}
      </span>
    )
  }
  const delivery = task.delivery
  return (
    <span className="inline-flex flex-wrap items-center gap-1.5" data-testid="hs-publish-status" data-stage="published" data-delivery={delivery?.state}>
      <span className="hs-badge is-success"><CheckCircle2 size={11} />{t('homestation.publish.published', 'Published')}</span>
      {delivery?.state === 'delivering' ? <span className="hs-badge"><Loader2 size={11} className="animate-spin" />{t('homestation.publish.delivering', 'Delivering {{done}}/{{total}}', { done: delivery.delivered, total: delivery.total })}</span> : null}
      {delivery?.state === 'delivered' ? <span className="hs-badge">{t('homestation.publish.delivered', 'Delivered {{done}}/{{total}}', { done: delivery.delivered, total: delivery.total })}</span> : null}
      {delivery?.state === 'partially_failed' ? (
        <>
          <span className="hs-badge is-warning">{t('homestation.publish.partial', 'Delivered {{done}}/{{total}} · some receivers unreachable', { done: delivery.delivered, total: delivery.total })}</span>
          {onRetry ? <button type="button" className="hs-badge is-accent" onClick={onRetry}><RotateCw size={11} />{t('homestation.publish.retryDelivery', 'Retry delivery')}</button> : null}
        </>
      ) : null}
    </span>
  )
}

export function FeedCard({ objId, variant = 'feed', mode = 'standard', footer }: { objId: string; variant?: CardVariant; mode?: ReadingMode; footer?: React.ReactNode }) {
  const { t } = useI18n()
  const nav = useHsNav()
  const view = useCardView(objId)
  const menuRef = useRef<ItemMenuHandle>(null)
  const pressTimer = useRef(0)

  if (!view) return null
  const { item } = view
  const withdrawn = item.entry?.state === 'withdrawn'
  if (withdrawn && variant !== 'published') return <WithdrawnCard view={view} variant={variant} footer={footer} />

  const open = () => nav.openDetail(objId)
  const interactive = variant !== 'visitor' && variant !== 'profile'
  const startPress = (event: React.PointerEvent) => {
    if (event.pointerType !== 'touch' || !interactive) return
    window.clearTimeout(pressTimer.current)
    pressTimer.current = window.setTimeout(() => menuRef.current?.open(), 550)
  }
  const cancelPress = () => window.clearTimeout(pressTimer.current)

  return (
    <article
      className="hs-card"
      data-testid="hs-card"
      data-objid={objId}
      data-content-type={item.contentType}
      onPointerDown={startPress}
      onPointerUp={cancelPress}
      onPointerMove={cancelPress}
      onPointerCancel={cancelPress}
      onContextMenu={event => {
        if (!interactive) return
        event.preventDefault()
        menuRef.current?.open()
      }}
    >
      <CardHeader item={item} trailing={interactive ? <ItemMenu ref={menuRef} view={view} allowOwnerActions={variant === 'published'} /> : null} />
      <ContentBody view={view} mode={mode} onOpen={open} />
      {item.embedded ? <EmbeddedCard embedded={item.embedded} onOpen={nav.openDetail} /> : null}
      {variant === 'feed' ? <TagRow reading={view.reading} /> : null}
      {variant === 'feed' ? <ReasonLine view={view} /> : null}
      {view.stats?.claimed?.likes && variant !== 'published' ? (
        <p className="mt-1.5 flex flex-wrap gap-1.5 text-[11px]" style={{ color: 'var(--cp-muted)' }} data-testid="hs-claimed">
          <span className="hs-badge">{t('homestation.stats.claimedLikes', 'Author claims {{n}} likes', { n: formatCount(view.stats.claimed.likes) })}</span>
          <span className="hs-badge is-success">{t('homestation.stats.verifiedLikes', '{{n}} verified locally', { n: formatCount(view.stats.likes) })}</span>
        </p>
      ) : null}
      {variant === 'feed' || variant === 'saved' ? <ActionBar view={view} onComment={open} /> : null}
      {(variant === 'published' || variant === 'profile' || variant === 'visitor') && item.contentType !== 'reaction' && item.object.comment_type !== 'repost' ? <ReadOnlyCounts view={view} /> : null}
      {footer}
    </article>
  )
}
