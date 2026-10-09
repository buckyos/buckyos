import { FileText, History, Loader2, Lock, Pause, Play, RefreshCw, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import useSWR from 'swr'
import { ContentPreview } from '../../../components/ContentPreview'
import type { PreviewSessionContext } from '../../../components/preview/types'
import { useI18n } from '../../../i18n/provider'
import { MessageMarkdown } from '../../messagehub/conversation/history/MessageMarkdown'
import { ActionBar, ReadOnlyCounts } from '../card/ActionBar'
import { ItemMenu } from '../card/ItemMenu'
import { AudioBlock, CardHeader, EmbeddedCard, LinkPreview, ReasonLine, ResourceOverlay, TagRow } from '../card/parts'
import { formatCount, formatDuration } from '../datamodel/format'
import type { CardView } from '../datamodel/types'
import { useItemActions } from '../actions'
import { fileSourceRef, mediaUrl } from '../mock/media'
import { useHsNav } from '../navContext'
import { useCardView, useHomeStationStore } from '../store/context'
import { EmptyState, PageHeader } from '../ui/primitives'
import { CommentSection } from './CommentSection'

function VersionBanner({ view }: { view: CardView }) {
  const { t } = useI18n()
  const nav = useHsNav()
  const entry = view.item.entry
  if (!entry) return null
  if (entry.state === 'withdrawn') {
    return (
      <div className="mb-3 flex items-start gap-2 rounded-xl px-3 py-2 text-xs leading-5" style={{ background: 'color-mix(in srgb, var(--cp-danger) 12%, transparent)' }} data-testid="hs-version-banner" data-state="withdrawn">
        <Trash2 size={14} className="mt-0.5 flex-shrink-0" />
        {t('homestation.version.withdrawnBanner', 'The author withdrew this post. You are looking at a copy your HomeStation kept; it is no longer current and isn’t recommended.')}
      </div>
    )
  }
  if (!entry.isLatest && entry.currentObjId) {
    return (
      <div className="mb-3 flex flex-wrap items-center gap-2 rounded-xl px-3 py-2 text-xs leading-5" style={{ background: 'color-mix(in srgb, var(--cp-warning) 16%, transparent)' }} data-testid="hs-version-banner" data-state="outdated">
        <History size={14} />
        <span className="flex-1">{t('homestation.version.outdatedBanner', 'You’re viewing version {{n}} of {{total}}. The author published a newer version; comments and likes here stay on this version.', { n: entry.version, total: entry.versionCount })}</span>
        <button type="button" className="hs-badge is-accent" onClick={() => nav.openDetail(entry.currentObjId!)} data-testid="hs-open-latest">{t('homestation.version.openLatest', 'View latest')}</button>
      </div>
    )
  }
  if (entry.versionCount > 1) {
    return <p className="mb-3 text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.version.latestOf', 'Latest version ({{n}} of {{total}}). Earlier versions keep their own comments.', { n: entry.version, total: entry.versionCount })}</p>
  }
  return null
}

function ArticleBody({ view }: { view: CardView }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const body = useSWR(['hs-body', store.id, view.item.objId], () => store.getWrappedBody(view.item.objId), { revalidateOnFocus: false, shouldRetryOnError: false })
  const content = view.item.object.content
  const state = body.isLoading ? 'loading' : body.data?.state ?? 'unavailable'
  return (
    <div className="mt-3" data-testid="hs-article-body" data-state={state}>
      {content?.title ? <h1 className="text-2xl font-bold leading-tight">{content.title}</h1> : null}
      {content?.summary ? <p className="mt-2 text-sm italic leading-relaxed" style={{ color: 'var(--cp-muted)' }}>{content.summary}</p> : null}
      {view.item.cover ? <img src={mediaUrl(view.item.cover.object, view.item.cover.file, content?.title)} alt="" className="mt-4 w-full rounded-2xl object-cover" style={{ aspectRatio: '16 / 9' }} /> : null}
      <div className="mt-4">
        {state === 'loading' ? (
          <p className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-muted)' }}><Loader2 size={14} className="animate-spin" />{t('homestation.article.loading', 'Fetching the wrapped text by its object ID…')}</p>
        ) : null}
        {state === 'preparing' ? (
          <div className="flex flex-wrap items-center gap-2 rounded-xl px-3 py-2 text-sm" style={{ background: 'var(--hs-subtle-bg)' }}>
            <Loader2 size={14} className="animate-spin" />
            <span className="flex-1">{t('homestation.article.preparing', 'Preparing the full text…')}</span>
            <button type="button" className="hs-btn" onClick={() => void body.mutate()} data-testid="hs-article-retry"><RefreshCw size={13} />{t('homestation.article.check', 'Check again')}</button>
          </div>
        ) : null}
        {state === 'unavailable' ? (
          <div className="flex flex-wrap items-center gap-2 rounded-xl px-3 py-2 text-sm" style={{ background: 'color-mix(in srgb, var(--cp-warning) 14%, transparent)' }}>
            <FileText size={14} />
            <span className="flex-1">{t('homestation.article.unavailable', 'The full text isn’t available from any known holder right now.')}</span>
            <button type="button" className="hs-btn" onClick={() => void body.mutate()}><RefreshCw size={13} />{t('common.retry', 'Retry')}</button>
          </div>
        ) : null}
        {body.data?.state === 'ready' ? (
          <div className="hs-article text-[15px] leading-7">
            <MessageMarkdown text={body.data.markdown.replace(/^#\s+.*\n+/, '')} />
            <p className="mt-6 text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.article.fileInfo', 'Body: {{name}} · {{mime}} · verified by object ID', { name: body.data.file.name, mime: body.data.file.meta.mime })}</p>
          </div>
        ) : null}
      </div>
    </div>
  )
}

function ImageGallery({ view }: { view: CardView }) {
  const session = useMemo<PreviewSessionContext>(() => ({
    kind: 'list',
    sessionId: `hs-${view.item.objId}`,
    items: view.item.media.map(part => ({ id: part.object, source: fileSourceRef(part.object, part.file, part.alt), title: part.alt })),
    currentIndex: 0,
    navigation: 'bounded',
  }), [view.item.media, view.item.objId])
  const first = view.item.media[0]
  if (!first) return null
  return (
    <div className="mt-3 overflow-hidden rounded-2xl" style={{ height: 'min(60vh, 480px)', background: 'var(--hs-subtle-bg)' }}>
      <ContentPreview source={fileSourceRef(first.object, first.file, first.alt)} session={session} uiMode="auto" fitMode="contain" data-testid="hs-image-preview" />
    </div>
  )
}

function VideoPlayer({ view }: { view: CardView }) {
  const { t } = useI18n()
  const [playing, setPlaying] = useState(false)
  const resources = view.resources
  const ready = resources === 'local' || resources === 'reachable'
  const duration = view.item.wrappedFile?.meta.duration_ms
  return (
    <div className="relative mt-3 w-full overflow-hidden rounded-2xl" style={{ aspectRatio: '16 / 9', background: '#000' }} data-testid="hs-video-player" data-resources={resources}>
      {view.item.cover ? <img src={mediaUrl(view.item.cover.object, view.item.cover.file, view.item.object.content?.title)} alt="" className="absolute inset-0 h-full w-full object-cover" style={{ opacity: playing ? 0.35 : ready ? 1 : 0.45 }} /> : null}
      <div className="absolute inset-0 flex items-center justify-center">
        {ready ? (
          <button type="button" className="flex h-16 w-16 items-center justify-center rounded-full" style={{ background: 'rgba(0,0,0,0.55)', color: 'white' }} aria-label={playing ? t('homestation.media.pause', 'Pause') : t('homestation.media.playVideo', 'Play video')} onClick={() => setPlaying(value => !value)} data-testid="hs-play">
            {playing ? <Pause size={28} /> : <Play size={28} fill="white" />}
          </button>
        ) : (
          <ResourceOverlay state={resources} objId={view.item.objId} kind="video" />
        )}
      </div>
      {playing ? (
        <div className="absolute inset-x-0 bottom-0 px-3 pb-2">
          <p className="mb-1 text-[11px]" style={{ color: 'rgba(255,255,255,0.75)' }}>{t('homestation.media.simulated', 'Prototype playback — no real video bytes')}</p>
          <div className="h-1 overflow-hidden rounded-full" style={{ background: 'rgba(255,255,255,0.25)' }}><div className="hs-playing-bar h-full" style={{ background: 'var(--cp-accent)' }} /></div>
        </div>
      ) : ready && duration ? (
        <span className="absolute bottom-3 right-3 rounded px-2 py-0.5 text-xs font-medium" style={{ background: 'rgba(0,0,0,0.7)', color: 'white' }}>{formatDuration(duration)}</span>
      ) : null}
    </div>
  )
}

function DetailBody({ view }: { view: CardView }) {
  const actions = useItemActions()
  const { item } = view
  const content = item.object.content
  const text = content?.text ? <p className="mt-3 whitespace-pre-line text-[15px] leading-relaxed">{content.text}</p> : null
  switch (item.contentType) {
    case 'article':
      return <ArticleBody view={view} />
    case 'image':
      return <>{text}<ImageGallery view={view} /></>
    case 'video':
      return <>{content?.title ? <h1 className="mt-3 text-xl font-bold leading-snug">{content.title}</h1> : null}<VideoPlayer view={view} />{text}</>
    case 'audio':
      return <>{text}<AudioBlock item={item} resources={view.resources} /></>
    case 'link':
      return <>{text}<LinkPreview item={item} onOpenLink={actions.openLink} /></>
    case 'product':
      return (
        <>
          {content?.title ? <h1 className="mt-3 text-xl font-bold leading-snug">{content.title}</h1> : null}
          {content?.summary ? <p className="mt-2 text-sm leading-relaxed">{content.summary}</p> : null}
          <ImageGallery view={view} />
          {item.object.link ? <button type="button" className="hs-btn is-primary mt-3" onClick={() => actions.openLink(item.object.link!)} data-testid="hs-product-open">{item.object.link.replace(/^https?:\/\//, '')}</button> : null}
        </>
      )
    default:
      return text
  }
}

export function ItemDetail({ objId }: { objId: string }) {
  const { t } = useI18n()
  const nav = useHsNav()
  const view = useCardView(objId)
  const owner = nav.perspective === 'owner'
  const titleOf = (card: CardView) => {
    switch (card.item.contentType) {
      case 'article':
        return t('homestation.detail.article', 'Article')
      case 'image':
        return t('homestation.detail.images', 'Images')
      case 'video':
        return t('homestation.detail.video', 'Video')
      case 'comment':
        return t('homestation.detail.comment', 'Comment')
      default:
        return t('homestation.detail.post', 'Post')
    }
  }
  if (!view) {
    return (
      <div className="flex h-full flex-col">
        <PageHeader title={t('homestation.detail.post', 'Post')} onBack={nav.back} backLabel={t('common.back', 'Back')} />
        <EmptyState icon={<Lock size={32} strokeWidth={1.4} />} title={t('homestation.detail.notVisible', 'This item isn’t visible to you')} body={t('homestation.detail.notVisibleBody', 'It may be shared with a restricted audience, or no known source has it.')} />
      </div>
    )
  }
  const restricted = view.item.audience.restricted
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-detail" data-objid={objId}>
      <PageHeader title={titleOf(view)} onBack={nav.back} backLabel={t('common.back', 'Back')} actions={owner ? <ItemMenu view={view} allowOwnerActions={view.item.isOwn} /> : undefined} />
      <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-2xl px-4 py-4">
          <VersionBanner view={view} />
          <CardHeader item={view.item} />
          <DetailBody view={view} />
          {view.item.embedded ? <EmbeddedCard embedded={view.item.embedded} onOpen={nav.openDetail} /> : null}
          {owner ? <TagRow reading={view.reading} /> : null}
          {owner ? <ReasonLine view={view} /> : null}
          {restricted && !view.item.isOwn ? (
            <p className="mt-3 flex items-start gap-1.5 text-xs leading-5" style={{ color: 'var(--cp-muted)' }} data-testid="hs-restricted-note">
              <Lock size={13} className="mt-0.5 flex-shrink-0" />
              {t('homestation.detail.restrictedNote', 'Restricted audience: it can’t be reposted or quoted, and your comments and likes go only to the author.')}
            </p>
          ) : null}
          {view.stats?.claimed?.likes ? (
            <p className="mt-2 text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.stats.claimedDetail', 'The author’s page shows {{claimed}} likes. That number is the author’s claim; your HomeStation verified {{verified}} like records.', { claimed: formatCount(view.stats.claimed.likes), verified: formatCount(view.stats.likes) })}</p>
          ) : null}
          {owner ? <ActionBar view={view} onComment={() => document.querySelector('[data-testid="hs-comment-form"] textarea')?.scrollIntoView({ block: 'center' })} /> : <ReadOnlyCounts view={view} />}
          <CommentSection key={objId} view={view} />
        </div>
      </div>
    </div>
  )
}
