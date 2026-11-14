/* A home card's cover (UI 框架 PRD §9.2, K6 — a stopgap until canvases are pictured): `ws.list` and `session.list_open`
 * name the first picture Block of a canvas the caller reads; its bytes come from the asset route with the caller's own
 * token once the card is near the screen (covers load lazily, §9.3) and stay for the page as a blob URL. While it loads,
 * or when it cannot be loaded or drawn, the card keeps its placeholder (an icon on a colour of its own). */

import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import type { Transport } from '../../api/transport'
import type { HomeCover } from '../../api/types'

const pictures = new Map<string, Promise<string>>()

/** The cover's bytes as a blob URL, fetched once per page (a bounded cache: the oldest pictures go, their URLs too). */
function pictureUrl(transport: Transport, cover: HomeCover): Promise<string> {
  const key = `${transport.target}|${cover.workspace_id}/${cover.object_id}`
  let url = pictures.get(key)
  if (!url) {
    url = transport.download(`asset/${cover.workspace_id}/${cover.object_id}`)
      .then((blob) => URL.createObjectURL(blob.type === cover.media_type ? blob : new Blob([blob], { type: cover.media_type })))
    url.catch(() => pictures.delete(key))
    pictures.set(key, url)
    if (pictures.size > 200) {
      const [oldest, pending] = pictures.entries().next().value as [string, Promise<string>]
      pictures.delete(oldest)
      void pending.then((u) => URL.revokeObjectURL(u), () => undefined)
    }
  }
  return url
}

type Shown = { key: string; url: string | null }

/** The cover area of a card: the picture once loaded, else `children` (the placeholder). */
export function CoverPicture({ transport, cover, className, style, children }: {
  transport: Transport
  cover: HomeCover | null | undefined
  className: string
  style?: CSSProperties
  children: ReactNode
}) {
  const ref = useRef<HTMLSpanElement>(null)
  const key = cover ? `${cover.workspace_id}/${cover.object_id}` : null
  const [shown, setShown] = useState<Shown | null>(null)
  useEffect(() => {
    const el = ref.current
    if (!cover || !key || !el) return
    let live = true
    const load = () => pictureUrl(transport, cover).then((url) => { if (live) setShown({ key, url }) }, () => { if (live) setShown({ key, url: null }) })
    if (typeof IntersectionObserver === 'undefined') { void load(); return () => { live = false } }
    const observer = new IntersectionObserver((records) => {
      if (!records.some((r) => r.isIntersecting)) return
      observer.disconnect()
      void load()
    }, { rootMargin: '200px' })
    observer.observe(el)
    return () => { live = false; observer.disconnect() }
    // the cover's identity is its key
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, transport])
  const current = shown && shown.key === key ? shown : null
  const state = !key ? 'none' : !current ? 'loading' : current.url ? 'shown' : 'failed'
  return (
    <span ref={ref} className={`${className}${state === 'shown' ? ' has-picture' : ''}`} style={style} aria-hidden="true" data-testid="aiws-home-cover" data-cover={state}>
      {current?.url
        ? <img className="aiws-home-cover-picture" src={current.url} alt="" draggable={false} data-testid="aiws-home-cover-picture" onError={() => setShown({ key: key!, url: null })} />
        : children}
    </span>
  )
}
