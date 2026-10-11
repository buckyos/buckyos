import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent as ReactDragEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import { fetchCurrentAccount } from '../api/account'
import { fetchSystemMetrics, type SystemMetrics } from '../api/system_metrics'
import { fetchUserDetail } from '../api/user_mgr'
import { onOwnProfileChanged } from '../app/agent-setup/events'
import { presentUser, textValue } from '../presentation/model'
import { presentationStore } from '../presentation/store'

const longPressMs = 450
const longPressTolerance = 12

/**
 * Touch long-press for elements that otherwise only have a right-click
 * menu. The handlers can be shared by a whole list: the callback gets the
 * pressed element. The click that follows a fired press must be swallowed
 * with `consumeLongPress()`.
 */
export function useLongPress(onLongPress: (element: HTMLElement, point: { x: number; y: number }) => void) {
  const timer = useRef<number | null>(null)
  const fired = useRef(false)
  const origin = useRef({ x: 0, y: 0 })

  const cancel = useCallback(() => {
    if (timer.current === null) return
    window.clearTimeout(timer.current)
    timer.current = null
  }, [])
  useEffect(() => cancel, [cancel])

  const handlers = useMemo(
    () => ({
      onPointerDown: (event: ReactPointerEvent<HTMLElement>) => {
        // A press that never ended in a click must not swallow the next one.
        fired.current = false
        if (event.pointerType === 'mouse') return
        cancel()
        const element = event.currentTarget
        const point = { x: event.clientX, y: event.clientY }
        origin.current = point
        timer.current = window.setTimeout(() => {
          timer.current = null
          fired.current = true
          onLongPress(element, point)
        }, longPressMs)
      },
      onPointerMove: (event: ReactPointerEvent<HTMLElement>) => {
        if (timer.current === null) return
        const dx = event.clientX - origin.current.x
        const dy = event.clientY - origin.current.y
        if (dx * dx + dy * dy > longPressTolerance * longPressTolerance) cancel()
      },
      onPointerUp: cancel,
      onPointerCancel: cancel,
    }),
    [cancel, onLongPress],
  )

  const consumeLongPress = useCallback(() => {
    const wasFired = fired.current
    fired.current = false
    return wasFired
  }, [])

  return { handlers, consumeLongPress }
}

const dragThreshold = 5

interface ReorderDrag {
  /** The list's items at drag start, in order. */
  ids: string[]
  /** Each item's top at drag start. */
  tops: number[]
  from: number
  to: number
  offset: number
}

/**
 * Mouse and pen drag-to-reorder for a vertical list whose items carry
 * `data-app-id`; touch keeps its long-press menu. Spread `listProps` on the
 * list and `itemStyle(id)` on each item. Items slide aside while one is
 * dragged; `onMove` gets the new index on drop, and the click that ends the
 * drag is swallowed.
 */
export function useDragReorder(onMove: (id: string, index: number) => void) {
  const press = useRef<{ id: string; pointerId: number; x: number; y: number } | null>(null)
  const dragRef = useRef<ReorderDrag | null>(null)
  const [drag, setDrag] = useState<ReorderDrag | null>(null)
  const swallowClick = useRef(false)

  const update = (next: ReorderDrag | null) => {
    dragRef.current = next
    setDrag(next)
  }
  const finish = (drop: boolean) => {
    const current = dragRef.current
    press.current = null
    if (!current) return
    update(null)
    if (!drop) return
    swallowClick.current = true
    if (current.to !== current.from) onMove(current.ids[current.from], current.to)
  }

  const listProps = {
    onPointerDown: (event: ReactPointerEvent<HTMLElement>) => {
      swallowClick.current = false
      if (event.pointerType === 'touch' || event.button !== 0) return
      const id = (event.target as HTMLElement).closest<HTMLElement>('[data-app-id]')?.dataset.appId
      press.current = id ? { id, pointerId: event.pointerId, x: event.clientX, y: event.clientY } : null
    },
    onPointerMove: (event: ReactPointerEvent<HTMLElement>) => {
      const start = press.current
      if (!start || start.pointerId !== event.pointerId) return
      let current = dragRef.current
      if (!current) {
        if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < dragThreshold) return
        const items = [...event.currentTarget.querySelectorAll<HTMLElement>('[data-app-id]')]
        const ids = items.map((item) => item.dataset.appId ?? '')
        const from = ids.indexOf(start.id)
        if (from < 0) {
          press.current = null
          return
        }
        // Captured only once dragging, so a plain click still lands on its button.
        event.currentTarget.setPointerCapture(event.pointerId)
        current = { ids, tops: items.map((item) => item.getBoundingClientRect().top), from, to: from, offset: 0 }
      }
      const { from, tops } = current
      const offset = Math.min(
        Math.max(event.clientY - start.y, tops[0] - tops[from]),
        tops[tops.length - 1] - tops[from],
      )
      const top = tops[from] + offset
      const to = tops.reduce((best, slot, i) => (Math.abs(slot - top) < Math.abs(tops[best] - top) ? i : best), 0)
      update({ ...current, offset, to })
    },
    onPointerUp: () => finish(true),
    onPointerCancel: () => finish(false),
    onLostPointerCapture: () => finish(false),
    onClickCapture: (event: ReactMouseEvent<HTMLElement>) => {
      if (!swallowClick.current) return
      swallowClick.current = false
      event.preventDefault()
      event.stopPropagation()
    },
    // A native image drag would cancel the pointer stream.
    onDragStart: (event: ReactDragEvent<HTMLElement>) => event.preventDefault(),
  }

  /** Where an item sits while one is dragged; `translate` stays clear of the press `transform`. */
  const itemStyle = (id: string): CSSProperties | undefined => {
    if (!drag) return undefined
    const { from, ids, offset, to, tops } = drag
    const i = ids.indexOf(id)
    if (i === from) {
      return { translate: `0 ${offset}px`, scale: '1.06', zIndex: 1, cursor: 'grabbing', transition: 'scale 120ms' }
    }
    const shift =
      from < to && i > from && i <= to ? tops[i - 1] - tops[i] : to < from && i >= to && i < from ? tops[i + 1] - tops[i] : 0
    return { translate: `0 ${shift}px`, transition: 'translate 160ms var(--cp-ease-emphasis)' }
  }

  return { dragging: drag !== null, itemStyle, listProps }
}

