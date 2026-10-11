import { useMemo } from 'react'
import { useI18n } from '../i18n/provider'
import type { ObjectPresentation, PresentationKind } from './model'
import { didPresentationLoader, usePresentation } from './store'

/** The presentation of any DID, read from its document; shared and deduplicated across components. */
export function useDidPresentation(did: string | undefined, kind: PresentationKind): ObjectPresentation | undefined {
  const { locale } = useI18n()
  const load = useMemo(() => (did ? didPresentationLoader(did, kind, locale) : undefined), [did, kind, locale])
  return usePresentation(did, load)
}
