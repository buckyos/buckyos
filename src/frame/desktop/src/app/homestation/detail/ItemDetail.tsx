import { FileText, History, Loader2, Lock, RefreshCw, Trash2 } from 'lucide-react'
import { useMemo } from 'react'
import useSWR from 'swr'
import { ContentPreview } from '../../../components/ContentPreview'
import type { PreviewSessionContext } from '../../../components/preview/types'
import { useI18n } from '../../../i18n/provider'
import { LinkifiedText, MessageMarkdown } from '../../../components/conversation/history/MessageMarkdown'
import { ActionBar, ReadOnlyCounts } from '../card/ActionBar'
import { ItemMenu } from '../card/ItemMenu'
import { AudioBlock, CardHeader, EmbeddedCard, LinkPreview, ReasonLine, ResourceOverlay, TagRow } from '../card/parts'
import { formatCount } from '../datamodel/format'
import type { CardView } from '../datamodel/types'
import { useItemActions } from '../actions'
import { fileSourceRef } from '../media'
import { MediaThumb } from '../MediaThumb'
import { useHsNav } from '../navContext'
import { useCardView, useHomeStationStore } from '../store/context'
import { ColumnBody, EmptyState, PageHeader } from '../ui/primitives'
import { CommentSection } from './CommentSection'

