import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import { calcGridItemPosition, calcXY, type PositionParams } from 'react-grid-layout/core'
import { longPressMs, longPressTolerance } from './taskbarHooks'

/**
 * Press, drag and hold on a launcher tile. The input decides the gesture, not the screen size: an iPad shows the
 * desktop layout but is driven by touch.
 *
 * - Mouse: a click opens, moving past `mouseDragDistance` drags, the right button opens the menu.
 * - Touch and pen: a tap opens; holding still for `longPressMs` lifts the tile, then moving drags it and letting go
 *   opens the menu; moving past `touchSlop` before that is a page swipe, left to the pager.
 *
 * react-grid-layout only places the tiles. Its own drag cancelled `touchstart` (so iOS never sends the click) and,
 * from 2.3.0, redrew the dragged tile only when it crossed into another cell; here the drag writes the tile's
 * transform every frame and hands the drop to `onMove`.
 */

/** Movement within this is still a tap or a hold; past it, before the hold, it is a swipe. */
export const touchSlop = longPressTolerance
const mouseDragDistance = 4
const liftedDragDistance = 4

export interface LauncherCell {
  i: string
  x: number
  y: number
  w: number
  h: number
}

export interface LauncherGestureOptions {
  /** The tile's cell; null while it has none, and then it cannot be dragged. */
  cell: LauncherCell | null
  /** The page grid's geometry, as react-grid-layout lays it out. */
  grid: PositionParams
  /** A touch tap. Without it a touch tap is left to the element's own click. */
  onTap?: () => void
  onMenu: (point: { x: number; y: number }) => void
  /** Commits a drop; returns the cell the tile ended up in (a collision may put it elsewhere, or back). */
  onMove: (from: LauncherCell, to: LauncherCell) => LauncherCell | null
  /** Stops page swiping while a touch holds or drags the tile. */
  lockPaging: (locked: boolean) => void
}

interface Drag {
  /** Pointer position when the tile started to follow it. */
  fromX: number
  fromY: number
  origin: { left: number; top: number }
  target: { x: number; y: number }
  placeholder: HTMLElement
  frame: number | null
}

interface Press {
  pointerId: number
  touch: boolean
  phase: 'pending' | 'held' | 'dragging'
  /** The react-grid-layout item element, which carries the tile's position. */
  item: HTMLElement
  startX: number
  startY: number
  x: number
  y: number
  heldX: number
  heldY: number
  timer: number | null
  pagingLocked: boolean
  drag: Drag | null
  detach: () => void
}

type PressEnd = 'up' | 'cancel' | 'swipe'

function translate(left: number, top: number) {
  return `translate(${left}px,${top}px)`
}

/**
 * Drops the click a held touch ends with. The menu the hold opens is already up by then, so the click lands on
 * whatever is under the finger, the menu's backdrop too, which would close the menu at once. A new press, or a
 * moment without the click, ends the wait.
 */
function swallowClickAfterHold() {
  const stop = () => {
    window.removeEventListener('click', onClick, true)
    window.removeEventListener('pointerdown', stop, true)
    window.clearTimeout(timer)
  }
  const onClick = (event: MouseEvent) => {
    stop()
    event.preventDefault()
    event.stopPropagation()
  }
  window.addEventListener('click', onClick, true)
  window.addEventListener('pointerdown', stop, true)
  const timer = window.setTimeout(stop, 400)
}

