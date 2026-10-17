import { dictionaries } from './dictionaries'

const LOCALE_STORAGE_KEY = 'buckyos.prototype.locale.v1'

/** Text outside React: in the locale the I18nProvider persisted, English as the fallback. */
export function translateNow(key: string, fallback?: string): string {
  let locale = 'en'
  try { locale = window.localStorage.getItem(LOCALE_STORAGE_KEY) ?? 'en' } catch { /* no storage */ }
  const table = (dictionaries as Record<string, Record<string, string>>)[locale] ?? dictionaries.en
  return table[key] ?? dictionaries.en[key] ?? fallback ?? key
}
