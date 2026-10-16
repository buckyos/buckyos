/* The AI Workspace text helpers for React code (see i18n.ts): the Desktop's language from its I18nProvider,
 * so a component re-renders when the language changes, plus `tNode` for templates with React-node parts. */

import { createElement, Fragment, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { aiwsText, type AiwsText } from './i18n'

export interface AiwsUiText extends AiwsText {
  /** A template whose placeholders are React nodes (a bold name, a link, an icon); `count` picks the singular. */
  tNode(key: string, vars: Record<string, ReactNode>): ReactNode
}

const cache = new Map<string, AiwsUiText>()

function uiText(locale: string): AiwsUiText {
  const known = cache.get(locale)
  if (known) return known
  const text = aiwsText(locale)
  const ui: AiwsUiText = {
    ...text,
    tNode: (key, vars) => {
      const parts = text.template(key, typeof vars.count === 'number' ? vars.count : undefined).split(/\{\{(\w+)\}\}/)
      return createElement(Fragment, null, ...parts.map((part, i) => (i % 2 === 1 ? (part in vars ? vars[part] : `{{${part}}}`) : part)))
    },
  }
  cache.set(locale, ui)
  return ui
}

/** The text helpers in the Desktop's language; the same object while the language stays the same. */
export function useAiwsText(): AiwsUiText {
  const { locale } = useI18n()
  return uiText(locale)
}
