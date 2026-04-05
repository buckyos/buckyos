/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type PropsWithChildren,
} from 'react'
import type { DesktopWallpaper } from '../models/ui'

export interface DesktopBackgroundState {
  wallpaper: DesktopWallpaper
  pageCount: number
  viewportProgress: number
}

const defaultDesktopBackgroundState: DesktopBackgroundState = {
  wallpaper: { mode: 'infinite' },
  pageCount: 1,
  viewportProgress: 0,
}

interface DesktopBackgroundContextValue {
  background: DesktopBackgroundState
  setBackground: (background: DesktopBackgroundState) => void
  resetBackground: () => void
}

const DesktopBackgroundContext =
  createContext<DesktopBackgroundContextValue | null>(null)

export function DesktopBackgroundProvider({ children }: PropsWithChildren) {
  const [background, setBackgroundRaw] = useState<DesktopBackgroundState>(
    defaultDesktopBackgroundState,
  )
  const setBackground = useCallback((next: DesktopBackgroundState) => {
    setBackgroundRaw((prev) => {
      if (
        prev.wallpaper === next.wallpaper &&
        prev.pageCount === next.pageCount &&
        prev.viewportProgress === next.viewportProgress
      ) {
        return prev
      }
      return next
    })
  }, [])
  const resetBackground = useCallback(() => {
    setBackgroundRaw(defaultDesktopBackgroundState)
  }, [])

  const value = useMemo<DesktopBackgroundContextValue>(
    () => ({
      background,
      setBackground,
      resetBackground,
    }),
    [background, setBackground, resetBackground],
  )

  return (
    <DesktopBackgroundContext.Provider value={value}>
      {children}
    </DesktopBackgroundContext.Provider>
  )
}

export function useDesktopBackground() {
  const context = useContext(DesktopBackgroundContext)

  if (!context) {
    throw new Error(
      'useDesktopBackground must be used within a DesktopBackgroundProvider',
    )
  }

  return context
}
