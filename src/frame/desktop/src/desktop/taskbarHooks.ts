import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import { fetchCurrentAccount } from '../api/account'
import { fetchSystemMetrics, type SystemMetrics } from '../api/system_metrics'
import { fetchUserDetail } from '../api/user_mgr'
import { usableImageUrl } from '../app/agent-setup/model'

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

function firstText(...values: unknown[]) {
  for (const value of values) {
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return undefined
}

async function loadCurrentUser(): Promise<CurrentUser | null> {
  const account = await fetchCurrentAccount().catch(() => null)
  if (!account) return null
  const { data: detail } = await fetchUserDetail({ userId: account.user_id })
  const profile = (detail?.profile ?? {}) as Record<string, unknown>
  const localProfile = (detail?.local_profile ?? {}) as Record<string, unknown>
  return {
    userId: account.user_id,
    displayName:
      firstText(profile.display_name, localProfile.display_name, detail?.show_name, account.user_name) ??
      account.user_id,
    userType: account.user_type,
    did: firstText(profile.did, localProfile.did, detail?.did_document?.id),
    avatarUrl: usableImageUrl(firstText(profile.avatar, localProfile.avatar)),
  }
}

/** The signed-in user as the shell shows them; null until loaded or when signed out. */
export function useCurrentUser() {
  const [user, setUser] = useState<CurrentUser | null>(null)
  useEffect(() => {
    let cancelled = false
    void loadCurrentUser().then((next) => {
      if (!cancelled) setUser(next)
    })
    return () => {
      cancelled = true
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
