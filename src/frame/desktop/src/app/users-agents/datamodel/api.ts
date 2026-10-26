import { fetchCurrentAccount } from '../../../api/account.ts'
import { userTypeCanCreateAgents } from '../../../api/user_type.ts'
import { fetchAppList } from '../../../api/app_mgr.ts'
import {
  fetchAgentList,
  fetchUserDetail,
  fetchUserGroups,
  fetchUserList,
} from '../../../api/user_mgr.ts'
import type { UserContactSettings, UserDetail, UserInfo } from '../../../api/user_mgr.ts'
import { isPrivilegedUserType } from '../../../api/user_type.ts'
import { buckyos } from 'buckyos'
import { usableImageUrl } from '../../../presentation/model.ts'
import {
  mockLocalUsers,
  mockSelf,
} from '../mock/seed'
import type {
  LocalUserEntity,
  UserGroupsInfo,
  UsersAgentsSnapshot,
} from './types'
import {
  appListTargetUserIds,
  toAgentEntity,
  toLocalUserEntity,
  toSelfEntity,
  toSocialAccounts,
  toUserGroupsInfo,
  toVisibleLocalUserEntities,
} from './transforms'

interface AccountInfo {
  user_name: string
  user_id: string
  user_type?: string
}

function accountDetail(account: AccountInfo): UserDetail {
  return {
    user_id: account.user_id,
    show_name: account.user_name || account.user_id,
    user_type: account.user_type || 'user',
    state: 'active',
    res_pool_id: '',
    profile: {
      display_name: account.user_name || account.user_id,
    },
    allow_password_change: false,
  }
}

const APP_LIST_CONCURRENCY = 4

async function forEachWithConcurrency<T>(
  items: readonly T[],
  limit: number,
  worker: (item: T) => Promise<void>,
): Promise<void> {
  let nextIndex = 0
  const runners = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (nextIndex < items.length) {
      const item = items[nextIndex]
      nextIndex += 1
      await worker(item)
    }
  })
  await Promise.all(runners)
}

/** Groups and projection rules are for admins only; anyone else (or a failed read) gets null. */
async function loadUserGroups(viewerIsAdmin: boolean): Promise<UserGroupsInfo | null> {
  if (!viewerIsAdmin) return null
  const { data } = await fetchUserGroups()
  return data ? toUserGroupsInfo(data) : null
}

export async function fetchUsersAgentsSnapshot(): Promise<UsersAgentsSnapshot> {
  const accountInfo = await buckyos.getAccountInfo() as AccountInfo | null
  const selfUserId = accountInfo?.user_id
  const [
    usersResult,
    selfResult,
    agentsResult,
  ] = await Promise.all([
    fetchUserList(),
    fetchUserDetail(selfUserId ? { userId: selfUserId } : {}),
    fetchAgentList(),
  ])
  const viewerType = accountInfo?.user_type || String(selfResult.data?.user_type ?? '')
  const viewerIsAdmin = isPrivilegedUserType(viewerType)
  if (!usersResult.data) throw usersResult.error ?? new Error('user.list failed')
  const userGroups = await loadUserGroups(viewerIsAdmin)

  const targetUserIds = appListTargetUserIds(
    selfUserId,
    viewerType,
    usersResult.data?.users ?? [],
  )
  const appsByUser = new Map<string, NonNullable<Awaited<ReturnType<typeof fetchAppList>>['data']>['apps']>()
  await forEachWithConcurrency(targetUserIds, APP_LIST_CONCURRENCY, async (userId) => {
    const result = await fetchAppList({ userId })
    if (result.data) appsByUser.set(userId, result.data.apps)
  })

  const selfDetail = selfResult.data ?? (accountInfo ? accountDetail(accountInfo) : null)
  const hasCoreData = Boolean(
    selfDetail ||
      usersResult.data ||
      appsByUser.size > 0 ||
      agentsResult.data,
  )
  if (!hasCoreData) {
    throw new Error('users-agents core APIs unavailable')
  }

  const empty = createEmptyUsersAgentsSnapshot()
  const selfApps = selfUserId ? appsByUser.get(selfUserId) ?? [] : []
  const self = toSelfEntity(selfDetail, selfApps, empty.self)
  const now = new Date().toISOString()
  const localUsers = usersResult.data
    ? toVisibleLocalUserEntities(usersResult.data.users, appsByUser, self.id, now)
    : []
  const userType = viewerType || String(selfDetail?.user_type ?? '')

  return {
    self,
    agents: (agentsResult.data?.agents ?? []).map(toAgentEntity),
    localUsers,
    entityGroups: [],
    canCreateAgents: Boolean(userType) && userTypeCanCreateAgents(userType),
    viewerIsAdmin,
    userGroups,
  }
}

