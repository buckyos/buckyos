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
import { toAgentEntity, toSelfEntity, toSocialAccounts } from './transforms'
import {
  deleteAgent,
  fetchUserDetail,
  removeUserMsgTunnel,
  setUserMsgTunnel,
  setUserProfile,
  updateAgent,
} from '../../../api/user_mgr'
import { isMockRuntime } from '../../../runtime'
import { notifyAgentsChanged, notifyOwnProfileChanged } from '../../agent-setup/events'

const SELF_INFO_FIELDS: Record<string, keyof Parameters<typeof setUserProfile>[0]> = {
  displayName: 'displayName',
  title: 'title',
  bio: 'bio',
  location: 'location',
  organization: 'organization',
  website: 'website',
  email: 'email',
  phone: 'phone',
}


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

  // ── The signed-in user's own profile (`user.profile.set`, then read back) ──

  /** `null` removes the local avatar so the DID document's one, if any, shows again. */
  updateSelfAvatar(avatar: string | null): Promise<unknown> {
    return this.saveSelfProfile({ avatar })
  }

  updateSelfBio(bio: string): Promise<unknown> {
    return this.saveSelfProfile({ bio: bio.trim() || null })
  }

  /** Saves one of SELF_EDITABLE_INFO_KEYS; an empty value clears it. */
  updateSelfInfo(key: string, value: string): Promise<unknown> {
    const field = SELF_INFO_FIELDS[key]
    if (!field) return Promise.resolve(new Error(`${key} cannot be edited`))
    return this.saveSelfProfile({ [field]: value.trim() || null })
  }

  /** Resolves with an error, or null once the saved profile has been read back and shown. */
  private async saveSelfProfile(patch: Parameters<typeof setUserProfile>[0]): Promise<unknown> {
    const { data, error } = await setUserProfile(patch)
    if (!data) return error ?? new Error('user.profile.set failed')
    const { data: detail } = await fetchUserDetail(this.self.id === 'self' ? {} : { userId: this.self.id })
    if (detail) {
      // Only identity and account settings carry over; profile fields come from the read-back alone.
      const base = { ...this.self, info: {}, bio: undefined, avatarUrl: undefined, email: undefined, phone: undefined }
      const next = toSelfEntity(detail, [], base)
      this.self = this.useMock ? { ...next, settings: this.self.settings } : next
      this.notify()
    }
    notifyOwnProfileChanged()
    return null
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
