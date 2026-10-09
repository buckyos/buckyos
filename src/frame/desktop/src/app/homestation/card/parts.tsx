import clsx from 'clsx'
import {
  ExternalLink,
  Eye,
  FileText,
  Globe,
  Headphones,
  History,
  Loader2,
  Lock,
  Pause,
  Play,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  ShoppingBag,
  Sparkles,
  TriangleAlert,
  VideoOff,
} from 'lucide-react'
import { useCallback, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useItemActions } from '../actions'
import { audienceLabel, formatDuration, formatTimeAgo, restrictedLabel } from '../datamodel/format'
import { actionText, GENERATION_TAGS, reasonLabel, tagLabel } from './labels'
import type { CardView, EffectiveTag, EmbeddedView, FeedItemView, ReadingEntry, ResolvedMedia, ResourceState, Verification } from '../datamodel/types'
import { mediaUrl } from '../media'
import type { HomeStationStore } from '../store/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useNow, useStoreSelector } from '../store/context'
import type { ReadingMode } from '../types'
import { Avatar } from '../ui/primitives'
import { ReasonDetail } from './ReasonDetail'
import { Popover } from '../ui/Popover'

export function VerificationBadge({ verification }: { verification: Verification }) {
  const { t } = useI18n()
  if (verification === 'verified') {
    const label = t('homestation.verify.verified', 'Signature verified')
    return <span title={label} aria-label={label} role="img" className="inline-flex flex-shrink-0" style={{ color: 'var(--cp-success)' }}><ShieldCheck size={13} /></span>
  }
  if (verification === 'unverified') {
    return (
      <span className="hs-badge is-warning" title={t('homestation.verify.unverifiedHint', 'The signature could not be checked yet. Treat the publisher as unconfirmed.')}>
        <ShieldAlert size={11} />
        {t('homestation.verify.unverified', 'Unverified')}
      </span>
    )
  }
  const label = t('homestation.verify.wrapperOnly', 'Signed by the sharer; the original author did not sign')
  return <span title={label} aria-label={label} role="img" className="inline-flex flex-shrink-0" style={{ color: 'var(--cp-muted)' }}><ShieldCheck size={13} /></span>
}

const selectGroups = (store: HomeStationStore) => store.peekGroups()

export function AudienceBadge({ item }: { item: FeedItemView }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const groups = useStoreSelector(selectGroups)
  if (item.isPrivateCapture) {
    return <span className="hs-badge" data-testid="hs-audience-badge"><Lock size={10} />{t('homestation.audience.privateCapture', 'Only you · private capture')}</span>
  }
  if (!item.audience.restricted) return null
  const label = item.isOwn ? audienceLabel(t, item.audience.spec, groups, did => store.peekIdentity(did).name) : restrictedLabel(t, item.audience.spec)
  return <span className="hs-badge is-accent" data-testid="hs-audience-badge"><Lock size={10} />{label}</span>
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
        className="hs-badge is-warning"
        data-testid="hs-updated-badge"
        title={t('homestation.version.updatedHint', 'The author published a newer version. Your interactions stay on this one.')}
        onClick={event => {
          event.stopPropagation()
          onOpen(entry.currentObjId!)
        }}
      >
        <History size={10} />
        {t('homestation.version.updated', 'Updated · view latest')}
      </button>
    )
  }
  if (entry.versionCount > 1) return <span className="hs-badge">{t('homestation.version.edited', 'Edited · v{{n}}', { n: entry.version })}</span>
  return null
}

