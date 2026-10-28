/* A mind map on a flow page (脑图方案 §6.3, TODO M1): the whole control as one self-contained Block of fixed height,
 * drawn as static SVG from the same layout (the reader's folds included) and moved inside its box — drag to pan,
 * Ctrl+wheel or the buttons to zoom, "fit" to see all of it. Editing happens on a free canvas. */

import { useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent } from 'react'
import { Maximize, ZoomIn, ZoomOut } from 'lucide-react'
import { controlSvg } from '@controls/svg'
import { useOutlineVersion, useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { foldBook } from '../folds'
import { nodeMeasurer } from '../measure'
import { controlRegistry } from '../registry'

const HEIGHT = 360
const THEME = { fill: 'var(--cp-surface-opaque, #fff)', rootFill: 'var(--cp-text, #1f2937)', text: 'var(--cp-text, #1f2937)', rootText: 'var(--cp-surface-opaque, #fff)', stroke: 'var(--cp-border, #d0d5dd)' }

export function FlowMindmap({ containerId }: { containerId: string }) {
  const store = useStore()
  const { t } = useAiwsText()
  const outlineVersion = useOutlineVersion()
  const folds = foldBook(store)
  const foldsVersion = useSyncExternalStore(folds.subscribe, folds.snapshot)
  const kind = store.outline.get(containerId)?.kind
  const { svg, w, h } = useMemo(() => {
    const def = controlRegistry.get(kind)
    if (!def) return { svg: '', w: 0, h: 0 }
    const solved = def.layout.solve(store.outline, containerId, { measurer: nodeMeasurer, collapsed: (id) => folds.folded(id) })
    const svg = controlSvg(store.outline, solved, { measurer: nodeMeasurer, theme: THEME })
    return { svg, w: solved.layout.bounds.w + 16, h: solved.layout.bounds.h + 16 }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion / foldsVersion are the invalidation signals
  }, [store, containerId, kind, folds, outlineVersion, foldsVersion])
  const boxRef = useRef<HTMLDivElement>(null)
  const [view, setView] = useState<{ x: number; y: number; k: number } | null>(null)
  const fit = () => {
    const box = boxRef.current
    if (!box || !w || !h) return
    const k = Math.min(1, (box.clientWidth - 16) / w, (HEIGHT - 16) / h)
    setView({ k, x: (box.clientWidth - w * k) / 2, y: (HEIGHT - h * k) / 2 })
  }
  // fitted on first sight and whenever the map's extent changes
  useLayoutEffect(fit, [w, h])
  const drag = useRef<{ x: number; y: number; vx: number; vy: number } | null>(null)
  const zoom = (factor: number, at?: { x: number; y: number }) => setView((v) => {
    if (!v) return v
    const k = Math.min(4, Math.max(0.1, v.k * factor))
    const p = at ?? { x: (boxRef.current?.clientWidth ?? 0) / 2, y: HEIGHT / 2 }
    return { k, x: p.x - ((p.x - v.x) * k) / v.k, y: p.y - ((p.y - v.y) * k) / v.k }
  })
  const down = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !view || (event.target as Element).closest('button')) return
    event.currentTarget.setPointerCapture(event.pointerId)
    drag.current = { x: event.clientX, y: event.clientY, vx: view.x, vy: view.y }
  }
  const move = (event: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current
    if (d) setView((v) => (v ? { ...v, x: d.vx + event.clientX - d.x, y: d.vy + event.clientY - d.y } : v))
  }
  useLayoutEffect(() => {
    const box = boxRef.current
    if (!box) return
    // Ctrl+wheel zooms inside the map; a plain wheel scrolls the page
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return
      event.preventDefault()
      const r = box.getBoundingClientRect()
      zoom(Math.exp(-event.deltaY * 0.0015), { x: event.clientX - r.left, y: event.clientY - r.top })
    }
    box.addEventListener('wheel', wheel, { passive: false })
    return () => box.removeEventListener('wheel', wheel)
  })
  return (
    <div ref={boxRef} className="aiws-flow-mindmap" style={{ height: HEIGHT }} data-testid={`aiws-flow-mindmap-${containerId}`}
      onPointerDown={down} onPointerMove={move} onPointerUp={() => { drag.current = null }} onPointerCancel={() => { drag.current = null }}>
      <div className="aiws-flow-mindmap-art" style={view ? { transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})` } : { visibility: 'hidden' }} dangerouslySetInnerHTML={{ __html: svg }} />
      <div className="aiws-flow-mindmap-tools">
        <button type="button" aria-label={t('aiworkspace.mindmap.zoomIn')} title={t('aiworkspace.mindmap.zoomIn')} onClick={() => zoom(1.25)}><ZoomIn size={14} aria-hidden="true" /></button>
        <button type="button" aria-label={t('aiworkspace.mindmap.zoomOut')} title={t('aiworkspace.mindmap.zoomOut')} onClick={() => zoom(0.8)}><ZoomOut size={14} aria-hidden="true" /></button>
        <button type="button" aria-label={t('aiworkspace.mindmap.fit')} title={t('aiworkspace.mindmap.fit')} data-testid={`aiws-flow-mindmap-fit-${containerId}`} onClick={fit}><Maximize size={14} aria-hidden="true" /></button>
      </div>
    </div>
  )
}
