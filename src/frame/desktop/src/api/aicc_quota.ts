// Token quota and resource visibility of AICC (`quota.*`, `visibility.*`); the mock runtime
// answers from MockQuotaWorld.
import { buckyos } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { prepareSessionToken, type AICCMgr } from './aicc_mgr'
import { toAiccRpcCallOptions } from './aicc_rpc_options'
import { fetchCurrentAccount } from './account'
import { fetchUserGroups, fetchUserList } from './user_mgr'
import { MockQuotaError, MockQuotaWorld, type MockAccount, type MockInventory } from '../app/ai-center/mock/quota'
import type {
  QuotaGrantRecord,
  QuotaAccountView,
  QuotaQueryResponse,
  QuotaRechargeRequest,
  QuotaRechargeResponse,
  QuotaRuleDetail,
  QuotaRuleDraft,
  QuotaRulePreview,
  QuotaRuleUpdate,
  QuotaRuleView,
  VisibilityEntry,
  VisibilityPreview,
  VisibilityState,
} from '../app/ai-center/datamodel/quota'

/** A refused or failed quota call; `code` is the AICC wire code when the service sent one. */
export class QuotaApiError extends Error {
  readonly code?: string
  readonly data?: unknown

  constructor(message: string, code?: string, data?: unknown) {
    super(message)
    this.name = 'QuotaApiError'
    this.code = code
    this.data = data
  }
}

export interface QuotaApi {
  query(appId?: string): Promise<QuotaQueryResponse>
  listRules(scope: 'mine' | 'manage'): Promise<QuotaRuleView[]>
  getRule(ruleId: string): Promise<QuotaRuleDetail>
  previewRule(draft: QuotaRuleDraft): Promise<QuotaRulePreview>
  createRule(draft: QuotaRuleDraft): Promise<QuotaRuleView>
  updateRule(update: QuotaRuleUpdate): Promise<QuotaRuleView>
  deleteRule(ruleId: string, expectedVersion: number): Promise<void>
  recharge(request: QuotaRechargeRequest): Promise<QuotaRechargeResponse>
  getRecharge(rechargeId: string): Promise<{ recharge?: QuotaGrantRecord; account?: QuotaAccountView }>
  getVisibility(): Promise<VisibilityState>
  updateVisibility(expectedRevision: number, subject: string, entries: VisibilityEntry[]): Promise<VisibilityState>
  previewVisibility(userId?: string, draft?: { subject: string; entries: VisibilityEntry[] }): Promise<VisibilityPreview>
}

interface RpcClient { call(method: string, params: Record<string, unknown>, options?: { sessionToken: string }): Promise<unknown> }

class RpcQuotaApi implements QuotaApi {
  private client: RpcClient | null = null

  private async call<T>(method: string, params: Record<string, unknown>): Promise<T> {
    this.client ??= buckyos.getServiceRpcClient('aicc') as unknown as RpcClient
    const sessionToken = await prepareSessionToken(true)
    try {
      return await this.client.call(method, params, toAiccRpcCallOptions(sessionToken)) as T
    } catch (error) {
      const e = error as { message?: string; code?: unknown; data?: unknown }
      // Other AICC errors carry their structured JSON body (`AiccError`) after the kRPC prefix.
      const message = typeof e.message === 'string' ? e.message : ''
      const start = message.indexOf('{')
      const end = message.lastIndexOf('}')
      let parsed: { code?: string; message?: string } | undefined
      try { parsed = start >= 0 && end > start ? JSON.parse(message.slice(start, end + 1)) : undefined } catch { parsed = undefined }
      throw new QuotaApiError(parsed?.message ?? e.message ?? String(error), typeof e.code === 'string' ? e.code : parsed?.code, e.data ?? parsed)
    }
  }

  query(appId?: string) { return this.call<QuotaQueryResponse>('quota.query', appId ? { app_id: appId } : {}) }
  async listRules(scope: 'mine' | 'manage') { return (await this.call<{ rules: QuotaRuleView[] }>('quota.rule.list', { scope })).rules }
  getRule(ruleId: string) { return this.call<QuotaRuleDetail>('quota.rule.get', { rule_id: ruleId }) }
  previewRule(draft: QuotaRuleDraft) { return this.call<QuotaRulePreview>('quota.rule.preview', { draft }) }
  createRule(draft: QuotaRuleDraft) { return this.call<QuotaRuleView>('quota.rule.create', { draft }) }
  updateRule(update: QuotaRuleUpdate) { return this.call<QuotaRuleView>('quota.rule.update', { ...update }) }
  async deleteRule(ruleId: string, expectedVersion: number) { await this.call('quota.rule.delete', { rule_id: ruleId, expected_version: expectedVersion }) }
  recharge(request: QuotaRechargeRequest) { return this.call<QuotaRechargeResponse>('quota.account.recharge', { ...request }) }
  getRecharge(rechargeId: string) { return this.call<{ recharge?: QuotaGrantRecord; account?: QuotaAccountView }>('quota.recharge.get', { recharge_id: rechargeId }) }
  getVisibility() { return this.call<VisibilityState>('visibility.get', {}) }
  updateVisibility(expectedRevision: number, subject: string, entries: VisibilityEntry[]) {
    return this.call<VisibilityState>('visibility.update', { expected_revision: expectedRevision, subject, entries })
  }
  previewVisibility(userId?: string, draft?: { subject: string; entries: VisibilityEntry[] }) {
    return this.call<VisibilityPreview>('visibility.preview', { ...(userId ? { user_id: userId } : {}), ...(draft ? { draft } : {}) })
  }
}

