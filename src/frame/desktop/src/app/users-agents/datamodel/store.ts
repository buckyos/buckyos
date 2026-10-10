import type {
  AgentEntity,
  AnyEntity,
  EntityGroupEntity,
  LocalUserEntity,
  SelfEntity,
  SocialAccount,
  UsersAgentsSnapshot,
} from './types'
import {
  createEmptyUsersAgentsSnapshot,
  createMockUsersAgentsSnapshot,
  fetchMockUsersAgentsSnapshot,
  fetchUsersAgentsSnapshot,
} from './api'
import { toAgentEntity, toSocialAccounts } from './transforms'
import {
  deleteAgent,
  removeUserMsgTunnel,
  setUserMsgTunnel,
  updateAgent,
} from '../../../api/user_mgr'
import { isMockRuntime } from '../../../runtime'
import { notifyAgentsChanged, notifyOwnProfileChanged } from '../../agent-setup/events'

export interface UsersAgentsStoreOptions {
  useMock?: boolean
}

const mockModeValues = new Set(['1', 'true', 'yes', 'mock'])

function defaultUseMock(): boolean {
  const value = import.meta.env.VITE_USERS_AGENTS_USE_MOCK
  if (value !== undefined) {
    return mockModeValues.has(String(value).toLowerCase())
  }
  return isMockRuntime()
}

export class UsersAgentsStore {
  private readonly useMock: boolean

  private self: SelfEntity
  private agents: AgentEntity[]
  private localUsers: LocalUserEntity[]
  private entityGroups: EntityGroupEntity[]
  private canCreateAgents: boolean

  private snapshot: UsersAgentsSnapshot
  private listeners = new Set<() => void>()
  private reloadSeq = 0

  constructor(options: UsersAgentsStoreOptions = {}) {
    this.useMock = options.useMock ?? defaultUseMock()
    const initial = this.useMock
      ? createMockUsersAgentsSnapshot()
      : createEmptyUsersAgentsSnapshot()
    this.self = initial.self
    this.agents = initial.agents
    this.localUsers = initial.localUsers
    this.entityGroups = initial.entityGroups
    this.canCreateAgents = initial.canCreateAgents
    this.snapshot = this.buildSnapshot()

    void this.reload().catch((error) => {
      console.warn('Failed to load users-agents datamodel.', error)
    })
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = (): UsersAgentsSnapshot => this.snapshot

  async reload(): Promise<UsersAgentsSnapshot> {
    const seq = ++this.reloadSeq
    const snapshot = this.useMock ? await fetchMockUsersAgentsSnapshot() : await fetchUsersAgentsSnapshot()
    // A newer reload has started meanwhile; keep its result instead of this stale one.
    if (seq === this.reloadSeq) {
      this.applySnapshot(snapshot)
      this.notify()
    }
    return snapshot
  }

  private applySnapshot(snapshot: UsersAgentsSnapshot) {
    this.self = snapshot.self
    this.agents = snapshot.agents
    this.localUsers = snapshot.localUsers
    this.entityGroups = snapshot.entityGroups
    this.canCreateAgents = snapshot.canCreateAgents
  }

  private notify() {
    this.snapshot = this.buildSnapshot()
    this.listeners.forEach((listener) => listener())
  }

  private buildSnapshot(): UsersAgentsSnapshot {
    return {
      self: this.self,
      agents: this.agents,
      localUsers: this.localUsers,
      entityGroups: this.entityGroups,
      canCreateAgents: this.canCreateAgents,
    }
  }

  findEntity(id: string): AnyEntity | undefined {
    if (this.self.id === id) return this.self
    const agent = this.agents.find((item) => item.id === id)
    if (agent) return agent
    return (
      this.localUsers.find((user) => user.id === id) ??
      this.entityGroups.find((group) => group.id === id)
    )
  }

  removeLocalUser(id: string) {
    this.localUsers = this.localUsers.filter((user) => user.id !== id)
    this.notify()
  }

  updateSelfAvatar(avatarUrl?: string) {
    this.self = { ...this.self, avatarUrl }
    this.notify()
  }

  updateSelfBio(bio: string) {
    this.self = {
      ...this.self,
      bio,
      info: {
        ...this.self.info,
        bio,
      },
    }
    this.notify()
  }

  updateSelfInfo(key: string, value: string) {
    this.self = {
      ...this.self,
      displayName: key === 'displayName' ? value : this.self.displayName,
      bio: key === 'bio' ? value : this.self.bio,
      info: {
        ...this.self.info,
        [key]: value,
      },
    }
    this.notify()
  }

  // ── Agents ──

  /** `agent.update`; resolves with an error message, or null on success. */
  async setAgentAllowGroup(agentId: string, allowGroup: boolean): Promise<unknown> {
    const { data, error } = await updateAgent({ agentId, allowGroup })
    if (!data) return error ?? new Error('agent.update failed')
    this.agents = this.agents.map((agent) => (agent.id === agentId ? toAgentEntity(data) : agent))
    this.notify()
    notifyAgentsChanged()
    return null
  }

  async deleteAgent(agentId: string): Promise<unknown> {
    const { data, error } = await deleteAgent(agentId)
    if (!data) return error ?? new Error('agent.delete failed')
    this.agents = this.agents.filter((agent) => agent.id !== agentId)
    this.notify()
    notifyAgentsChanged()
    return null
  }

  // ── The signed-in user's own message identities (Telegram is stored by the control panel) ──

  async addOwnTelegram(accountId: string): Promise<unknown> {
    const { data, error } = await setUserMsgTunnel({ platform: 'telegram', accountId: accountId.trim() })
    if (!data) return error ?? new Error('user.set_msg_tunnel failed')
    this.self = { ...this.self, socialAccounts: toSocialAccounts(data.contact?.bindings) }
    this.notify()
    notifyOwnProfileChanged()
    return null
  }

  async removeOwnTelegram(): Promise<unknown> {
    const { data, error } = await removeUserMsgTunnel({ platform: 'telegram' })
    if (!data) return error ?? new Error('user.remove_msg_tunnel failed')
    this.self = { ...this.self, socialAccounts: toSocialAccounts(data.contact?.bindings) }
    this.notify()
    notifyOwnProfileChanged()
    return null
  }

  addSocialAccount(entityId: string, account: SocialAccount) {
    this.updateSocialAccounts(entityId, (accounts) => [...accounts, account])
  }

  removeSocialAccount(entityId: string, accountId: string) {
    this.updateSocialAccounts(entityId, (accounts) =>
      accounts.filter((account) => account.id !== accountId),
    )
  }

  toggleSocialAccountVisibility(entityId: string, accountId: string) {
    this.updateSocialAccounts(entityId, (accounts) =>
      accounts.map((account) =>
        account.id === accountId
          ? { ...account, isPublic: !account.isPublic }
          : account,
      ),
    )
  }

  private updateSocialAccounts(
    entityId: string,
    updater: (accounts: SocialAccount[]) => SocialAccount[],
  ) {
    if (this.self.id === entityId) {
      this.self = { ...this.self, socialAccounts: updater(this.self.socialAccounts) }
      this.notify()
      return
    }

    this.localUsers = this.localUsers.map((user) =>
      user.id === entityId
        ? { ...user, socialAccounts: updater(user.socialAccounts) }
        : user,
    )
    this.entityGroups = this.entityGroups.map((group) =>
      group.id === entityId
        ? { ...group, socialAccounts: updater(group.socialAccounts) }
        : group,
    )
    this.notify()
  }
}
