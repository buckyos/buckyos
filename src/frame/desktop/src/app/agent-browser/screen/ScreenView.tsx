/* The screen of the current group (original page mode): frames painted on a canvas and acknowledged once shown;
 * touch, mouse and wheel sent as `input` in CSS pixels of the service's viewport; text typed into a hidden input
 * that takes the on-screen keyboard and the input method. The keyboard follows the page's own focus (`focus`): it
 * comes up when what the user pressed takes text and goes away when the focus leaves it. While the Agent operates
 * the page is view only. */

import { useMediaQuery } from '@mui/material'
import { Eye, Keyboard, KeyboardOff, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { useI18n } from '../../../i18n/provider'
import type { ScreenFocus, ScreenFrame, ScreenInput, TouchPoint } from '../api/types'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { canOperate } from '../store/store'
import { buttonOf, diffTyped, heldButton, inputModeFor, isImeKey, keyboardNeedsGesture, keyModifiers, keyText, modifiersOf, wheelPixels } from './input'
import type { Size } from './viewport'

interface Point { x: number; y: number }
interface FrameSize { width: number; height: number }
interface View { zoom: number; x: number; y: number }

const MAX_ZOOM = 4
const TAP_SLOP = 8
/** How long after a press the page's focus still counts as its result: a field the page focuses by itself raises no keyboard. */
const PRESS_FOCUS_MS = 3000
/** What the hidden input holds between keystrokes: something to delete, so Backspace always reports itself. */
const BUFFER = '\n\n\n\n'

const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value))

/** How much of the element the on-screen keyboard covers. */
function useKeyboardOverlap(ref: RefObject<HTMLElement | null>): number {
  const [overlap, setOverlap] = useState(0)
  useEffect(() => {
    const viewport = window.visualViewport
    if (!viewport) return
    const update = () => {
      const element = ref.current
      if (element) setOverlap(Math.max(0, Math.round(element.getBoundingClientRect().bottom - (viewport.offsetTop + viewport.height))))
    }
    viewport.addEventListener('resize', update)
    viewport.addEventListener('scroll', update)
    return () => { viewport.removeEventListener('resize', update); viewport.removeEventListener('scroll', update) }
  }, [ref])
  return overlap
}

