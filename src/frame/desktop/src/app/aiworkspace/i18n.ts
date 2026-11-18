/* AI Workspace text. Every string the app shows is a key `aiworkspace.<area>.<name>` with an English and a
 * Chinese entry in src/i18n/aiworkspace/<area>.ts, and an entry in each other language's table
 * src/i18n/aiworkspace/locales/<language>.ts; tests/datamodel/aiworkspace-i18n.test.ts keeps the tables
 * complete and the source free of hard-coded Chinese. A Desktop language without a table (Arabic) reads English.
 *
 * English and Simplified Chinese are in the bundle; every other table is a chunk of its own, loaded when its
 * language is first asked for (`loadAiwsLocale`; the app's entries wait for it before they render). Until it
 * has arrived, text stays in the language shown before — never a mix of two — and the table's arrival tells
 * `subscribeAiwsText` listeners (useAiwsText re-renders with it).
 *
 * React code takes the language from the Desktop's I18nProvider (`useAiwsText` in useAiwsText.ts) and
 * re-renders when it changes. Code outside React (store, session, offline replica, extension runtime) asks
 * `aiwsT` / `aiwsText()` here, which read the language the provider persisted; a context without page
 * storage (the replica worker, Node test fixtures) is told its language with `setAiwsLocale`. This module
 * imports neither React nor the Desktop dictionaries, so the worker can use it.
 *
 * Templates use `{{name}}` placeholders. A `count` picks the plural form of the language's CLDR rules: the
 * entry `<key>_<category>` (`_one`; Russian also `_few`) when the table has one, else `<key>`. Lists and
 * clauses are joined with the language's separators (`list`, `clauses`); numbers and times are formatted for
 * the language of the text (`intl`), never for a fixed one. */

import { aiWorkspaceEn, aiWorkspaceZh } from '../../i18n/aiworkspace'

export type TextVars = Record<string, string | number>

export interface AiwsText {
  /** The Desktop language this text is for (a language without a table reads English). */
  locale: string
  /** The language the text is in, as Intl takes it: `locale` when it has a table, else 'en'. */
  intl: string
  t(key: string, vars?: TextVars): string
  /** The template itself, placeholders kept (for useAiwsText's `tNode`). */
  template(key: string, count?: number): string
  num(value: number, options?: Intl.NumberFormatOptions): string
  dateTime(value: number | string | Date): string
  date(value: number | string | Date): string
  time(value: number | string | Date): string
  /** Names in a sentence: 「a、b、c」 / "a, b, c". */
  list(items: readonly string[]): string
  /** Independent statements in one line: 「a；b」 / "a; b". */
  clauses(items: readonly string[]): string
  /** Sorting labels for display (numeric-aware, in the language's collation). */
  compare(a: string, b: string): number
}

type Table = Record<string, string>

const LOCALE_STORAGE_KEY = 'buckyos.prototype.locale.v1'

/** The languages with a table of their own, loaded on first use (keys: the Desktop's locale codes). */
const LOADERS: Record<string, () => Promise<{ default: Table }>> = {
  'zh-TW': () => import('../../i18n/aiworkspace/locales/zh-TW'),
  ja: () => import('../../i18n/aiworkspace/locales/ja'),
  ko: () => import('../../i18n/aiworkspace/locales/ko'),
  fr: () => import('../../i18n/aiworkspace/locales/fr'),
  de: () => import('../../i18n/aiworkspace/locales/de'),
  es: () => import('../../i18n/aiworkspace/locales/es'),
  ru: () => import('../../i18n/aiworkspace/locales/ru'),
}

/** Every language AI Workspace has text for. */
export const AIWS_LOCALES: readonly string[] = ['en', 'zh-CN', ...Object.keys(LOADERS)]

const tables = new Map<string, Table>([['en', aiWorkspaceEn], ['zh-CN', aiWorkspaceZh]])
const loading = new Map<string, Promise<void>>()
const listeners = new Set<() => void>()
let version = 0
/** The current language the last time its text was ready: what text stays in while the next table loads. */
let shown = 'en'
let pinned: string | null = null

/** For a context without the page's storage (the replica worker, Node fixtures): the language to use; null unpins. */
export function setAiwsLocale(locale: string | null): void {
  pinned = locale
}

/** The language code-outside-React text is in right now: pinned, else the one the I18nProvider persisted. */
export function currentAiwsLocale(): string {
  if (pinned) return pinned
  try {
    const saved = globalThis.localStorage?.getItem(LOCALE_STORAGE_KEY)
    if (saved) return saved
  } catch { /* no storage */ }
  return globalThis.document?.documentElement.lang || 'en'
}

