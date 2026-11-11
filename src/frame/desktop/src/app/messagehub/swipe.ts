import { useRef } from 'react'

const MIN_DISTANCE = 64
const MAX_DURATION_MS = 700

/** Whether something between `target` and `root` scrolls sideways (a wide code block, a table): a swipe there belongs to it. */
function scrollsSideways(target: EventTarget | null, root: HTMLElement): boolean {
  for (let node = target instanceof Element ? target : null; node && node !== root; node = node.parentElement) {
    if (node.scrollWidth > node.clientWidth + 1 && /(auto|scroll)/.test(getComputedStyle(node).overflowX)) return true
  }
  return false
}

/**
 * A horizontal one-finger swipe over an element: far enough, quick, and
 * clearly more sideways than vertical, so scrolling the list never counts.
 * Touches that start on a control that owns its gestures (`data-no-swipe`,
 * a text field, a range slider) or inside something that scrolls sideways
 * are left alone.
 */
export function useSwipe({ onLeft, onRight }: { onLeft?: () => void; onRight?: () => void }) {
  const start = useRef<{ x: number; y: number; at: number } | null>(null)
  if (!onLeft && !onRight) return {}
  return {
    onTouchStart: (event: React.TouchEvent<HTMLElement>) => {
      const target = event.target
      const owned = target instanceof Element && target.closest('[data-no-swipe], input, textarea, [contenteditable="true"]')
      start.current = event.touches.length === 1 && !owned && !scrollsSideways(target, event.currentTarget) ? { x: event.touches[0].clientX, y: event.touches[0].clientY, at: Date.now() } : null
    },
    onTouchMove: (event: React.TouchEvent<HTMLElement>) => { if (event.touches.length !== 1) start.current = null },
    onTouchCancel: () => { start.current = null },
    onTouchEnd: (event: React.TouchEvent<HTMLElement>) => {
      const from = start.current
      start.current = null
      const touch = event.changedTouches[0]
      if (!from || !touch || Date.now() - from.at > MAX_DURATION_MS) return
      const dx = touch.clientX - from.x, dy = touch.clientY - from.y
      if (Math.abs(dx) < MIN_DISTANCE || Math.abs(dx) < Math.abs(dy) * 2) return
      if (dx < 0) onLeft?.(); else onRight?.()
    },
  }
}
