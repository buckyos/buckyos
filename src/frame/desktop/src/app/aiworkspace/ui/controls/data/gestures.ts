/* Gestures of a data control (控件机制 TODO G4): a press on a projected Block becomes a tap, a double tap, a long press or
 * a drag. A mouse drags after a few pixels; a finger scrolls the list as usual and drags only after holding still
 * (a long press, with a short vibration) — the control may also offer "Move to…" for it. While a drag runs only the
 * preview changes (the control's Session state); the drop is one Commit, Escape or a cancelled pointer drops nothing.
 * Coordinates are the control's own (`toLocal` turns client points into them, so the canvas zoom does not matter). */

const MOUSE_SLOP = 4
const TOUCH_SLOP = 8
const LONG_PRESS_MS = 420
const DOUBLE_MS = 320

export interface Point { x: number; y: number }

export interface PressHandlers {
  onTap?: (p: Point, event: PointerEvent) => void
  onDoubleTap?: (p: Point, event: PointerEvent) => void
  /** A touch held still (when no drag is wanted, or before it moves); a mouse never long-presses. */
  onLongPress?: (p: Point) => void
  /** The drag begins (after the slop, or the long press on touch); false refuses it. */
  onDragStart?: (start: Point, event: PointerEvent) => boolean
  onDragMove?: (p: Point, start: Point) => void
  onDragEnd?: (p: Point, start: Point, cancelled: boolean) => void
}

let lastTap: { key: string; at: number } | null = null

/** Follow one press from its pointerdown. `key` names what was pressed (a double tap needs two on the same thing). */
export function trackPress(down: PointerEvent, key: string, toLocal: (clientX: number, clientY: number) => Point, handlers: PressHandlers) {
  if (down.button !== 0 && down.pointerType === 'mouse') return
  const touch = down.pointerType !== 'mouse'
  const start = toLocal(down.clientX, down.clientY)
  const startClient = { x: down.clientX, y: down.clientY }
  const target = down.target as Element | null
  let dragging = false
  let longPressed = false
  let done = false
  let timer: number | null = null
  // a finger that held still may drag: the page must not scroll from then on
  const blockScroll = (event: TouchEvent) => { if (dragging || longPressed) event.preventDefault() }
  const finish = () => {
    done = true
    if (timer !== null) window.clearTimeout(timer)
    window.removeEventListener('pointermove', move, true)
    window.removeEventListener('pointerup', up, true)
    window.removeEventListener('pointercancel', cancel, true)
    window.removeEventListener('keydown', key_, true)
    window.removeEventListener('touchmove', blockScroll)
  }
  const beginDrag = (event: PointerEvent): boolean => {
    if (!handlers.onDragStart) return false
    if (!handlers.onDragStart(start, event)) return false
    dragging = true
    try { (target as HTMLElement | null)?.setPointerCapture?.(down.pointerId) } catch { /* the element may be gone */ }
    return true
  }
  const move = (event: PointerEvent) => {
    if (event.pointerId !== down.pointerId || done) return
    const p = toLocal(event.clientX, event.clientY)
    if (dragging) { event.preventDefault(); handlers.onDragMove?.(p, start); return }
    const dist = Math.hypot(event.clientX - startClient.x, event.clientY - startClient.y)
    if (touch) {
      // moving before the long press: the finger scrolls, this press is over
      if (!longPressed && dist > TOUCH_SLOP) { finish(); return }
      if (longPressed && dist > TOUCH_SLOP && beginDrag(event)) handlers.onDragMove?.(p, start)
      return
    }
    if (dist > MOUSE_SLOP && beginDrag(event)) handlers.onDragMove?.(p, start)
  }
  const up = (event: PointerEvent) => {
    if (event.pointerId !== down.pointerId || done) return
    const p = toLocal(event.clientX, event.clientY)
    finish()
    if (dragging) { handlers.onDragEnd?.(p, start, false); return }
    if (longPressed) { handlers.onLongPress?.(start); return }
    const now = performance.now()
    if (lastTap && lastTap.key === key && now - lastTap.at < DOUBLE_MS && handlers.onDoubleTap) {
      lastTap = null
      handlers.onDoubleTap(p, event)
      return
    }
    lastTap = { key, at: now }
    handlers.onTap?.(p, event)
  }
  const cancel = (event: PointerEvent) => {
    if (event.pointerId !== down.pointerId || done) return
    finish()
    if (dragging) handlers.onDragEnd?.(start, start, true)
  }
  const key_ = (event: KeyboardEvent) => {
    if (event.key !== 'Escape' || done) return
    event.preventDefault()
    event.stopPropagation()
    finish()
    if (dragging) handlers.onDragEnd?.(start, start, true)
  }
  if (touch) {
    timer = window.setTimeout(() => {
      if (done) return
      longPressed = true
      try { navigator.vibrate?.(12) } catch { /* not supported */ }
      // without a drag, the long press is answered when the finger lifts (a menu); with one, by moving
      if (!handlers.onDragStart) { finish(); handlers.onLongPress?.(start) }
    }, LONG_PRESS_MS)
    window.addEventListener('touchmove', blockScroll, { passive: false })
  }
  window.addEventListener('pointermove', move, true)
  window.addEventListener('pointerup', up, true)
  window.addEventListener('pointercancel', cancel, true)
  window.addEventListener('keydown', key_, true)
}

/** Client points into an element's own coordinates (its CSS size), whatever transform scales it. */
export function localPoint(el: HTMLElement | null, clientX: number, clientY: number): Point {
  if (!el) return { x: clientX, y: clientY }
  const r = el.getBoundingClientRect()
  const sx = el.offsetWidth ? r.width / el.offsetWidth : 1
  const sy = el.offsetHeight ? r.height / el.offsetHeight : 1
  return { x: (clientX - r.left) / (sx || 1) + el.scrollLeft, y: (clientY - r.top) / (sy || 1) + el.scrollTop }
}

/** Scroll a container while a drag holds the pointer near its edge (one step per call; call it from onDragMove). */
export function edgeScroll(el: HTMLElement | null, clientX: number, clientY: number, axis: 'x' | 'y' | 'both' = 'y') {
  if (!el) return
  const r = el.getBoundingClientRect()
  const band = 32
  const step = 14
  if (axis !== 'x') {
    if (clientY < r.top + band) el.scrollTop -= step
    else if (clientY > r.bottom - band) el.scrollTop += step
  }
  if (axis !== 'y') {
    if (clientX < r.left + band) el.scrollLeft -= step
    else if (clientX > r.right - band) el.scrollLeft += step
  }
}
