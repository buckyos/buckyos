import { Play } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { fileIcon, fileVisual, wantsThumbnail } from '../../app/filebrowser/fileDisplay'
import type { FileEntry } from '../../app/filebrowser/types'
import { refIdentity } from './session'
import { usePreviewThumbnail } from './thumbnail'
import type { ContentRef } from './types'

/**
 * Contents that answered "no thumbnail" (`UNSUPPORTED`: no converter, no embedded
 * picture) are not asked again this session until they change. Timeouts and
 * network errors are not remembered, and Preview itself is unaffected.
 */
const noThumbnail = new Set<string>()

export interface FileThumbProps {
  entry: Pick<FileEntry, 'kind' | 'name' | 'thumbnailUrl' | 'modifiedAt'>
  /** Where the Preview Pipeline reads the content; null = only the listing's own thumbnail. */
  source: ContentRef | null
  /** Fallback type icon edge. */
  iconSize: number
  /** Pixel box the thumbnail fills (the request size follows it). */
  box: number
  /** `media`: only images and videos (small list rows); `all`: also PDFs and documents. */
  scope?: 'media' | 'all'
  /** Corner marks that keep the type visible over a picture (videos, PDFs, documents). */
  marks?: boolean
}

/**
 * A file's thumbnail: the listing's own when it has one, else the Preview
 * Pipeline's, asked for only once the item scrolls near the view; the type icon
 * when there is none.
 */
export function FileThumb({ entry, source, iconSize, box, scope = 'all', marks = false }: FileThumbProps) {
  const visual = fileVisual(entry)
  const typed = scope === 'media' ? visual === 'image' || visual === 'video' : wantsThumbnail(entry)
  const negativeKey = source ? `${refIdentity(source)}|${entry.modifiedAt}` : null
  const wanted = typed && !entry.thumbnailUrl && !!source && !noThumbnail.has(negativeKey!)
  const element = useRef<HTMLSpanElement>(null)
  const [near, setNear] = useState(() => typeof IntersectionObserver === 'undefined')
  useEffect(() => {
    const el = element.current
    if (!wanted || near || !el) return
    const observer = new IntersectionObserver((records) => {
      if (records.some((record) => record.isIntersecting)) setNear(true)
    }, { rootMargin: '200px' })
    observer.observe(el)
    return () => observer.disconnect()
  }, [wanted, near])
  const request = useMemo(() => ({ width: box, height: box, prefer: 'pipeline' as const }), [box])
  const preview = usePreviewThumbnail(wanted && near ? source : null, request)
  useEffect(() => {
    if (preview.status === 'error' && preview.error.code === 'UNSUPPORTED' && negativeKey) noThumbnail.add(negativeKey)
  }, [preview, negativeKey])
  const [failed, setFailed] = useState<string | null>(null)
  const url = entry.thumbnailUrl ?? (preview.status === 'ready' ? preview.thumbnail.url : undefined)
  const picture = url && failed !== url
  return (
    <span ref={element} className="relative flex size-full items-center justify-center" data-thumb={picture ? 'image' : 'icon'}>
      {picture ? (
        <img src={url} loading="lazy" decoding="async" alt="" draggable={false} className="size-full object-cover" onError={() => setFailed(url)} />
      ) : (
        fileIcon(entry, iconSize)
      )}
      {picture && marks && visual === 'video' ? (
        <span className="pointer-events-none absolute inset-0 flex items-center justify-center" data-testid="thumb-play">
          <span className="flex size-7 items-center justify-center rounded-full bg-black/45 text-white">
            <Play size={14} fill="currentColor" />
          </span>
        </span>
      ) : null}
      {picture && marks && visual !== 'video' && visual !== 'image' ? (
        <span className="pointer-events-none absolute bottom-1 right-1 flex size-6 items-center justify-center rounded-[6px] bg-[color:var(--cp-surface-opaque)] shadow-[0_1px_2px_color-mix(in_srgb,var(--cp-shadow)_20%,transparent)]" data-testid="thumb-type">
          {fileIcon(entry, 14)}
        </span>
      ) : null}
    </span>
  )
}
