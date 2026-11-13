/* The permission panel and the share dialog still speak in grants (`subject × scope → capabilities`). The service's
 * model is entries of single callers, logical groups and fixed groups (README “Permissions”, 权限需求 §4); until the
 * permission UI of 权限需求 §6 replaces these screens, this maps the old shape onto it:
 * - `public` is `@guest` with anonymous access switched on;
 * - `*`, `group:<id>`, `contact:<did>#<tag>` each become a one-member logical group (`zone`, `sys-…`, `contact-…`);
 * - a share entry's capability preset becomes a logical group `share-<caps>` allowed those capabilities on `/`. */

import { unwrap, type AiwsClient } from './client'
import type { Capability, Grant, GrantList, ShareEntry } from './types'

type Ws = { workspace_id: string }

export const PUBLIC_SUBJECT = 'public'
const ORDER: Capability[] = ['read', 'append', 'update', 'delete', 'structure', 'comment', 'export', 'manage', 'run']

function slug(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 56) || 'x'
}

/** The logical group standing for a member spec (`*`, `group:`, `contact:`), or null for a single caller. */
function groupOf(spec: string): string | null {
  if (spec === '*') return 'zone'
  if (spec.startsWith('group:')) return `sys-${slug(spec.slice(6))}`
  if (spec.startsWith('contact:')) return `contact-${slug(spec.slice(8))}`
  return null
}

async function subjectFor(client: AiwsClient, ws: Ws, spec: string, creating: boolean): Promise<string> {
  if (spec === PUBLIC_SUBJECT) {
    if (creating) unwrap(await client.aclSetAnonymous(ws, true))
    return '@guest'
  }
  const group = groupOf(spec)
  if (!group) return spec
  if (creating) {
    unwrap(await client.aclSetGroup(ws, group, spec))
    unwrap(await client.aclMembers(ws, group, [spec]))
  }
  return `@${group}`
}

export async function grant(client: AiwsClient, ws: Ws, spec: string, capabilities: Capability[], scope?: string) {
  const subject = await subjectFor(client, ws, spec, true)
  unwrap(await client.aclSet(ws, [{ subject, resource: scope ?? '/', allow: capabilities }]))
}

export async function revoke(client: AiwsClient, ws: Ws, spec: string, scope?: string) {
  const subject = await subjectFor(client, ws, spec, false)
  unwrap(await client.aclSet(ws, [{ subject, resource: scope ?? '/', allow: [], deny: [] }]))
  if (spec === PUBLIC_SUBJECT && !scope) unwrap(await client.aclSetAnonymous(ws, false))
}

/** Every entry as a grant (a manager), or the caller's own: one-member groups show as their member spec. */
export async function listGrants(client: AiwsClient, ws: Ws): Promise<GrantList> {
  const all = unwrap(await client.aclQueryAll(ws))
  const specs = new Map<string, string>()
  if (all.complete) {
    for (const g of unwrap(await client.aclGroups(ws)).groups) {
      const only = g.members?.length === 1 ? g.members[0].member : null
      if (only && groupOf(only) === g.group_id) specs.set(g.subject, only)
    }
  }
  const grants: Grant[] = all.entries
    .filter((e) => e.allow.length > 0)
    .filter((e) => e.subject !== '@guest' || all.anonymous)
    .map((e) => ({ subject: e.subject === '@guest' ? PUBLIC_SUBJECT : specs.get(e.subject) ?? e.subject, scope_entity_id: e.resource.entity_id, capabilities: e.allow }))
  // `@owner` holds everything without an entry: the panel shows it as the owner's row (never revocable)
  if (all.owner) grants.unshift({ subject: all.owner, scope_entity_id: null, capabilities: [...ORDER] })
  return { grants, complete: all.complete, owner: all.owner, principal: all.principal }
}

/** The group of a share entry made from a capability preset. */
function shareGroup(capabilities: Capability[]): string {
  return `share-${ORDER.filter((c) => capabilities.includes(c)).join('-')}`
}

/** What an entry's group allows, when it is a preset's group (else nothing known). */
export function entryCapabilities(entry: ShareEntry): Capability[] {
  return entry.group_id.startsWith('share-') ? entry.group_id.slice(6).split('-').filter((c): c is Capability => (ORDER as string[]).includes(c)) : []
}

export async function createEntry(client: AiwsClient, ws: Ws, capabilities: Capability[], methods: ShareEntry['methods']) {
  const group = shareGroup(capabilities)
  unwrap(await client.aclSetGroup(ws, group))
  unwrap(await client.aclSet(ws, [{ subject: `@${group}`, resource: '/', allow: capabilities }]))
  return unwrap(await client.wsCreateEntry(ws, { group_id: group, methods })).entry
}
