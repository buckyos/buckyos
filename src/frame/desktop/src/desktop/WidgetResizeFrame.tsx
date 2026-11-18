import { useEffect, useRef, type PointerEvent as ReactPointerEvent } from 'react'
import { calcGridColWidth, calcGridItemPosition, type PositionParams } from 'react-grid-layout/core'
import { resizeGridRect, type GridRect, type ResizeEdges } from '../models/layout'
import type { WidgetSize } from './widgets/catalog'

interface Handle extends ResizeEdges {
  name: 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw'
}

const handles: readonly Handle[] = [
  { name: 'nw', dx: -1, dy: -1 },
  { name: 'n', dx: 0, dy: -1 },
  { name: 'ne', dx: 1, dy: -1 },
  { name: 'e', dx: 1, dy: 0 },
  { name: 'se', dx: 1, dy: 1 },
  { name: 's', dx: 0, dy: 1 },
  { name: 'sw', dx: -1, dy: 1 },
  { name: 'w', dx: -1, dy: 0 },
]

function sameRect(a: GridRect, b: GridRect) {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h
}

/**
 * The handles of a widget being resized (its menu's "Resize"). Dragging one moves that edge or corner a whole cell
 * at a time, within the widget's own size limits; the tile follows, and the size is kept when the handle is let go.
 * Only the axes the widget can change get handles.
 */
export function WidgetResizeFrame({
  cell,
  grid,
  itemId,
  limits,
  lockPaging,
  onResize,
}: {
  cell: GridRect
  grid: PositionParams
  itemId: string
  limits: { min: WidgetSize; max: WidgetSize }
  lockPaging: (locked: boolean) => void
  /** Commits the cells the handle was let go on; returns where the widget ended up (others may not make room). */
  onResize: (next: GridRect) => GridRect | null
}) {
  const latest = useRef({ cell, grid, limits, lockPaging, onResize })
  useEffect(() => {
    latest.current = { cell, grid, limits, lockPaging, onResize }
  })
  const active = useRef<(() => void) | null>(null)
  useEffect(() => () => active.current?.(), [])

  const resizesW = limits.min.w !== limits.max.w
  const resizesH = limits.min.h !== limits.max.h
  const shown = handles.filter((handle) => (handle.dx === 0 || resizesW) && (handle.dy === 0 || resizesH))

  const startResize = (handle: Handle) => (event: ReactPointerEvent<HTMLSpanElement>) => {
    if (event.pointerType === 'mouse' && event.button !== 0) return
    if (active.current) return
    event.preventDefault()
    event.stopPropagation()
    const item = event.currentTarget.closest<HTMLElement>('.react-grid-item')
    if (!item) return
    const { cell: start, grid: params, limits: bounds } = latest.current
    const pointerId = event.pointerId
    const origin = { x: event.clientX, y: event.clientY }
    const colStep = calcGridColWidth(params) + params.margin[0]
    const rowStep = params.rowHeight + params.margin[1]
    let rect = start

    const draw = (target: GridRect) => {
      const position = calcGridItemPosition(params, target.x, target.y, target.w, target.h)
      item.style.width = `${position.width}px`
      item.style.height = `${position.height}px`
      item.style.transform = `translate(${position.left}px,${position.top}px)`
    }
    const onMove = (moveEvent: PointerEvent) => {
      if (moveEvent.pointerId !== pointerId) return
      const next = resizeGridRect(
        start,
        handle,
        {
          cols: Math.round((moveEvent.clientX - origin.x) / colStep),
          rows: Math.round((moveEvent.clientY - origin.y) / rowStep),
        },
        bounds,
        { cols: params.cols, rows: params.maxRows },
      )
      if (sameRect(next, rect)) return
      rect = next
      draw(rect)
    }
    const finish = (commit: boolean) => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      active.current = null
      latest.current.lockPaging(false)
      delete item.dataset.resizing
      // The cells the store kept: the new ones, or the old ones when the others could not make room.
      draw(commit && !sameRect(rect, start) ? latest.current.onResize(rect) ?? start : start)
    }
    const onUp = (upEvent: PointerEvent) => {
      if (upEvent.pointerId === pointerId) finish(true)
    }
    const onCancel = (cancelEvent: PointerEvent) => {
      if (cancelEvent.pointerId === pointerId) finish(false)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
    active.current = () => finish(false)
    item.dataset.resizing = 'true'
    latest.current.lockPaging(true)
  }

  return (
    <div className="desktop-resize-frame" data-item-id={itemId} data-testid={`desktop-resize-${itemId}`}>
      {shown.map((handle) => (
        <span
          key={handle.name}
          // Swiper leaves a press here alone, so a touch drags the handle instead of the page.
          className="desktop-resize-handle swiper-no-swiping"
          data-edge={handle.name}
          data-testid={`desktop-resize-handle-${handle.name}`}
          aria-hidden="true"
          onPointerDown={startResize(handle)}
        />
      ))}
    </div>
  )
}
