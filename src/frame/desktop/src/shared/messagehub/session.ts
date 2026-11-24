/** Rules of the session list that MessageHub's store and Activity both apply. */
import type { GroupDoc, GroupDocEnvelope, GroupsByMember, UiSessionStateEntry } from './wire.ts'
import type { Session } from './types.ts'

/** The owner's own settings of a session (`ui_session.list_state`). */
export interface SessionPrefs {
  title: string
  description: string
  pinned: boolean
  muted: boolean
}

export const defaultSessionPrefs: SessionPrefs = { title: '', description: '', pinned: false, muted: false }

export function applySessionPref(prefs: SessionPrefs, entry: UiSessionStateEntry) {
  if (entry.key === 'ui.title' && typeof entry.value === 'string' && entry.value.trim().length <= 64) prefs.title = entry.value.trim()
  if (entry.key === 'ui.description' && typeof entry.value === 'string' && entry.value.trim().length <= 500) prefs.description = entry.value.trim()
  if (entry.key === 'ui.pinned' && typeof entry.value === 'boolean') prefs.pinned = entry.value
  if (entry.key === 'ui.muted' && typeof entry.value === 'boolean') prefs.muted = entry.value
}

export function sessionTitle(session: Pick<Session, 'title' | 'shared'>, preferences: Pick<SessionPrefs, 'title'>) {
  return preferences.title || session.shared.title || session.title
}

export function groupDocOf(value: GroupDocEnvelope | GroupDoc | null | undefined): GroupDoc | null {
  if (!value || typeof value !== 'object') return null
  return 'doc' in value && value.doc && typeof value.doc === 'object' ? value.doc : 'id' in value ? value as GroupDoc : null
}

export function groupName(doc: GroupDoc | null): string {
  return typeof doc?.profile?.name === 'string' ? doc.profile.name.trim() : ''
}

/** The groups the caller participates in: joined elsewhere, and hosted by this Zone. */
export function participatedGroups(result: GroupsByMember): Array<{ did: string; doc: GroupDoc | null; hosted: boolean }> {
  return [
    ...(result.joined ?? []).filter(joined => !joined.stopped).map(joined => ({ did: joined.group_did, doc: groupDocOf(joined.doc_cache), hosted: false })),
    ...(result.items ?? []).flatMap(item => (item.doc?.id ? [{ did: item.doc.id, doc: item.doc, hosted: true }] : [])),
  ]
}
