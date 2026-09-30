import { File, FileArchive, FileText, Film, Loader2, Music, Play } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { usePreviewThumbnail } from '../../../../components/preview/thumbnail'
import { useI18n } from '../../../../i18n/provider'
import { getObjectAccess, type ObjectInfo } from '../history/objectAccess'
import { useOpenAttachment } from './context'
import { useMediaSettings } from './settings'
import {
  attachmentKindOf,
  attachmentMediaType,
  installMessageHubPreviewSources,
  isInlineImageType,
  type AttachmentKind,
  type MessageAttachment,
} from './source'

installMessageHubPreviewSources()

const THUMBNAIL_BOX = { width: 320, height: 240 }

function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${Math.round(size / 102.4) / 10} KB`
  return `${Math.round(size / (1024 * 102.4)) / 10} MB`
}

function formatDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds))
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${String(s).padStart(2, '0')}`
}

export function MessageAttachmentView({ attachment, isSelf }: { attachment: MessageAttachment; isSelf: boolean }) {
  if (attachment.uri) {
    const mediaType = attachmentMediaType(undefined, attachment.uri)
    return <InlineImage attachment={attachment} url={attachment.uri} isGif={mediaType === 'image/gif'} />
  }
  return <ObjectAttachment attachment={attachment} isSelf={isSelf} />
}

type ObjectState =
  | { phase: 'loading' }
  | { phase: 'ready'; info: ObjectInfo; contentUrl?: string }
  | { phase: 'error'; message: string }

type ReadyObjectState = Extract<ObjectState, { phase: 'ready' }>

// Resolved attachments by object id and label: a bubble that is re-mounted
// (virtualized history) renders at its final size at once instead of going
// through the loading state and resizing the rows around it.
const readyObjects = new Map<string, ReadyObjectState>()
const READY_OBJECTS_LIMIT = 256

function rememberReadyObject(key: string, state: ReadyObjectState) {
  readyObjects.delete(key)
  readyObjects.set(key, state)
  if (readyObjects.size > READY_OBJECTS_LIMIT) readyObjects.delete(readyObjects.keys().next().value!)
}

function ObjectAttachment({ attachment, isSelf }: { attachment: MessageAttachment; isSelf: boolean }) {
  const { t } = useI18n()
  const objId = attachment.objId ?? ''
  const cacheKey = `${objId}\n${attachment.label}`
  const [state, setState] = useState<ObjectState>(() => readyObjects.get(cacheKey) ?? { phase: 'loading' })
  const [retry, setRetry] = useState(0)
  useEffect(() => {
    if (readyObjects.has(cacheKey) && retry === 0) return
    const access = getObjectAccess()
    let cancelled = false
    const resolve = access ? access.describe(objId) : Promise.reject(new Error('unavailable'))
    void resolve.then(async (info) => {
      if (cancelled) return
      const inline = info.isFile && isInlineImageType(attachmentMediaType(info, attachment.label))
      const contentUrl = inline && access ? await access.contentUrl(objId).catch(() => undefined) : undefined
      if (cancelled) return
      const ready: ReadyObjectState = { phase: 'ready', info, contentUrl }
      if (!inline || contentUrl) rememberReadyObject(cacheKey, ready)
      setState(ready)
    }).catch((error: unknown) => {
      if (!cancelled) setState({ phase: 'error', message: error instanceof Error ? error.message : String(error) })
    })
    return () => { cancelled = true }
  }, [objId, attachment.label, cacheKey, retry])

  const label = attachment.label
  if (state.phase === 'loading') {
    return <span className="text-xs" data-testid="attachment-loading">{t('messagehub.attachmentLoading')} · {label}</span>
  }
  if (state.phase === 'error') {
    return <span className="text-xs break-all" role="alert" data-testid="attachment-error">{label} · {t('messagehub.attachmentUnavailable')} <button type="button" className="underline" onClick={() => { setState({ phase: 'loading' }); setRetry(value => value + 1) }}>{t('messagehub.retry')}</button></span>
  }
  const { info } = state
  if (!info.isFile) {
    return <span className="flex items-center gap-2 text-sm break-all" data-testid="attachment-ready" data-kind="object"><FileText size={14} />{info.name ?? label} · {t('messagehub.attachmentNotFile')}</span>
  }
  const mediaType = attachmentMediaType(info, label)
  const kind = attachmentKindOf(mediaType, info.name ?? label) ?? 'file'
  let body: ReactNode
  if (kind === 'image' && state.contentUrl) {
    body = <InlineImage attachment={attachment} url={state.contentUrl} isGif={mediaType === 'image/gif'} />
  } else if (kind === 'video' || kind === 'image') {
    body = <ThumbnailTile attachment={attachment} kind={kind} info={info} />
  } else {
    body = <FileCard attachment={attachment} kind={kind} info={info} mediaType={mediaType} isSelf={isSelf} />
  }
  return <div className="flex min-w-0 flex-col gap-1" data-testid="attachment-ready" data-kind={kind}>{body}</div>
}

