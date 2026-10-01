import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'

export interface PaneResizerOptions {
  /** The committed width when a drag starts. */
  getWidth: () => number
  clamp: (width: number) => number
  /**
   * Shows a width while dragging. Called at most once per animation frame and
   * expected to touch the DOM directly: a React render per pointer move is
   * what made the splitter stutter.
   */
  preview: (width: number) => void
  /** Stores the final width once the drag ends (one React render). */
  commit: (width: number) => void
  disabled?: boolean
  /** Width change of one arrow key press. */
  keyboardStep?: number
}

interface DragState {
  pointerId: number
  startX: number
  startWidth: number
  width: number
  latestX: number
  frame: number | null
  target: HTMLElement
  cleanup: () => void
}

/**
 * Pointer-driven resizing of a pane next to a vertical splitter.
 *
 * The drag listens on `window`, so it survives the splitter element losing
 * focus or being re-rendered, and it always ends: on pointer up or cancel,
 * on losing pointer capture, when the window loses focus or the tab is
 * hidden, and on Escape (which restores the start width). Ending the drag
 * restores the document cursor and text selection.
 */
export function usePaneResizer({ getWidth, clamp, preview, commit, disabled = false, keyboardStep = 16 }: PaneResizerOptions) {
  const [active, setActive] = useState(false)
  const drag = useRef<DragState | null>(null)
  const options = useRef({ getWidth, clamp, preview, commit })
  useLayoutEffect(() => {
    options.current = { getWidth, clamp, preview, commit }
  })

  const finish = useCallback((width: number | null) => {
    const state = drag.current
    if (!state) return
    drag.current = null
    if (state.frame !== null) cancelAnimationFrame(state.frame)
    state.cleanup()
    setActive(false)
    const finalWidth = width ?? state.width
    options.current.preview(finalWidth)
    options.current.commit(finalWidth)
  }, [])

  useEffect(() => () => {
    const state = drag.current
    if (!state) return
    drag.current = null
    if (state.frame !== null) cancelAnimationFrame(state.frame)
    state.cleanup()
  }, [])

  const onPointerDown = useCallback((event: React.PointerEvent<HTMLElement>) => {
    if (disabled || drag.current || event.button !== 0) return
    // No focus change and no text selection while dragging.
    event.preventDefault()
    const target = event.currentTarget
    const startWidth = options.current.getWidth()
    const body = document.body
    const previous = { cursor: body.style.cursor, userSelect: body.style.userSelect }
    body.style.cursor = 'col-resize'
    body.style.userSelect = 'none'

    const onMove = (move: PointerEvent) => {
      const state = drag.current
      if (!state || move.pointerId !== state.pointerId) return
      state.latestX = move.clientX
      if (state.frame !== null) return
      state.frame = requestAnimationFrame(() => {
        const current = drag.current
        if (!current) return
        current.frame = null
        current.width = options.current.clamp(current.startWidth + (current.latestX - current.startX))
        options.current.preview(current.width)
      })
    }
    const onUp = (up: PointerEvent) => { if (drag.current && up.pointerId === drag.current.pointerId) finish(null) }
    const onLostCapture = (lost: PointerEvent) => { if (drag.current && lost.pointerId === drag.current.pointerId) finish(null) }
    const onBlur = () => finish(null)
    const onVisibility = () => { if (document.visibilityState === 'hidden') finish(null) }
    const onKeyDown = (key: KeyboardEvent) => {
      if (key.key !== 'Escape' || !drag.current) return
      key.preventDefault()
      finish(drag.current.startWidth)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
    window.addEventListener('blur', onBlur)
    window.addEventListener('keydown', onKeyDown, true)
    document.addEventListener('visibilitychange', onVisibility)
    target.addEventListener('lostpointercapture', onLostCapture)
    try { target.setPointerCapture(event.pointerId) } catch { /* capture is a convenience: window listeners carry the drag */ }

    drag.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth,
      width: startWidth,
      latestX: event.clientX,
      frame: null,
      target,
      cleanup: () => {
        window.removeEventListener('pointermove', onMove)
        window.removeEventListener('pointerup', onUp)
        window.removeEventListener('pointercancel', onUp)
        window.removeEventListener('blur', onBlur)
        window.removeEventListener('keydown', onKeyDown, true)
        document.removeEventListener('visibilitychange', onVisibility)
        target.removeEventListener('lostpointercapture', onLostCapture)
        if (target.hasPointerCapture?.(event.pointerId)) { try { target.releasePointerCapture(event.pointerId) } catch { /* already released */ } }
        body.style.cursor = previous.cursor
        body.style.userSelect = previous.userSelect
      },
    }
    setActive(true)
  }, [disabled, finish])

  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLElement>) => {
    if (disabled) return
    const delta = event.key === 'ArrowLeft' ? -keyboardStep : event.key === 'ArrowRight' ? keyboardStep : 0
    if (!delta) return
    event.preventDefault()
    const width = options.current.clamp(options.current.getWidth() + delta)
    options.current.preview(width)
    options.current.commit(width)
  }, [disabled, keyboardStep])

  return { active, onPointerDown, onKeyDown }
}
