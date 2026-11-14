/* The permission model as the editor shows it (权限需求 §4; UI 框架 PRD §8): the nine capabilities, the three states
 * an entry gives each (allow, deny, not set), the fixed groups and what they may never be allowed (R18), subjects and
 * member specs by kind. No calls here: aclData.ts talks to the service. */

import type { AclEntry, Capability } from '../../api/types'

export const CAPS: readonly Capability[] = ['read', 'append', 'update', 'delete', 'structure', 'comment', 'export', 'manage', 'run']

export type CapState = 'allow' | 'deny' | 'unset'

/** What one entry says: allowed and refused capabilities (both empty: no entry). */
export interface EntryCaps { allow: Capability[]; deny: Capability[] }

export const EMPTY_CAPS: EntryCaps = { allow: [], deny: [] }

export function sortCaps(caps: readonly Capability[]): Capability[] {
  return CAPS.filter((c) => caps.includes(c))
}

export function capState(entry: EntryCaps, cap: Capability): CapState {
  return entry.deny.includes(cap) ? 'deny' : entry.allow.includes(cap) ? 'allow' : 'unset'
}

export function withCap(entry: EntryCaps, cap: Capability, state: CapState): EntryCaps {
  const allow = entry.allow.filter((c) => c !== cap)
  const deny = entry.deny.filter((c) => c !== cap)
  if (state === 'allow') allow.push(cap)
  if (state === 'deny') deny.push(cap)
  return { allow: sortCaps(allow), deny: sortCaps(deny) }
}

export function sameCaps(a: EntryCaps, b: EntryCaps): boolean {
  return sortCaps(a.allow).join() === sortCaps(b.allow).join() && sortCaps(a.deny).join() === sortCaps(b.deny).join()
}

export function isEmptyEntry(entry: EntryCaps): boolean {
  return entry.allow.length === 0 && entry.deny.length === 0
}

/** A preset (ui/sources/presets.ts: a quick choice, §8.3) applied to an entry: its capabilities allowed (those the subject may hold), refusals of others kept. */
export function applyPreset(entry: EntryCaps, caps: readonly Capability[], forbidden: readonly Capability[]): EntryCaps {
  const allow = caps.filter((c) => !forbidden.includes(c))
  return { allow: sortCaps(allow), deny: sortCaps(entry.deny.filter((c) => !allow.includes(c))) }
}

export const OWNER = '@owner'
export const ALL = '@all'
export const GUEST = '@guest'
export const CREATOR = '@creator'
export const FIXED_GROUPS = [OWNER, ALL, GUEST, CREATOR] as const
export const PUB_PREFIX = '@pub:'

export type SubjectKind = 'owner' | 'fixed' | 'group' | 'publication' | 'single'

export function subjectKind(subject: string): SubjectKind {
  if (subject === OWNER) return 'owner'
  if ((FIXED_GROUPS as readonly string[]).includes(subject)) return 'fixed'
  if (subject.startsWith(PUB_PREFIX)) return 'publication'
  if (subject.startsWith('@')) return 'group'
  return 'single'
}

/** What a subject may never be allowed (R18): `@all` / `@guest` neither `manage` nor `run`, `@creator` not `manage`; a
 * share entry's group neither, a macro's group not `manage`; a publication is allowed by `ws.publish` only. */
export function forbiddenAllow(subject: string, limits?: { entryTarget?: boolean; macroGroup?: boolean }): Capability[] {
  if (subject === ALL || subject === GUEST) return ['manage', 'run']
  if (subject === CREATOR) return ['manage']
  if (subject.startsWith(PUB_PREFIX)) return [...CAPS]
  if (limits?.entryTarget) return ['manage', 'run']
  if (limits?.macroGroup) return ['manage']
  return []
}

const RESERVED_GROUP_IDS = ['owner', 'all', 'guest', 'creator', 'pub']

/** A logical group's id as the service takes it: `[a-z0-9][a-z0-9_-]{0,63}`, none of the fixed names. */
export function isGroupId(id: string): boolean {
  return /^[a-z0-9][a-z0-9_-]{0,63}$/.test(id) && !RESERVED_GROUP_IDS.includes(id)
}

/** Where a member spec comes from (§8.2). */
export type MemberKind = 'user' | 'agent' | 'did' | 'sysgroup' | 'contact' | 'zone' | 'guest' | 'link'

export function memberKind(member: string, agents?: ReadonlySet<string>): MemberKind {
  if (member === '*') return 'zone'
  if (member.startsWith('group:')) return 'sysgroup'
  if (member.startsWith('contact:')) return 'contact'
  if (member.startsWith('guest:')) return 'guest'
  if (member.startsWith('link:')) return 'link'
  if (agents?.has(member)) return 'agent'
  if (member.startsWith('did:')) return 'did'
  return 'user'
}

/** A member spec the service accepts (a group is never a member of another group): `null` when acceptable, else why. */
export function memberProblem(member: string): 'empty' | 'group' | 'spaces' | 'public' | null {
  if (!member) return 'empty'
  if (member.startsWith('@')) return 'group'
  if (/\s/.test(member)) return 'spaces'
  if (member === 'public') return 'public'
  return null
}

/** The groups holding `manage` somewhere: only the owner changes them and their members (R17). */
export function groupsHoldingManage(entries: readonly AclEntry[]): Set<string> {
  return new Set(entries.filter((e) => e.kind === 'group' && e.allow.includes('manage')).map((e) => e.subject))
}
