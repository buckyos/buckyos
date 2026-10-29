import { useMemo, type CSSProperties } from 'react'
import { usePreviewThumbnail } from '../../components/preview/thumbnail'
import { homeStationTransport } from './api/transport'
import { fileSourceRef, mediaUrl } from './media'
import type { FileObject, ObjId } from './protocol/feed'

const THUMB_EDGE = 640

/**
 * A feed picture (card, cover, background): the Preview Pipeline's
 * `thumbnail` JPEG, so HEIC/HEVC show on every browser and the feed loads
 * less; the original file only when no Pipeline answers.
 */
export function MediaThumb({ objId, file, alt, className, style, loading }: {
  objId: ObjId
  file?: FileObject
  alt?: string
  className?: string
  style?: CSSProperties
  loading?: 'lazy' | 'eager'
}) {
  const transport = homeStationTransport()
  const source = useMemo(() => (transport ? fileSourceRef(objId, file, alt) : null), [transport, objId, file, alt])
  const thumb = usePreviewThumbnail(source, { width: THUMB_EDGE, height: THUMB_EDGE, prefer: 'pipeline' })
  const src = !source ? mediaUrl(objId, file, alt) : thumb.status === 'ready' ? thumb.thumbnail.url : thumb.status === 'error' ? mediaUrl(objId, file, alt) : undefined
  const via = !source ? 'original' : thumb.status === 'ready' ? thumb.thumbnail.via : thumb.status
  if (!src) return <span className={className} style={style} aria-hidden data-thumb={via} data-object={objId} />
  return <img src={src} alt={alt ?? ''} loading={loading} className={className} style={style} data-thumb={via} data-object={objId} />
}
