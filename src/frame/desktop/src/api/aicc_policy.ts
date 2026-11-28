// AICC user policies, Credit price list and Credit accounts (`policy.*`, `price.*`, `credit.*`); the mock
// runtime answers from MockPolicyWorld.
import { buckyos } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { prepareSessionToken, type AICCMgr } from './aicc_mgr'
import { toAiccRpcCallOptions } from './aicc_rpc_options'
import { fetchCurrentAccount } from './account'
import { fetchUserGroups, fetchUserList } from './user_mgr'
import { MockPolicyError, MockPolicyWorld, type MockAccount, type MockInventoryModel } from '../app/ai-center/mock/policy'
import type {
  AiccPolicyView,
  CreditAccount,
  CreditEntry,
  CreditEntryResponse,
  CreditLedgerListResponse,
  PolicyDraft,
  PolicyEffectiveResponse,
  PolicyGetResponse,
  PolicyStatusResponse,
  PriceGetResponse,
  PriceUpdateRequest,
} from '../app/ai-center/datamodel/policy'

/** A refused or failed policy call; `code` is the AICC wire code when the service sent one. */
export class PolicyApiError extends Error {
  readonly code?: string
  readonly data?: unknown

  constructor(message: string, code?: string, data?: unknown) {
    super(message)
    this.name = 'PolicyApiError'
    this.code = code
    this.data = data
  }
}

export interface EffectiveQuery { userId?: string; appId?: string; draft?: PolicyDraft }

export interface PolicyApi {
  listPolicies(scope: 'mine' | 'manage'): Promise<AiccPolicyView[]>
  getPolicy(policyId: string): Promise<PolicyGetResponse>
  upsertPolicy(draft: PolicyDraft, expectedVersion?: number): Promise<AiccPolicyView>
  deletePolicy(policyId: string, expectedVersion: number): Promise<void>
  effective(query?: EffectiveQuery): Promise<PolicyEffectiveResponse>
  status(appId?: string): Promise<PolicyStatusResponse>
  getPrices(): Promise<PriceGetResponse>
  updatePrices(request: PriceUpdateRequest): Promise<PriceGetResponse>
  getAccount(userId?: string): Promise<CreditAccount>
  listAccounts(group?: string): Promise<CreditAccount[]>
  ledger(userId?: string, cursor?: string, limit?: number): Promise<CreditLedgerListResponse>
  recharge(request: { user_id: string; recharge_id: string; amount: string; note?: string }): Promise<CreditEntryResponse>
  getRecharge(rechargeId: string): Promise<{ entry?: CreditEntry; account?: CreditAccount }>
  adjust(request: { user_id: string; adjust_id: string; amount: string; note: string }): Promise<CreditEntryResponse>
}

interface RpcClient { call(method: string, params: Record<string, unknown>, options?: { sessionToken: string }): Promise<unknown> }

class RpcPolicyApi implements PolicyApi {
  private client: RpcClient | null = null

  private async call<T>(method: string, params: Record<string, unknown>): Promise<T> {
    this.client ??= buckyos.getServiceRpcClient('aicc') as unknown as RpcClient
    const sessionToken = await prepareSessionToken(true)
    try {
      return await this.client.call(method, params, toAiccRpcCallOptions(sessionToken)) as T
    } catch (error) {
      const e = error as { message?: string; code?: unknown; data?: unknown }
      // AICC errors carry their structured JSON body (`AiccError`) after the kRPC prefix.
      const message = typeof e.message === 'string' ? e.message : ''
      const start = message.indexOf('{')
      const end = message.lastIndexOf('}')
      let parsed: { code?: string; message?: string } | undefined
      try { parsed = start >= 0 && end > start ? JSON.parse(message.slice(start, end + 1)) : undefined } catch { parsed = undefined }
      throw new PolicyApiError(parsed?.message ?? e.message ?? String(error), typeof e.code === 'string' ? e.code : parsed?.code, e.data ?? parsed)
    }
  }