function VersionBanner({ view }: { view: CardView }) {
  const { t } = useI18n()
  const nav = useHsNav()
  const entry = view.item.entry
  if (!entry) return null
  if (entry.state === 'withdrawn') {
    return (
      <div className="hs-note is-danger mb-4" data-testid="hs-version-banner" data-state="withdrawn">
        <Trash2 size={16} />
        {t('homestation.version.withdrawnBanner', 'The author withdrew this post. You are looking at a copy your HomeStation kept; it is no longer current and isn’t recommended.')}
      </div>
    )
  }
  if (!entry.isLatest && entry.currentObjId) {
    return (
      <div className="hs-note is-warning mb-4 flex-wrap" data-testid="hs-version-banner" data-state="outdated">
        <History size={16} />
        <span className="min-w-0 flex-1">{t('homestation.version.outdatedBanner', 'You’re viewing version {{n}} of {{total}}. The author published a newer version; comments and likes here stay on this version.', { n: entry.version, total: entry.versionCount })}</span>
        <button type="button" className="hs-link" onClick={() => nav.openDetail(entry.currentObjId!)} data-testid="hs-open-latest">{t('homestation.version.openLatest', 'View latest')}</button>
      </div>
    )
  }
  if (entry.versionCount > 1) {
    return <p className="mb-4 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.version.latestOf', 'Latest version ({{n}} of {{total}}). Earlier versions keep their own comments.', { n: entry.version, total: entry.versionCount })}</p>
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
    <div className="mt-4" data-testid="hs-article-body" data-state={state}>
      {content?.title ? <h1 className="text-[24px] font-semibold leading-8">{content.title}</h1> : null}
      {content?.summary ? <p className="mt-2 text-[17px] leading-7" style={{ color: 'var(--hs-text-2)' }}>{content.summary}</p> : null}
      {view.item.cover ? <MediaThumb objId={view.item.cover.object} file={view.item.cover.file} className="hs-media mt-4 w-full rounded-xl object-cover" style={{ aspectRatio: '16 / 9' }} /> : null}
      <div className="mt-6">
        {state === 'loading' ? (
          <p className="flex items-center gap-2 text-[15px]" style={{ color: 'var(--cp-muted)' }}><Loader2 size={16} className="animate-spin" />{t('homestation.article.loading', 'Fetching the wrapped text by its object ID…')}</p>
        ) : null}
        {state === 'preparing' ? (
          <div className="hs-note flex-wrap items-center">
            <Loader2 size={16} className="animate-spin" />
            <span className="min-w-0 flex-1">{t('homestation.article.preparing', 'Preparing the full text…')}</span>
            <button type="button" className="hs-btn is-small" onClick={() => void body.mutate()} data-testid="hs-article-retry"><RefreshCw size={14} />{t('homestation.article.check', 'Check again')}</button>
          </div>
        ) : null}
        {state === 'unavailable' ? (
          <div className="hs-note is-warning flex-wrap items-center">
            <FileText size={16} />
            <span className="min-w-0 flex-1">{t('homestation.article.unavailable', 'The full text isn’t available from any known holder right now.')}</span>
            <button type="button" className="hs-btn is-small" onClick={() => void body.mutate()}><RefreshCw size={14} />{t('common.retry', 'Retry')}</button>
          </div>
        ) : null}
        {body.data?.state === 'ready' ? (
          <div className="hs-article text-[16px] leading-7">
            <MessageMarkdown text={body.data.markdown.replace(/^#\s+.*\n+/, '')} />
            <p className="mt-6 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.article.fileInfo', 'Body: {{name}} · {{mime}} · verified by object ID', { name: body.data.file.name ?? '', mime: body.data.file.mime ?? '' })}</p>
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
    <div className="hs-media mt-3 rounded-xl" style={{ height: 'min(60vh, 520px)' }}>
      <ContentPreview source={fileSourceRef(first.object, first.file, first.alt)} session={session} uiMode="auto" fitMode="contain" data-testid="hs-image-preview" />
    </div>
  )
}

function VideoPlayer({ view }: { view: CardView }) {
  const { item } = view
  const resources = view.resources
  const ready = resources === 'local' || resources === 'reachable'
  return (
    <div className="hs-media relative mt-3 w-full rounded-xl" style={{ aspectRatio: '16 / 9', background: '#000' }} data-testid="hs-video-player" data-resources={resources}>
      {ready && item.object.wraps ? (
        <ContentPreview source={fileSourceRef(item.object.wraps, item.wrappedFile, item.object.content?.title)} uiMode="auto" fitMode="contain" />
      ) : (
        <>
          {item.cover ? <MediaThumb objId={item.cover.object} file={item.cover.file} className="absolute inset-0 h-full w-full object-cover opacity-50" /> : null}
          <div className="absolute inset-0 flex items-center justify-center">
            <ResourceOverlay state={ready ? 'unavailable' : resources} objId={item.objId} kind="video" />
          </div>
        </>
      )}
    </div>
  )
}

function DetailBody({ view }: { view: CardView }) {
  const actions = useItemActions()
  const { item } = view
  const content = item.object.content
  const text = content?.text ? <p className="mt-3 whitespace-pre-line text-[16px] leading-6" style={{ overflowWrap: 'anywhere' }}><LinkifiedText text={content.text} /></p> : null
  switch (item.contentType) {
    case 'article':
      return <ArticleBody view={view} />
    case 'image':
      return <>{text}<ImageGallery view={view} /></>
    case 'video':
      return <>{content?.title ? <h1 className="mt-4 text-[20px] font-semibold leading-7">{content.title}</h1> : null}<VideoPlayer view={view} />{text}</>
    case 'audio':
      return <>{text}<AudioBlock item={item} resources={view.resources} /></>
    case 'link':
      return <>{text}<LinkPreview item={item} onOpenLink={actions.openLink} /></>
    case 'product':
      return (
        <>
          {content?.title ? <h1 className="mt-4 text-[20px] font-semibold leading-7">{content.title}</h1> : null}
          {content?.summary ? <p className="mt-2 text-[16px] leading-6" style={{ color: 'var(--hs-text-2)' }}>{content.summary}</p> : null}
          <ImageGallery view={view} />
          {item.object.link ? <button type="button" className="hs-btn is-primary mt-4" onClick={() => actions.openLink(item.object.link!)} data-testid="hs-product-open">{item.object.link.replace(/^https?:\/\//, '')}</button> : null}
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
        <ColumnBody><EmptyState icon={<Lock size={32} />} title={t('homestation.detail.notVisible', 'This item isn’t visible to you')} body={t('homestation.detail.notVisibleBody', 'It may be shared with a restricted audience, or no known source has it.')} /></ColumnBody>
      </div>
    )
  }
  const restricted = view.item.audience.restricted
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-detail" data-objid={objId}>
      <PageHeader title={titleOf(view)} onBack={nav.back} backLabel={t('common.back', 'Back')} actions={owner ? <ItemMenu view={view} allowOwnerActions={view.item.isOwn} /> : undefined} />
      <ColumnBody>
        <div className={nav.isDesktop ? 'px-6 pb-6 pt-5' : 'px-4 pb-6 pt-4'}>
          <VersionBanner view={view} />
          {owner ? <ReasonLine view={view} /> : null}
          <CardHeader item={view.item} />
          <DetailBody view={view} />
          {view.item.embedded ? <EmbeddedCard embedded={view.item.embedded} onOpen={nav.openDetail} /> : null}
          {owner ? <TagRow reading={view.reading} /> : null}
          {restricted && !view.item.isOwn ? (
            <p className="mt-4 flex items-start gap-2 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }} data-testid="hs-restricted-note">
              <Lock size={14} className="mt-[3px] flex-shrink-0" />
              {t('homestation.detail.restrictedNote', 'Restricted audience: it can’t be reposted or quoted, and your comments and likes go only to the author.')}
            </p>
          ) : null}
          {view.stats?.claimed?.likes ? (
            <p className="mt-2 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.stats.claimedDetail', 'The author’s page shows {{claimed}} likes. That number is the author’s claim; your HomeStation verified {{verified}} like records.', { claimed: formatCount(view.stats.claimed.likes), verified: formatCount(view.stats.likes) })}</p>
          ) : null}
          {owner ? <ActionBar view={view} onComment={() => document.querySelector('[data-testid="hs-comment-form"] textarea')?.scrollIntoView({ block: 'center' })} /> : <ReadOnlyCounts view={view} />}
        </div>
        <CommentSection key={objId} view={view} />
      </ColumnBody>
    </div>
  )
}