/** The mock keeps its seed people and takes the account facts (type, groups, permissions) from the mock control panel. */
function withAccountFacts(user: LocalUserEntity, listed: UserInfo | undefined, now: string): LocalUserEntity {
  if (!listed) return user
  const facts = toLocalUserEntity(listed, [], now)
  return {
    ...user,
    role: facts.role,
    groups: facts.groups,
    passwordChangeAllowed: facts.passwordChangeAllowed,
    passwordChangeOverride: facts.passwordChangeOverride,
    contactEditAllowed: facts.contactEditAllowed,
    contactEditOverride: facts.contactEditOverride,
    canChangePassword: facts.passwordChangeAllowed && user.status === 'active',
  }
}

/** The mock runtime keeps its seed people and reads Agents and its own identities from the mock control panel. */
export async function fetchMockUsersAgentsSnapshot(): Promise<UsersAgentsSnapshot> {
  const [account, selfResult, agentsResult, usersResult] = await Promise.all([
    fetchCurrentAccount(),
    fetchUserDetail(),
    fetchAgentList(),
    fetchUserList(),
  ])
  const viewerIsAdmin = isPrivilegedUserType(account?.user_type)
  if (!usersResult.data) throw usersResult.error ?? new Error('user.list failed')
  const userGroups = await loadUserGroups(viewerIsAdmin)
  const listed = new Map((usersResult.data?.users ?? []).map((user) => [user.user_id, user]))
  const now = new Date().toISOString()
  const base = createMockUsersAgentsSnapshot()
  const contact = (selfResult.data?.local_profile?.private_extra?.system_contact ?? {}) as UserContactSettings
  const profile = (selfResult.data?.profile ?? {}) as Record<string, unknown>
  const avatarUrl = usableImageUrl(profile.avatar)
  return {
    ...base,
    self: {
      ...base.self,
      id: account?.user_id ?? base.self.id,
      socialAccounts: toSocialAccounts(contact.bindings),
      ...(avatarUrl ? { avatarUrl } : {}),
      ...(typeof profile.bio === 'string' ? { bio: profile.bio } : {}),
    },
    agents: (agentsResult.data?.agents ?? []).map(toAgentEntity),
    localUsers: toVisibleLocalUserEntities(usersResult.data.users, new Map(), account?.user_id ?? base.self.id, now)
      .map((user) => {
        const seed = base.localUsers.find((item) => item.id === user.id)
        return seed ? { ...withAccountFacts(seed, listed.get(user.id), now), status: user.status, isOnline: user.isOnline } : user
      }),
    canCreateAgents: userTypeCanCreateAgents(account?.user_type),
    viewerIsAdmin,
    userGroups,
  }
}

export function createEmptyUsersAgentsSnapshot(): UsersAgentsSnapshot {
  const createdAt = '1970-01-01T00:00:00Z'
  const self: UsersAgentsSnapshot['self'] = {
    id: 'self',
    kind: 'self',
    displayName: 'Current User',
    socialAccounts: [],
    createdAt,
    info: {},
    settings: {},
    twoFactorEnabled: false,
    lastLogin: 'Unknown',
  }
  return {
    self,
    agents: [],
    localUsers: [],
    entityGroups: [],
    canCreateAgents: false,
    viewerIsAdmin: false,
    userGroups: null,
  }
}

export function createMockUsersAgentsSnapshot(): UsersAgentsSnapshot {
  return {
    self: structuredClone(mockSelf),
    agents: [],
    localUsers: structuredClone(mockLocalUsers),
    entityGroups: [],
    canCreateAgents: true,
    viewerIsAdmin: true,
    userGroups: null,
  }
}