  async listPolicies(scope: 'mine' | 'manage') { return (await this.call<{ policies: AiccPolicyView[] }>('policy.list', { scope })).policies }
  getPolicy(policyId: string) { return this.call<PolicyGetResponse>('policy.get', { policy_id: policyId }) }
  upsertPolicy(draft: PolicyDraft, expectedVersion?: number) {
    return this.call<AiccPolicyView>('policy.upsert', { draft, ...(expectedVersion !== undefined ? { expected_version: expectedVersion } : {}) })
  }
  async deletePolicy(policyId: string, expectedVersion: number) { await this.call('policy.delete', { policy_id: policyId, expected_version: expectedVersion }) }
  effective(query: EffectiveQuery = {}) {
    return this.call<PolicyEffectiveResponse>('policy.effective', {
      ...(query.userId ? { user_id: query.userId } : {}),
      ...(query.appId ? { app_id: query.appId } : {}),
      ...(query.draft ? { draft: query.draft } : {}),
    })
  }
  status(appId?: string) { return this.call<PolicyStatusResponse>('policy.status', appId ? { app_id: appId } : {}) }
  getPrices() { return this.call<PriceGetResponse>('price.get', {}) }
  updatePrices(request: PriceUpdateRequest) { return this.call<PriceGetResponse>('price.update', { ...request }) }
  getAccount(userId?: string) { return this.call<CreditAccount>('credit.account.get', userId ? { user_id: userId } : {}) }
  async listAccounts(group?: string) { return (await this.call<{ accounts: CreditAccount[] }>('credit.account.list', group ? { group } : {})).accounts }
  ledger(userId?: string, cursor?: string, limit?: number) {
    return this.call<CreditLedgerListResponse>('credit.ledger.list', { ...(userId ? { user_id: userId } : {}), ...(cursor ? { cursor } : {}), ...(limit ? { limit } : {}) })
  }
  recharge(request: { user_id: string; recharge_id: string; amount: string; note?: string }) { return this.call<CreditEntryResponse>('credit.recharge', { ...request }) }
  getRecharge(rechargeId: string) { return this.call<{ entry?: CreditEntry; account?: CreditAccount }>('credit.recharge.get', { recharge_id: rechargeId }) }
  adjust(request: { user_id: string; adjust_id: string; amount: string; note: string }) { return this.call<CreditEntryResponse>('credit.adjust', { ...request }) }
}

// One mock world per page, with the identity and inventory of the latest call: every MockPolicyApi (one per
// component) refreshes them before it calls into the world.
let mockWorld: MockPolicyWorld | null = null
const mockState: { viewer: MockAccount; accounts: MockAccount[]; groups: string[]; store: AICCMgr | null } = {
  viewer: { user_id: 'alice', user_type: 'admin', groups: [] },
  accounts: [],
  groups: [],
  store: null,
}

/** Model Driver identities of the mock catalog (mock/model-catalog.ts `definitions`), by Provider profile. Kept
 * here so the policy mock never calls the catalog mock, which e2e tests stub with call counting. */
const MOCK_DRIVER_MODELS: Record<string, string[]> = {
  openai: ['gpt-5.1', 'gpt-5.1-mini', 'gpt-5.6'],
  claude: ['claude-sonnet-4.5', 'claude-opus-4.7'],
  qwen: ['qwen2.5-coder-32b', 'qwen3.5-27b'],
}

const TOKEN_METERED = (apiType: string) => apiType.startsWith('llm') || apiType.startsWith('embedding') || apiType === 'decision'

class MockPolicyApi implements PolicyApi {
  private readonly store: AICCMgr
  private world: MockPolicyWorld

  constructor(store: AICCMgr) {
    this.store = store
    mockWorld ??= new MockPolicyWorld(
      () => (mockState.store ? this.inventory(mockState.store) : []),
      () => mockState.viewer,
      () => mockState.accounts,
      () => mockState.groups,
      (userId) => (mockState.store ? this.appsOf(mockState.store, userId) : ['aiworkspace']),
    )
    this.world = mockWorld
  }

