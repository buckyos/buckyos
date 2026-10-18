/* ── My Network views (see ../UI_DATAMODEL.md); relationship data comes from components/network ── */

import type { ContactTagKind } from 'buckyos'

export type MyNetworkView =
  | { kind: 'home' }
  | { kind: 'contacts' }
  | { kind: 'friends' }
  | { kind: 'requests' }
  | { kind: 'groups' }
  | { kind: 'collection'; tag: string }

/** A Collection is a contact tag: user tags are editable, app and system tags are read-only views. */
export interface CollectionSummary {
  tag: string
  kind: ContactTagKind
  /** The app that owns an app tag. */
  app?: string
  count: number
  editable: boolean
}

export interface CollectionSections {
  user: CollectionSummary[]
  app: CollectionSummary[]
  system: CollectionSummary[]
}

export type PeopleFilter = 'all' | 'friends' | 'others' | 'blocked'
export type PeopleSort = 'name' | 'recent'