export function ScreenView() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const operable = useAgentBrowser(canOperate)
  const state = useAgentBrowser((snapshot) => snapshot.state)
  const status = useAgentBrowser((snapshot) => snapshot.screenStatus)
  const failure = useAgentBrowser((snapshot) => snapshot.screenFailure)
  const attempts = useAgentBrowser((snapshot) => snapshot.screenAttempts)
  const agentName = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const restartFailed = useAgentBrowser((snapshot) => snapshot.restartFailed)
  const coarse = useMediaQuery('(pointer: coarse)')

  const hostRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const [box, setBox] = useState<Size | null>(null)
  const [frame, setFrame] = useState<FrameSize | null>(null)
  const [view, setView] = useState<View>({ zoom: 1, x: 0, y: 0 })
  const [typing, setTyping] = useState(false)
  const [touched, setTouched] = useState(false)
  /** Where the user last pressed, in page pixels: the hidden input sits there and the keyboard never covers it. */
  const [focus, setFocus] = useState<Point | null>(null)
  const overlap = useKeyboardOverlap(hostRef)

  const mobile = state?.viewport.mobile ?? true
  const hasTab = !!state?.active_tab_id
  const fit = box && frame ? box.width / frame.width : 1
  const scale = fit * view.zoom
  const shownWidth = frame ? frame.width * scale : 0
  const shownHeight = frame ? frame.height * scale : 0
  const visibleHeight = box ? Math.max(0, box.height - overlap) : 0
  const maxX = Math.max(0, shownWidth - (box?.width ?? 0))
  const maxY = Math.max(0, shownHeight - visibleHeight)
  const panX = clamp(view.x, 0, maxX)
  const lifted = overlap > 0 && focus ? clamp(focus.y * scale - visibleHeight * 0.5, 0, maxY) : 0
  const panY = Math.max(clamp(view.y, 0, maxY), lifted)

  const touch = coarse || touched
  const live = useRef({ operable, mobile, touch, frame, fit, scale, zoom: view.zoom, panX, panY })
  useEffect(() => { live.current = { operable, mobile, touch, frame, fit, scale, zoom: view.zoom, panX, panY } })

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    const observer = new ResizeObserver(() => {
      const size = { width: host.clientWidth, height: host.clientHeight }
      setBox((previous) => (previous && previous.width === size.width && previous.height === size.height ? previous : size))
      store.setScreenSize(size)
    })
    observer.observe(host)
    return () => { observer.disconnect(); store.setScreenSize(null) }
  }, [store])

  useEffect(() => {
    let alive = true
    let busy = false
    let queued: ScreenFrame | null = null
    const paint = async (next: ScreenFrame) => {
      if (busy) { queued = next; return }
      busy = true
      try {
        const bitmap = await createImageBitmap(new Blob([next.data], { type: 'image/jpeg' }))
        const canvas = canvasRef.current
        if (alive && canvas) {
          if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) { canvas.width = bitmap.width; canvas.height = bitmap.height }
          canvas.getContext('2d')?.drawImage(bitmap, 0, 0)
          canvas.dataset.seq = String(next.seq)
          canvas.dataset.tab = next.tab_id
          setFrame((previous) => (previous && previous.width === next.width && previous.height === next.height ? previous : { width: next.width, height: next.height }))
        }
        bitmap.close()
      } catch { /* a frame that does not decode is skipped */ }
      busy = false
      if (!alive) return
      store.ackFrame(next.seq)
      if (queued) { const waiting = queued; queued = null; void paint(waiting) }
    }
    const last = store.lastFrame()
    if (last) void paint(last)
    const unsubscribe = store.subscribeFrames((next) => void paint(next))
    return () => { alive = false; unsubscribe() }
  }, [store])

  const pagePoint = useCallback((clientX: number, clientY: number): Point | null => {
    const canvas = canvasRef.current
    const size = live.current.frame
    if (!canvas || !size) return null
    const rect = canvas.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0) return null
    return {
      x: clamp(Math.round(((clientX - rect.left) / rect.width) * size.width), 0, size.width),
      y: clamp(Math.round(((clientY - rect.top) / rect.height) * size.height), 0, size.height),
    }
  }, [])

  // ── Text: the hidden input ──

  const composing = useRef(false)
  const held = useRef(new Map<string, string>())

  const resetBuffer = useCallback(() => {
    const input = inputRef.current
    if (!input) return
    input.value = BUFFER
    input.setSelectionRange(BUFFER.length, BUFFER.length)
  }, [])

  /** Focuses the hidden input with the on-screen keyboard of `mode` (the one it had when none is given); `none` takes the focus without a keyboard. */
  const focusInput = useCallback((mode?: string) => {
    const input = inputRef.current
    if (!input) return
    input.inputMode = mode ?? (input.inputMode === 'none' ? 'text' : input.inputMode || 'text')
    resetBuffer()
    if (document.activeElement !== input) input.focus({ preventScroll: true })
    setTyping(input.inputMode !== 'none')
  }, [resetBuffer])

  const pressedAt = useRef(-PRESS_FOCUS_MS)
  /** The user pressed the page. Where only a gesture can raise the keyboard, the focus is taken now, without one, and kept or dropped when the page says what was pressed. */
  const pressed = useCallback(() => {
    pressedAt.current = performance.now()
    const input = inputRef.current
    const active = document.activeElement
    if (!input || active === input) return
    if (active instanceof HTMLElement && active !== document.body) active.blur()
    if (keyboardNeedsGesture() && live.current.operable) focusInput('none')
  }, [focusInput])

  useEffect(() => {
    const follow = (next: ScreenFocus) => {
      const input = inputRef.current
      if (!input) return
      const active = document.activeElement
      if (!next.editable) {
        // A touch device puts its keyboard away; a hardware keyboard keeps typing into the page.
        if (live.current.touch && active === input) input.blur()
        return
      }
      if (!live.current.operable) return
      // What the user is typing into elsewhere in the App keeps the focus.
      if (active instanceof HTMLElement && active !== input && (active.isContentEditable || active.matches('input, textarea, select') || !!active.closest('[role="dialog"]'))) return
      // A keyboard that is up follows the page's focus. One that is not comes up only for what the user just pressed
      // (not for a field the page focuses by itself), and where that takes a gesture, only with the focus taken in it.
      const up = active === input && input.inputMode !== 'none'
      if (live.current.touch && !up && (performance.now() - pressedAt.current > PRESS_FOCUS_MS || (keyboardNeedsGesture() && active !== input))) return
      if (next.rect) setFocus({ x: next.rect.x + next.rect.width / 2, y: next.rect.y + next.rect.height / 2 })
      focusInput(inputModeFor(next.input_type))
    }
    let last = store.getSnapshot().pageFocus
    return store.subscribe(() => {
      const next = store.getSnapshot().pageFocus
      if (next === last) return
      last = next
      if (next) follow(next)
    })
  }, [store, focusInput])

  useEffect(() => { if (!operable) inputRef.current?.blur() }, [operable])

  const tapKey = useCallback((key: string) => {
    if (store.sendInput({ kind: 'key', action: 'down', key, code: key, modifiers: 0, text: '' })) store.sendInput({ kind: 'key', action: 'up', key, code: key, modifiers: 0, text: '' }, true)
  }, [store])

  const onInput = () => {
    const input = inputRef.current
    if (!input || composing.current) return
    const { deleted, inserted } = diffTyped(BUFFER, input.value)
    for (let i = 0; i < deleted; i++) tapKey('Backspace')
    inserted.split('\n').forEach((text, index) => {
      if (index > 0) tapKey('Enter')
      if (text) store.sendInput({ kind: 'text', text })
    })
    resetBuffer()
  }

  const onCompositionEnd = (data: string) => {
    composing.current = false
    store.sendInput(data ? { kind: 'text', text: data } : { kind: 'composition', text: '', selection_start: 0, selection_end: 0 })
    resetBuffer()
  }

  const onKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const native = event.nativeEvent
    if (isImeKey(native)) return
    if ((native.ctrlKey || native.metaKey) && native.key.toLowerCase() === 'v') return
    event.preventDefault()
    if (store.sendInput({ kind: 'key', action: 'down', key: native.key, code: native.code, modifiers: keyModifiers(native), text: keyText(native) })) held.current.set(native.code, native.key)
  }

  const onKeyUp = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const native = event.nativeEvent
    if (!held.current.delete(native.code)) return
    event.preventDefault()
    store.sendInput({ kind: 'key', action: 'up', key: native.key, code: native.code, modifiers: keyModifiers(native), text: '' }, true)
  }

  const onBlur = () => {
    setTyping(false)
    if (composing.current) onCompositionEnd('')
    for (const [code, key] of held.current) store.sendInput({ kind: 'key', action: 'up', key, code, modifiers: 0, text: '' }, true)
    held.current.clear()
  }

  // ── Mouse ──

  const moveFrame = useRef(0)
  const pendingMove = useRef<ScreenInput | null>(null)
  const queueMove = useCallback((input: ScreenInput) => {
    pendingMove.current = input
    if (moveFrame.current) return
    moveFrame.current = window.requestAnimationFrame(() => {
      moveFrame.current = 0
      const next = pendingMove.current
      pendingMove.current = null
      if (next && live.current.operable) store.sendInput(next, true)
    })
  }, [store])
  const flushMove = useCallback(() => {
    window.cancelAnimationFrame(moveFrame.current)
    moveFrame.current = 0
    const next = pendingMove.current
    pendingMove.current = null
    if (next && live.current.operable) store.sendInput(next, true)
  }, [store])

  const releaseDrag = useRef<(() => void) | null>(null)
  useEffect(() => () => { releaseDrag.current?.(); window.cancelAnimationFrame(moveFrame.current) }, [])

  const onMouseDown = (event: React.MouseEvent) => {
    const point = pagePoint(event.clientX, event.clientY)
    event.preventDefault()
    if (!point) return
    pressedAt.current = performance.now()
    focusInput()
    setFocus(point)
    flushMove()
    const button = buttonOf(event.button)
    if (!store.sendInput({ kind: 'mouse', action: 'down', ...point, button, click_count: Math.max(1, event.detail), modifiers: modifiersOf(event) })) return
    const clicks = Math.max(1, event.detail)
    const onMove = (move: MouseEvent) => {
      const at = pagePoint(move.clientX, move.clientY)
      if (at) queueMove({ kind: 'mouse', action: 'move', ...at, button, click_count: 0, modifiers: modifiersOf(move) })
    }
    const onUp = (up: MouseEvent) => {
      if (buttonOf(up.button) !== button) return
      release()
      flushMove()
      const at = pagePoint(up.clientX, up.clientY) ?? point
      store.sendInput({ kind: 'mouse', action: 'up', ...at, button, click_count: clicks, modifiers: modifiersOf(up) }, true)
    }
    const release = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      releaseDrag.current = null
    }
    releaseDrag.current?.()
    releaseDrag.current = release
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  const onMouseMove = (event: React.MouseEvent) => {
    if (releaseDrag.current || !live.current.operable) return
    const point = pagePoint(event.clientX, event.clientY)
    if (point) queueMove({ kind: 'mouse', action: 'move', ...point, button: heldButton(event.buttons), click_count: 0, modifiers: modifiersOf(event) })
  }

  // ── Wheel and touch (native listeners: they must be able to cancel the browser's own scrolling) ──

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    let wheelFrame = 0
    let wheel = { x: 0, y: 0, dx: 0, dy: 0, modifiers: 0 }
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      const point = pagePoint(event.clientX, event.clientY)
      const size = live.current.frame
      if (!point || !size || !live.current.operable) return
      const delta = wheelPixels(event, size.height)
      wheel = { ...point, dx: wheel.dx + delta.x / live.current.scale, dy: wheel.dy + delta.y / live.current.scale, modifiers: modifiersOf(event) }
      if (wheelFrame) return
      wheelFrame = window.requestAnimationFrame(() => {
        wheelFrame = 0
        const { x, y, dx, dy, modifiers } = wheel
        wheel = { ...wheel, dx: 0, dy: 0 }
        store.sendInput({ kind: 'wheel', x, y, delta_x: Math.round(dx), delta_y: Math.round(dy), modifiers }, true)
      })
    }

    const ids = new Map<number, number>()
    let refused = false
    let gesture: { start: Point; last: Point; scrolling: boolean; pinched: boolean } | null = null
    let pinch: { distance: number; zoom: number; anchor: Point } | null = null
    /** Where one finger went down on a phone page, to tell a tap when it is lifted. */
    let down: Point | null = null
    const idOf = (touch: Touch) => {
      let id = ids.get(touch.identifier)
      if (id === undefined) {
        id = 0
        const used = new Set(ids.values())
        while (used.has(id)) id += 1
        ids.set(touch.identifier, id)
      }
      return id
    }
    const points = (touches: TouchList): TouchPoint[] => {
      const list: TouchPoint[] = []
      for (const touch of Array.from(touches)) {
        const at = pagePoint(touch.clientX, touch.clientY)
        if (at) list.push({ id: idOf(touch), ...at })
      }
      return list
    }
    const forget = (touches: TouchList) => { for (const touch of Array.from(touches)) ids.delete(touch.identifier) }
    const between = (a: Touch, b: Touch) => Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY)

    const onTouchStart = (event: TouchEvent) => {
      event.preventDefault()
      setTouched(true)
      const first = event.touches[0]
      const at = first ? pagePoint(first.clientX, first.clientY) : null
      if (!at) { refused = true; gesture = null; down = null; return }
      if (event.touches.length === 1) { resetBuffer(); setFocus(at) }
      down = event.touches.length === 1 && first ? { x: first.clientX, y: first.clientY } : null
      if (live.current.mobile) {
        if (event.touches.length === 1) refused = false
        if (!refused) refused = !store.sendInput({ kind: 'touch', phase: 'start', points: points(event.touches) })
        return
      }
      if (event.touches.length === 1 && first) {
        gesture = { start: { x: first.clientX, y: first.clientY }, last: { x: first.clientX, y: first.clientY }, scrolling: false, pinched: false }
      } else if (event.touches.length === 2) {
        const [a, b] = [event.touches[0], event.touches[1]]
        const rect = host.getBoundingClientRect()
        const { fit: fitNow, zoom, panX: x, panY: y } = live.current
        const middle = { x: (a.clientX + b.clientX) / 2 - rect.left, y: (a.clientY + b.clientY) / 2 - rect.top }
        pinch = { distance: Math.max(1, between(a, b)), zoom, anchor: { x: (middle.x + x) / (fitNow * zoom), y: (middle.y + y) / (fitNow * zoom) } }
        if (gesture) gesture.pinched = true
      }
    }
    const onTouchMove = (event: TouchEvent) => {
      event.preventDefault()
      if (live.current.mobile) {
        if (!refused) queueMove({ kind: 'touch', phase: 'move', points: points(event.touches) })
        return
      }
      if (event.touches.length >= 2 && pinch) {
        const [a, b] = [event.touches[0], event.touches[1]]
        const rect = host.getBoundingClientRect()
        const zoom = clamp((pinch.zoom * between(a, b)) / pinch.distance, 1, MAX_ZOOM)
        const middle = { x: (a.clientX + b.clientX) / 2 - rect.left, y: (a.clientY + b.clientY) / 2 - rect.top }
        const next = live.current.fit * zoom
        setView({ zoom, x: Math.max(0, pinch.anchor.x * next - middle.x), y: Math.max(0, pinch.anchor.y * next - middle.y) })
        return
      }
      const touch = event.touches[0]
      if (!gesture || gesture.pinched || !touch) return
      const dx = touch.clientX - gesture.last.x
      const dy = touch.clientY - gesture.last.y
      if (!gesture.scrolling && Math.hypot(touch.clientX - gesture.start.x, touch.clientY - gesture.start.y) > TAP_SLOP) gesture.scrolling = true
      if (!gesture.scrolling) return
      gesture.last = { x: touch.clientX, y: touch.clientY }
      const at = pagePoint(touch.clientX, touch.clientY)
      if (at && live.current.operable) store.sendInput({ kind: 'wheel', ...at, delta_x: Math.round(-dx / live.current.scale), delta_y: Math.round(-dy / live.current.scale), modifiers: 0 }, true)
    }
    const onTouchEnd = (event: TouchEvent) => {
      event.preventDefault()
      if (live.current.mobile) {
        flushMove()
        if (!refused) store.sendInput({ kind: 'touch', phase: 'end', points: points(event.touches) }, true)
        forget(event.changedTouches)
        const lifted = event.changedTouches[0]
        if (event.touches.length === 0 && down && lifted && !refused && Math.hypot(lifted.clientX - down.x, lifted.clientY - down.y) <= TAP_SLOP) pressed()
        return
      }
      forget(event.changedTouches)
      if (event.touches.length < 2) pinch = null
      if (event.touches.length > 0 || !gesture) return
      const { start, scrolling, pinched } = gesture
      gesture = null
      const at = pagePoint(start.x, start.y)
      if (scrolling || pinched || !at) return
      const press = { ...at, button: 'left' as const, modifiers: 0 }
      if (!store.sendInput({ kind: 'mouse', action: 'move', ...press, button: 'none', click_count: 0 })) return
      store.sendInput({ kind: 'mouse', action: 'down', ...press, click_count: 1 }, true)
      store.sendInput({ kind: 'mouse', action: 'up', ...press, click_count: 1 }, true)
      pressed()
    }
    const onTouchCancel = (event: TouchEvent) => {
      forget(event.changedTouches)
      gesture = null
      pinch = null
      down = null
      if (live.current.mobile && !refused) store.sendInput({ kind: 'touch', phase: 'cancel', points: [] }, true)
    }

    host.addEventListener('wheel', onWheel, { passive: false })
    host.addEventListener('touchstart', onTouchStart, { passive: false })
    host.addEventListener('touchmove', onTouchMove, { passive: false })
    host.addEventListener('touchend', onTouchEnd, { passive: false })
    host.addEventListener('touchcancel', onTouchCancel)
    return () => {
      window.cancelAnimationFrame(wheelFrame)
      host.removeEventListener('wheel', onWheel)
      host.removeEventListener('touchstart', onTouchStart)
      host.removeEventListener('touchmove', onTouchMove)
      host.removeEventListener('touchend', onTouchEnd)
      host.removeEventListener('touchcancel', onTouchCancel)
    }
  }, [store, pagePoint, queueMove, flushMove, resetBuffer, pressed])

  const inputLeft = focus && box ? clamp(focus.x * scale - panX, 0, Math.max(0, box.width - 8)) : 0
  const inputTop = focus ? clamp(focus.y * scale - panY, 0, Math.max(0, visibleHeight - 24)) : 0
  /** The browser stopped under the open connection: it is being started again (store.restartBrowser). */
  const stopped = status === 'open' && state?.running === false
  const waiting = status === 'connecting' || (status === 'open' && hasTab && !frame) || (stopped && !restartFailed)

  return (
    <div className="ab-screen" data-testid="ab-screen" data-readonly={operable ? undefined : 'true'} data-status={status}>
      <div
        ref={hostRef}
        className="ab-screen-surface"
        data-testid="ab-screen-surface"
        data-viewport={frame ? `${frame.width}x${frame.height}` : undefined}
        data-mobile={mobile ? 'true' : 'false'}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onContextMenu={(event) => event.preventDefault()}
      >
        <canvas
          ref={canvasRef}
          className="ab-screen-canvas"
          data-testid="ab-screen-canvas"
          style={{ width: shownWidth, height: shownHeight, transform: `translate(${-panX}px, ${-panY}px)`, visibility: frame && hasTab ? 'visible' : 'hidden' }}
        />
        <textarea
          ref={inputRef}
          className="ab-screen-input"
          data-testid="ab-screen-input"
          aria-label={t('agentBrowser.screen.typeHere', 'Type into the page')}
          style={{ left: inputLeft, top: inputTop }}
          defaultValue={BUFFER}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
          tabIndex={-1}
          onFocus={(event) => setTyping(event.currentTarget.inputMode !== 'none')}
          onBlur={onBlur}
          onInput={onInput}
          onKeyDown={onKeyDown}
          onKeyUp={onKeyUp}
          onPaste={(event) => {
            event.preventDefault()
            const text = event.clipboardData.getData('text/plain')
            if (text) store.sendInput({ kind: 'text', text })
          }}
          onCompositionStart={() => { composing.current = true }}
          onCompositionUpdate={(event) => { store.sendInput({ kind: 'composition', text: event.data, selection_start: event.data.length, selection_end: event.data.length }) }}
          onCompositionEnd={(event) => onCompositionEnd(event.data)}
        />
      </div>

      {state && !hasTab && status === 'open' && !stopped ? (
        <div className="ab-screen-cover" data-testid="ab-screen-empty">
          <p className="ab-screen-cover-title">{t('agentBrowser.screen.emptyTitle', 'This is {{name}}’s browser', { name: agentName })}</p>
          <p>{t('agentBrowser.screen.emptyHint', 'Enter an address above to start browsing.')}</p>
        </div>
      ) : null}
      {waiting ? (
        <div className="ab-screen-cover" data-testid="ab-screen-connecting">
          <LoadingOrb size={36} />
          <p>{t('agentBrowser.screen.connecting', 'Connecting to the browser…')}</p>
        </div>
      ) : null}
      {status === 'reconnecting' ? (
        <div className={frame ? 'ab-screen-banner' : 'ab-screen-cover'} role="status" data-testid="ab-screen-reconnecting">
          <RefreshCw size={14} className="ab-spin" aria-hidden />
          <span>{attempts > 2 ? t('agentBrowser.screen.reconnectingLong', 'Still trying to reach the browser…') : t('agentBrowser.screen.reconnecting', 'Connection lost. Reconnecting…')}</span>
        </div>
      ) : null}
      {(status === 'failed' && failure?.code !== 'browser_unavailable') || (stopped && restartFailed) ? (
        <div className="ab-screen-cover" role="alert" data-testid="ab-screen-failed" data-code={failure?.code}>
          <p className="ab-screen-cover-title">{failure?.code === 'unauthorized' ? t('agentBrowser.screen.unauthorized', 'Only the owner of {{name}} can use its browser.', { name: agentName }) : t('agentBrowser.screen.failed', 'The browser could not be reached.')}</p>
          <button type="button" className="ab-button" onClick={() => store.retryScreen()}>{t('common.retry')}</button>
        </div>
      ) : null}
      {!operable ? (
        <div className="ab-screen-readonly" role="status" data-testid="ab-screen-readonly">
          <Eye size={14} aria-hidden />
          <span>{t('agentBrowser.screen.readonly', '{{name}} is operating · view only', { name: agentName })}</span>
        </div>
      ) : null}
      {operable && hasTab && (coarse || touched) ? (
        <button
          type="button"
          className="ab-screen-keyboard"
          style={{ bottom: overlap + 12 }}
          aria-label={typing ? t('agentBrowser.screen.hideKeyboard', 'Hide keyboard') : t('agentBrowser.screen.showKeyboard', 'Show keyboard')}
          aria-pressed={typing}
          data-testid="ab-keyboard"
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            const input = inputRef.current
            if (!input) return
            // Focused without its keyboard (the press was not on a field): the focus is taken again inside this press.
            if (typing || document.activeElement === input) input.blur()
            if (!typing) focusInput(inputModeFor(store.getSnapshot().pageFocus?.input_type))
          }}
        >
          {typing ? <KeyboardOff size={20} aria-hidden /> : <Keyboard size={20} aria-hidden />}
        </button>
      ) : null}
    </div>
  )
}
