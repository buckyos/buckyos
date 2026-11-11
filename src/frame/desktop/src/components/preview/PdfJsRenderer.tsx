/* PDF drawn by pdf.js: where the Runtime has no built-in viewer (Android Chrome) and on touch devices. Pages scroll
 * in one column; only the pages in view and their neighbours hold a canvas. A URL is read by HTTP Range, so a long
 * document opens on its first pages. */

import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask } from 'pdfjs-dist'
import { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type Ref } from 'react'
import { loadPdfjs, pdfjsAssetOptions } from './pdfjs'
import { usePinchZoom, type TranslateFn, type ZoomHandle, type ZoomViewInfo } from './pinchZoom'
import { RendererNotice } from './RendererNotice'
import type { PreviewReadRef } from './types'

const PAGE_GAP = 8
const PAD = 8
const PAD_TOP = 48
const MAX_FIT_WIDTH = 1100
const MIN_ZOOM = 0.5
const MAX_ZOOM = 5
// iOS refuses a canvas above 16 777 216 pixels.
const CANVAS_PIXEL_LIMIT = 16_000_000
const RANGE_CHUNK = 256 * 1024

interface PageSize {
  width: number
  height: number
}

export function PdfJsRenderer({
  ref,
  readRef,
  title,
  onRendered,
  onFailure,
  onDownload,
  onOpenWith,
  onViewChange,
  t,
}: {
  ref: Ref<ZoomHandle>
  readRef: PreviewReadRef
  title: string
  onRendered: (media?: { width?: number; height?: number; pageCount?: number }, opts?: { degraded?: boolean }) => void
  onFailure: (failure: 'corrupted' | 'network') => void
  onDownload: () => void
  onOpenWith?: () => void
  onViewChange: (view: ZoomViewInfo | null) => void
  t: TranslateFn
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const contentRef = useRef<HTMLDivElement | null>(null)
  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null)
  const [sizes, setSizes] = useState<PageSize[]>([])
  const [locked, setLocked] = useState(false)
  const [zoom, setZoom] = useState(1)
  const [viewportWidth, setViewportWidth] = useState(0)
  const [range, setRange] = useState({ first: 0, last: 0, current: 0 })
  const [pageDraft, setPageDraft] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    let task: PDFDocumentLoadingTask | null = null
    void (async () => {
      try {
        const pdfjs = await loadPdfjs()
        const source = readRef.kind === 'url'
          ? { url: readRef.url, withCredentials: true, rangeChunkSize: RANGE_CHUNK, disableAutoFetch: true, disableStream: true }
          : { data: new Uint8Array(await readRef.blob.arrayBuffer()) }
        if (cancelled) return
        task = pdfjs.getDocument({ ...source, ...pdfjsAssetOptions() })
        const loaded = await task.promise
        const viewport = (await loaded.getPage(1)).getViewport({ scale: 1 })
        if (cancelled) return
        setSizes(Array.from({ length: loaded.numPages }, () => ({ width: viewport.width, height: viewport.height })))
        setDoc(loaded)
      } catch (err) {
        if (cancelled) return
        const name = (err as { name?: string } | null)?.name ?? ''
        if (name === 'PasswordException') {
          setLocked(true)
          onRendered(undefined, { degraded: true })
        } else {
          onFailure(/Response|Abort/.test(name) || err instanceof TypeError ? 'network' : 'corrupted')
        }
      }
    })()
    return () => {
      cancelled = true
      void task?.destroy().catch(() => undefined)
    }
  }, [onFailure, onRendered, readRef])

  useLayoutEffect(() => {
    const scroller = scrollRef.current
    if (!scroller) return
    const measure = () => setViewportWidth(scroller.clientWidth)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(scroller)
    return () => observer.disconnect()
  }, [doc])

  const layout = useMemo(() => {
    const pageWidth = Math.min(Math.max(viewportWidth - 2 * PAD, 48), MAX_FIT_WIDTH) * zoom
    const tops: number[] = []
    const heights: number[] = []
    let top = PAD_TOP
    for (const size of sizes) {
      const height = (pageWidth * size.height) / Math.max(size.width, 1)
      tops.push(top)
      heights.push(height)
      top += height + PAGE_GAP
    }
    const width = Math.max(viewportWidth, pageWidth + 2 * PAD)
    return { pageWidth, tops, heights, width, height: top + PAD, left: (width - pageWidth) / 2 }
  }, [sizes, viewportWidth, zoom])

  const updateRange = useCallback(() => {
    const scroller = scrollRef.current
    if (!scroller || layout.tops.length === 0) return
    const top = scroller.scrollTop
    const bottom = top + scroller.clientHeight
    const middle = top + scroller.clientHeight / 2
    let first = -1
    let last = 0
    let current = 0
    for (let i = 0; i < layout.tops.length; i += 1) {
      const pageTop = layout.tops[i]
      const pageBottom = pageTop + layout.heights[i]
      if (pageBottom < top) continue
      if (pageTop > bottom) break
      if (first < 0) first = i
      last = i
      if (pageTop <= middle) current = i
    }
    if (first < 0) first = last = current = layout.tops.length - 1
    setRange((prev) => (prev.first === first && prev.last === last && prev.current === current ? prev : { first, last, current }))
  }, [layout])

  useLayoutEffect(updateRange, [updateRange])

  const zoomTo = usePinchZoom(scrollRef, contentRef, zoom, MIN_ZOOM, MAX_ZOOM, setZoom, doc !== null)
  useImperativeHandle(ref, () => ({ zoomBy: (factor) => zoomTo(zoom * factor), fit: () => zoomTo(1) }), [zoom, zoomTo])
  useEffect(() => {
    if (doc) onViewChange({ scale: zoom, mode: zoom === 1 ? 'fit' : 'custom', rotation: 0 })
  }, [doc, onViewChange, zoom])

  const painted = useRef(false)
  const handlePainted = useCallback(() => {
    if (painted.current || !doc) return
    painted.current = true
    onRendered({ pageCount: doc.numPages })
  }, [doc, onRendered])

  const handleSize = useCallback((index: number, width: number, height: number) => {
    setSizes((prev) => {
      const known = prev[index]
      if (!known || (Math.abs(known.width - width) < 0.5 && Math.abs(known.height - height) < 0.5)) return prev
      const next = prev.slice()
      next[index] = { width, height }
      return next
    })
  }, [])

  const goToPage = (value: string) => {
    setPageDraft(null)
    const page = Math.round(Number(value))
    const scroller = scrollRef.current
    if (!scroller || !Number.isFinite(page) || page < 1 || layout.tops.length === 0) return
    scroller.scrollTo({ top: layout.tops[Math.min(page, layout.tops.length) - 1] - PAD_TOP })
  }

  if (locked) {
    return (
      <RendererNotice
        testId="content-preview-pdf-locked"
        title={t('preview.pdf.locked', 'This PDF is password protected')}
        body={t('preview.pdf.lockedBody', 'Download the file or open it with another application to enter the password.')}
        onDownload={onDownload}
        onOpenWith={onOpenWith}
        t={t}
      />
    )
  }
  if (!doc) return null

  const pages: number[] = []
  for (let i = Math.max(0, range.first - 1); i <= Math.min(sizes.length - 1, range.last + 1); i += 1) pages.push(i)

  return (
    <>
      <div
        ref={scrollRef}
        className="desktop-scrollbar absolute inset-0 overflow-auto bg-[color:var(--cp-surface-2-opaque)]"
        style={{ touchAction: 'pan-x pan-y' }}
        data-testid="content-preview-pdfjs"
        data-page-count={doc.numPages}
        data-page={range.current + 1}
        data-zoom={zoom}
        aria-label={title}
        onScroll={updateRange}
      >
        <div ref={contentRef} className="relative" style={{ width: layout.width, height: layout.height }}>
          {pages.map((index) => (
            <PdfPage
              key={index}
              doc={doc}
              index={index}
              top={layout.tops[index]}
              left={layout.left}
              width={layout.pageWidth}
              height={layout.heights[index]}
              label={t('preview.pdf.page', 'Page {{page}}', { page: index + 1 })}
              onSize={handleSize}
              onPainted={handlePainted}
            />
          ))}
        </div>
      </div>
      <form
        className="absolute bottom-3 left-1/2 z-10 flex -translate-x-1/2 items-center gap-1 rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_92%,transparent)] px-3 py-1 text-[12px] tabular-nums text-[color:var(--cp-text)] shadow"
        onSubmit={(event) => {
          event.preventDefault()
          goToPage(pageDraft ?? '')
        }}
      >
        <input
          className="w-10 select-text bg-transparent text-right outline-none"
          inputMode="numeric"
          aria-label={t('preview.pdf.goToPage', 'Go to page')}
          data-testid="content-preview-pdf-page"
          value={pageDraft ?? String(range.current + 1)}
          onChange={(event) => setPageDraft(event.target.value.replace(/\D/g, ''))}
          onFocus={(event) => event.target.select()}
          onBlur={() => (pageDraft === null ? undefined : goToPage(pageDraft))}
        />
        <span className="text-[color:var(--cp-muted)]">/ {doc.numPages}</span>
      </form>
    </>
  )
}

