/* The non-visual rules of the permission views: what a typed path names, how rules are ordered, why a typed principal
 * cannot be a subject, which binding the operation record leaves for a macro. */

import { resolvePath } from '../../api/paths'
import type { AclAuditItem, AclEntry, AclGroup, EntityEnvelope } from '../../api/types'
import type { OutlineModel } from '../../state/outline'
import { field } from './aclData'
import { ALL, CREATOR, GUEST } from './model'

export type PathCheck =
  | { ok: true; entity: EntityEnvelope | null }
  | { ok: false; reason: 'empty' | 'missing' | 'scope' | 'deleted' }

/** What a typed path names for this caller: the Workspace (`entity: null`), an entity that can carry entries, or why not. */
export function checkResourcePath(outline: OutlineModel, path: string): PathCheck {
  const p = path.trim()
  if (!p) return { ok: false, reason: 'empty' }
  if (p === '/') return { ok: true, entity: null }
  const entity = resolvePath(outline, p)
  if (!entity) {
    const raw = !p.startsWith('/') && !p.startsWith('~') ? outline.get(p) : undefined
    return { ok: false, reason: raw?.deleted ? 'deleted' : 'missing' }
  }
  if (entity.entity_id === 'root') return { ok: true, entity: null }
  if ((entity.scope ?? 'shared') !== 'shared') return { ok: false, reason: 'scope' }
  return { ok: true, entity }
}

export function sortEntries(entries: readonly AclEntry[]): AclEntry[] {
  return [...entries].sort((a, b) => (a.resource.path === b.resource.path ? a.subject.localeCompare(b.subject) : a.resource.path === '/' ? -1 : b.resource.path === '/' ? 1 : a.resource.path.localeCompare(b.resource.path)))
}

/** Why a typed principal cannot be a subject, or null. */
export function subjectProblem(value: string, groups: readonly AclGroup[]): string | null {
  const v = value.trim()
  if (!v) return 'aiworkspace.perm.subject.required'
  if (/\s/.test(v)) return 'aiworkspace.perm.subject.spaces'
  if (v === '*' || v === 'public' || v.startsWith('group:') || v.startsWith('contact:')) return 'aiworkspace.perm.subject.memberSpec'
  if (v === '@owner') return 'aiworkspace.perm.subject.owner'
  if (v.startsWith('@') && ![ALL, GUEST, CREATOR].includes(v) && !groups.some((g) => g.subject === v)) return 'aiworkspace.perm.subject.noGroup'
  return null
}

export type Binding = { group_id: string; mode: 'union' | 'intersection' } | null

/** The binding the record leaves for `wishId`: the last `acl.bind_macro` of it, gone when its group was deleted after;
 * `undefined` when a permission design was loaded afterwards (its bindings are not itemised). */
export function bindingFromAudit(items: readonly AclAuditItem[], wishId: string): Binding | undefined {
  let binding: Binding | undefined = null
  for (const item of items) {
    if (item.action === 'acl.bind_macro' && field(item.target, 'wish', 'entity_id') === wishId) {
      const after = item.after as { group_id?: string; mode?: string } | null
      binding = after?.group_id ? { group_id: after.group_id, mode: after.mode === 'intersection' ? 'intersection' : 'union' } : null
    } else if (item.action === 'acl.delete_group' && binding && field(item.target, 'group_id') === binding.group_id) {
      binding = null
    } else if (item.action === 'acl.load_design') {
      binding = undefined
    }
  }
  return binding
}
