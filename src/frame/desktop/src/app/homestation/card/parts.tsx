import { ObjectAttachment } from './ObjectAttachment'
import clsx from 'clsx'
import {
  ExternalLink,
  FileText,
  Globe,
  Headphones,
  History,
  Loader2,
  Lock,
  Pause,
  Play,
  RefreshCw,
  Shield,
  ShieldAlert,
  ShieldCheck,
  ShoppingBag,
  TriangleAlert,
  VideoOff,
} from 'lucide-react'
import { useCallback, useState } from 'react'
import { LinkifiedText } from '../../../components/conversation/history/MessageMarkdown'
import { useI18n } from '../../../i18n/provider'
import { useItemActions } from '../actions'
import { audienceLabel, formatDuration, formatTimeAgo, restrictedLabel } from '../datamodel/format'
import { actionText, GENERATION_TAGS, reasonLabel, tagLabel } from './labels'
import type { CardView, EffectiveTag, EmbeddedView, FeedItemView, ReadingEntry, ResolvedMedia, ResourceState, Verification } from '../datamodel/types'
import { MediaThumb } from '../MediaThumb'
import type { HomeStationStore } from '../store/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useNow, useStoreSelector } from '../store/context'
import type { ReadingMode } from '../types'
import { Avatar } from '../ui/primitives'
import { ReasonDetail } from './ReasonDetail'
import { Popover } from '../ui/Popover'

const ON_MEDIA = 'rgba(0,0,0,0.6)'

export function VerificationBadge({ verification }: { verification: Verification }) {
  const { t } = useI18n()
  if (verification === 'verified') {
    const label = t('homestation.verify.verified', 'Signature verified')
    return <span title={label} aria-label={label} role="img" className="inline-flex flex-shrink-0" style={{ color: 'var(--hs-text-2)' }}><ShieldCheck size={14} /></span>
  }
  if (verification === 'unverified') {
    return (
      <span className="hs-badge is-warning" title={t('homestation.verify.unverifiedHint', 'The signature could not be checked yet. Treat the publisher as unconfirmed.')}>
        <ShieldAlert size={14} />
        {t('homestation.verify.unverified', 'Unverified')}
      </span>
    )
  }
  const label = t('homestation.verify.wrapperOnly', 'Signed by the sharer; the original author did not sign')
  return <span title={label} aria-label={label} role="img" className="inline-flex flex-shrink-0" style={{ color: 'var(--cp-muted)' }}><Shield size={14} /></span>
}

const selectGroups = (store: HomeStationStore) => store.peekGroups()

export function AudienceBadge({ item }: { item: FeedItemView }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const groups = useStoreSelector(selectGroups)
  if (item.isPrivateCapture) {
    return <span className="hs-badge" data-testid="hs-audience-badge"><Lock size={12} />{t('homestation.audience.privateCapture', 'Only you · private capture')}</span>
  }
  if (!item.audience.restricted) return null
  const label = item.isOwn ? audienceLabel(t, item.audience.spec, groups, did => store.peekIdentity(did).name) : restrictedLabel(t, item.audience.spec)
  return <span className="hs-badge" data-testid="hs-audience-badge"><Lock size={12} />{label}</span>
}

export function VersionBadge({ item, onOpen }: { item: FeedItemView; onOpen: (objId: string) => void }) {
  const { t } = useI18n()
  const entry = item.entry
  if (!entry) return null
  if (entry.state === 'withdrawn') return <span className="hs-badge is-danger">{t('homestation.version.withdrawn', 'Withdrawn')}</span>
  if (entry.state === 'conflict') return <span className="hs-badge is-warning">{t('homestation.version.conflict', 'Conflicting versions')}</span>
  if (!entry.isLatest && entry.currentObjId) {
    return (
      <button
        type="button"
        className="hs-badge"
        data-testid="hs-updated-badge"
        title={t('homestation.version.updatedHint', 'The author published a newer version. Your interactions stay on this one.')}
        onClick={event => {
          event.stopPropagation()
          onOpen(entry.currentObjId!)
        }}
      >
        <History size={14} />
        {t('homestation.version.updated', 'Updated · view latest')}
      </button>
    )
  }
  if (entry.versionCount > 1) return <span className="hs-badge">{t('homestation.version.edited', 'Edited · v{{n}}', { n: entry.version })}</span>
  return null
}