// Natural sizes of images already decoded once, so a re-mounted image keeps
// its box before it decodes again.
const imageSizes = new Map<string, { width: number; height: number }>()

function rememberImageSize(url: string, image: HTMLImageElement) {
  if (image.naturalWidth > 0 && image.naturalHeight > 0) imageSizes.set(url, { width: image.naturalWidth, height: image.naturalHeight })
}

/** The box the unsized image would settle into (at most 420 × 320). */
function knownImageBox(size: { width: number; height: number }) {
  return {
    width: Math.min(size.width, 420, Math.round((320 * size.width) / size.height)),
    maxWidth: '100%',
    height: 'auto',
    aspectRatio: `${size.width} / ${size.height}`,
  }
}

function InlineImage({ attachment, url, isGif }: { attachment: MessageAttachment; url: string; isGif: boolean }) {
  const { t } = useI18n()
  const open = useOpenAttachment()
  const { autoplayGif } = useMediaSettings()
  const still = isGif && !autoplayGif
  const size = imageSizes.get(url)
  return (
    <button
      type="button"
      onClick={() => open(attachment, 'image')}
      className="group relative block w-fit max-w-[min(100%,420px)] cursor-zoom-in overflow-hidden rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)]"
      style={{ background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)' }}
      title={t('messagehub.media.open', undefined, { name: attachment.label })}
      data-testid="attachment-image"
      data-gif={isGif ? (still ? 'still' : 'animated') : undefined}
    >
      {still
        ? <GifStill url={url} label={attachment.label} />
        : <img src={url} alt={attachment.label} onLoad={event => rememberImageSize(url, event.currentTarget)} className={size ? 'block' : 'block h-auto max-h-[320px] w-auto max-w-full'} style={size ? knownImageBox(size) : undefined} decoding="async" />}
      {isGif ? <span className="pointer-events-none absolute left-2 top-2 rounded px-1.5 py-0.5 text-[10px] font-semibold text-white" style={{ background: 'rgba(0,0,0,0.55)' }}>GIF</span> : null}
    </button>
  )
}

function GifStill({ url, label }: { url: string; label: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    const image = new Image()
    image.decoding = 'async'
    image.onload = () => {
      rememberImageSize(url, image)
      const canvas = canvasRef.current
      if (!canvas) return
      canvas.width = image.naturalWidth
      canvas.height = image.naturalHeight
      canvas.getContext('2d')?.drawImage(image, 0, 0)
    }
    image.src = url
    return () => { image.onload = null }
  }, [url])
  const size = imageSizes.get(url)
  return <canvas ref={canvasRef} role="img" aria-label={label} width={size?.width ?? 160} height={size?.height ?? 100} className="block h-auto max-h-[320px] w-auto max-w-full" data-testid="attachment-gif-still" />
}

