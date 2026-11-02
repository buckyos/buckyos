/* An asset on the canvas (多媒体对象 §4): a picture shows itself; any other file shows its poster (a video frame, a
 * PDF's first page, an album cover) or, without one, its icon card. A video or sound this browser plays is played in
 * place by the system Preview component; everything else opens the preview on top of the workspace. */

import { useEffect, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { File as FileIcon, FileText, Film, Music, Play, X } from 'lucide-react'
import { ContentPreview } from '../../../../components/ContentPreview'
import type { ContentRef } from '../../../../components/preview/types'
import { describeError, type ReadOk } from '../../api/session'
import type { AssetContent } from '../../api/types'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { consumeIntent, useIntent } from '../blocks/editorToolbar'
import { assetMediaType, fileBadge, formatDuration, mediaKindOf, playsInPlace, type MediaKind } from './kinds'
import { assetSource, surfaceMediaSession } from './source'
import { useSession } from '../../session/context'
import { SessionMediaPlayer } from '../../session/media'

/** A stored picture (the asset itself or its poster) as a blob URL: the asset route needs the session token. */
function usePictureUrl(store: WorkspaceStore, objectId: string | undefined, mediaType: string) {
  const [picture, setPicture] = useState<{ objectId: string; url: string | null; error: string | null } | null>(null)
  useEffect(() => {
    if (!objectId) return
    let live = true
    let url: string | null = null
    store.session.fetchAsset(objectId).then(
      (blob) => {
        if (!live) return
        // the asset route serves unlisted types as octet-stream (e.g. SVG); the <img> needs the real type
        url = URL.createObjectURL(mediaType && blob.type !== mediaType ? new Blob([blob], { type: mediaType }) : blob)
        setPicture({ objectId, url, error: null })
      },
      (error: unknown) => { if (live) setPicture({ objectId, url: null, error: describeError(error) }) },
    )
    return () => { live = false; if (url) URL.revokeObjectURL(url) }
  }, [store, objectId, mediaType])
  return picture && picture.objectId === objectId ? picture : null
}

const KIND_ICON: Record<MediaKind, typeof FileIcon> = { image: FileIcon, video: Film, audio: Music, pdf: FileText, file: FileIcon }

export interface AssetMediaProps {
  asset: ReadOk<AssetContent>
  /** The Block showing it (toolbar and canvas intents, the preview's list); null in the data-source view. */
  cellId: string | null
  fit: 'contain' | 'cover'
  /** Mounted by an explicit activation (double-click, Enter, "play" on a free canvas): it plays, or opens the preview. */
  activated?: boolean
  /** On a free canvas in edit mode "play" activates the Block, so the canvas lets the controls have the pointer. */
  onActivate?: () => void
  /** Playback stopped or the preview closed. */
  onDone?: () => void
}

export function AssetMedia(props: AssetMediaProps) {
  const session = useSession()
  const payload = props.asset.content.payload
  const kind = mediaKindOf(assetMediaType(payload.media_type, payload.file_name))
  // on a session's stage a video or sound plays from the run Workspace, kept to the presenter's playback (第四期规划 §7.3.2)
  if (session && (kind === 'video' || kind === 'audio')) {
    return <SessionMediaPlayer entityId={props.asset.entity_id} objectId={payload.object_id} kind={kind} name={payload.file_name ?? props.asset.name ?? props.asset.entity_id} />
  }
  return <PlainAssetMedia {...props} />
}

function PlainAssetMedia({ asset, cellId, fit, activated = false, onActivate, onDone }: AssetMediaProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const payload = asset.content.payload
  const mediaType = assetMediaType(payload.media_type, payload.file_name)
  const kind = mediaKindOf(mediaType)
  const playable = playsInPlace(kind, mediaType)
  const name = payload.file_name ?? asset.name ?? asset.entity_id
  const [playing, setPlaying] = useState(activated && playable)
  const [previewing, setPreviewing] = useState(activated && !playable)
  // "preview" (the toolbar) and "open" (a double-click while reading: play in place, or preview)
  const intent = useIntent(cellId ?? '')
  const [seen, setSeen] = useState(0)
  const wanted = intent && (intent.value === 'preview' || intent.value === 'open') ? intent : undefined
  if (wanted && wanted.seq !== seen) {
    setSeen(wanted.seq)
    if (wanted.value === 'open' && playable) setPlaying(true)
    else setPreviewing(true)
  }
  useEffect(() => { if (wanted && cellId) consumeIntent(cellId, wanted) }, [wanted, cellId])
  const picture = usePictureUrl(store, kind === 'image' ? payload.object_id : payload.poster, kind === 'image' ? mediaType : '')
  const current = assetSource(store, payload, name)
  const play = () => { if (onActivate) onActivate(); else setPlaying(true) }
  const stop = () => { setPlaying(false); onDone?.() }
  const closePreview = () => { setPreviewing(false); onDone?.() }
  const Icon = KIND_ICON[kind]
  const facts = [
    payload.duration_ms ? formatDuration(payload.duration_ms) : null,
    payload.page_count ? t('aiworkspace.blocks.media.pages', { count: payload.page_count }) : null,
  ].filter(Boolean).join(' · ')

  let face: ReactNode
  if (kind === 'image' || payload.poster) {
    face = picture?.url ? (
      <img className="aiws-media-picture" src={picture.url} alt={name} style={{ objectFit: fit }} draggable={false} data-testid={kind === 'image' ? 'aiws-asset-image' : 'aiws-media-poster'} />
    ) : picture?.error ? (
      <div className="aiws-error" role="alert">{t('aiworkspace.blocks.asset.imageFailed', { error: picture.error })}</div>
    ) : <div className="aiws-muted aiws-media-loading">{t('aiworkspace.blocks.asset.readingImage')}</div>
  } else {
    face = (
      <div className="aiws-media-card" data-testid="aiws-media-card">
        <span className="aiws-media-card-icon"><Icon size={28} aria-hidden="true" /><span className="aiws-media-card-badge">{fileBadge(mediaType, payload.file_name)}</span></span>
        <span className="aiws-media-card-text">
          <span className="aiws-media-card-name" title={name}>{name}</span>
          {facts && <span className="aiws-media-card-facts">{facts}</span>}
        </span>
      </div>
    )
  }
  const poster = Boolean(payload.poster) || kind === 'image'
  return (
    <div className="aiws-media" data-kind={kind} data-poster={poster ? 'true' : 'false'} data-state={playing ? 'playing' : 'idle'} data-testid={`aiws-media-${asset.entity_id}`}>
      {face}
      {!playing && playable && (
        <button type="button" className="aiws-media-play" aria-label={t('aiworkspace.blocks.media.play', { name })} title={t('aiworkspace.blocks.media.play', { name })} data-testid="aiws-media-play" onClick={play}><Play size={22} aria-hidden="true" /></button>
      )}
      {!playing && poster && kind !== 'image' && (
        <span className="aiws-media-badge" data-testid="aiws-media-badge">{facts || fileBadge(mediaType, payload.file_name)}</span>
      )}
      {playing && (
        <div className={`aiws-media-player aiws-preview-host is-${kind}`} data-role="editor" data-testid="aiws-media-player">
          <ContentPreview source={current} variant="inline" autoPlay autoFocus allowExport={false} onRequestExit={stop} data-testid="aiws-media-inline" />
          <button type="button" className="aiws-media-stop" aria-label={t('aiworkspace.blocks.media.stop')} title={t('aiworkspace.blocks.media.stop')} data-testid="aiws-media-stop" onClick={stop}><X size={14} aria-hidden="true" /></button>
        </div>
      )}
      {previewing && <MediaPreviewDialog store={store} cellId={cellId} source={current} name={name} onClose={closePreview} />}
    </div>
  )
}

/** The preview over the workspace (not the canvas world, which is transformed): previous / next go through the media
 * of the same Surface. */
function MediaPreviewDialog({ store, cellId, source, name, onClose }: { store: WorkspaceStore; cellId: string | null; source: ContentRef; name: string; onClose: () => void }) {
  const { t } = useAiwsText()
  const [host, setHost] = useState<HTMLElement | null>(null)
  const [session] = useState(() => (cellId ? surfaceMediaSession(store, cellId, source) : undefined))
  const bindAnchor = (element: HTMLSpanElement | null) => { if (element) setHost(element.closest<HTMLElement>('.aiws-root') ?? document.body) }
  const stop = (event: { stopPropagation: () => void }) => event.stopPropagation()
  return (
    <>
      <span ref={bindAnchor} hidden />
      {host && createPortal(
        <div className={`aiws-media-dialog aiws-preview-host${host === document.body ? ' is-page' : ''}`} role="dialog" aria-modal="true" aria-label={t('aiworkspace.blocks.media.previewOf', { name })}
          data-role="editor" data-testid="aiws-media-preview"
          onPointerDown={stop} onDoubleClick={stop} onContextMenu={stop} onWheel={stop}
          onClick={(event) => { if (event.target === event.currentTarget) onClose() }}>
          <ContentPreview source={source} session={session} uiMode="visible" autoFocus prefetchAdjacent={false} onRequestExit={onClose} className="aiws-media-dialog-body" />
        </div>,
        host,
      )}
    </>
  )
}
