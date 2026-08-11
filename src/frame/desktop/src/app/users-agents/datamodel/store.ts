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
  fetchUsersAgentsSnapshot,
} from './api'
import { isMockRuntime } from '../../../runtime'

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
  private agent: AgentEntity
  private agents: AgentEntity[]
  private localUsers: LocalUserEntity[]
  private entityGroups: EntityGroupEntity[]

  private snapshot: UsersAgentsSnapshot
  private listeners = new Set<() => void>()

  constructor(options: UsersAgentsStoreOptions = {}) {
    this.useMock = options.useMock ?? defaultUseMock()
    const initial = this.useMock
      ? createMockUsersAgentsSnapshot()
      : createEmptyUsersAgentsSnapshot()
    this.self = initial.self
    this.agent = initial.agent
    this.agents = initial.agents
    this.localUsers = initial.localUsers
    this.entityGroups = initial.entityGroups
    this.snapshot = this.buildSnapshot()

    if (!this.useMock) {
      void this.reload()
    }
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = (): UsersAgentsSnapshot => this.snapshot

  async reload() {
    if (this.useMock) {
      this.applySnapshot(createMockUsersAgentsSnapshot())
      this.notify()
      return
    }

    try {
      this.applySnapshot(await fetchUsersAgentsSnapshot())
      this.notify()
    } catch (error) {
      console.warn('Failed to load users-agents datamodel from backend.', error)
    }
  }

  private applySnapshot(snapshot: UsersAgentsSnapshot) {
    this.self = snapshot.self
    this.agent = snapshot.agent
    this.agents = snapshot.agents
    this.localUsers = snapshot.localUsers
    this.entityGroups = snapshot.entityGroups
  }

  private notify() {
    this.snapshot = this.buildSnapshot()
    this.listeners.forEach((listener) => listener())
  }

  private buildSnapshot(): UsersAgentsSnapshot {
    return {
      self: this.self,
      agent: this.agent,
      agents: this.agents,
      localUsers: this.localUsers,
      entityGroups: this.entityGroups,
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

  addLocalUser(user: LocalUserEntity) {
    this.localUsers = [...this.localUsers, user]
    this.notify()
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

  addSocialAccount(entityId: string, account: SocialAccount) {
    if (this.self.id === entityId) {
      this.self = { ...this.self, socialAccounts: [...this.self.socialAccounts, account] }
      this.notify()
      return
    }

    const targetAgent = this.agents.find((agent) => agent.id === entityId)
    if (targetAgent) {
      this.agents = this.agents.map((agent) =>
        agent.id === entityId
          ? { ...agent, socialAccounts: [...agent.socialAccounts, account] }
          : agent,
      )
      this.agent = this.agents[0] ?? this.agent
      this.notify()
      return
    }

    this.localUsers = this.localUsers.map((user) =>
      user.id === entityId
        ? { ...user, socialAccounts: [...user.socialAccounts, account] }
        : user,
    )
    this.entityGroups = this.entityGroups.map((group) =>
      group.id === entityId
        ? { ...group, socialAccounts: [...group.socialAccounts, account] }
        : group,
    )
    this.notify()
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

    const targetAgent = this.agents.find((agent) => agent.id === entityId)
    if (targetAgent) {
      this.agents = this.agents.map((agent) =>
        agent.id === entityId
          ? { ...agent, socialAccounts: updater(agent.socialAccounts) }
          : agent,
      )
      this.agent = this.agents[0] ?? this.agent
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
