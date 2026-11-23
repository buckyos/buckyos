import { browserLocale, matchSupportedLocale, type SupportedLocale } from '../../../shared/locale'
import en from './locales/en.json'

const packs = import.meta.glob<Record<string, string>>('./locales/*.json', { eager: true, import: 'default' })
const sessionKey = 'buckyos.systest.locale'
function initialLocale(): SupportedLocale {
  const query = matchSupportedLocale(new URLSearchParams(location.search).get('locale'))
  if (query) return query
  try { return matchSupportedLocale(sessionStorage.getItem(sessionKey)) ?? browserLocale() }
  catch { return browserLocale() }
}

export let locale = initialLocale()
type Values = Record<string, string | number>
export function hasTranslation(key: string) { return Object.hasOwn(en, key) }

export function t(key: string, values: Values = {}): string {
  const text = packs[`./locales/${locale}.json`]?.[key] ?? (en as Record<string, string>)[key] ?? key
  return text.replace(/\{\{(\w+)\}\}/g, (original, name: string) => String(values[name] ?? original))
}

export function bindText(element: HTMLElement, key: string, values?: Values) {
  element.dataset.i18n = key
  if (values) element.dataset.i18nValues = JSON.stringify(values)
  else delete element.dataset.i18nValues
  element.textContent = t(key, values)
}

export function localize(root: ParentNode = document) {
  for (const el of root.querySelectorAll<HTMLElement>('[data-i18n]')) {
    el.textContent = t(el.dataset.i18n!, el.dataset.i18nValues ? JSON.parse(el.dataset.i18nValues) as Values : {})
  }
  for (const el of root.querySelectorAll<HTMLElement>('[data-i18n-label]')) el.setAttribute('aria-label', t(el.dataset.i18nLabel!))
  for (const el of root.querySelectorAll<HTMLInputElement>('[data-i18n-placeholder]')) el.placeholder = t(el.dataset.i18nPlaceholder!)
  for (const el of root.querySelectorAll<HTMLInputElement>('[data-i18n-default]')) el.defaultValue = t(el.dataset.i18nDefault!)
}

export function setLocale(tag: string) {
  locale = matchSupportedLocale(tag) ?? 'en'
  document.documentElement.lang = locale
  document.documentElement.dir = locale === 'ar' ? 'rtl' : 'ltr'
  try { sessionStorage.setItem(sessionKey, locale) } catch { /* Storage may be unavailable. */ }
  localize()
}

setLocale(locale)