function PdfPage({
  doc,
  index,
  top,
  left,
  width,
  height,
  label,
  onSize,
  onPainted,
}: {
  doc: PDFDocumentProxy
  index: number
  top: number
  left: number
  width: number
  height: number
  label: string
  onSize: (index: number, width: number, height: number) => void
  onPainted: () => void
}) {
  const holderRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    let cancelled = false
    let task: RenderTask | null = null
    void (async () => {
      try {
        const page = await doc.getPage(index + 1)
        if (cancelled) return
        const base = page.getViewport({ scale: 1 })
        onSize(index, base.width, base.height)
        let scale = (width * Math.min(window.devicePixelRatio || 1, 3)) / base.width
        const pixels = base.width * base.height * scale * scale
        if (pixels > CANVAS_PIXEL_LIMIT) scale *= Math.sqrt(CANVAS_PIXEL_LIMIT / pixels)
        const viewport = page.getViewport({ scale })
        const canvas = document.createElement('canvas')
        canvas.width = Math.max(1, Math.floor(viewport.width))
        canvas.height = Math.max(1, Math.floor(viewport.height))
        canvas.style.cssText = 'display:block;width:100%;height:100%'
        task = page.render({ canvas, viewport })
        await task.promise
        const holder = holderRef.current
        if (cancelled || !holder) {
          canvas.width = 0
          return
        }
        const previous = holder.firstElementChild as HTMLCanvasElement | null
        holder.replaceChildren(canvas)
        if (previous) previous.width = 0
        holder.dataset.rendered = 'true'
        onPainted()
      } catch {
        // A cancelled render (the page left the view or was zoomed) is not a failure.
      }
    })()
    return () => {
      cancelled = true
      task?.cancel()
    }
  }, [doc, index, onPainted, onSize, width])

  useEffect(() => {
    const holder = holderRef.current
    return () => {
      const canvas = holder?.firstElementChild as HTMLCanvasElement | null
      if (canvas) canvas.width = 0
    }
  }, [])

  return (
    <div
      ref={holderRef}
      role="img"
      aria-label={label}
      className="absolute bg-white shadow"
      style={{ top, left, width, height }}
      data-testid="content-preview-pdf-canvas"
      data-page-index={index}
    />
  )
}
