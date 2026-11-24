/* ── What a page that shows one app alone knows about where it runs ── */

import { createContext, useContext } from 'react'
import type { RuntimeContainer } from 'buckyos'

export interface StandalonePageValue {
  container: RuntimeContainer
  /** `tab`: the page fills a tab of the BuckyOS client, which has its own navigation. */
  presentation: 'window' | 'tab'
}

export const StandalonePageContext = createContext<StandalonePageValue | null>(null)

/** Null in a window of the desktop. */
export function useStandalonePage(): StandalonePageValue | null {
  return useContext(StandalonePageContext)
}