export function useLauncherGesture(options: LauncherGestureOptions) {
  const latest = useRef(options)
  useLayoutEffect(() => {
    latest.current = options
  })
  const press = useRef<Press | null>(null)
  // The click that follows a press the gesture already handled (a touch tap, a hold, a drag, a swipe) is dropped.
  // Reset by the next press or key, so a click that never came cannot swallow a later one.
  const swallowClick = useRef(false)
  const lastPressWasTouch = useRef(false)

  const drawDrag = useCallback((current: Press) => {
    const { drag } = current
    const { cell, grid } = latest.current
    if (!drag || !cell) return
    if (drag.frame !== null) {
      cancelAnimationFrame(drag.frame)
      drag.frame = null
    }
    const left = drag.origin.left + current.x - drag.fromX
    const top = drag.origin.top + current.y - drag.fromY
    current.item.style.transform = translate(left, top)
    const target = calcXY(grid, top, left, cell.w, cell.h)
    if (target.x === drag.target.x && target.y === drag.target.y) return
    drag.target = target
    const slot = calcGridItemPosition(grid, target.x, target.y, cell.w, cell.h)
    drag.placeholder.style.transform = translate(slot.left, slot.top)
  }, [])

  const end = useCallback((how: PressEnd) => {
    const current = press.current
    if (!current) return
    press.current = null
    current.detach()
    if (current.timer !== null) window.clearTimeout(current.timer)
    if (current.pagingLocked) latest.current.lockPaging(false)
    const { drag } = current
    if (drag) {
      if (how === 'up') drawDrag(current)
      if (drag.frame !== null) cancelAnimationFrame(drag.frame)
      drag.placeholder.remove()
    }
    delete current.item.dataset.gesture

    const { cell, onTap, onMenu, onMove, grid } = latest.current
    if (current.phase === 'pending' && how === 'up') {
      // A mouse click, or a touch tap on a tile without `onTap`, goes on to the element's click.
      if (current.touch && onTap) {
        swallowClick.current = true
        onTap()
      }
      return
    }
    swallowClick.current = true
    if (how !== 'up') {
      if (drag && cell) {
        const home = calcGridItemPosition(grid, cell.x, cell.y, cell.w, cell.h)
        current.item.style.transform = translate(home.left, home.top)
      }
      return
    }
    if (current.phase === 'held') {
      swallowClickAfterHold()
      onMenu({ x: current.x, y: current.y })
      return
    }
    if (!drag || !cell) return
    // From under the pointer to the cell the store put it in, carried by the transform transition. React draws the
    // same position when it catches up; writing the old cell meanwhile would send the tile back for a frame.
    const moved = drag.target.x !== cell.x || drag.target.y !== cell.y
    const rest = (moved ? onMove(cell, { ...cell, ...drag.target }) : null) ?? cell
    const slot = calcGridItemPosition(grid, rest.x, rest.y, rest.w, rest.h)
    current.item.style.transform = translate(slot.left, slot.top)
  }, [drawDrag])

  useEffect(() => () => end('cancel'), [end])

  const startDrag = useCallback((current: Press) => {
    const { cell, grid } = latest.current
    if (!cell) return
    const origin = calcGridItemPosition(grid, cell.x, cell.y, cell.w, cell.h)
    const placeholder = document.createElement('div')
    placeholder.className = 'react-grid-item react-grid-placeholder'
    Object.assign(placeholder.style, {
      position: 'absolute',
      top: '0',
      left: '0',
      width: `${origin.width}px`,
      height: `${origin.height}px`,
      transform: translate(origin.left, origin.top),
    })
    current.item.parentElement?.append(placeholder)
    // The tile keeps the spot it was grabbed by: where the press started, or where the hold lifted it.
    const held = current.phase === 'held'
    current.drag = {
      fromX: held ? current.heldX : current.startX,
      fromY: held ? current.heldY : current.startY,
      origin,
      target: { x: cell.x, y: cell.y },
      placeholder,
      frame: null,
    }
    current.phase = 'dragging'
    current.item.dataset.gesture = 'dragging'
    drawDrag(current)
  }, [drawDrag])

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    swallowClick.current = false
    const touch = event.pointerType !== 'mouse'
    lastPressWasTouch.current = touch
    if (press.current) return
    if (!touch && event.button !== 0) return
    if ((event.target as Element).closest('.widget-interactive')) return
    const item = event.currentTarget.closest<HTMLElement>('.react-grid-item')
    if (!item) return
    // No text selection or native drag under a mouse drag. A touch keeps its default: it is what makes the click.
    if (!touch) event.preventDefault()

    const pointerId = event.pointerId
    const onWindowMove = (moveEvent: PointerEvent) => {
      const current = press.current
      if (!current || moveEvent.pointerId !== pointerId) return
      current.x = moveEvent.clientX
      current.y = moveEvent.clientY
      if (current.phase === 'dragging') {
        const { drag } = current
        if (drag && drag.frame === null) {
          drag.frame = requestAnimationFrame(() => {
            drag.frame = null
            drawDrag(current)
          })
        }
        return
      }
      if (current.phase === 'held') {
        if (Math.hypot(current.x - current.heldX, current.y - current.heldY) > liftedDragDistance) startDrag(current)
        return
      }
      const distance = Math.hypot(current.x - current.startX, current.y - current.startY)
      if (current.touch) {
        if (distance > touchSlop) end('swipe')
      } else if (distance > mouseDragDistance) {
        startDrag(current)
      }
    }
    const onWindowUp = (upEvent: PointerEvent) => {
      if (upEvent.pointerId === pointerId) end('up')
    }
    const onWindowCancel = (cancelEvent: PointerEvent) => {
      if (cancelEvent.pointerId === pointerId) end('cancel')
    }
    window.addEventListener('pointermove', onWindowMove)
    window.addEventListener('pointerup', onWindowUp)
    window.addEventListener('pointercancel', onWindowCancel)

    const current: Press = {
      pointerId,
      touch,
      phase: 'pending',
      item,
      startX: event.clientX,
      startY: event.clientY,
      x: event.clientX,
      y: event.clientY,
      heldX: event.clientX,
      heldY: event.clientY,
      timer: null,
      pagingLocked: false,
      drag: null,
      detach: () => {
        window.removeEventListener('pointermove', onWindowMove)
        window.removeEventListener('pointerup', onWindowUp)
        window.removeEventListener('pointercancel', onWindowCancel)
      },
    }
    if (touch) {
      current.timer = window.setTimeout(() => {
        current.timer = null
        if (press.current !== current || current.phase !== 'pending') return
        current.phase = 'held'
        current.heldX = current.x
        current.heldY = current.y
        current.item.dataset.gesture = 'held'
        current.pagingLocked = true
        latest.current.lockPaging(true)
      }, longPressMs)
    }
    press.current = current
  }, [drawDrag, end, startDrag])

  const onClickCapture = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (!swallowClick.current) return
    swallowClick.current = false
    event.preventDefault()
    event.stopPropagation()
  }, [])

  const onKeyDownCapture = useCallback(() => {
    swallowClick.current = false
  }, [])

  // A touch hold opens the menu itself; the browser's long-press menu (Android) is dropped.
  const onContextMenu = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    event.preventDefault()
    if (lastPressWasTouch.current) return
    latest.current.onMenu({ x: event.clientX, y: event.clientY })
  }, [])

  return { onPointerDown, onClickCapture, onKeyDownCapture, onContextMenu }
}

