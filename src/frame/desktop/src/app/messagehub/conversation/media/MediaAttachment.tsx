import { Bookmark, BookmarkCheck, Download, File, FileArchive, FileText, Film, FolderCheck, FolderDown, FolderOpen, Loader2, Music, Play } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { networkCache } from '../../../../components/network/cache'
import { useSaveToZone } from '../../../../components/saveToZone'
import { usePreviewThumbnail } from '../../../../components/preview/thumbnail'
import { useI18n } from '../../../../i18n/provider'
import { getObjectAccess, isLocalObjectId, objectFailure, type ObjectFailure, type ObjectInfo } from '../history/objectAccess'
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
/** A local read answers well before this; anything slower is waiting for another Zone. */
const FETCHING_HINT_MS = 1200

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
  | { phase: 'fetching' }
  | { phase: 'awaiting' }
  | ReadyObjectState
  | { phase: 'error'; message: string; failure: ObjectFailure }

/** Failures a retry cannot change, with what to say about them. */
const FINAL_FAILURES: Partial<Record<ObjectFailure, string>> = {
  directory: 'messagehub.attachmentDirectoryUnsupported',
  too_large: 'messagehub.attachmentTooLargeToOpen',
  corrupt: 'messagehub.attachmentCorrupt',
}

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
    const slow = window.setTimeout(() => { if (!cancelled) setState(current => current.phase === 'loading' ? { phase: 'fetching' } : current) }, FETCHING_HINT_MS)
    const failed = (error: unknown) => {
      if (cancelled) return
      const failure = objectFailure(error)
      setState(failure === 'awaiting_acceptance' ? { phase: 'awaiting' } : { phase: 'error', message: error instanceof Error ? error.message : String(error), failure })
    }
    const resolve = access ? access.describe(objId, attachment.context) : Promise.reject(new Error('unavailable'))
    void resolve.then(async (info) => {
      if (cancelled) return
      const inline = info.isFile && isInlineImageType(attachmentMediaType(info, attachment.label))
      let contentUrl: string | undefined
      if (inline && access) {
        try {
          contentUrl = await access.contentUrl(objId, attachment.context)
        } catch (error) {
          const failure = objectFailure(error)
          if (failure === 'fetching' || failure === 'unreachable' || failure === 'corrupt') {
            failed(error)
            return
          }
        }
      }
      if (cancelled) return
      const ready: ReadyObjectState = { phase: 'ready', info, contentUrl }
      if ((!inline || contentUrl) && !info.remote) rememberReadyObject(cacheKey, ready)
      setState(ready)
    }).catch(failed).finally(() => window.clearTimeout(slow))
    return () => { cancelled = true; window.clearTimeout(slow) }
  }, [objId, attachment.label, attachment.context, cacheKey, retry])

  // Accepting the sender changes the answer: nothing was cached, ask again.
  const awaiting = state.phase === 'awaiting'
  useEffect(() => {
    if (!awaiting) return
    const seen = networkCache.getEpoch()
    return networkCache.subscribe(() => {
      if (networkCache.getEpoch() === seen) return
      setState({ phase: 'loading' })
      setRetry(value => value + 1)
    })
  }, [awaiting])

  const label = attachment.label
  const readyKind = state.phase === 'ready' ? readyObjectKind(state, label) : undefined
  useEffect(() => { if (readyKind) onKind?.(readyKind) }, [readyKind, onKind])
  if (state.phase === 'loading') {
    return <span className="text-xs" data-testid="attachment-loading">{t('messagehub.attachmentLoading')} · {label}</span>
  }
  if (state.phase === 'fetching') {
    return <span className="flex items-center gap-1.5 text-xs break-all" data-testid="attachment-fetching"><Loader2 size={12} className="shrink-0 animate-spin" />{t('messagehub.attachmentFetching')} · {label}</span>
  }
  if (state.phase === 'awaiting') {
    return <span className="text-xs break-all" data-testid="attachment-awaiting">{label} · {t('messagehub.attachmentAwaitingAcceptance')}</span>
  }
  if (state.phase === 'error') {
    const final = FINAL_FAILURES[state.failure]
    return <span className="text-xs break-all" role="alert" data-testid="attachment-error" data-failure={state.failure}>{label} · {t(final ?? 'messagehub.attachmentUnavailable')} {final ? null : <button type="button" className="underline" onClick={() => { setState({ phase: 'loading' }); setRetry(value => value + 1) }}>{t('messagehub.retry')}</button>}</span>
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
        <span className="absolute right-2 top-2 flex gap-1">
          <SaveButton attachment={attachment} name={info.name ?? label} kind="file" overlay />
          <CollectButton attachment={attachment} name={info.name ?? label} overlay />
        </span>
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
  // A thumbnail needs the bytes: one still in another Zone waits for a click.
  const thumbnail = usePreviewThumbnail(info.remote ? null : attachment.source, THUMBNAIL_BOX)
  const ready = thumbnail.status === 'ready' ? thumbnail.thumbnail : null
  const duration = ready?.durationSeconds
  const isVideo = kind === 'video'
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <button
        type="button"
        onClick={() => open(attachment, kind)}
        className="relative block w-fit max-w-[min(100%,420px)] cursor-pointer overflow-hidden rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)]"
        style={{ background: ready ? undefined : 'color-mix(in srgb, var(--cp-text) 10%, transparent)' }}
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
        <SaveButton attachment={attachment} name={info.name ?? attachment.label} kind="file" compact />
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
      <SaveButton attachment={attachment} name={info.name ?? attachment.label} kind="file" />
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
    ? `h-8 w-8 rounded-full text-white ${state === 'idle' ? 'opacity-0 group-hover/collect:opacity-100 focus-visible:opacity-100' : ''}`
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

/** The bytes of an attachment, or of every file of a folder attachment by its path below it (the mock runtime's save). */
async function attachmentFiles(objId: string, context: string | undefined, innerPath: string | undefined, kind: 'file' | 'dir'): Promise<Array<{ path: string; blob: Blob }>> {
  const access = getObjectAccess()
  if (!access) throw new Error('unavailable')
  const read = async (path: string | undefined) => (await fetch(await access.contentUrl(objId, context, path))).blob()
  if (kind === 'file') return [{ path: '', blob: await read(innerPath) }]
  const files: Array<{ path: string; blob: Blob }> = []
  const walk = async (path: string | undefined, below: string) => {
    for (const name of (await access.describe(objId, context, path)).entries ?? []) {
      const child = path ? `${path}/${name}` : name
      const relative = below ? `${below}/${name}` : name
      if ((await access.describe(objId, context, child)).isDirectory) await walk(child, relative)
      else files.push({ path: relative, blob: await read(child) })
    }
  }
  await walk(innerPath, '')
  return files
}

/**
 * Saves the attachment (or what `innerPath` names in a folder attachment) into the viewer's own files, at a place
 * they pick in the Zone. The main way to keep what was received; downloading is the desktop's extra.
 */
function SaveButton({ attachment, name, kind, innerPath, compact = false, overlay = false }: { attachment: MessageAttachment; name: string; kind: 'file' | 'dir'; innerPath?: string; compact?: boolean; overlay?: boolean }) {
  const { t } = useI18n()
  const save = useSaveToZone(t('messagehub.media.saveNameTaken'))
  const [state, setState] = useState<{ phase: 'idle' | 'saving' | 'failed' } | { phase: 'saved'; path: string }>({ phase: 'idle' })
  const access = getObjectAccess()
  const objId = attachment.objId
  const context = attachment.context
  if (!access?.prepareSave || !objId || !context || isLocalObjectId(objId)) return null
  const prepare = access.prepareSave
  const run = () => {
    if (state.phase === 'saving') return
    setState({ phase: 'saving' })
    save({
      name: name.split('/').pop() || name,
      kind,
      grant: () => prepare(objId, context, innerPath).then(prepared => prepared.objId),
      mockFiles: () => attachmentFiles(objId, context, innerPath, kind),
    }).then(result => setState(result.status === 'saved' ? { phase: 'saved', path: result.path } : { phase: 'idle' }), () => setState({ phase: 'failed' }))
  }
  const label = state.phase === 'saved' ? t('messagehub.media.saved', undefined, { path: state.path })
    : state.phase === 'failed' ? `${t('messagehub.media.saveFailed')} · ${t('messagehub.retry')}`
      : t('messagehub.media.save', undefined, { name })
  const size = compact || overlay ? 14 : 18
  const placement = overlay
    ? `h-8 w-8 rounded-full text-white ${state.phase === 'idle' ? 'opacity-0 group-hover/collect:opacity-100 focus-visible:opacity-100' : ''}`
    : `rounded-lg ${compact ? 'h-8 w-8' : 'h-11 w-11'}`
  return (
    <button
      type="button"
      onClick={run}
      className={`flex shrink-0 items-center justify-center focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)] ${placement}`}
      style={overlay ? { background: 'rgba(0,0,0,0.55)', color: state.phase === 'failed' ? 'var(--cp-danger)' : undefined } : { color: state.phase === 'failed' ? 'var(--cp-danger)' : 'inherit', opacity: state.phase === 'idle' ? 0.8 : 1 }}
      aria-label={label}
      title={label}
      aria-busy={state.phase === 'saving'}
      data-testid="attachment-save"
      data-state={state.phase}
    >
      {state.phase === 'saving' ? <Loader2 size={size} className="animate-spin" /> : state.phase === 'saved' ? <FolderCheck size={size} /> : <FolderDown size={size} />}
    </button>
  )
}

/** Downloading to this device: a desktop's extra next to saving into the Zone, not offered on phones. */
const canDownload = () => !window.matchMedia?.('(pointer: coarse)').matches

function DownloadButton({ attachment, name, innerPath, compact = false }: { attachment: MessageAttachment; name: string; innerPath?: string; compact?: boolean }) {
  const { t } = useI18n()
  const [state, setState] = useState<'idle' | 'loading' | 'failed'>('idle')
  const [shown] = useState(canDownload)
  if (!shown) return null
  const download = () => {
    const access = getObjectAccess()
    if (!access || !attachment.objId || state === 'loading') return
    setState('loading')
    access.contentUrl(attachment.objId, attachment.context, innerPath).then(url => {
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
  const name = info.name || (innerPath ? innerPath.split('/').pop()! : attachment.label)
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
  return <div className="p-2" data-testid="attachment-directory">
    <span className="flex items-center gap-1">
      <button type="button" onClick={() => void load()} className="flex min-w-0 flex-1 items-center gap-2 text-left"><FolderOpen size={18} className="shrink-0" /><span className="truncate">{name}</span></button>
      <SaveButton attachment={attachment} name={name} kind="dir" innerPath={innerPath || undefined} compact />
    </span>
    {error ? <p role="alert">{error}</p> : null}
    {opened ? <ul className="ml-4">{entries.map(entry => <li key={entry.path}>{entry.info.isDirectory
      ? <DirectoryAttachment attachment={attachment} info={entry.info} innerPath={entry.path} />
      : <span className="flex items-center gap-1 p-1" data-testid="attachment-directory-file">
        <File size={14} className="shrink-0" /><span className="min-w-0 flex-1 truncate">{entry.name}</span>
        <SaveButton attachment={attachment} name={entry.name} kind="file" innerPath={entry.path} compact />
        <DownloadButton attachment={attachment} name={entry.name} innerPath={entry.path} compact />
      </span>}</li>)}</ul> : null}
  </div>
}
