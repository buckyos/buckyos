import { useCallback } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { GuestTextKey } from '../../../i18n/app-ui'

export type { GuestTextKey }
export type GuestText = (key: GuestTextKey, variables?: Record<string, string | number>) => string

export function useGuestText(): GuestText {
  const { t } = useI18n()
  return useCallback((key, variables) => t(`messagehub.sso.${key}`, undefined, variables), [t])
}