/** What a press on the desktop's empty space leaves alone: the tiles, the page dots and any control. */
const notBackdrop = '.react-grid-item, .swiper-pagination, button, a, input, textarea, select, [contenteditable="true"]'

function onBackdrop(target: EventTarget | null) {
  return target instanceof Element && !target.closest(notBackdrop)
}

/**
 * The menu of the desktop's empty space, by the same rules as a tile's: the right button, or a touch held still for
 * `longPressMs` and let go there. Moving first is a page swipe.
 */
export function useBackdropMenuGesture(onMenu: (point: { x: number; y: number }) => void) {
  const latest = useRef(onMenu)
  useLayoutEffect(() => {
    latest.current = onMenu
  })
  const press = useRef<{ end: (how: PressEnd) => void } | null>(null)
  const swallowClick = useRef(false)
  const lastPressWasTouch = useRef(false)

  useEffect(() => () => press.current?.end('cancel'), [])

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    swallowClick.current = false
    const touch = event.pointerType !== 'mouse'
    lastPressWasTouch.current = touch
    if (!touch || press.current || !onBackdrop(event.target)) return
    const pointerId = event.pointerId
    const start = { x: event.clientX, y: event.clientY }
    const point = { ...start }
    let held = false
    const timer = window.setTimeout(() => {
      held = true
    }, longPressMs)
    const onWindowMove = (moveEvent: PointerEvent) => {
      if (moveEvent.pointerId !== pointerId) return
      point.x = moveEvent.clientX
      point.y = moveEvent.clientY
      if (Math.hypot(point.x - start.x, point.y - start.y) > touchSlop) end('swipe')
    }
    const onWindowUp = (upEvent: PointerEvent) => {
      if (upEvent.pointerId === pointerId) end('up')
    }
    const onWindowCancel = (cancelEvent: PointerEvent) => {
      if (cancelEvent.pointerId === pointerId) end('cancel')
    }
    const end = (how: PressEnd) => {
      if (press.current !== current) return
      press.current = null
      window.clearTimeout(timer)
      window.removeEventListener('pointermove', onWindowMove)
      window.removeEventListener('pointerup', onWindowUp)
      window.removeEventListener('pointercancel', onWindowCancel)
      if (how !== 'up' || !held) return
      swallowClick.current = true
      swallowClickAfterHold()
      latest.current({ ...point })
    }
    const current = { end }
    window.addEventListener('pointermove', onWindowMove)
    window.addEventListener('pointerup', onWindowUp)
    window.addEventListener('pointercancel', onWindowCancel)
    press.current = current
  }, [])

  const onClickCapture = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (!swallowClick.current) return
    swallowClick.current = false
    event.preventDefault()
    event.stopPropagation()
  }, [])

  // A tile's own menu has handled its right click already; the browser's long-press menu (Android) is dropped.
  const onContextMenu = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (!onBackdrop(event.target)) return
    event.preventDefault()
    if (lastPressWasTouch.current) return
    latest.current({ x: event.clientX, y: event.clientY })
  }, [])

  return { onPointerDown, onClickCapture, onContextMenu }
}