  private inventory(store: AICCMgr): MockInventoryModel[] {
    const snapshot = store.getSnapshot()
    return snapshot.providers.filter((provider) => provider.config.enabled).flatMap((provider) => provider.inventory.models.map((model) => {
      const driver = provider.config.provider_profile_id
      const mapped = MOCK_DRIVER_MODELS[driver]?.includes(model.provider_model_id) ?? false
      return {
        exact_model: model.exact_model,
        provider_instance_name: provider.config.provider_instance_name,
        provider_model_id: model.provider_model_id,
        variant: model.variant,
        model_driver_id: mapped ? driver : '',
        origin_model_id: mapped ? model.provider_model_id : '',
        token_metered: model.api_types.some(TOKEN_METERED),
        provider_cost_currency: model.pricing.input_token_usd !== undefined ? 'USD' : undefined,
      }
    }))
  }

  private appsOf(store: AICCMgr, userId: string): string[] {
    const events = store.getSnapshot().usageEvents.filter((event) => (event.user_id ?? event.tenant_id) === userId)
    return [...new Set(['aiworkspace', ...events.flatMap((event) => [event.app_id, event.agent_id].filter((item): item is string => !!item))])]
  }

  private async run<T>(work: () => T): Promise<T> {
    const [account, users, groups] = await Promise.all([fetchCurrentAccount().catch(() => null), fetchUserList(), fetchUserGroups()])
    const typeOf = (value: unknown): MockAccount['user_type'] => (value === 'admin' || value === 'root' ? 'admin' : value === 'limited' ? 'limited' : 'user')
    mockState.store = this.store
    mockState.accounts = (users.data?.users ?? []).map((user) => ({
      user_id: user.user_id,
      user_type: typeOf(user.user_type),
      groups: (user as { groups?: string[] }).groups ?? [],
    }))
    mockState.groups = (groups.data?.groups ?? []).map((group) => group.group_id)
    if (account) {
      mockState.viewer = mockState.accounts.find((item) => item.user_id === account.user_id) ?? { user_id: account.user_id, user_type: typeOf(account.user_type), groups: [] }
    }
    try {
      return structuredClone(work())
    } catch (error) {
      if (error instanceof MockPolicyError) throw new PolicyApiError(error.message, error.code)
      throw error
    }
  }

  listPolicies(scope: 'mine' | 'manage') { return this.run(() => this.world.listPolicies(scope)) }
  getPolicy(policyId: string) { return this.run(() => this.world.getPolicy(policyId)) }
  upsertPolicy(draft: PolicyDraft, expectedVersion?: number) { return this.run(() => this.world.upsertPolicy(draft, expectedVersion)) }
  deletePolicy(policyId: string, expectedVersion: number) { return this.run(() => this.world.deletePolicy(policyId, expectedVersion)) }
  effective(query: EffectiveQuery = {}) { return this.run(() => this.world.effective({ user_id: query.userId, app_id: query.appId, draft: query.draft })) }
  status(appId?: string) { return this.run(() => this.world.status(appId)) }
  getPrices() { return this.run(() => this.world.getPrices()) }
  updatePrices(request: PriceUpdateRequest) { return this.run(() => this.world.updatePrices(request)) }
  getAccount(userId?: string) { return this.run(() => this.world.accountGet(userId)) }
  listAccounts(group?: string) { return this.run(() => this.world.accountList(group)) }
  ledger(userId?: string, cursor?: string, limit?: number) { return this.run(() => this.world.ledger({ user_id: userId, cursor, limit })) }
  recharge(request: { user_id: string; recharge_id: string; amount: string; note?: string }) { return this.run(() => this.world.recharge(request)) }
  getRecharge(rechargeId: string) { return this.run(() => this.world.rechargeGet(rechargeId)) }
  adjust(request: { user_id: string; adjust_id: string; amount: string; note: string }) { return this.run(() => this.world.adjust(request)) }
}

export function createPolicyApi(store: AICCMgr): PolicyApi {
  return isMockRuntime() ? new MockPolicyApi(store) : new RpcPolicyApi()
}