export function CardHeader({ item, compact = false, trailing }: { item: FeedItemView; compact?: boolean; trailing?: React.ReactNode }) {
  const { t } = useI18n()
  const now = useNow()
  const nav = useHsNav()
  const action = actionText(t, item)
  const name = item.isPrivateCapture ? item.capturedFrom ?? item.publisher.name : item.publisher.name
  const avatar = item.isPrivateCapture ? { name: name, hue: 200 } : item.publisher
  return (
    <div className="flex items-start gap-2.5">
      {item.isPrivateCapture ? (
        <span className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full" style={{ background: 'var(--hs-chip-bg)', color: 'var(--cp-muted)' }}><Globe size={16} /></span>
      ) : (
        <Avatar identity={avatar} size={compact ? 28 : 36} />
      )}
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[13px] leading-5">
          <span className="truncate font-semibold" data-testid="hs-card-publisher">{name}</span>
          <VerificationBadge verification={item.verification} />
          {action ? <span style={{ color: 'var(--cp-muted)' }}>{action}</span> : null}
          <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>· {formatTimeAgo(t, item.createdAt, now)}</span>
          <AudienceBadge item={item} />
          <VersionBadge item={item} onOpen={nav.openDetail} />
        </div>
        {item.isCapture ? (
          <p className="truncate text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }} data-testid="hs-capture-line">
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

export function MediaGrid({ media, onOpen, large = false }: { media: ResolvedMedia[]; onOpen: () => void; large?: boolean }) {
  const { t } = useI18n()
  const images = media.slice(0, 4)
  if (images.length === 0) return null
  const single = images.length === 1
  return (
    <button
      type="button"
      onClick={onOpen}
      className={clsx('mt-2 grid w-full gap-1 overflow-hidden rounded-xl', single ? 'grid-cols-1' : 'grid-cols-2', images.length > 2 && 'grid-rows-2')}
      style={{ aspectRatio: single ? (large ? '4 / 3' : '16 / 10') : images.length === 2 ? '2 / 1' : large ? '1 / 1' : '16 / 10', maxHeight: large ? 640 : 420 }}
      aria-label={t('homestation.media.open', 'Open images')}
      data-testid="hs-media-grid"
    >
      {images.map((part, index) => (
        <img
          key={part.object}
          src={mediaUrl(part.object, part.file, part.alt)}
          alt={part.alt ?? ''}
          loading="lazy"
          className={clsx('h-full min-h-0 w-full object-cover', images.length === 3 && index === 0 && 'row-span-2')}
          style={{ background: 'var(--hs-chip-bg)' }}
        />
      ))}
      {media.length > 4 ? <span className="sr-only">{t('homestation.media.more', '+{{n}} more', { n: media.length - 4 })}</span> : null}
    </button>
  )
}

export function ResourceOverlay({ state, objId, kind }: { state: ResourceState; objId: string; kind: 'video' | 'audio' }) {
  const { t } = useI18n()
  const actions = useItemActions()
  if (state === 'preparing') {
    return (
      <span className="flex items-center gap-2 rounded-full px-3 py-1.5 text-xs font-medium" style={{ background: 'rgba(0,0,0,0.62)', color: 'white' }} data-testid="hs-resource-state" data-state="preparing">
        <Loader2 size={14} className="animate-spin" />
        {kind === 'video' ? t('homestation.resources.preparingVideo', 'Preparing video…') : t('homestation.resources.preparingAudio', 'Preparing audio…')}
      </span>
    )
  }
  return (
    <span className="flex flex-col items-center gap-2" data-testid="hs-resource-state" data-state="unavailable">
      <span className="flex items-center gap-2 rounded-full px-3 py-1.5 text-xs font-medium" style={{ background: 'rgba(0,0,0,0.62)', color: 'white' }}>
        <VideoOff size={14} />
        {t('homestation.resources.unavailable', 'Unavailable: no reachable source')}
      </span>
      <button
        type="button"
        className="flex items-center gap-1 rounded-full px-3 py-1 text-xs font-semibold"
        style={{ background: 'rgba(255,255,255,0.88)', color: '#111' }}
        onClick={event => {
          event.stopPropagation()
          void actions.retryResources(objId)
        }}
      >
        <RefreshCw size={12} />
        {t('common.retry', 'Retry')}
      </button>
    </span>
  )
}

export function VideoBlock({ item, resources, onOpen }: { item: FeedItemView; resources: ResourceState; onOpen: () => void }) {
  const { t } = useI18n()
  const ready = resources === 'local' || resources === 'reachable'
  const duration = item.wrappedFile?.meta.duration_ms
  return (
    <div className="relative mt-2 w-full overflow-hidden rounded-xl" style={{ aspectRatio: '16 / 9', background: '#111' }} data-testid="hs-video-block" data-resources={resources}>
      {item.cover ? <img src={mediaUrl(item.cover.object, item.cover.file, item.object.content?.title)} alt="" className="absolute inset-0 h-full w-full object-cover" style={{ opacity: ready ? 1 : 0.55 }} /> : null}
      <div className="absolute inset-0 flex items-center justify-center">
        {ready ? (
          <button type="button" onClick={onOpen} className="flex h-12 w-12 items-center justify-center rounded-full" style={{ background: 'rgba(0,0,0,0.6)', color: 'white' }} aria-label={t('homestation.media.playVideo', 'Play video')} data-testid="hs-play">
            <Play size={20} fill="white" />
          </button>
        ) : (
          <ResourceOverlay state={resources} objId={item.objId} kind="video" />
        )}
      </div>
      {ready && duration ? (
        <span className="absolute bottom-2 right-2 rounded px-1.5 py-0.5 text-[11px] font-medium" style={{ background: 'rgba(0,0,0,0.7)', color: 'white' }}>{formatDuration(duration)}</span>
      ) : null}
      {ready && resources === 'reachable' ? (
        <span className="absolute left-2 top-2 rounded px-1.5 py-0.5 text-[10px] font-medium" style={{ background: 'rgba(0,0,0,0.6)', color: 'white' }} title={t('homestation.resources.reachableHint', 'Not stored on your HomeStation yet; it streams from a reachable holder.')}>
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
  const duration = part?.file?.meta.duration_ms
  return (
    <div className="mt-2 flex items-center gap-3 rounded-xl px-3 py-2.5" style={{ background: 'var(--hs-subtle-bg)' }} data-testid="hs-audio-block">
      {ready ? (
        <button type="button" className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full" style={{ background: 'var(--cp-accent)', color: 'var(--hs-on-accent)' }} aria-label={playing ? t('homestation.media.pause', 'Pause') : t('homestation.media.playAudio', 'Play voice note')} aria-pressed={playing} onClick={event => { event.stopPropagation(); setPlaying(value => !value) }}>
          {playing ? <Pause size={16} /> : <Play size={16} />}
        </button>
      ) : (
        <span className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full" style={{ background: 'var(--hs-chip-bg)', color: 'var(--cp-muted)' }}><Headphones size={16} /></span>
      )}
      <div className="flex h-8 flex-1 items-center gap-[3px]" aria-hidden="true">
        {Array.from({ length: 32 }, (_, index) => (
          <span key={index} className="w-[3px] rounded-full" style={{ height: `${20 + ((index * 37) % 70)}%`, background: playing && index < 12 ? 'var(--cp-accent)' : 'color-mix(in srgb, var(--cp-muted) 45%, transparent)' }} />
        ))}
      </div>
      <span className="text-xs tabular-nums" style={{ color: 'var(--cp-muted)' }}>{ready && duration ? formatDuration(duration) : t('homestation.resources.preparingShort', 'Preparing')}</span>
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
      className="mt-2 flex w-full overflow-hidden rounded-xl border text-left"
      style={{ borderColor: 'var(--hs-divider)' }}
      onClick={event => {
        event.stopPropagation()
        onOpenLink(url)
      }}
      data-testid="hs-link-card"
      aria-label={t('homestation.link.openNamed', 'Open {{host}} in a new tab', { host })}
    >
      {item.cover ? <img src={mediaUrl(item.cover.object, item.cover.file, content?.title)} alt="" className="h-auto w-28 flex-shrink-0 object-cover sm:w-36" /> : null}
      <span className="min-w-0 flex-1 px-3 py-2.5">
        <span className="line-clamp-2 block text-sm font-semibold leading-5">{content?.title}</span>
        {content?.summary ? <span className="mt-0.5 line-clamp-2 block text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{content.summary}</span> : null}
        <span className="mt-1 flex items-center gap-1 text-[11px]" style={{ color: 'var(--cp-muted)' }}><ExternalLink size={11} />{host}</span>
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
    <p className={clsx('mt-1.5 cursor-pointer whitespace-pre-line text-sm leading-relaxed', clamp)} style={{ color: 'color-mix(in srgb, var(--cp-text) 88%, transparent)' }} onClick={onOpen}>
      {text}
    </p>
  ) : null

  switch (item.contentType) {
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
          {title ? <h3 className="mt-2 cursor-pointer text-[15px] font-semibold leading-snug" onClick={onOpen}>{title}</h3> : null}
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
          {item.cover && mode === 'image' ? <img src={mediaUrl(item.cover.object, item.cover.file, title)} alt="" className="mt-2 w-full rounded-xl object-cover" style={{ aspectRatio: '16 / 9' }} /> : null}
          {title ? <h3 className="mt-2 text-[15px] font-semibold leading-snug">{title}</h3> : null}
          {content?.summary ? <p className={clsx('mt-1 text-sm leading-relaxed', mode === 'longform' ? 'line-clamp-6' : 'line-clamp-3')} style={{ color: 'color-mix(in srgb, var(--cp-text) 82%, transparent)' }}>{content.summary}</p> : null}
          <span className="mt-1.5 inline-flex items-center gap-1 text-[11px]" style={{ color: 'var(--cp-muted)' }}><FileText size={11} />{t('homestation.article.readFull', 'Long read · open to read the full text')}</span>
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
          {title ? <h3 className="mt-2 text-[15px] font-semibold leading-snug">{title}</h3> : null}
          {content?.summary ? <p className="mt-1 text-sm leading-relaxed" style={{ color: 'color-mix(in srgb, var(--cp-text) 82%, transparent)' }}>{content.summary}</p> : null}
          {mode !== 'longform' && item.media.length > 0 ? <MediaGrid media={item.media} onOpen={() => item.object.link && actions.openLink(item.object.link)} /> : null}
          {item.object.link ? (
            <button type="button" className="hs-btn mt-2" onClick={event => { event.stopPropagation(); actions.openLink(item.object.link!) }} data-testid="hs-product-open">
              <ShoppingBag size={14} />
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
      <div className="hs-embed mt-2 flex items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }} data-testid="hs-embed-placeholder" data-visibility={embedded.visibility}>
        {embedded.visibility === 'not_visible' ? <Lock size={13} /> : <TriangleAlert size={13} />}
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
      className="hs-embed mt-2 cursor-pointer"
      data-testid="hs-embed"
      onClick={event => {
        event.stopPropagation()
        onOpen(inner.objId)
      }}
      onKeyDown={event => {
        if (event.key === 'Enter') onOpen(inner.objId)
      }}
    >
      <div className="flex items-center gap-1.5 text-xs">
        <Avatar identity={inner.isPrivateCapture ? { name: inner.capturedFrom ?? '?', hue: 200 } : inner.publisher} size={20} />
        <span className="truncate font-semibold">{inner.isPrivateCapture ? inner.capturedFrom : inner.publisher.name}</span>
        <VerificationBadge verification={inner.verification} />
        <span style={{ color: 'var(--cp-muted)' }}>· {formatTimeAgo(t, inner.createdAt, now)}</span>
        <AudienceBadge item={inner} />
        <VersionBadge item={inner} onOpen={onOpen} />
      </div>
      <div className="mt-1.5 flex gap-3">
        <div className="min-w-0 flex-1">
          {content?.title ? <p className="line-clamp-2 text-sm font-semibold leading-5">{content.title}</p> : null}
          {content?.text || content?.summary ? <p className="line-clamp-3 text-[13px] leading-5" style={{ color: 'color-mix(in srgb, var(--cp-text) 82%, transparent)' }}>{content?.text ?? content?.summary}</p> : null}
        </div>
        {firstImage ? <img src={mediaUrl(firstImage.object, firstImage.file, firstImage.alt)} alt="" className="h-16 w-16 flex-shrink-0 rounded-lg object-cover" /> : null}
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
    <div className="space-y-2 p-3 text-xs leading-5">
      <p className="text-sm font-semibold">{tagLabel(t, tag)}</p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
        <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.source', 'Source')}</dt>
        <dd>{sourceLabel}</dd>
        <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.status', 'Status')}</dt>
        <dd>{tag.status === 'inferred' ? t('homestation.tags.statusInferred', 'Inferred, not a fact') : tag.status === 'confirmed' ? t('homestation.tags.statusConfirmed', 'Confirmed') : t('homestation.tags.statusDeclared', 'Declared')}</dd>
        <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.scope', 'Scope')}</dt>
        <dd>{tag.scope === 'whole_content' ? t('homestation.tags.scopeWhole', 'Whole content') : t('homestation.tags.scopePart', 'Part of the content')}</dd>
        {tag.confidence !== undefined ? (
          <>
            <dt style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.confidence', 'Model score')}</dt>
            <dd>{tag.confidence.toFixed(2)} <span style={{ color: 'var(--cp-muted)' }}>{t('homestation.tags.confidenceNote', '(a score, not a proof)')}</span></dd>
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
        <div className="flex flex-wrap gap-1.5 pt-1">
          <button type="button" className="hs-btn" onClick={() => apply('remove')} data-testid="hs-tag-remove">{t('homestation.tags.remove', 'Remove tag')}</button>
          {tag.tag === 'ai_full' ? <button type="button" className="hs-btn" onClick={() => apply('to_assisted')}>{t('homestation.tags.toAssisted', 'Change to AI-assisted')}</button> : null}
          {tag.status !== 'confirmed' ? <button type="button" className="hs-btn" onClick={() => apply('confirm')}>{t('homestation.tags.confirm', 'Confirm')}</button> : null}
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
        className={generation ? 'hs-badge is-warning' : 'hs-badge'}
        style={!generation && tag.source === 'model' ? { textDecoration: 'underline dotted', textUnderlineOffset: 3 } : undefined}
        title={tag.source === 'model' ? t('homestation.tags.localHint', 'Local classification') : undefined}
        aria-expanded={!!anchor}
        data-testid="hs-tag"
        data-tag={tag.tag}
        onClick={event => {
          event.stopPropagation()
          setAnchor(anchor ? null : event.currentTarget)
        }}
      >
        {generation ? <Sparkles size={10} /> : null}
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
    <div className="mt-2 flex flex-wrap items-center gap-1.5">
      {reading?.filteredBy.length ? <span className="hs-badge is-danger" data-testid="hs-filtered-badge">{t('homestation.filters.filteredBadge', 'Hidden by your filter · shown on request')}</span> : null}
      {tags.map(tag => <TagChip key={`${tag.tag}-${tag.source}`} tag={tag} objId={reading?.objId ?? ''} />)}
    </div>
  )
}

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
        className="mt-2 flex max-w-full items-center gap-1 text-left text-[11px]"
        style={{ color: 'var(--cp-muted)' }}
        data-testid="hs-reason"
        aria-expanded={!!anchor}
        onClick={event => setAnchor(anchor ? null : event.currentTarget)}
      >
        <Eye size={11} className="flex-shrink-0" />
        <span className="truncate underline decoration-dotted underline-offset-2">{reasonLabel(t, reading)}</span>
      </button>
      <Popover anchor={anchor} open={!!anchor} onClose={close} align="start" width={320} label={t('homestation.reason.title', 'Why you’re seeing this')} testId="hs-reason-detail">
        <div className="space-y-2 p-3 text-xs leading-5">
          <p className="text-sm font-semibold">{t('homestation.reason.title', 'Why you’re seeing this')}</p>
          <ReasonDetail view={view} />
          <div className="flex flex-wrap gap-1.5 pt-1">
            {person ? (
              <button type="button" className="hs-btn" onClick={() => { close(); void actions.mute({ kind: 'person', did: view.item.publisher.did!, name: view.item.publisher.name }) }}>
                {t('homestation.menu.mutePerson', 'Don’t show {{name}}', { name: view.item.publisher.name })}
              </button>
            ) : null}
            <button type="button" className="hs-btn" onClick={() => { close(); void actions.lessLike(view) }}>{t('homestation.menu.lessLike', 'Show less like this')}</button>
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
    <span className="ml-auto hidden items-center gap-1 text-[10px] sm:inline-flex" style={{ color: 'var(--cp-muted)' }} title={t('homestation.stats.scopeHint', 'Counts come from interaction records your HomeStation verified. They are not a network-wide total.')}>
      {t('homestation.stats.localVerified', 'Local verified')}
    </span>
  )
}