/** Whether a language's text can be shown now (its table is here, or it has none and reads English). */
export function aiwsLocaleReady(locale: string = currentAiwsLocale()): boolean {
  return tables.has(locale) || !(locale in LOADERS)
}

/** Load a language's table (default: the current language). Resolves at once when it is here or there is none;
 * a failed load (offline, chunk gone) is logged and leaves the language reading the one shown before. */
export function loadAiwsLocale(locale: string = currentAiwsLocale()): Promise<void> {
  if (aiwsLocaleReady(locale)) return Promise.resolve()
  let pending = loading.get(locale)
  if (!pending) {
    pending = LOADERS[locale]()
      .then((module) => {
        tables.set(locale, module.default)
        version += 1
        for (const listener of [...listeners]) listener()
      })
      .catch((error) => console.warn(`[aiworkspace] text for ${locale} could not be loaded:`, error))
      .finally(() => loading.delete(locale))
    loading.set(locale, pending)
  }
  return pending
}

/** Told when a language's table arrives (for useSyncExternalStore with `aiwsTextVersion`). */
export function subscribeAiwsText(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Changes whenever a table arrives. */
export function aiwsTextVersion(): number {
  return version
}

function pick(table: Table, rules: Intl.PluralRules, key: string, count: unknown): string | undefined {
  if (typeof count === 'number' && Number.isFinite(count)) {
    const form = table[`${key}_${rules.select(count)}`]
    if (form !== undefined) return form
  }
  return table[key]
}

function interpolate(template: string, vars?: TextVars): string {
  if (!vars) return template
  return template.replace(/\{\{(\w+)\}\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole))
}

function toDate(value: number | string | Date): Date {
  return value instanceof Date ? value : new Date(value)
}

const englishRules = new Intl.PluralRules('en')
const cache = new Map<string, AiwsText>()

function build(locale: string): AiwsText {
  const own = tables.get(locale)
  const table = own ?? aiWorkspaceEn
  const intl = own ? locale : 'en'
  const rules = new Intl.PluralRules(intl)
  const raw = (key: string, count?: unknown) => pick(table, rules, key, count) ?? pick(aiWorkspaceEn, englishRules, key, count) ?? key
  const collator = new Intl.Collator(intl, { numeric: true })
  return {
    locale,
    intl,
    t: (key, vars) => interpolate(raw(key, vars?.count), vars),
    template: (key, count) => raw(key, count),
    num: (value, options) => value.toLocaleString(intl, options),
    dateTime: (value) => toDate(value).toLocaleString(intl),
    date: (value) => toDate(value).toLocaleDateString(intl),
    time: (value) => toDate(value).toLocaleTimeString(intl),
    list: (items) => items.join(raw('aiworkspace.common.listSeparator')),
    clauses: (items) => items.join(raw('aiworkspace.common.clauseSeparator')),
    compare: (a, b) => collator.compare(a, b),
  }
}

/** The text helpers for a language (default: the current one). Cached: the same object for the same language.
 * A language whose table has not arrived yet starts loading it and reads the language shown before meanwhile. */
export function aiwsText(wanted?: string): AiwsText {
  const current = currentAiwsLocale()
  const locale = wanted ?? current
  if (!aiwsLocaleReady(locale)) {
    void loadAiwsLocale(locale)
    return aiwsText(shown)
  }
  if (locale === current) shown = locale
  let text = cache.get(locale)
  if (!text) {
    text = build(locale)
    cache.set(locale, text)
  }
  return text
}

/** One string in the current language, for code outside React. */
export function aiwsT(key: string, vars?: TextVars): string {
  return aiwsText().t(key, vars)
}

/**
 * A label table read at render time: `labels.edit` is the current language's text for the key given here.
 * For exported `Record<K, string>` label maps used by code that already re-renders with the language.
 */
export function localizedLabels<K extends string>(keys: Record<K, string>): Readonly<Record<K, string>> {
  const out = {} as Record<K, string>
  for (const name of Object.keys(keys) as K[]) {
    Object.defineProperty(out, name, { enumerable: true, get: () => aiwsT(keys[name]) })
  }
  return out
}

/** The language the shared SDK helpers (buckyos: standard tables, recurrence texts) take for the current one. */
export type SdkLocale = 'zh' | 'zh-TW' | 'en' | 'ja' | 'ko' | 'fr' | 'de' | 'es' | 'ru'
export function sdkLocale(locale: string = aiwsText().intl): SdkLocale {
  if (locale === 'zh-CN') return 'zh'
  return (['zh-TW', 'ja', 'ko', 'fr', 'de', 'es', 'ru'] as const).find((l) => l === locale) ?? 'en'
}
