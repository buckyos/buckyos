import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type PropsWithChildren,
} from 'react'
import { runtimeEnv } from '../runtime/env'
import type { ThemeMode } from './tokens'

interface ThemeContextValue {
  themeMode: ThemeMode
  setThemeMode: (mode: ThemeMode) => void
}

const storageKey = `${runtimeEnv.storagePrefix}.theme.v1`
const ThemeContext = createContext<ThemeContextValue | null>(null)

export function ThemeProvider({ children }: PropsWithChildren) {
  const [themeMode, setThemeMode] = useState<ThemeMode>(() => {
    const saved = window.localStorage.getItem(storageKey) as ThemeMode | null
    return saved ?? 'light'
  })

  useEffect(() => {
    window.localStorage.setItem(storageKey, themeMode)
    document.documentElement.dataset.theme = themeMode
  }, [themeMode])

  const value = useMemo(() => ({ themeMode, setThemeMode }), [themeMode])

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
}

export function useThemeMode() {
  const context = useContext(ThemeContext)
  if (!context) {
    throw new Error('useThemeMode must be used within ThemeProvider')
  }
  return context
}
