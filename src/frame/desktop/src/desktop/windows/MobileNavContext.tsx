import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react'

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
  setBackHandler: (handler: (() => void) | null) => void
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
  const handlerRef = useRef<(() => void) | null>(null)
  const [canGoBack, setCanGoBack] = useState(false)
  const [titleOverride, setTitleOverrideState] = useState<MobileTitleOverride | null>(null)

  const setBackHandler = useCallback((handler: (() => void) | null) => {
    handlerRef.current = handler
    setCanGoBack(handler !== null)
  }, [])

  const setTitleOverride = useCallback((override: MobileTitleOverride | null) => {
    setTitleOverrideState(override)
  }, [])

  const goBack = useCallback(() => {
    handlerRef.current?.()
  }, [])

  const state: MobileNavState = { canGoBack, goBack, titleOverride }
  const controller: MobileNavController = { setBackHandler, setTitleOverride }

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

  useEffect(() => {
    setBackHandler(handler)
    return () => setBackHandler(null)
  }, [handler, setBackHandler])
}

/**
 * Used by apps in standard mobileStatusBarMode to override the title/subtitle
 * shown in the shell status bar with dynamic content (e.g. current path).
 * Pass null to fall back to the static app label/summary.
 */
export function useMobileTitleOverride(override: MobileTitleOverride | null) {
  const { setTitleOverride } = useContext(MobileNavControllerContext)

  useEffect(() => {
    setTitleOverride(override)
    return () => setTitleOverride(null)
  }, [override?.title, override?.subtitle, setTitleOverride])
}
