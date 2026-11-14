/* A tap and a long press on the same element (UI 框架 PRD §7.3 "长按卡片进入多选"): a press held still for half a
 * second is a long press and the click after it is swallowed; moving the finger (scrolling) cancels it. */

import { useEffect, useRef, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react'

const LONG_MS = 500
const SLOP = 10

export function usePress({ onTap, onLong }: { onTap: () => void; onLong: () => void }) {
  const timer = useRef<number | null>(null)
  const start = useRef<{ x: number; y: number } | null>(null)
  const fired = useRef(false)
  const handlers = useRef({ onTap, onLong })
  useEffect(() => { handlers.current = { onTap, onLong } })
  useEffect(() => () => { if (timer.current !== null) window.clearTimeout(timer.current) }, [])
  const clear = () => { if (timer.current !== null) { window.clearTimeout(timer.current); timer.current = null } }
  return {
    onPointerDown: (event: ReactPointerEvent) => {
      if (event.button !== 0) return
      fired.current = false
      start.current = { x: event.clientX, y: event.clientY }
      clear()
      timer.current = window.setTimeout(() => { timer.current = null; fired.current = true; handlers.current.onLong() }, LONG_MS)
    },
    onPointerMove: (event: ReactPointerEvent) => {
      const s = start.current
      if (s && Math.hypot(event.clientX - s.x, event.clientY - s.y) > SLOP) { clear(); start.current = null }
    },
    onPointerUp: () => { clear(); start.current = null },
    onPointerCancel: () => { clear(); start.current = null },
    onClick: (event: ReactMouseEvent) => {
      if (fired.current) { fired.current = false; event.preventDefault(); return }
      handlers.current.onTap()
    },
    // the browser's own long-press menu would cover the selection
    onContextMenu: (event: ReactMouseEvent) => { if (start.current || fired.current) event.preventDefault() },
  }
}
