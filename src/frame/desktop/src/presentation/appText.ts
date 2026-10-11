/* ── An app's title and summary: its document text first, the built-in catalog's i18n keys as defaults ── */

import { useCallback } from 'react'
import { dictionaries } from '../i18n/dictionaries'
import { useI18n } from '../i18n/provider'
import type { AppDefinition } from '../models/ui'
import { pickLocalized } from './model'

type Translate = (key: string, fallback?: string) => string
type AppTextSource = Pick<AppDefinition, 'labelKey' | 'text'> & Partial<Pick<AppDefinition, 'summaryKey'>>

export function appDisplayText(app: AppTextSource, locale: string, t: Translate): { title: string; summary?: string } {
  const title = pickLocalized(app.text?.title, locale) ?? t(app.labelKey)
  const summary = pickLocalized(app.text?.summary, locale) ?? (app.summaryKey ? t(app.summaryKey, '') : '')
  return summary && summary !== title ? { title, summary } : { title }
}

/** Re-renders with the language, so every entry shows the same text for the same app. */
export function useAppText() {
  const { locale, t } = useI18n()
  return useCallback((app: AppTextSource) => appDisplayText(app, locale, t), [locale, t])
}

/** For code outside React (menus built by plain functions): the page's current language. */
export function appTitleNow(app: AppTextSource): string {
  const locale = document.documentElement.lang || 'en'
  const table = (dictionaries as Record<string, Record<string, string>>)[locale] ?? dictionaries.en
  return appDisplayText(app, locale, (key, fallback) => table[key] ?? dictionaries.en[key] ?? fallback ?? key).title
}