type HoverCardMode = 'closed' | 'hover' | 'held'

/**
 * A card that opens while its anchor is hovered and can also be held open by
 * a click. A hovered card stays while the pointer moves from the anchor onto
 * the card; a held one stays until it is toggled or dismissed.
 */
export function useHoverCard({ openDelay = 380, closeDelay = 200 } = {}) {
  const [mode, setMode] = useState<HoverCardMode>('closed')
  const timer = useRef<number | null>(null)

  const cancelPending = useCallback(() => {
    if (timer.current === null) return
    window.clearTimeout(timer.current)
    timer.current = null
  }, [])
  useEffect(() => cancelPending, [cancelPending])

  const hoverProps = useMemo(
    () => ({
      onPointerEnter: (event: ReactPointerEvent) => {
        cancelPending()
        if (event.pointerType === 'touch') return
        timer.current = window.setTimeout(
          () => setMode((current) => (current === 'closed' ? 'hover' : current)),
          openDelay,
        )
      },
      onPointerLeave: () => {
        cancelPending()
        timer.current = window.setTimeout(
          () => setMode((current) => (current === 'hover' ? 'closed' : current)),
          closeDelay,
        )
      },
    }),
    [cancelPending, closeDelay, openDelay],
  )

  const toggleHeld = useCallback(() => {
    cancelPending()
    setMode((current) => (current === 'held' ? 'closed' : 'held'))
  }, [cancelPending])

  const close = useCallback(() => {
    cancelPending()
    setMode('closed')
  }, [cancelPending])

  return {
    open: mode !== 'closed',
    held: mode === 'held',
    hoverProps,
    toggleHeld,
    close,
    cancelPending,
  }
}

export interface CurrentUser {
  userId: string
  displayName: string
  userType: string
  did?: string
  avatarUrl?: string
}

async function loadCurrentUser(): Promise<CurrentUser | null> {
  const account = await fetchCurrentAccount().catch(() => null)
  if (!account) return null
  const { data: detail } = await fetchUserDetail({ userId: account.user_id })
  const user = presentUser({
    userId: account.user_id,
    did: textValue(detail?.local_profile?.did) ?? textValue(detail?.did_document?.id),
    profile: detail?.profile,
    document: detail?.did_document,
  })
  presentationStore.put(user)
  return {
    userId: account.user_id,
    displayName: user.title,
    userType: account.user_type,
    did: user.did,
    avatarUrl: user.icons[0],
  }
}

/** The signed-in user as the shell shows them; null until loaded or when signed out. Reloads after a profile save. */
export function useCurrentUser() {
  const [user, setUser] = useState<CurrentUser | null>(null)
  useEffect(() => {
    let cancelled = false
    const load = () => {
      void loadCurrentUser().then((next) => {
        if (!cancelled) setUser(next)
      })
    }
    load()
    const stop = onOwnProfileChanged(load)
    return () => {
      cancelled = true
      stop()
    }
  }, [])
  return user
}

export interface SystemStatusState {
  metrics: SystemMetrics | null
  /** The last fetch failed; `metrics` may still hold an older reading. */
  failed: boolean
}

const metricsPollMs = 5_000
const metricsRetryMs = 30_000

/** Polls `system.metrics` while `active` and the page is visible. */
export function useSystemStatus(active: boolean): SystemStatusState {
  const [state, setState] = useState<SystemStatusState>({ metrics: null, failed: false })
  useEffect(() => {
    if (!active) return
    let cancelled = false
    let timer: number | undefined
    const tick = async () => {
      let delay = metricsPollMs
      if (document.visibilityState === 'visible') {
        const { data } = await fetchSystemMetrics()
        if (cancelled) return
        setState((previous) =>
          data ? { metrics: data, failed: false } : { metrics: previous.metrics, failed: true },
        )
        if (!data) delay = metricsRetryMs
      }
      timer = window.setTimeout(() => void tick(), delay)
    }
    void tick()
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [active])
  return state
}
