import { createContext, useContext } from 'react'
import type { ObjId } from './protocol/feed'
import type { ReaderIdentity } from './datamodel/types'
import type { ViewPerspective } from './types'

export type HsPage =
  | { name: 'feed' }
  | { name: 'profile' }
  | { name: 'published' }
  | { name: 'candidates' }
  | { name: 'saved'; kind: 'bookmark' | 'read_later' }
  | { name: 'sources' }
  | { name: 'prefs' }
  | { name: 'detail'; objId: ObjId }
  | { name: 'me' }
  | { name: 'publish' }

export interface HsNav {
  page: HsPage
  perspective: ViewPerspective
  reader: ReaderIdentity
  isDesktop: boolean
  /** Shown in a tab of the BuckyOS client: there is no tab bar of its own. */
  inTab: boolean
  navigate: (page: HsPage, options?: { reset?: boolean }) => void
  back: () => void
  openDetail: (objId: ObjId) => void
  showFilteredInFeed: () => void
}

const noop = () => {}

export const HsNavContext = createContext<HsNav>({
  page: { name: 'feed' },
  perspective: 'owner',
  reader: { kind: 'owner' },
  isDesktop: true,
  inTab: false,
  navigate: noop,
  back: noop,
  openDetail: noop,
  showFilteredInFeed: noop,
})

export function useHsNav() {
  return useContext(HsNavContext)
}
