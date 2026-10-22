/* ── Account types, system user groups and contact projection rules ──
 *
 * One place for the user type definition (product/users_and_agents/UserType.md):
 * mirrors `UserType` and `UserGroupRef` in buckyos-api. Every Desktop check of an
 * account type goes through these helpers instead of comparing strings.
 */

/** Account types of a Zone account record; `root` is the Zone owner record and never logs in. */
export type UserType = 'admin' | 'user' | 'limited' | 'root'

/** Types an admin may create, invite or change an account to. */
export type AssignableUserType = Exclude<UserType, 'root'>

export const ASSIGNABLE_USER_TYPES: readonly AssignableUserType[] = ['admin', 'user', 'limited']

/** Types `user.create` accepts; Admin is reached by changing the type. */
export const CREATABLE_USER_TYPES: readonly AssignableUserType[] = ['user', 'limited']

export function parseUserType(value: unknown): UserType | null {
  const normalized = typeof value === 'string' ? value.trim().toLowerCase() : ''
  return normalized === 'admin' || normalized === 'user' || normalized === 'limited' || normalized === 'root'
    ? normalized
    : null
}

/** Admin and the owner record manage the Zone. */
export function isPrivilegedUserType(value: unknown): boolean {
  const type = parseUserType(value)
  return type === 'admin' || type === 'root'
}

/** Only Limited accounts are refused; an unknown type is left to the backend. */
export function userTypeCanCreateAgents(value: unknown): boolean {
  return parseUserType(value) !== 'limited'
}

/** Limited accounts may not change their password or address book unless an admin allows it. */
export function defaultCanChangePassword(value: unknown): boolean {
  return parseUserType(value) !== 'limited'
}

export function defaultCanEditContacts(value: unknown): boolean {
  return parseUserType(value) !== 'limited'
}

/** The types an account can be changed to; the owner record cannot change. */
export function changeTypeTargets(current: unknown): AssignableUserType[] {
  const type = parseUserType(current)
  if (type === 'root') return []
  return ASSIGNABLE_USER_TYPES.filter((target) => target !== type)
}

// ── System user groups ──

export const USER_GROUP_ID_PATTERN = /^[a-z][a-z0-9_-]{0,31}$/
export const USER_TYPE_GROUP_REF_PREFIX = 'type:'
export const USER_GROUP_REF_PREFIX = 'group:'

export type UserGroupRefInfo =
  | { kind: 'type'; userType: AssignableUserType }
  | { kind: 'group'; groupId: string }

/** `type:admin|user|limited` or `group:<id>`; anything else is null. */
export function parseUserGroupRef(ref: string): UserGroupRefInfo | null {
  if (ref.startsWith(USER_GROUP_REF_PREFIX)) {
    const groupId = ref.slice(USER_GROUP_REF_PREFIX.length)
    return USER_GROUP_ID_PATTERN.test(groupId) ? { kind: 'group', groupId } : null
  }
  if (ref.startsWith(USER_TYPE_GROUP_REF_PREFIX)) {
    const type = parseUserType(ref.slice(USER_TYPE_GROUP_REF_PREFIX.length))
    return type && type !== 'root' ? { kind: 'type', userType: type } : null
  }
  return null
}

export function userTypeGroupRef(type: AssignableUserType): string {
  return `${USER_TYPE_GROUP_REF_PREFIX}${type}`
}

export function systemGroupRef(groupId: string): string {
  return `${USER_GROUP_REF_PREFIX}${groupId}`
}

/** Members of `source` appear, read-only, in the address book of members of `target`. */
export interface ContactProjectionRule {
  source: string
  target: string
}

/** What the Zone uses until an admin saves rules: Admin and User accounts see each other. */
export function defaultContactProjectionRules(): ContactProjectionRule[] {
  const admin = userTypeGroupRef('admin')
  const user = userTypeGroupRef('user')
  return [
    { source: admin, target: admin },
    { source: admin, target: user },
    { source: user, target: admin },
    { source: user, target: user },
  ]
}

/** App availability group of an account type (`guest` is the anonymous pseudo-group, not a type). */
export function availabilityGroupOfUserType(value: unknown): 'admins' | 'users' | 'limited' | null {
  switch (parseUserType(value)) {
    case 'admin':
      return 'admins'
    case 'user':
      return 'users'
    case 'limited':
      return 'limited'
    default:
      return null
  }
}
