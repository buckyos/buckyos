import type { ReactNode } from 'react'
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'

import {
  fetchControlPanelLocale,
  saveControlPanelLocale,
  type ControlPanelLocale,
} from '@/api'

import {
  SUPPORTED_CONTROL_PANEL_LOCALES,
  translate,
  translateSource,
  type TranslationValues,
} from './resources'

type I18nContextValue = {
  locale: ControlPanelLocale
  ready: boolean
  loading: boolean
  settingKey: string
  t: (key: string, fallback?: string, values?: TranslationValues) => string
  refreshLocale: () => Promise<ControlPanelLocale>
  setPersistedLocale: (locale: ControlPanelLocale) => Promise<{ locale: ControlPanelLocale; error: unknown }>
}

const I18nContext = createContext<I18nContextValue | null>(null)

const DEFAULT_KEY = 'services/control_panel/settings/locale'
const ORIGINAL_TEXT = new WeakMap<Node, string>()

const TEXT_ATTRS = ['placeholder', 'title', 'aria-label'] as const

const translateDomTree = (root: ParentNode, locale: ControlPanelLocale) => {
  const textWalker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  let currentText = textWalker.nextNode()
  while (currentText) {
    const parent = currentText.parentElement
    if (parent && !['SCRIPT', 'STYLE'].includes(parent.tagName)) {
      if (!ORIGINAL_TEXT.has(currentText)) {
        ORIGINAL_TEXT.set(currentText, currentText.textContent ?? '')
      }
      const original = ORIGINAL_TEXT.get(currentText) ?? currentText.textContent ?? ''
      const translated = translateSource(locale, original)
      if (translated !== currentText.textContent) {
        currentText.textContent = translated
      }
    }
    currentText = textWalker.nextNode()
  }

  const elements = root instanceof Element ? [root, ...Array.from(root.querySelectorAll('*'))] : Array.from(root.querySelectorAll('*'))
  for (const element of elements) {
    for (const attr of TEXT_ATTRS) {
      const originalKey = `data-cp-i18n-${attr}`
      const existing = element.getAttribute(attr)
      if (existing == null) continue
      if (!element.hasAttribute(originalKey)) {
        element.setAttribute(originalKey, existing)
      }
      const original = element.getAttribute(originalKey) ?? existing
      const translated = translateSource(locale, original)
      if (translated !== existing) {
        element.setAttribute(attr, translated)
      }
    }

    if (element instanceof HTMLInputElement) {
      const inputType = (element.type || '').toLowerCase()
      if (['button', 'submit', 'reset'].includes(inputType) && element.value) {
        const originalKey = 'data-cp-i18n-value'
        if (!element.hasAttribute(originalKey)) {
          element.setAttribute(originalKey, element.value)
        }
        const original = element.getAttribute(originalKey) ?? element.value
        const translated = translateSource(locale, original)
        if (translated !== element.value) {
          element.value = translated
        }
      }
    }
  }
}

export const I18nProvider = ({ children }: { children: ReactNode }) => {
  const [locale, setLocale] = useState<ControlPanelLocale>('en')
  const [ready, setReady] = useState(false)
  const [loading, setLoading] = useState(false)
  const [settingKey, setSettingKey] = useState(DEFAULT_KEY)

  const refreshLocale = useCallback(async () => {
    setLoading(true)
    const { data, error } = await fetchControlPanelLocale()
    const nextLocale = data?.locale === 'zh-CN' ? 'zh-CN' : 'en'
    if (!error) {
      setLocale(nextLocale)
      setSettingKey(data?.key ?? DEFAULT_KEY)
    }
    setReady(true)
    setLoading(false)
    return nextLocale
  }, [])

  useEffect(() => {
    void refreshLocale()
  }, [refreshLocale])

  useEffect(() => {
    document.documentElement.lang = locale
  }, [locale])

  useEffect(() => {
    if (typeof document === 'undefined' || !ready) return

    const run = () => translateDomTree(document.body, locale)
    run()

    const observer = new MutationObserver(() => {
      run()
    })

    observer.observe(document.body, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: [...TEXT_ATTRS, 'value'],
    })

    return () => {
      observer.disconnect()
    }
  }, [locale, ready])

  const setPersistedLocale = useCallback(async (nextLocale: ControlPanelLocale) => {
    const previousLocale = locale
    setLocale(nextLocale)
    const { data, error } = await saveControlPanelLocale(nextLocale)
    const normalized: ControlPanelLocale = data?.locale === 'zh-CN' ? 'zh-CN' : 'en'
    if (!error) {
      setLocale(normalized)
      setSettingKey(data?.key ?? DEFAULT_KEY)
    } else {
      setLocale(previousLocale)
    }
    return { locale: normalized, error }
  }, [locale])

  const value = useMemo<I18nContextValue>(
    () => ({
      locale,
      ready,
      loading,
      settingKey,
      t: (key, fallback, values) => translate(locale, key, fallback, values),
      refreshLocale,
      setPersistedLocale,
    }),
    [locale, ready, loading, settingKey, refreshLocale, setPersistedLocale],
  )

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export const useI18n = () => {
  const context = useContext(I18nContext)
  if (!context) {
    throw new Error('useI18n must be used within I18nProvider')
  }
  return context
}

export const getLocaleLabel = (locale: ControlPanelLocale, t: I18nContextValue['t']) => {
  if (locale === 'zh-CN') return t('settings.localeChineseSimplified', 'Simplified Chinese')
  return t('settings.localeEnglish', 'English')
}

export { SUPPORTED_CONTROL_PANEL_LOCALES }
