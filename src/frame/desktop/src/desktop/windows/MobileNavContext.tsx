import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react'

interface MobileNavState {
  canGoBack: boolean
  goBack: () => void
}

interface MobileNavController {
  setBackHandler: (handler: (() => void) | null) => void
}

const MobileNavStateContext = createContext<MobileNavState>({
  canGoBack: false,
  goBack: () => {},
})

const MobileNavControllerContext = createContext<MobileNavController>({
  setBackHandler: () => {},
})

/**
 * Provider that lives in DesktopRoute, shared between StatusBar and MobileWindowSheet.
 */
export function MobileNavProvider({ children }: { children: React.ReactNode }) {
  const handlerRef = useRef<(() => void) | null>(null)
  const [canGoBack, setCanGoBack] = useState(false)

  const setBackHandler = useCallback((handler: (() => void) | null) => {
    handlerRef.current = handler
    setCanGoBack(handler !== null)
  }, [])

  const goBack = useCallback(() => {
    handlerRef.current?.()
  }, [])

  const state: MobileNavState = { canGoBack, goBack }
  const controller: MobileNavController = { setBackHandler }

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
