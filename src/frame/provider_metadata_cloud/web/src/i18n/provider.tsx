import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type PropsWithChildren,
} from 'react'
import { dictionaries } from './dictionaries'
import { runtimeEnv } from '../runtime/env'

export type SupportedLocale = 'en-US' | 'zh-CN'

interface I18nContextValue {
  locale: SupportedLocale
  setLocale: (locale: SupportedLocale) => void
  t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string
}

const storageKey = `${runtimeEnv.storagePrefix}.locale.v1`
const I18nContext = createContext<I18nContextValue | null>(null)

function interpolate(message: string, variables?: Record<string, string | number>) {
  if (!variables) {
    return message
  }
  return Object.entries(variables).reduce((acc, [key, value]) => {
    return acc.replaceAll(`{{${key}}}`, String(value))
  }, message)
}

export function I18nProvider({ children }: PropsWithChildren) {
  const [locale, setLocale] = useState<SupportedLocale>(() => {
    const saved = window.localStorage.getItem(storageKey) as SupportedLocale | null
    return saved ?? 'en-US'
  })

  useEffect(() => {
    window.localStorage.setItem(storageKey, locale)
    document.documentElement.lang = locale
  }, [locale])

  const value = useMemo<I18nContextValue>(() => {
    return {
      locale,
      setLocale,
      t: (key, fallback = key, variables) => {
        const current = dictionaries[locale][key] ?? dictionaries['en-US'][key] ?? fallback
        return interpolate(current, variables)
      },
    }
  }, [locale])

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n() {
  const context = useContext(I18nContext)
  if (!context) {
    throw new Error('useI18n must be used within I18nProvider')
  }
  return context
}
