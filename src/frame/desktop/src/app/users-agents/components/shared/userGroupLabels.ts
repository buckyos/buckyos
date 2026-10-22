import { parseUserGroupRef } from '../../../../api/user_type'
import type { LocalUserEntity, SelfEntity, SystemUserGroup } from '../../datamodel/types'

type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

/** A group reference as people read it: the account-type groups by name, a system group by its name (its id when unknown). */
export function groupRefLabel(ref: string, groups: readonly SystemUserGroup[] | undefined, t: Translate): string {
  const parsed = parseUserGroupRef(ref)
  if (!parsed) return ref
  if (parsed.kind === 'type') return t(`usersAgents.groupRef.type.${parsed.userType}`)
  return groups?.find((group) => group.id === parsed.groupId)?.name ?? parsed.groupId
}

export function systemGroupName(groupId: string, groups: readonly SystemUserGroup[] | undefined): string {
  return groups?.find((group) => group.id === groupId)?.name ?? groupId
}

export interface AccountOption {
  id: string
  name: string
}

/** Accounts that can join system groups: the signed-in account and the listed Zone users. */
export function accountOptions(self: SelfEntity, localUsers: readonly LocalUserEntity[]): AccountOption[] {
  const options = new Map<string, AccountOption>()
  if (self.id && self.id !== 'self') options.set(self.id, { id: self.id, name: self.displayName })
  for (const user of localUsers) options.set(user.id, { id: user.id, name: user.displayName })
  return [...options.values()].sort((a, b) => a.name.localeCompare(b.name))
}

export function accountName(id: string, options: readonly AccountOption[]): string {
  return options.find((option) => option.id === id)?.name ?? id
}
