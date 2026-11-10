/* eslint-disable react-refresh/only-export-components */
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react'

export interface MobileTitleOverride {
  title: string
  subtitle?: string
}

interface MobileNavState {
  canGoBack: boolean
  goBack: () => void
  titleOverride: MobileTitleOverride | null
}

interface MobileNavController {
  setBackHandler: (id: symbol, handler: (() => void) | null, remove?: boolean) => void
  setTitleOverride: (override: MobileTitleOverride | null) => void
}

const MobileNavStateContext = createContext<MobileNavState>({
  canGoBack: false,
  goBack: () => {},
  titleOverride: null,
})

const MobileNavControllerContext = createContext<MobileNavController>({
  setBackHandler: () => {},
  setTitleOverride: () => {},
})

/**
 * Provider that lives in DesktopRoute, shared between StatusBar and MobileWindowSheet.
 */
export function MobileNavProvider({ children }: { children: React.ReactNode }) {
  // Registered handlers in mount order; the latest one (a dialog over an app) handles Back.
  const handlersRef = useRef<Array<{ id: symbol; handler: (() => void) | null }>>([])
  const [canGoBack, setCanGoBack] = useState(false)
  const [titleOverride, setTitleOverrideState] = useState<MobileTitleOverride | null>(null)

  const setBackHandler = useCallback((id: symbol, handler: (() => void) | null, remove = false) => {
    const slots = handlersRef.current
    const index = slots.findIndex((slot) => slot.id === id)
    if (remove) { if (index >= 0) slots.splice(index, 1) }
    else if (index >= 0) slots[index].handler = handler
    else slots.push({ id, handler })
    setCanGoBack(slots.some((slot) => slot.handler !== null))
  }, [])

  const setTitleOverride = useCallback((override: MobileTitleOverride | null) => {
    setTitleOverrideState(override)
  }, [])

  const goBack = useCallback(() => {
    handlersRef.current.findLast((slot) => slot.handler !== null)?.handler?.()
  }, [])

  // Both context values must be referentially stable across re-renders of the
  // provider (which lives in DesktopRoute and re-renders on every store
  // change); otherwise every consumer — the status bar and every app that
  // registers a back handler / title — re-renders along with it.
  const state = useMemo<MobileNavState>(
    () => ({ canGoBack, goBack, titleOverride }),
    [canGoBack, goBack, titleOverride],
  )
  const controller = useMemo<MobileNavController>(
    () => ({ setBackHandler, setTitleOverride }),
    [setBackHandler, setTitleOverride],
  )

  return (
    <MobileNavStateContext.Provider value={state}>
      <MobileNavControllerContext.Provider value={controller}>
        {children}
      </MobileNavControllerContext.Provider>
    </MobileNavStateContext.Provider>
  )
}

/**
 * Used by StatusBar to read navigation state.
 */
export function useMobileNavState() {
  return useContext(MobileNavStateContext)
}

/**
 * Used by apps inside MobileWindowSheet to register/unregister a back handler.
 * Pass a callback when on a secondary page, or null when on the root page.
 */
export function useMobileBackHandler(handler: (() => void) | null) {
  const { setBackHandler } = useContext(MobileNavControllerContext)

  const [id] = useState(() => Symbol('back'))

  useEffect(() => () => setBackHandler(id, null, true), [id, setBackHandler])
  useEffect(() => {
    setBackHandler(id, handler)
  }, [handler, id, setBackHandler])
}

/**
 * Used by apps in standard mobileStatusBarMode to override the title/subtitle
 * shown in the shell status bar with dynamic content (e.g. current path).
 * Pass null to fall back to the static app label/summary.
 */
export function useMobileTitleOverride(override: MobileTitleOverride | null) {
  const { setTitleOverride } = useContext(MobileNavControllerContext)
  // Depend on the primitive fields, not the object: callers usually pass an
  // inline literal, which would otherwise re-run the effect on every render.
  const title = override?.title
  const subtitle = override?.subtitle

  useEffect(() => {
    setTitleOverride(title === undefined ? null : { title, subtitle })
    return () => setTitleOverride(null)
  }, [title, subtitle, setTitleOverride])
}
