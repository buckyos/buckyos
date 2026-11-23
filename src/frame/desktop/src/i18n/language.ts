import { matchSupportedLocale, type SupportedLocale } from '../../../../apps/shared/locale'

const storageKey = 'buckyos.prototype.locale.v1'
let pageLocale: SupportedLocale | undefined

export function savedLocale(): SupportedLocale | null {
  try { return matchSupportedLocale(globalThis.localStorage?.getItem(storageKey)) }
  catch { return null }
}

export function saveLocale(locale: SupportedLocale) {
  try { globalThis.localStorage?.setItem(storageKey, locale) } catch { /* storage is optional in frames */ }
}

/** Same-origin standalone app tabs follow changes made in Settings. */
export function subscribeLocalePreference(listener: (locale: SupportedLocale) => void) {
  const onStorage = (event: StorageEvent) => {
    if (event.key === storageKey) listener(matchSupportedLocale(event.newValue) ?? 'en')
  }
  window.addEventListener('storage', onStorage)
  return () => window.removeEventListener('storage', onStorage)
}

/** The dictionary currently displayed by this document, including in a fixed-locale iframe. */
export function currentTextLocale(): SupportedLocale {
  return pageLocale ?? savedLocale() ?? matchSupportedLocale(globalThis.document?.documentElement.lang) ?? 'en'
}

export function showTextLocale(locale: SupportedLocale) {
  pageLocale = locale
  document.documentElement.lang = locale
  document.documentElement.dir = locale === 'ar' ? 'rtl' : 'ltr'
}
