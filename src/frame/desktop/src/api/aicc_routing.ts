// The usecase table, routing groups and App bindings (`routing.usecases`, `routing.usecase.set`, `routing.groups.*`,
// `routing.bindings.*`); the mock runtime answers from MockUsecaseWorld.
import { buckyos } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { isSettingsRevisionConflict, prepareSessionToken, type AICCMgr } from './aicc_mgr'
import { toAiccRpcCallOptions } from './aicc_rpc_options'
import { PolicyApiError, toPolicyApiError } from './aicc_policy'
import { fetchCurrentAccount } from './account'
import { isPrivilegedUserType } from './user_type'
import { fetchAppList } from './app_mgr'
import { fetchAgentList } from './user_mgr'
import { MockRoutingError, mockUsecaseWorld, type MockRoutingViewer } from '../app/ai-center/mock/usecases'
import type {
  RoutingApp,
  RoutingBinding,
  RoutingGroup,
  RoutingUsecasesResponse,
  UsecaseRoutes,
  UsecaseSetRequest,
} from '../app/ai-center/datamodel/usecases'
import type { StoreSnapshot } from '../app/ai-center/mock/types'

export interface UsecasesQuery {
  /** `self` (default), `zone`, `group:<id>`, `app:<app_id>`. */
  view?: string
  /** Administrators: the table as if this Provider were deleted or disabled. */
  withoutProvider?: string
}

export interface GroupUpdate { group_id: string; version: number; name?: string; usecases?: UsecaseRoutes }

export interface RoutingApi {
  usecases(query?: UsecasesQuery): Promise<RoutingUsecasesResponse>
  setUsecase(request: UsecaseSetRequest): Promise<{ ok: boolean; revision: number }>
  listGroups(): Promise<RoutingGroup[]>
  createGroup(name: string, usecases?: UsecaseRoutes): Promise<RoutingGroup>
  updateGroup(request: GroupUpdate): Promise<RoutingGroup>
  deleteGroup(groupId: string, version: number): Promise<void>
  listBindings(): Promise<RoutingBinding[]>
  bind(appId: string, groupId: string): Promise<void>
  unbind(appId: string): Promise<void>
  /** Apps and Agents a group can be bound to. */
  listApps(): Promise<RoutingApp[]>
}

/** Someone else changed the layer or group first; reload and try again. */
export function isRevisionConflict(error: unknown): boolean {
  return (error instanceof PolicyApiError && error.code === 'settings_revision_conflict') || isSettingsRevisionConflict(error)
}

function sortApps(apps: RoutingApp[]): RoutingApp[] {
  const unique = new Map<string, RoutingApp>()
  for (const app of apps) if (app.app_id && !unique.has(app.app_id)) unique.set(app.app_id, app)
  return [...unique.values()].sort((left, right) => left.label.localeCompare(right.label))
}

async function agentApps(): Promise<RoutingApp[]> {
  const { data } = await fetchAgentList()
  return (data?.agents ?? []).map((agent) => ({ app_id: agent.agent_id, label: agent.display_name || agent.name || agent.agent_id, agent: true }))
}

interface RpcClient { call(method: string, params: Record<string, unknown>, options?: { sessionToken: string }): Promise<unknown> }

class RpcRoutingApi implements RoutingApi {
  private client: RpcClient | null = null

  private async call<T>(method: string, params: Record<string, unknown>): Promise<T> {
    this.client ??= buckyos.getServiceRpcClient('aicc') as unknown as RpcClient
    const sessionToken = await prepareSessionToken(true)
    try {
      return await this.client.call(method, params, toAiccRpcCallOptions(sessionToken)) as T
    } catch (error) {
      throw toPolicyApiError(error)
    }
  }

