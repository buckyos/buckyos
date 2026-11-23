/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type PropsWithChildren,
} from 'react'
import { dictionaries, localeDictionary } from './dictionaries'
import { browserLocale, matchSupportedLocale, type SupportedLocale } from '../../../../apps/shared/locale'
import { savedLocale, saveLocale, showTextLocale, subscribeLocalePreference } from './language'

interface I18nContextValue {
  /** Language of the text currently on screen. */
  locale: SupportedLocale
  /** Chosen language, including while its dictionary is loading. */
  preferredLocale: SupportedLocale
  setLocale: (locale: SupportedLocale) => void
  t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string
}

/** A standalone workspace receives the Desktop language in its URL. A direct visit uses its
 * preference on this origin, then the browser's languages. */
export function defaultLocaleFromBrowser(preferred?: string | null): SupportedLocale {
  const locale = matchSupportedLocale(preferred) ?? savedLocale() ?? browserLocale()
  saveLocale(locale)
  return locale
}

const I18nContext = createContext<I18nContextValue | null>(null)

function interpolate(
  message: string,
  variables?: Record<string, string | number>,
) {
  if (!variables) {
    return message
  }

  return message.replace(/\{\{(\w+)\}\}/g, (placeholder, name: string) =>
    Object.hasOwn(variables, name) ? String(variables[name]) : placeholder,
  )
}

/** `locale` fixes the language for an embedded page (the embedding App decides it); it is not stored. */
export function I18nProvider({ children, locale: fixed, initialLocale }: PropsWithChildren<{ locale?: SupportedLocale; initialLocale?: SupportedLocale }>) {
  const [preference, setPreference] = useState<SupportedLocale>(() => initialLocale ?? savedLocale() ?? 'en')
  const preferredLocale = fixed ?? preference
  const [shown, setShown] = useState<SupportedLocale | null>(null)
  const lastShown = useRef<SupportedLocale | null>(null)

  useEffect(() => {
    if (!fixed) return subscribeLocalePreference(setPreference)
  }, [fixed])

  useLayoutEffect(() => {
    let current = true
    const show = (locale: SupportedLocale) => {
      if (!current) return
      showTextLocale(locale)
      lastShown.current = locale
      setShown(locale)
    }
    const load = () => {
      const resource = localeDictionary(preferredLocale)
      if (resource instanceof Promise) {
        void resource.then(() => show(preferredLocale), (error: unknown) => {
          if (!current) return
          console.warn(`Could not load the ${preferredLocale} language pack`, error)
          if (!lastShown.current) show('en')
        })
      } else show(preferredLocale)
    }
    load()
    window.addEventListener('online', load)
    return () => { current = false; window.removeEventListener('online', load) }
  }, [preferredLocale])

  const setLocale = useCallback((next: SupportedLocale) => {
    if (fixed) return
    saveLocale(next)
    setPreference(next)
  }, [fixed])

  const value = useMemo<I18nContextValue | null>(() => {
    if (!shown) return null
    const table = dictionaries[shown]
    return {
      locale: shown, preferredLocale, setLocale,
      t: (key, fallback = key, variables) => interpolate(table[key] ?? dictionaries.en[key] ?? fallback, variables),
    }
  }, [shown, preferredLocale, setLocale])

  return value && <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n() {
  const context = useContext(I18nContext)

  if (!context) {
    throw new Error('useI18n must be used within I18nProvider')
  }

  return context
}
