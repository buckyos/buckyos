/* The AI Workspace text helpers for React code (see i18n.ts): the Desktop's language from its I18nProvider,
 * so a component re-renders when the language changes — and again when that language's table arrives —
 * plus `tNode` for templates with React-node parts. */

import { createElement, Fragment, useSyncExternalStore, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { aiwsText, aiwsTextVersion, subscribeAiwsText, type AiwsText } from './i18n'

export interface AiwsUiText extends AiwsText {
  /** A template whose placeholders are React nodes (a bold name, a link, an icon); `count` picks the plural form. */
  tNode(key: string, vars: Record<string, ReactNode>): ReactNode
}

const cache = new WeakMap<AiwsText, AiwsUiText>()

function uiText(text: AiwsText): AiwsUiText {
  const known = cache.get(text)
  if (known) return known
  const ui: AiwsUiText = {
    ...text,
    tNode: (key, vars) => {
      const parts = text.template(key, typeof vars.count === 'number' ? vars.count : undefined).split(/\{\{(\w+)\}\}/)
      return createElement(Fragment, null, ...parts.map((part, i) => (i % 2 === 1 ? (part in vars ? vars[part] : `{{${part}}}`) : part)))
    },
  }
  cache.set(text, ui)
  return ui
}

/** The text helpers in the Desktop's language; the same object while the text stays the same. While the
 * language's table loads, the text stays in the language shown before. */
export function useAiwsText(): AiwsUiText {
  const { locale } = useI18n()
  useSyncExternalStore(subscribeAiwsText, aiwsTextVersion, aiwsTextVersion)
  return uiText(aiwsText(locale))
}