export function CardAvatar({ item, size = 36 }: { item: FeedItemView; size?: number }) {
  if (item.isPrivateCapture) {
    return <span className="flex flex-shrink-0 items-center justify-center rounded-full" style={{ width: size, height: size, background: 'var(--hs-chip-bg)', color: 'var(--cp-muted)' }}><Globe size={Math.round(size / 2)} /></span>
  }
  return <Avatar identity={item.publisher} size={size} />
}

/** Name, signature, what they did and when, on one line; the capture line under it. */
export function CardByline({ item, trailing }: { item: FeedItemView; trailing?: React.ReactNode }) {
  const { t } = useI18n()
  const now = useNow()
  const nav = useHsNav()
  const action = actionText(t, item)
  const name = item.isPrivateCapture ? item.capturedFrom ?? item.publisher.name : item.publisher.name
  return (
    <div className="flex items-start gap-2">
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 text-[15px] leading-[22px]">
          <span className="inline-flex min-w-0 items-center gap-1">
            <span className="truncate font-semibold" data-testid="hs-card-publisher">{name}</span>
            <VerificationBadge verification={item.verification} />
          </span>
          {action ? <span style={{ color: 'var(--cp-muted)' }}>{action}</span> : null}
          <span className="tabular-nums" style={{ color: 'var(--cp-muted)' }}>{formatTimeAgo(t, item.createdAt, now)}</span>
          <AudienceBadge item={item} />
          <VersionBadge item={item} onOpen={nav.openDetail} />
        </div>
        {item.isCapture ? (
          <p className="truncate text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }} data-testid="hs-capture-line">
            {item.isPrivateCapture
              ? t('homestation.capture.private', 'Captured by your Spider · original author {{author}}', { author: item.originalAuthor ?? item.capturedFrom ?? '' })
              : t('homestation.capture.shared', 'Shared by {{sharer}} · captured from {{host}} · original author {{author}}', { sharer: item.publisher.name, host: item.capturedFrom ?? '', author: item.originalAuthor ?? '' })}
          </p>
        ) : null}
      </div>
      {trailing}
    </div>
  )
}

/** Avatar and byline side by side, for the one post a detail page is about. */
export function CardHeader({ item, trailing }: { item: FeedItemView; trailing?: React.ReactNode }) {
  return (
    <div className="flex items-center gap-3">
      <CardAvatar item={item} size={40} />
      <div className="min-w-0 flex-1"><CardByline item={item} trailing={trailing} /></div>
    </div>
  )
}

function ratioOf(part: ResolvedMedia, fallback: number) {
  const width = part.file?.width
  const height = part.file?.height
  if (!width || !height) return fallback
  return Math.min(Math.max(width / height, 0.5), 2)
}

/** Pictures at their own proportions: one fits the column, several sit in a row you can swipe. */
export function MediaGrid({ media, onOpen, large = false }: { media: ResolvedMedia[]; onOpen: () => void; large?: boolean }) {
  const { t } = useI18n()
  if (media.length === 0) return null
  if (media.length === 1) {
    const part = media[0]
    const ratio = ratioOf(part, 4 / 3)
    const maxHeight = large ? 560 : 440
    return (
      <button type="button" onClick={onOpen} className="hs-media mt-3 block" style={{ aspectRatio: String(ratio), width: `min(100%, ${Math.round(maxHeight * ratio)}px)` }} aria-label={t('homestation.media.open', 'Open images')} data-testid="hs-media-grid">
        <MediaThumb objId={part.object} file={part.file} alt={part.alt} loading="lazy" className="h-full w-full object-cover" />
      </button>
    )
  }
  const height = large ? 360 : 260
  return (
    <div className="hs-media-strip mt-3" role="group" aria-label={t('homestation.media.open', 'Open images')} data-testid="hs-media-grid">
      {media.map(part => (
        <button key={part.object} type="button" onClick={onOpen} className="hs-media" style={{ height, aspectRatio: String(ratioOf(part, 4 / 5)) }} aria-label={part.alt ?? t('homestation.media.open', 'Open images')}>
          <MediaThumb objId={part.object} file={part.file} alt={part.alt} loading="lazy" className="h-full w-full object-cover" />
        </button>
      ))}
    </div>
  )
}

