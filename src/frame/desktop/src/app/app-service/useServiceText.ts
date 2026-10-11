import { useCallback } from 'react'
import { useI18n } from '../../i18n/provider'
import { pickLocalized } from '../../presentation/model'
import type { AppServiceItem } from './types'

/** A real app's own document text; the prototype fixtures' name and description key otherwise. */
export function useServiceText() {
  const { locale, t } = useI18n()
  return useCallback((service: Pick<AppServiceItem, 'name' | 'description' | 'text'>) => {
    const title = pickLocalized(service.text?.title, locale) ?? service.name
    const summary = service.text
      ? pickLocalized(service.text.summary, locale)
      : service.description ? t(service.description) : undefined
    return { title, summary: summary && summary !== title ? summary : undefined }
  }, [locale, t])
}
