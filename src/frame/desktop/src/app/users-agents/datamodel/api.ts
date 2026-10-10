import { fetchCurrentAccount, isLimitedUserType } from '../../../api/account.ts'
import { fetchAppList } from '../../../api/app_mgr.ts'
import {
  fetchAgentList,
  fetchUserDetail,
  fetchUserList,
} from '../../../api/user_mgr.ts'
import type { UserContactSettings, UserDetail } from '../../../api/user_mgr.ts'
import { buckyos } from 'buckyos'
import {
  mockLocalUsers,
  mockSelf,
} from '../mock/seed'
import type {
  UsersAgentsSnapshot,
} from './types'
import {
  appListTargetUserIds,
  toAgentEntity,
  toSelfEntity,
  toSocialAccounts,
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

  const targetUserIds = appListTargetUserIds(
    selfUserId,
    accountInfo?.user_type,
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
  const userType = accountInfo?.user_type || String(selfDetail?.user_type ?? '')

  return {
    self,
    agents: (agentsResult.data?.agents ?? []).map(toAgentEntity),
    localUsers,
    entityGroups: [],
    canCreateAgents: Boolean(userType) && !isLimitedUserType(userType),
  }
}

/** The mock runtime keeps its seed people and reads Agents and its own identities from the mock control panel. */
export async function fetchMockUsersAgentsSnapshot(): Promise<UsersAgentsSnapshot> {
  const [account, selfResult, agentsResult] = await Promise.all([
    fetchCurrentAccount(),
    fetchUserDetail(),
    fetchAgentList(),
  ])
  const base = createMockUsersAgentsSnapshot()
  const contact = (selfResult.data?.local_profile?.private_extra?.system_contact ?? {}) as UserContactSettings
  return {
    ...base,
    self: {
      ...base.self,
      id: account?.user_id ?? base.self.id,
      socialAccounts: toSocialAccounts(contact.bindings),
    },
    agents: (agentsResult.data?.agents ?? []).map(toAgentEntity),
    canCreateAgents: !isLimitedUserType(account?.user_type),
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
  }
}

export function createMockUsersAgentsSnapshot(): UsersAgentsSnapshot {
  return {
    self: structuredClone(mockSelf),
    agents: [],
    localUsers: structuredClone(mockLocalUsers),
    entityGroups: [],
    canCreateAgents: true,
  }
}
