/* Zooming a scrolled surface (the PDF and sheet renderers): the handle the component drives and two-finger zoom. */

import { useCallback, useEffect, useLayoutEffect, useRef, type RefObject } from 'react'
import type { useI18n } from '../../i18n/provider'

export type TranslateFn = ReturnType<typeof useI18n>['t']

export interface ZoomHandle {
  zoomBy(factor: number): void
  fit(): void
}

export interface ZoomViewInfo {
  scale: number
  mode: 'fit' | 'custom'
  rotation: number
}

/**
 * Two-finger zoom of `content` inside the scroller: a CSS transform follows the fingers, and on release the new
 * scale is committed (the consumer lays out again) with the point between the fingers kept in place. Returns
 * `zoomTo(scale, x?, y?)` for the other ways to zoom; the anchor defaults to the middle of the viewport.
 * `mounted` tells when the scroller is in the page (a renderer shows it only once the content is loaded).
 */
export function usePinchZoom(
  scrollRef: RefObject<HTMLElement | null>,
  contentRef: RefObject<HTMLElement | null>,
  scale: number,
  min: number,
  max: number,
  onScale: (scale: number) => void,
  mounted: boolean,
): (next: number, x?: number, y?: number) => void {
  const state = useRef({ scale, min, max, onScale })
  useEffect(() => {
    state.current = { scale, min, max, onScale }
  })
  const pendingScroll = useRef<{ left: number; top: number } | null>(null)

  useLayoutEffect(() => {
    const scroller = scrollRef.current
    const pending = pendingScroll.current
    pendingScroll.current = null
    if (scroller && pending) scroller.scrollTo({ left: pending.left, top: pending.top })
  }, [scale, scrollRef])

  const zoomTo = useCallback(
    (next: number, x?: number, y?: number) => {
      const scroller = scrollRef.current
      const { scale: current, min: lo, max: hi, onScale: commit } = state.current
      const target = Math.min(hi, Math.max(lo, next))
      if (!scroller || target === current) return
      const ax = x ?? scroller.clientWidth / 2
      const ay = y ?? scroller.clientHeight / 2
      const ratio = target / current
      pendingScroll.current = { left: (scroller.scrollLeft + ax) * ratio - ax, top: (scroller.scrollTop + ay) * ratio - ay }
      commit(target)
    },
    [scrollRef],
  )

  useEffect(() => {
    const scroller = scrollRef.current
    if (!mounted || !scroller) return
    let gesture: { distance: number; x: number; y: number; ratio: number } | null = null
    const distanceOf = (touches: TouchList) => Math.hypot(touches[0].clientX - touches[1].clientX, touches[0].clientY - touches[1].clientY)
    const finish = () => {
      const content = contentRef.current
      if (!gesture) return
      const { x, y, ratio } = gesture
      gesture = null
      if (content) {
        content.style.transform = ''
        content.style.transformOrigin = ''
      }
      if (Math.abs(ratio - 1) >= 0.02) zoomTo(state.current.scale * ratio, x, y)
    }
    const onStart = (event: TouchEvent) => {
      if (event.touches.length !== 2) return
      const rect = scroller.getBoundingClientRect()
      gesture = {
        distance: Math.max(1, distanceOf(event.touches)),
        x: (event.touches[0].clientX + event.touches[1].clientX) / 2 - rect.left,
        y: (event.touches[0].clientY + event.touches[1].clientY) / 2 - rect.top,
        ratio: 1,
      }
    }
    const onMove = (event: TouchEvent) => {
      const content = contentRef.current
      if (!gesture || event.touches.length !== 2 || !content) return
      event.preventDefault()
      const { scale: current, min: lo, max: hi } = state.current
      gesture.ratio = Math.min(hi / current, Math.max(lo / current, distanceOf(event.touches) / gesture.distance))
      content.style.transformOrigin = `${scroller.scrollLeft + gesture.x}px ${scroller.scrollTop + gesture.y}px`
      content.style.transform = `scale(${gesture.ratio})`
    }
    const onEnd = (event: TouchEvent) => {
      if (event.touches.length < 2) finish()
    }
    scroller.addEventListener('touchstart', onStart, { passive: true })
    scroller.addEventListener('touchmove', onMove, { passive: false })
    scroller.addEventListener('touchend', onEnd)
    scroller.addEventListener('touchcancel', onEnd)
    return () => {
      scroller.removeEventListener('touchstart', onStart)
      scroller.removeEventListener('touchmove', onMove)
      scroller.removeEventListener('touchend', onEnd)
      scroller.removeEventListener('touchcancel', onEnd)
    }
  }, [contentRef, mounted, scrollRef, zoomTo])
  return zoomTo
}
