import { Bookmark, BookmarkCheck, Download, File, FileArchive, FileText, Film, FolderOpen, Loader2, Music, Play } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { usePreviewThumbnail } from '../../../../components/preview/thumbnail'
import { useI18n } from '../../../../i18n/provider'
import { getObjectAccess, isLocalObjectId, type ObjectInfo } from '../history/objectAccess'
import { useOpenAttachment } from './context'
import { knownReadyObject, objectCacheKey, readyObjectKind, readyObjects, rememberReadyObject, type ReadyObjectState } from './objectCache'
import { useMediaSettings } from './settings'
import {
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

export function MessageAttachmentView({ attachment, isSelf, onKind }: { attachment: MessageAttachment; isSelf: boolean; onKind?: (kind: AttachmentKind) => void }) {
  if (attachment.uri) {
    const mediaType = attachmentMediaType(undefined, attachment.uri)
    return <InlineImage attachment={attachment} url={attachment.uri} isGif={mediaType === 'image/gif'} />
  }
  return <ObjectAttachment attachment={attachment} isSelf={isSelf} onKind={onKind} />
}

type ObjectState =
  | { phase: 'loading' }
  | ReadyObjectState
  | { phase: 'error'; message: string }

function ObjectAttachment({ attachment, isSelf, onKind }: { attachment: MessageAttachment; isSelf: boolean; onKind?: (kind: AttachmentKind) => void }) {
  const { t } = useI18n()
  const objId = attachment.objId ?? ''
  const cacheKey = objectCacheKey(attachment)
  const [state, setState] = useState<ObjectState>(() => knownReadyObject(attachment) ?? { phase: 'loading' })
  const [retry, setRetry] = useState(0)
  useEffect(() => {
    if (readyObjects.has(cacheKey) && retry === 0) return
    const access = getObjectAccess()
    let cancelled = false
    const resolve = access ? access.describe(objId, attachment.context) : Promise.reject(new Error('unavailable'))
    void resolve.then(async (info) => {
      if (cancelled) return
      const inline = info.isFile && isInlineImageType(attachmentMediaType(info, attachment.label))
      const contentUrl = inline && access ? await access.contentUrl(objId, attachment.context).catch(() => undefined) : undefined
      if (cancelled) return
      const ready: ReadyObjectState = { phase: 'ready', info, contentUrl }
      if (!inline || contentUrl) rememberReadyObject(cacheKey, ready)
      setState(ready)
    }).catch((error: unknown) => {
      if (!cancelled) setState({ phase: 'error', message: error instanceof Error ? error.message : String(error) })
    })
    return () => { cancelled = true }
  }, [objId, attachment.label, attachment.context, cacheKey, retry])

  const label = attachment.label
  const readyKind = state.phase === 'ready' ? readyObjectKind(state, label) : undefined
  useEffect(() => { if (readyKind) onKind?.(readyKind) }, [readyKind, onKind])
  if (state.phase === 'loading') {
    return <span className="text-xs" data-testid="attachment-loading">{t('messagehub.attachmentLoading')} · {label}</span>
  }
  if (state.phase === 'error') {
    return <span className="text-xs break-all" role="alert" data-testid="attachment-error">{label} · {t('messagehub.attachmentUnavailable')} <button type="button" className="underline" onClick={() => { setState({ phase: 'loading' }); setRetry(value => value + 1) }}>{t('messagehub.retry')}</button></span>
  }
  const { info } = state
  if (info.isDirectory) return <DirectoryAttachment attachment={attachment} info={info} />
  if (!info.isFile) {
    return <span className="flex items-center gap-2 text-sm break-all" data-testid="attachment-ready" data-kind="object"><FileText size={14} />{info.name ?? label} · {t('messagehub.attachmentNotFile')}</span>
  }
  const mediaType = attachmentMediaType(info, label)
  const kind = readyKind ?? 'file'
  let body: ReactNode
  if (kind === 'image' && state.contentUrl) {
    body = (
      <div className="group/collect relative w-fit max-w-full">
        <InlineImage attachment={attachment} url={state.contentUrl} isGif={mediaType === 'image/gif'} />
        <CollectButton attachment={attachment} name={info.name ?? label} overlay />
      </div>
    )
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
      <span className="flex max-w-[min(100%,420px)] items-center gap-1">
        <span className="min-w-0 flex-1 truncate text-[11px] opacity-75">{info.name ?? attachment.label}{info.size !== undefined ? ` · ${formatBytes(info.size)}` : ''}</span>
        <CollectButton attachment={attachment} name={info.name ?? attachment.label} compact />
        <DownloadButton attachment={attachment} name={info.name ?? attachment.label} compact />
      </span>
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
    <div className="flex w-full min-w-[200px] items-center rounded-xl pr-1" style={{ background: 'color-mix(in srgb, currentColor 9%, transparent)' }} data-testid="attachment-file">
      <button
        type="button"
        onClick={() => open(attachment, kind)}
        className="flex min-w-0 flex-1 items-center gap-3 rounded-xl py-2 pl-3 pr-1 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)]"
        title={t('messagehub.media.open', undefined, { name: attachment.label })}
      >
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg" style={{ background: 'color-mix(in srgb, currentColor 12%, transparent)', color: isSelf ? 'var(--cp-message-self-link)' : 'var(--cp-accent)' }}>
          {fileIcon(kind, mediaType)}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium">{info.name ?? attachment.label}</span>
          <span className="block truncate text-[11px] opacity-75">{[info.size !== undefined ? formatBytes(info.size) : null, mediaType].filter(Boolean).join(' · ')}</span>
        </span>
      </button>
      <CollectButton attachment={attachment} name={info.name ?? attachment.label} />
      <DownloadButton attachment={attachment} name={info.name ?? attachment.label} />
    </div>
  )
}

/** Saves the attachment to the viewer's collection, which keeps it after the message is recalled. */
function CollectButton({ attachment, name, compact = false, overlay = false }: { attachment: MessageAttachment; name: string; compact?: boolean; overlay?: boolean }) {
  const { t } = useI18n()
  const [state, setState] = useState<'idle' | 'saving' | 'saved' | 'failed'>('idle')
  const access = getObjectAccess()
  const objId = attachment.objId
  if (!access?.collect || !objId || !attachment.context || isLocalObjectId(objId)) return null
  const collect = access.collect
  const save = () => {
    if (state === 'saving' || state === 'saved') return
    setState('saving')
    collect(objId, attachment.context).then(() => setState('saved'), () => setState('failed'))
  }
  const label = state === 'saved' ? t('messagehub.media.collected') : state === 'failed' ? `${t('messagehub.media.collectFailed')} · ${t('messagehub.retry')}` : t('messagehub.media.collect', undefined, { name })
  const size = compact || overlay ? 14 : 18
  const placement = overlay
    ? `absolute right-2 top-2 h-8 w-8 rounded-full text-white ${state === 'idle' ? 'opacity-0 group-hover/collect:opacity-100 focus-visible:opacity-100' : ''}`
    : `rounded-lg ${compact ? 'h-8 w-8' : 'h-11 w-11'}`
  return (
    <button
      type="button"
      onClick={save}
      className={`flex shrink-0 items-center justify-center focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)] ${placement}`}
      style={overlay ? { background: 'rgba(0,0,0,0.55)', color: state === 'failed' ? 'var(--cp-danger)' : undefined } : { color: state === 'failed' ? 'var(--cp-danger)' : 'inherit', opacity: state === 'idle' ? 0.8 : 1 }}
      aria-label={label}
      title={label}
      aria-busy={state === 'saving'}
      data-testid="attachment-collect"
      data-state={state}
    >
      {state === 'saving' ? <Loader2 size={size} className="animate-spin" /> : state === 'saved' ? <BookmarkCheck size={size} /> : <Bookmark size={size} />}
    </button>
  )
}

function DownloadButton({ attachment, name, compact = false }: { attachment: MessageAttachment; name: string; compact?: boolean }) {
  const { t } = useI18n()
  const [state, setState] = useState<'idle' | 'loading' | 'failed'>('idle')
  const download = () => {
    const access = getObjectAccess()
    if (!access || !attachment.objId || state === 'loading') return
    setState('loading')
    access.contentUrl(attachment.objId, attachment.context).then(url => {
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = name.split('/').pop() || name
      anchor.rel = 'noopener'
      anchor.click()
      setState('idle')
    }, () => setState('failed'))
  }
  const label = state === 'failed' ? `${t('messagehub.media.downloadFailed')} · ${t('messagehub.retry')}` : t('messagehub.media.download', undefined, { name })
  return (
    <button
      type="button"
      onClick={download}
      className={`flex shrink-0 items-center justify-center rounded-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)] ${compact ? 'h-8 w-8' : 'h-11 w-11'}`}
      style={{ color: state === 'failed' ? 'var(--cp-danger)' : 'inherit', opacity: state === 'failed' ? 1 : 0.8 }}
      aria-label={label}
      title={label}
      aria-busy={state === 'loading'}
      data-testid="attachment-download"
      data-state={state}
    >
      {state === 'loading' ? <Loader2 size={compact ? 14 : 18} className="animate-spin" /> : <Download size={compact ? 14 : 18} />}
    </button>
  )
}

function DirectoryAttachment({ attachment, info, innerPath = '' }: { attachment: MessageAttachment; info: ObjectInfo; innerPath?: string }) {
  const [opened, setOpened] = useState(false)
  const [entries, setEntries] = useState<Array<{ name: string; path: string; info: ObjectInfo }>>([])
  const [error, setError] = useState('')
  const load = async () => {
    if (opened) { setOpened(false); return }
    const access = getObjectAccess()
    if (!access || !attachment.objId) return
    try {
      setEntries(await Promise.all((info.entries ?? []).map(async name => {
        const path = innerPath ? `${innerPath}/${name}` : name
        return { name, path, info: await access.describe(attachment.objId!, attachment.context, path) }
      })))
      setError('')
      setOpened(true)
    } catch (error) { setError(error instanceof Error ? error.message : String(error)) }
  }
  const download = async (path: string, name: string) => {
    try {
      const url = await getObjectAccess()?.contentUrl(attachment.objId!, attachment.context, path)
      if (!url) return
      const anchor = document.createElement('a')
      anchor.href = url; anchor.download = name; anchor.click()
    } catch (error) { setError(error instanceof Error ? error.message : String(error)) }
  }
  const pickDirectory = (window as Window & { showDirectoryPicker?: () => Promise<FileSystemDirectoryHandle> }).showDirectoryPicker
  const saveDirectory = async () => {
    if (!pickDirectory || !attachment.objId) return
    try {
      const destination = await pickDirectory.call(window)
      const access = getObjectAccess()!
      const save = async (folder: ObjectInfo, path: string, parent: FileSystemDirectoryHandle) => {
        try { await parent.getDirectoryHandle(folder.name || attachment.label); throw new Error('A folder with this name already exists; choose another destination.') } catch (error) { if (!(error instanceof DOMException) || error.name !== 'NotFoundError') throw error }
        const target = await parent.getDirectoryHandle(folder.name || attachment.label, { create: true })
        for (const name of folder.entries ?? []) {
          const childPath = path ? `${path}/${name}` : name
          const child = await access.describe(attachment.objId!, attachment.context, childPath)
          if (child.isDirectory) await save(child, childPath, target)
          else {
            const response = await fetch(await access.contentUrl(attachment.objId!, attachment.context, childPath))
            const writable = await (await target.getFileHandle(name, { create: true })).createWritable()
            try { await writable.write(await response.blob()); await writable.close() } catch (error) { await writable.abort(); throw error }
          }
        }
      }
      await save(info, innerPath, destination)
      setError('')
    } catch (error) { setError(error instanceof Error ? error.message : String(error)) }
  }
  return <div className="p-2" data-testid="attachment-directory">
    <button type="button" onClick={() => void load()} className="flex items-center gap-2"><FolderOpen size={18} />{info.name ?? attachment.label}</button>
    {pickDirectory ? <button type="button" className="text-xs underline" onClick={() => void saveDirectory()}>Save folder</button> : <span className="text-xs">Open the folder to download individual files.</span>}
    {error ? <p role="alert">{error}</p> : null}
    {opened ? <ul className="ml-4">{entries.map(entry => <li key={entry.path}>{entry.info.isDirectory
      ? <DirectoryAttachment attachment={attachment} info={entry.info} innerPath={entry.path} />
      : <button type="button" className="flex items-center gap-2 p-1" onClick={() => void download(entry.path, entry.name)}><Download size={14} />{entry.name}</button>}</li>)}</ul> : null}
  </div>
}
