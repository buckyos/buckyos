import { fetchAppList } from '../../../api/app_mgr.ts'
import { mockSelfHostGroups } from '../../../api/self_host_groups.ts'
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
  toAgentEntity,
  toEntityGroupEntity,
  toLocalUserEntity,
  toSelfEntity,
} from './transforms'

type GroupListByMemberResponse = unknown[]

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

async function fetchGroupListByMember(memberDid?: string): Promise<{
  data: GroupListByMemberResponse | null
  error: unknown
}> {
  if (!memberDid) {
    return { data: [], error: null }
  }

  try {
    const rpcClient = buckyos.getServiceRpcClient('msg-center')
    const result = await rpcClient.call('group.list_by_member', {
      member_did: memberDid,
      host_owner: memberDid,
    })
    return { data: Array.isArray(result) ? result : [], error: null }
  } catch (error) {
    console.warn('Failed to load users-agents groups from msg-center.', error)
    return { data: null, error }
  }
}

async function fetchUsersAgentsCoreSnapshot(): Promise<UsersAgentsCoreSnapshot> {
  const accountInfo = await buckyos.getAccountInfo() as AccountInfo | null
  const selfUserId = accountInfo?.user_id
  const [
    usersResult,
    selfResult,
    appsResult,
    agentsResult,
  ] = await Promise.all([
    fetchUserList(),
    fetchUserDetail(selfUserId ? { userId: selfUserId } : {}),
    fetchAppList(selfUserId ? { userId: selfUserId } : {}),
    fetchAgentListWithRuntime(),
  ])

  const selfDetail = selfResult.data ?? (accountInfo ? accountDetail(accountInfo) : null)
  const hasCoreData = Boolean(
    selfDetail ||
      usersResult.data ||
      appsResult.data ||
      agentsResult.data,
  )
  if (!hasCoreData) {
    throw new Error('users-agents core APIs unavailable')
  }

  const empty = createEmptyUsersAgentsSnapshot()
  const apps = appsResult.data?.apps ?? []
  const self = toSelfEntity(selfDetail, apps, empty.self)
  const now = new Date().toISOString()
  const localUsers = usersResult.data
    ? usersResult.data.users
        .filter((user) => user.user_id !== self.id)
        .map((user) => toLocalUserEntity(user, apps, now))
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
  const groupsResult = await fetchGroupListByMember(core.self.did)
  const entityGroups = (groupsResult.data ?? [])
    .map(toEntityGroupEntity)
    .filter((group) => group.isHostedBySelf)

  return {
    ...core,
    entityGroups,
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
  const entityGroups = mockSelfHostGroups
    .map(toEntityGroupEntity)
    .filter((group) => group.isHostedBySelf)

  return {
    self: structuredClone(mockSelf),
    agent: structuredClone(mockAgent),
    agents: [structuredClone(mockAgent)],
    localUsers: structuredClone(mockLocalUsers),
    entityGroups,
  }
}