export function ResourceOverlay({ state, objId, kind }: { state: ResourceState; objId: string; kind: 'video' | 'audio' }) {
  const { t } = useI18n()
  const actions = useItemActions()
  if (state === 'preparing') {
    return (
      <span className="flex items-center gap-2 rounded-full px-3 py-1 text-[13px] font-medium" style={{ background: ON_MEDIA, color: 'white' }} data-testid="hs-resource-state" data-state="preparing">
        <Loader2 size={14} className="animate-spin" />
        {kind === 'video' ? t('homestation.resources.preparingVideo', 'Preparing video…') : t('homestation.resources.preparingAudio', 'Preparing audio…')}
      </span>
    )
  }
  return (
    <span className="flex flex-col items-center gap-2" data-testid="hs-resource-state" data-state="unavailable">
      <span className="flex items-center gap-2 rounded-full px-3 py-1 text-[13px] font-medium" style={{ background: ON_MEDIA, color: 'white' }}>
        <VideoOff size={14} />
        {t('homestation.resources.unavailable', 'Unavailable: no reachable source')}
      </span>
      <button
        type="button"
        className="flex h-8 items-center gap-1 rounded-full px-3 text-[13px] font-medium"
        style={{ background: 'white', color: '#111' }}
        onClick={event => {
          event.stopPropagation()
          void actions.retryResources(objId)
        }}
      >
        <RefreshCw size={14} />
        {t('common.retry', 'Retry')}
      </button>
    </span>
  )
}

export function VideoBlock({ item, resources, onOpen }: { item: FeedItemView; resources: ResourceState; onOpen: () => void }) {
  const { t } = useI18n()
  const ready = resources === 'local' || resources === 'reachable'
  const duration = item.wrappedFile?.duration_ms
  return (
    <div className="hs-media relative mt-3 w-full" style={{ aspectRatio: '16 / 9', background: '#000' }} data-testid="hs-video-block" data-resources={resources}>
      {item.cover ? <MediaThumb objId={item.cover.object} file={item.cover.file} className="absolute inset-0 h-full w-full object-cover" style={{ opacity: ready ? 1 : 0.5 }} /> : null}
      <div className="absolute inset-0 flex items-center justify-center">
        {ready ? (
          <button type="button" onClick={onOpen} className="flex h-14 w-14 items-center justify-center rounded-full transition-transform active:scale-95" style={{ background: ON_MEDIA, color: 'white' }} aria-label={t('homestation.media.playVideo', 'Play video')} data-testid="hs-play">
            <Play size={22} fill="white" />
          </button>
        ) : (
          <ResourceOverlay state={resources} objId={item.objId} kind="video" />
        )}
      </div>
      {ready && duration ? (
        <span className="absolute bottom-2 right-2 rounded-md px-2 text-[12px] font-medium leading-5 tabular-nums" style={{ background: ON_MEDIA, color: 'white' }}>{formatDuration(duration)}</span>
      ) : null}
      {ready && resources === 'reachable' ? (
        <span className="absolute left-2 top-2 rounded-md px-2 text-[12px] font-medium leading-5" style={{ background: ON_MEDIA, color: 'white' }} title={t('homestation.resources.reachableHint', 'Not stored on your HomeStation yet; it streams from a reachable holder.')}>
          {t('homestation.resources.reachable', 'Streams from source')}
        </span>
      ) : null}
    </div>
  )
}

