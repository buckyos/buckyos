/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type PropsWithChildren,
} from 'react'
import { dictionaries } from './dictionaries'
import type { SupportedLocale } from '../models/ui'

interface I18nContextValue {
  locale: SupportedLocale
  setLocale: (locale: SupportedLocale) => void
  t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string
}

const storageKey = 'buckyos.prototype.locale.v1'

const I18nContext = createContext<I18nContextValue | null>(null)

function interpolate(
  message: string,
  variables?: Record<string, string | number>,
) {
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
    return saved ?? 'en'
  })

  useEffect(() => {
    window.localStorage.setItem(storageKey, locale)
    document.documentElement.lang = locale
    document.documentElement.dir = locale === 'ar' ? 'rtl' : 'ltr'
  }, [locale])

  const value = useMemo<I18nContextValue>(() => {
    return {
      locale,
      setLocale,
      t: (key, fallback = key, variables) => {
        const current = dictionaries[locale][key] ?? dictionaries.en[key] ?? fallback
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