  usecases(query: UsecasesQuery = {}) {
    return this.call<RoutingUsecasesResponse>('routing.usecases', {
      ...(query.view ? { view: query.view } : {}),
      ...(query.withoutProvider ? { without_provider: query.withoutProvider } : {}),
    })
  }
  setUsecase(request: UsecaseSetRequest) { return this.call<{ ok: boolean; revision: number }>('routing.usecase.set', { ...request }) }
  async listGroups() { return (await this.call<{ groups: RoutingGroup[] }>('routing.groups.list', {})).groups }
  async createGroup(name: string, usecases?: UsecaseRoutes) {
    return (await this.call<{ group: RoutingGroup }>('routing.groups.create', { name, ...(usecases ? { usecases } : {}) })).group
  }
  async updateGroup(request: GroupUpdate) { return (await this.call<{ group: RoutingGroup }>('routing.groups.update', { ...request })).group }
  async deleteGroup(groupId: string, version: number) { await this.call('routing.groups.delete', { group_id: groupId, version }) }
  async listBindings() { return (await this.call<{ bindings: RoutingBinding[] }>('routing.bindings.list', {})).bindings }
  async bind(appId: string, groupId: string) { await this.call('routing.bindings.set', { app_id: appId, group_id: groupId }) }
  async unbind(appId: string) { await this.call('routing.bindings.remove', { app_id: appId }) }
  async listApps() {
    const [{ data }, agents] = await Promise.all([fetchAppList(), agentApps()])
    const apps = (data?.apps ?? [])
      .filter((app) => app.runtime_type !== 'agent')
      .map((app) => ({ app_id: String(app.app_id), label: app.show_name || app.name || String(app.app_id), agent: false }))
    return sortApps([...apps, ...agents])
  }
}

const MOCK_APPS = ['aiworkspace', 'messagehub', 'studio', 'codeassistant']

class MockRoutingApi implements RoutingApi {
  private readonly store: AICCMgr

  constructor(store: AICCMgr) {
    this.store = store
  }

  private async run<T>(work: (snapshot: StoreSnapshot, viewer: MockRoutingViewer) => T): Promise<T> {
    const account = await fetchCurrentAccount().catch(() => null)
    const viewer = { user_id: account?.user_id ?? 'alice', admin: account ? isPrivilegedUserType(account.user_type) : true }
    try {
      return structuredClone(work(this.store.getSnapshot(), viewer))
    } catch (error) {
      if (error instanceof MockRoutingError) throw new PolicyApiError(error.message, error.code)
      throw error
    }
  }

  usecases(query: UsecasesQuery = {}) {
    return this.run((snapshot, viewer) => mockUsecaseWorld().table(snapshot, viewer, { view: query.view, without_provider: query.withoutProvider }))
  }
  setUsecase(request: UsecaseSetRequest) { return this.run((snapshot, viewer) => mockUsecaseWorld().set(snapshot, viewer, request)) }
  listGroups() { return this.run((_, viewer) => mockUsecaseWorld().listGroups(viewer)) }
  createGroup(name: string, usecases?: UsecaseRoutes) { return this.run((snapshot, viewer) => mockUsecaseWorld().createGroup(snapshot, viewer, { name, usecases })) }
  updateGroup(request: GroupUpdate) { return this.run((snapshot, viewer) => mockUsecaseWorld().updateGroup(snapshot, viewer, request)) }
  async deleteGroup(groupId: string, version: number) { await this.run((_, viewer) => mockUsecaseWorld().deleteGroup(viewer, { group_id: groupId, version })) }
  listBindings() { return this.run((_, viewer) => mockUsecaseWorld().listBindings(viewer)) }
  async bind(appId: string, groupId: string) { await this.run((_, viewer) => mockUsecaseWorld().setBinding(viewer, { app_id: appId, group_id: groupId })) }
  async unbind(appId: string) { await this.run((_, viewer) => mockUsecaseWorld().removeBinding(viewer, { app_id: appId })) }
  async listApps() {
    return sortApps([...MOCK_APPS.map((app) => ({ app_id: app, label: app, agent: false })), ...await agentApps()])
  }
}

export function createRoutingApi(store: AICCMgr): RoutingApi {
  return isMockRuntime() ? new MockRoutingApi(store) : new RpcRoutingApi()
}
