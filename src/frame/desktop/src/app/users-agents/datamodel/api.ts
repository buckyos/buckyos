import { fetchAppList } from '../../../api/app_mgr.ts'
import {
  fetchAgentListWithRuntime,
  fetchUserDetail,
  fetchUserList,
} from '../../../api/user_mgr.ts'
import type { UserDetail } from '../../../api/user_mgr.ts'
import { buckyos } from 'buckyos'
import {
  mockAgent,
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
  toVisibleLocalUserEntities,
} from './transforms'

interface AccountInfo {
  user_name: string
  user_id: string
  user_type?: string
}

type UsersAgentsCoreSnapshot = Pick<
  UsersAgentsSnapshot,
  'self' | 'agent' | 'agents' | 'localUsers'
>

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

async function fetchUsersAgentsCoreSnapshot(): Promise<UsersAgentsCoreSnapshot> {
  const accountInfo = await buckyos.getAccountInfo() as AccountInfo | null
  const selfUserId = accountInfo?.user_id
  const [
    usersResult,
    selfResult,
    agentsResult,
  ] = await Promise.all([
    fetchUserList(),
    fetchUserDetail(selfUserId ? { userId: selfUserId } : {}),
    fetchAgentListWithRuntime(),
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

  const agents = agentsResult.data
    ? agentsResult.data.agents.map((agentInfo) => toAgentEntity(agentInfo, empty.agent))
    : []
  const agent = agents[0] ?? empty.agent

  return {
    self,
    agent,
    agents,
    localUsers,
  }
}

export async function fetchUsersAgentsSnapshot(): Promise<UsersAgentsSnapshot> {
  const core = await fetchUsersAgentsCoreSnapshot()
  return {
    ...core,
    entityGroups: [],
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
  const agent: UsersAgentsSnapshot['agent'] = {
    id: 'agent-unavailable',
    kind: 'agent',
    displayName: 'No agent configured',
    agentType: 'agent',
    version: 'unknown',
    status: 'stopped',
    capabilities: [],
    socialAccounts: [],
    info: {},
    settings: {},
    runtime: {
      uptime: 'Offline',
      memoryUsage: 'Unknown',
      cpuUsage: 'Unknown',
      lastActive: 'Unknown',
      runningTasks: 0,
      queuedTasks: 0,
      healthStatus: 'offline',
      uiSessions: 0,
      workSessions: 0,
      workspaces: 0,
    },
    createdAt,
  }
  return {
    self,
    agent,
    agents: [],
    localUsers: [],
    entityGroups: [],
  }
}

export function createMockUsersAgentsSnapshot(): UsersAgentsSnapshot {
  return {
    self: structuredClone(mockSelf),
    agent: structuredClone(mockAgent),
    agents: [structuredClone(mockAgent)],
    localUsers: structuredClone(mockLocalUsers),
    entityGroups: [],
  }
}
