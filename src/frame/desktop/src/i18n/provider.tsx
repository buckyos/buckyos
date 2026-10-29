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
import { supportedLocales, type SupportedLocale } from '../models/ui'

interface I18nContextValue {
  locale: SupportedLocale
  setLocale: (locale: SupportedLocale) => void
  t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string
}

const storageKey = 'buckyos.prototype.locale.v1'

/** A page on an origin of its own (a workspace's `aiws-<id>.<zone>`) starts with an empty storage: before the provider
 * mounts, the browser's language decides where there is a dictionary for it; the Desktop's own choice is per origin. */
export function defaultLocaleFromBrowser() {
  try {
    if (window.localStorage.getItem(storageKey)) return
    const wanted = (navigator.languages ?? [navigator.language]).map((l) => l.toLowerCase())
    const pick = supportedLocales.find((locale) => wanted.some((l) => l === locale.toLowerCase() || l.split('-')[0] === locale.toLowerCase().split('-')[0]))
    if (pick) window.localStorage.setItem(storageKey, pick)
  } catch { /* no storage */ }
}

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

/** `locale` fixes the language for an embedded page (the embedding App decides it); it is not stored. */
export function I18nProvider({ children, locale: fixed }: PropsWithChildren<{ locale?: SupportedLocale }>) {
  const [storedLocale, setLocale] = useState<SupportedLocale>(() => {
    // Validate the persisted value: a stale/foreign locale would make
    // `dictionaries[locale]` undefined and crash every `t()` call at render.
    const saved = window.localStorage.getItem(storageKey)
    return supportedLocales.includes(saved as SupportedLocale)
      ? (saved as SupportedLocale)
      : 'en'
  })

  const locale = fixed ?? storedLocale

  useEffect(() => {
    if (!fixed) window.localStorage.setItem(storageKey, locale)
    document.documentElement.lang = locale
    document.documentElement.dir = locale === 'ar' ? 'rtl' : 'ltr'
  }, [locale, fixed])

  const value = useMemo<I18nContextValue>(() => {
    return {
      locale,
      // Stored before the re-render, not only in the effect: text produced outside React during that render
      // (app translators that read the stored language) is already in the new language.
      setLocale: (next) => {
        try { window.localStorage.setItem(storageKey, next) } catch { /* no storage */ }
        setLocale(next)
      },
      t: (key, fallback = key, variables) => {
        const table = dictionaries[locale] ?? dictionaries.en
        const current = table[key] ?? dictionaries.en[key] ?? fallback
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