function ThumbnailTile({ attachment, kind, info }: { attachment: MessageAttachment; kind: AttachmentKind; info: ObjectInfo }) {
  const { t } = useI18n()
  const open = useOpenAttachment()
  const thumbnail = usePreviewThumbnail(attachment.source, THUMBNAIL_BOX)
  const ready = thumbnail.status === 'ready' ? thumbnail.thumbnail : null
  const duration = ready?.durationSeconds
  const isVideo = kind === 'video'
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <button
        type="button"
        onClick={() => open(attachment, kind)}
        className="relative block w-fit max-w-[min(100%,420px)] cursor-pointer overflow-hidden rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)]"
        style={{ background: 'color-mix(in srgb, var(--cp-text) 10%, transparent)' }}
        title={t('messagehub.media.open', undefined, { name: attachment.label })}
        data-testid={isVideo ? 'attachment-video' : 'attachment-thumbnail'}
        data-thumbnail={thumbnail.status}
        data-thumbnail-via={ready?.via}
      >
        {ready
          ? <img src={ready.url} alt={attachment.label} className="block h-auto max-h-[320px] w-auto max-w-full" decoding="async" />
          : (
            <div className="flex h-[150px] w-[240px] max-w-full items-center justify-center" style={{ color: 'var(--cp-muted)' }} role="img" aria-label={attachment.label}>
              {thumbnail.status === 'loading' ? <Loader2 size={20} className="animate-spin" /> : isVideo ? <Film size={28} /> : <File size={28} />}
            </div>
          )}
        {isVideo ? (
          <span className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <span className="flex h-12 w-12 items-center justify-center rounded-full text-white" style={{ background: 'rgba(0,0,0,0.55)' }}><Play size={22} fill="currentColor" /></span>
          </span>
        ) : null}
        {duration !== undefined ? <span className="pointer-events-none absolute bottom-2 right-2 rounded px-1.5 py-0.5 text-[10px] font-semibold tabular-nums text-white" style={{ background: 'rgba(0,0,0,0.55)' }}>{formatDuration(duration)}</span> : null}
      </button>
      <span className="truncate text-[11px] opacity-75">{info.name ?? attachment.label}{info.size !== undefined ? ` · ${formatBytes(info.size)}` : ''}</span>
    </div>
  )
}

function fileIcon(kind: AttachmentKind, mediaType: string | undefined) {
  if (kind === 'audio') return <Music size={18} />
  if (mediaType === 'application/pdf' || mediaType?.startsWith('text/')) return <FileText size={18} />
  if (mediaType && /zip|tar|gzip|7z|rar/.test(mediaType)) return <FileArchive size={18} />
  return <File size={18} />
}

function FileCard({ attachment, kind, info, mediaType, isSelf }: { attachment: MessageAttachment; kind: AttachmentKind; info: ObjectInfo; mediaType: string | undefined; isSelf: boolean }) {
  const { t } = useI18n()
  const open = useOpenAttachment()
  return (
    <button
      type="button"
      onClick={() => open(attachment, kind)}
      className="flex w-full min-w-[200px] items-center gap-3 rounded-xl px-3 py-2 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)]"
      style={{ background: 'color-mix(in srgb, currentColor 9%, transparent)' }}
      title={t('messagehub.media.open', undefined, { name: attachment.label })}
      data-testid="attachment-file"
    >
      <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg" style={{ background: 'color-mix(in srgb, currentColor 12%, transparent)', color: isSelf ? 'var(--cp-message-self-link)' : 'var(--cp-accent)' }}>
        {fileIcon(kind, mediaType)}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{info.name ?? attachment.label}</span>
        <span className="block truncate text-[11px] opacity-75">{[info.size !== undefined ? formatBytes(info.size) : null, mediaType].filter(Boolean).join(' · ')}</span>
      </span>
    </button>
  )
}