let mockWorld: MockQuotaWorld | null = null

class MockQuotaApi implements QuotaApi {
  private viewer: MockAccount = { user_id: 'alice', user_type: 'admin', groups: [] }
  private accounts: MockAccount[] = []
  private groups: string[] = []
  private world: MockQuotaWorld

  constructor(store: AICCMgr) {
    mockWorld ??= new MockQuotaWorld(
      () => this.inventory(store),
      () => this.viewer,
      () => this.accounts,
      () => this.groups,
    )
    this.world = mockWorld
  }

  private inventory(store: AICCMgr): MockInventory[] {
    return store.getSnapshot().providers.map((provider) => ({
      provider_instance_name: provider.config.provider_instance_name,
      enabled: provider.config.enabled,
      exact_models: provider.inventory.models.map((model) => model.exact_model),
    }))
  }

  private async run<T>(work: () => T): Promise<T> {
    const [account, users, groups] = await Promise.all([fetchCurrentAccount().catch(() => null), fetchUserList(), fetchUserGroups()])
    const typeOf = (value: unknown): MockAccount['user_type'] => (value === 'admin' || value === 'root' ? 'admin' : value === 'limited' ? 'limited' : 'user')
    this.accounts = (users.data?.users ?? []).map((user) => ({
      user_id: user.user_id,
      user_type: typeOf(user.user_type),
      groups: (user as { groups?: string[] }).groups ?? [],
    }))
    this.groups = (groups.data?.groups ?? []).map((group) => group.group_id)
    if (account) {
      this.viewer = this.accounts.find((item) => item.user_id === account.user_id) ?? { user_id: account.user_id, user_type: typeOf(account.user_type), groups: [] }
    }
    try {
      return work()
    } catch (error) {
      if (error instanceof MockQuotaError) throw new QuotaApiError(error.message, error.code)
      throw error
    }
  }

  query(appId?: string) { return this.run(() => this.world.query(appId)) }
  listRules(scope: 'mine' | 'manage') { return this.run(() => this.world.listRules(scope)) }
  getRule(ruleId: string) { return this.run(() => this.world.getRule(ruleId)) }
  previewRule(draft: QuotaRuleDraft) { return this.run(() => this.world.preview(draft)) }
  createRule(draft: QuotaRuleDraft) { return this.run(() => this.world.create(draft)) }
  updateRule(update: QuotaRuleUpdate) { return this.run(() => this.world.update(update)) }
  deleteRule(ruleId: string, expectedVersion: number) { return this.run(() => this.world.delete(ruleId, expectedVersion)) }
  recharge(request: QuotaRechargeRequest) { return this.run(() => this.world.recharge(request)) }
  getRecharge(rechargeId: string) { return this.run(() => this.world.rechargeOf(rechargeId)) }
  getVisibility() { return this.run(() => this.world.visibilityState()) }
  updateVisibility(expectedRevision: number, subject: string, entries: VisibilityEntry[]) {
    return this.run(() => this.world.updateVisibility(expectedRevision, subject, entries))
  }
  previewVisibility(userId?: string, draft?: { subject: string; entries: VisibilityEntry[] }) {
    return this.run(() => this.world.previewVisibility(userId, draft))
  }
}

export function createQuotaApi(store: AICCMgr): QuotaApi {
  return isMockRuntime() ? new MockQuotaApi(store) : new RpcQuotaApi()
}

/** A recharge id generated once per dialog: retries and repeated clicks credit once (§9.4). */
export function newRechargeId(): string {
  const random = typeof crypto !== 'undefined' && 'randomUUID' in crypto ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`
  return `rc-${random}`
}

/** Open a new Provider (all models, following its list) to the chosen groups (§3.4 Add Provider). */
export async function openProviderTo(api: QuotaApi, provider: string, subjects: string[]) {
  for (const subject of subjects) {
    const state = await api.getVisibility()
    const entries = state.subjects.find((item) => item.subject === subject)?.entries ?? []
    if (entries.some((entry) => entry.type === 'all_providers' || ('provider_instance_name' in entry && entry.provider_instance_name === provider))) continue
    await api.updateVisibility(state.revision, subject, [...entries, { type: 'provider_all_models', provider_instance_name: provider }])
  }
}