export function AudioBlock({ item, resources }: { item: FeedItemView; resources: ResourceState }) {
  const { t } = useI18n()
  const [playing, setPlaying] = useState(false)
  const part = item.media[0]
  const ready = resources === 'local' || resources === 'reachable'
  const duration = part?.file?.duration_ms
  return (
    <div className="mt-3 flex items-center gap-3 rounded-xl px-3 py-3" style={{ border: '1px solid var(--hs-line)' }} data-testid="hs-audio-block">
      {ready ? (
        <button type="button" className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full transition-transform active:scale-95" style={{ background: 'var(--hs-ink)', color: 'var(--hs-on-ink)' }} aria-label={playing ? t('homestation.media.pause', 'Pause') : t('homestation.media.playAudio', 'Play voice note')} aria-pressed={playing} onClick={event => { event.stopPropagation(); setPlaying(value => !value) }}>
          {playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" />}
        </button>
      ) : (
        <span className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full" style={{ background: 'var(--hs-chip-bg)', color: 'var(--cp-muted)' }}><Headphones size={16} /></span>
      )}
      <div className="flex h-8 flex-1 items-center gap-[3px]" aria-hidden="true">
        {Array.from({ length: 32 }, (_, index) => (
          <span key={index} className="w-[3px] rounded-full" style={{ height: `${20 + ((index * 37) % 70)}%`, background: playing && index < 12 ? 'var(--cp-text)' : 'var(--hs-line-strong)' }} />
        ))}
      </div>
      <span className="text-[13px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>{ready && duration ? formatDuration(duration) : t('homestation.resources.preparingShort', 'Preparing')}</span>
    </div>
  )
}

export function LinkPreview({ item, onOpenLink }: { item: FeedItemView; onOpenLink: (url: string) => void }) {
  const { t } = useI18n()
  const content = item.object.content
  const url = item.object.link
  if (!url) return null
  let host = url
  try {
    host = new URL(url).hostname
  } catch {
    host = url
  }
  return (
    <button
      type="button"
      className="mt-3 flex w-full overflow-hidden rounded-xl text-left transition-colors hover:bg-[var(--hs-subtle-bg)]"
      style={{ border: '1px solid var(--hs-line-strong)' }}
      onClick={event => {
        event.stopPropagation()
        onOpenLink(url)
      }}
      data-testid="hs-link-card"
      aria-label={t('homestation.link.openNamed', 'Open {{host}} in a new tab', { host })}
    >
      {item.cover ? <MediaThumb objId={item.cover.object} file={item.cover.file} className="h-auto w-28 flex-shrink-0 object-cover sm:w-36" style={{ borderRight: '1px solid var(--hs-line)' }} /> : null}
      <span className="min-w-0 flex-1 px-4 py-3">
        <span className="line-clamp-2 block text-[15px] font-semibold leading-[22px]">{content?.title}</span>
        {content?.summary ? <span className="mt-1 line-clamp-2 block text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{content.summary}</span> : null}
        <span className="mt-1 flex items-center gap-1 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}><ExternalLink size={12} />{host}</span>
      </span>
    </button>
  )
}

export function ContentBody({ view, mode = 'standard', onOpen, clampText = true }: { view: CardView; mode?: ReadingMode; onOpen: () => void; clampText?: boolean }) {
  const { t } = useI18n()
  const actions = useItemActions()
  const { item } = view
  const content = item.object.content
  const resources = view.resources
  const clamp = !clampText ? '' : mode === 'longform' ? 'line-clamp-6' : mode === 'image' ? 'line-clamp-1' : 'line-clamp-4'
  const text = content?.text
  const title = content?.title
  const textBlock = text ? (
    <p className={clsx('hs-post-text mt-1 cursor-pointer whitespace-pre-line', clamp)} onClick={event => { if (!(event.target as Element).closest('a')) onOpen() }}>
      <LinkifiedText text={text} />
    </p>
  ) : null
  const heading = (value: string, clickable: boolean) => <h3 className={clsx('mt-1 text-[17px] font-semibold leading-6', clickable && 'cursor-pointer')} onClick={clickable ? onOpen : undefined}>{value}</h3>

  switch (item.contentType) {
    case 'files':
      return <>{textBlock}<ul className="mt-3 space-y-2">{(content?.media ?? []).map(part => <li key={part.object}><ObjectAttachment objId={part.object} name={part.alt ?? part.object} /></li>)}</ul></>
    case 'image':
      return (
        <>
          {textBlock}
          {mode !== 'longform' ? <MediaGrid media={item.media} onOpen={onOpen} large={mode === 'image'} /> : null}
        </>
      )
    case 'video':
      return (
        <>
          {title ? heading(title, true) : null}
          {textBlock}
          {mode !== 'longform' ? <VideoBlock item={item} resources={resources} onOpen={onOpen} /> : null}
        </>
      )
    case 'audio':
      return (
        <>
          {textBlock}
          <AudioBlock item={item} resources={resources} />
        </>
      )
    case 'article':
      return (
        <div className="cursor-pointer" onClick={onOpen}>
          {item.cover && mode === 'image' ? <MediaThumb objId={item.cover.object} file={item.cover.file} className="hs-media mt-3 w-full object-cover" style={{ aspectRatio: '16 / 9' }} /> : null}
          {title ? heading(title, false) : null}
          {content?.summary ? <p className={clsx('hs-post-text mt-1', mode === 'longform' ? 'line-clamp-6' : 'line-clamp-3')} style={{ color: 'var(--hs-text-2)' }}>{content.summary}</p> : null}
          <span className="mt-2 inline-flex items-center gap-1 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}><FileText size={14} />{t('homestation.article.readFull', 'Long read')}</span>
        </div>
      )
    case 'link':
      return (
        <>
          {textBlock}
          <LinkPreview item={item} onOpenLink={actions.openLink} />
        </>
      )
    case 'product':
      return (
        <>
          {title ? heading(title, false) : null}
          {content?.summary ? <p className="hs-post-text mt-1" style={{ color: 'var(--hs-text-2)' }}>{content.summary}</p> : null}
          {mode !== 'longform' && item.media.length > 0 ? <MediaGrid media={item.media} onOpen={() => item.object.link && actions.openLink(item.object.link)} /> : null}
          {item.object.link ? (
            <button type="button" className="hs-btn mt-3" onClick={event => { event.stopPropagation(); actions.openLink(item.object.link!) }} data-testid="hs-product-open">
              <ShoppingBag size={16} />
              {t('homestation.product.open', 'View on the author’s page')}
            </button>
          ) : null}
        </>
      )
    default:
      return textBlock
  }
}

export function EmbeddedCard({ embedded, onOpen }: { embedded: EmbeddedView; onOpen: (objId: string) => void }) {
  const { t } = useI18n()
  const now = useNow()
  if (embedded.visibility !== 'visible' || !embedded.item) {
    const text = embedded.visibility === 'not_visible'
      ? t('homestation.embed.notVisible', 'Original not visible — it was shared with a restricted audience you are not part of.')
      : embedded.visibility === 'withdrawn'
        ? t('homestation.embed.withdrawn', 'Original withdrawn by its author.')
        : t('homestation.embed.missing', 'Original not available on any known source.')
    return (
      <div className="hs-embed mt-3 flex items-center gap-2 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }} data-testid="hs-embed-placeholder" data-visibility={embedded.visibility}>
        {embedded.visibility === 'not_visible' ? <Lock size={14} className="flex-shrink-0" /> : <TriangleAlert size={14} className="flex-shrink-0" />}
        {text}
      </div>
    )
  }
  const inner = embedded.item
  const content = inner.object.content
  const firstImage = inner.media[0] ?? inner.cover
  return (
    <div
      role="button"
      tabIndex={0}
      className="hs-embed mt-3 cursor-pointer"
      data-testid="hs-embed"
      onClick={event => {
        event.stopPropagation()
        onOpen(inner.objId)
      }}
      onKeyDown={event => {
        if (event.key === 'Enter') onOpen(inner.objId)
      }}
    >
      <div className="flex min-w-0 items-center gap-2 text-[13px] leading-5">
        <Avatar identity={inner.isPrivateCapture ? { name: inner.capturedFrom ?? '?', hue: 200 } : inner.publisher} size={20} />
        <span className="inline-flex min-w-0 items-center gap-1">
          <span className="truncate font-semibold">{inner.isPrivateCapture ? inner.capturedFrom : inner.publisher.name}</span>
          <VerificationBadge verification={inner.verification} />
        </span>
        <span className="tabular-nums" style={{ color: 'var(--cp-muted)' }}>{formatTimeAgo(t, inner.createdAt, now)}</span>
        <AudienceBadge item={inner} />
        <VersionBadge item={inner} onOpen={onOpen} />
      </div>
      <div className="mt-2 flex gap-3">
        <div className="min-w-0 flex-1">
          {content?.title ? <p className="line-clamp-2 text-[15px] font-semibold leading-[22px]">{content.title}</p> : null}
          {content?.text || content?.summary ? <p className="line-clamp-3 text-[14px] leading-5" style={{ color: 'var(--hs-text-2)' }}>{content?.text ?? content?.summary}</p> : null}
        </div>
        {firstImage ? <MediaThumb objId={firstImage.object} file={firstImage.file} className="hs-media h-16 w-16 flex-shrink-0 object-cover" /> : null}
      </div>
    </div>
  )
}

export function TagDetail({ tag, objId, onClose }: { tag: EffectiveTag; objId: string; onClose: () => void }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const sourceLabel = tag.source === 'author' ? t('homestation.tags.sourceAuthor', 'Declared by the author') : tag.source === 'model' ? t('homestation.tags.sourceModel', 'Inferred by your local classifier') : t('homestation.tags.sourceUser', 'Corrected by you')
  const apply = (override: 'remove' | 'confirm' | 'to_assisted') => {
    onClose()
    void store.setTagOverride(objId, tag.tag, override)
  }
  return (
    <div className="space-y-3 p-4 text-[13px] leading-5">
      <p className="text-[15px] font-semibold">{tagLabel(t, tag)}</p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
        <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.source', 'Source')}</dt>
        <dd>{sourceLabel}</dd>
        <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.status', 'Status')}</dt>
        <dd>{tag.status === 'inferred' ? t('homestation.tags.statusInferred', 'Inferred, not a fact') : tag.status === 'confirmed' ? t('homestation.tags.statusConfirmed', 'Confirmed') : t('homestation.tags.statusDeclared', 'Declared')}</dd>
        <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.scope', 'Scope')}</dt>
        <dd>{tag.scope === 'whole_content' ? t('homestation.tags.scopeWhole', 'Whole content') : t('homestation.tags.scopePart', 'Part of the content')}</dd>
        {tag.confidence !== undefined ? (
          <>
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.confidence', 'Model score')}</dt>
            <dd className="tabular-nums">{tag.confidence.toFixed(2)} <span style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.confidenceNote', '(a score, not a proof)')}</span></dd>
          </>
        ) : null}
        {tag.basis ? (
          <>
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.basis', 'Basis')}</dt>
            <dd>{tag.basis}</dd>
          </>
        ) : null}
        {tag.classifierRevision ? (
          <>
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.classifier', 'Classifier')}</dt>
            <dd>{tag.classifierRevision}</dd>
          </>
        ) : null}
      </dl>
      <p style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.localNote', 'This tag lives only in your HomeStation. The author’s signed object and its tags are unchanged.')}</p>
      {tag.source !== 'author' ? (
        <div className="flex flex-wrap gap-2 pt-1">
          <button type="button" className="hs-btn is-small" onClick={() => apply('remove')} data-testid="hs-tag-remove">{t('homestation.tags.remove', 'Remove tag')}</button>
          {tag.tag === 'ai_full' ? <button type="button" className="hs-btn is-small" onClick={() => apply('to_assisted')}>{t('homestation.tags.toAssisted', 'Change to AI-assisted')}</button> : null}
          {tag.status !== 'confirmed' ? <button type="button" className="hs-btn is-small" onClick={() => apply('confirm')}>{t('homestation.tags.confirm', 'Confirm')}</button> : null}
        </div>
      ) : null}
    </div>
  )
}

function TagChip({ tag, objId }: { tag: EffectiveTag; objId: string }) {
  const { t } = useI18n()
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const close = useCallback(() => setAnchor(null), [])
  const generation = GENERATION_TAGS.has(tag.tag)
  return (
    <>
      <button
        type="button"
        className={clsx('hs-tag', generation && 'is-generation', tag.source === 'model' && 'is-inferred')}
        title={tag.source === 'model' ? t('homestation.tags.localHint', 'Local classification') : undefined}
        aria-expanded={!!anchor}
        data-testid="hs-tag"
        data-tag={tag.tag}
        onClick={event => {
          event.stopPropagation()
          setAnchor(anchor ? null : event.currentTarget)
        }}
      >
        {tagLabel(t, tag)}
      </button>
      <Popover anchor={anchor} open={!!anchor} onClose={close} align="start" width={320} label={t('homestation.tags.detail', 'Tag details')} testId="hs-tag-detail">
        <TagDetail tag={tag} objId={objId} onClose={close} />
      </Popover>
    </>
  )
}

export function TagRow({ reading, authorTags }: { reading?: ReadingEntry; authorTags?: string[] }) {
  const { t } = useI18n()
  const tags: EffectiveTag[] = reading?.effectiveTags ?? (authorTags ?? []).map(tag => ({ tag, label: tag, source: 'author', status: 'declared', scope: 'whole_content' }))
  if (tags.length === 0 && !reading?.filteredBy.length) return null
  return (
    <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1">
      {reading?.filteredBy.length ? <span className="hs-badge is-warning" data-testid="hs-filtered-badge">{t('homestation.filters.filteredBadge', 'Hidden by your filter · shown on request')}</span> : null}
      {tags.map(tag => <TagChip key={`${tag.tag}-${tag.source}`} tag={tag} objId={reading?.objId ?? ''} />)}
    </div>
  )
}

/** The provenance line: why this item is in your reading list, set above the post like a dateline. */
export function ReasonLine({ view }: { view: CardView }) {
  const { t } = useI18n()
  const actions = useItemActions()
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const close = useCallback(() => setAnchor(null), [])
  const reading = view.reading
  if (!reading) return null
  const person = view.item.publisher.kind === 'person' && !view.item.isOwn && view.item.publisher.did
  return (
    <>
      <button
        type="button"
        className="hs-provenance"
        data-testid="hs-reason"
        data-reason={reading.reason.code}
        aria-expanded={!!anchor}
        onClick={event => {
          event.stopPropagation()
          setAnchor(anchor ? null : event.currentTarget)
        }}
      >
        <span className="hs-provenance-dot" aria-hidden="true"><span /></span>
        <span className="hs-provenance-text truncate">{reasonLabel(t, reading)}</span>
      </button>
      <Popover anchor={anchor} open={!!anchor} onClose={close} align="start" width={340} label={t('homestation.reason.title', 'Why you’re seeing this')} testId="hs-reason-detail">
        <div className="space-y-3 p-4">
          <p className="text-[15px] font-semibold">{t('homestation.reason.title', 'Why you’re seeing this')}</p>
          <ReasonDetail view={view} />
          <div className="flex flex-wrap gap-2 pt-1">
            {person ? (
              <button type="button" className="hs-btn is-small" onClick={() => { close(); void actions.mute({ kind: 'person', did: view.item.publisher.did!, name: view.item.publisher.name }) }}>
                {t('homestation.menu.mutePerson', 'Don’t show {{name}}', { name: view.item.publisher.name })}
              </button>
            ) : null}
            <button type="button" className="hs-btn is-small" onClick={() => { close(); void actions.lessLike(view) }}>{t('homestation.menu.lessLike', 'Show less like this')}</button>
          </div>
        </div>
      </Popover>
    </>
  )
}

export function CountScope({ view }: { view: CardView }) {
  const { t } = useI18n()
  if (!view.stats) return null
  return (
    <span className="ml-auto hidden items-center text-[13px] sm:inline-flex" style={{ color: 'var(--cp-muted)' }} title={t('homestation.stats.scopeHint', 'Counts come from interaction records your HomeStation verified. They are not a network-wide total.')}>
      {t('homestation.stats.localVerified', 'Verified locally')}
    </span>
  )
}
