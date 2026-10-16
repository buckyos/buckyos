/* AI Workspace text. Every string the app shows is a key `aiworkspace.<area>.<name>` with an English and a
 * Chinese entry in src/i18n/aiworkspace/<area>.ts; tests/datamodel/aiworkspace-i18n.test.ts keeps the two
 * tables complete and the source free of hard-coded Chinese. Other Desktop languages fall back to English,
 * like the rest of the Desktop.
 *
 * React code takes the language from the Desktop's I18nProvider (`useAiwsText` in useAiwsText.ts) and
 * re-renders when it changes. Code outside React (store, session, offline replica, extension runtime) asks
 * `aiwsT` / `aiwsText()` here, which read the language the provider persisted; a context without page
 * storage (the replica worker, Node test fixtures) is told its language with `setAiwsLocale`. This module
 * imports neither React nor the Desktop dictionaries, so the worker can use it.
 *
 * Templates use `{{name}}` placeholders. A count reads the `<key>_one` entry, when the table has one, for
 * `count: 1` (English singulars); Chinese needs none. Lists and clauses are joined with the language's
 * separators (`list`, `clauses`), numbers and times are formatted for the language, never for 'zh-CN'. */

import { aiWorkspaceEn, aiWorkspaceZh } from '../../i18n/aiworkspace'

export type TextVars = Record<string, string | number>

export interface AiwsText {
  /** The Desktop language this text is in (a language without a table reads English). */
  locale: string
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

const LOCALE_STORAGE_KEY = 'buckyos.prototype.locale.v1'
const TABLES: Record<string, Record<string, string>> = { en: aiWorkspaceEn, 'zh-CN': aiWorkspaceZh }

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

function pick(table: Record<string, string>, key: string, count: unknown): string | undefined {
  if (count === 1) {
    const one = table[`${key}_one`]
    if (one !== undefined) return one
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

const cache = new Map<string, AiwsText>()

/** The text helpers for a language (default: the current one). Cached: the same object for the same language. */
export function aiwsText(locale: string = currentAiwsLocale()): AiwsText {
  const known = cache.get(locale)
  if (known) return known
  const table = TABLES[locale] ?? aiWorkspaceEn
  const intl = TABLES[locale] ? locale : 'en'
  const raw = (key: string, count?: unknown) => pick(table, key, count) ?? pick(aiWorkspaceEn, key, count) ?? key
  const collator = new Intl.Collator(intl, { numeric: true })
  const text: AiwsText = {
    locale,
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
  cache.set(locale, text)
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
