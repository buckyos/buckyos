import { WELL_KNOWN_CONTACT_TAGS } from 'buckyos'
import { tagApp, tagKind, timeMs } from '../../../components/network/relation'
import type { Contact, LookupKind } from '../../../components/network/types'
import type { CollectionSections, CollectionSummary, MyNetworkView, PeopleFilter, PeopleSort } from './types'

/** The three Collection sections; well-known app and system tags are listed even while empty. */
export function collectionsOf(contacts: readonly Contact[]): CollectionSections {
  const counts = new Map<string, number>()
  for (const contact of contacts) for (const tag of new Set(contact.tags ?? [])) counts.set(tag, (counts.get(tag) ?? 0) + 1)
  for (const known of WELL_KNOWN_CONTACT_TAGS) if (!counts.has(known.key)) counts.set(known.key, 0)
  const sections: CollectionSections = { user: [], app: [], system: [] }
  for (const [tag, count] of counts) {
    const kind = tagKind(tag)
    const summary: CollectionSummary = { tag, kind, count, editable: kind === 'user', ...(kind === 'app' ? { app: tagApp(tag) ?? undefined } : {}) }
    sections[kind].push(summary)
  }
  sections.user.sort((a, b) => a.tag.localeCompare(b.tag))
  sections.app.sort((a, b) => a.tag.localeCompare(b.tag))
  sections.system.sort((a, b) => a.tag.localeCompare(b.tag))
  return sections
}

export function peopleOfView(contacts: readonly Contact[], view: MyNetworkView): Contact[] {
  if (view.kind === 'friends') return contacts.filter(contact => contact.friend_state === 'friend')
  if (view.kind === 'collection') return contacts.filter(contact => (contact.tags ?? []).includes(view.tag))
  return [...contacts]
}

export function filterPeople(contacts: readonly Contact[], query: string, filter: PeopleFilter, sort: PeopleSort): Contact[] {
  const needle = query.trim().toLowerCase()
  return contacts
    .filter((contact) => {
      if (filter === 'friends' && contact.friend_state !== 'friend') return false
      if (filter === 'others' && (contact.friend_state === 'friend' || contact.access_level === 'block')) return false
      if (filter === 'blocked' && contact.access_level !== 'block') return false
      if (!needle) return true
      return [contact.name, contact.did, contact.note, ...(contact.tags ?? []), ...(contact.bindings ?? []).map(binding => binding.display_id)]
        .some(value => value?.toLowerCase().includes(needle))
    })
    .sort((a, b) => sort === 'recent'
      ? (timeMs(b.created_at) ?? 0) - (timeMs(a.created_at) ?? 0)
      : (a.name || a.did).localeCompare(b.name || b.did))
}

export function recentlyAdded(contacts: readonly Contact[], count = 5): Contact[] {
  return [...contacts].sort((a, b) => (timeMs(b.created_at) ?? 0) - (timeMs(a.created_at) ?? 0)).slice(0, count)
}

/** A contact's kind as far as the record tells (lookup knows better when it has answered). */
export function kindOfContact(contact: Contact): LookupKind {
  if (contact.did.startsWith('did:msgtunnel:')) return 'msgtunnel'
  if ((contact.tags ?? []).includes('zone_agent')) return 'agent'
  return 'user'
}
