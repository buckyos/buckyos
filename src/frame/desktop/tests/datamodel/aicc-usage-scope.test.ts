import { buckyos } from 'buckyos'
import { createAICCMgr, type UsageScope } from '../../src/api/aicc_mgr.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

Deno.test('usage overview and event requests apply the same explicit scope across every aggregate', async () => {
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  const sent: Record<string, unknown>[] = []
  const aggregate = {
    total_requests: 3,
    total_tokens: 120,
    finance_totals: [{ currency: 'usd', amount: 0.25 }, { currency: 'EUR', amount: 0.5 }],
    finance_complete: false,
  }
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (method: string, params: Record<string, unknown>) => {
      assert(method === 'usage.query', 'usage reads must not require provider management RPCs')
      sent.push(params)
      const key = (params.group_by as string[] | undefined)?.[0]
      return {
        total: aggregate,
        grouped: key ? [{ group: { [key]: key === 'user_id' ? 'bob' : 'model@provider' }, aggregate }] : [],
        buckets: params.time_bucket ? [{ bucket_start_ms: 1_700_000_000_000, aggregate }] : [],
        events: params.output_mode === 'events' ? [{
          event_id: 'event-bob', tenant_id: 'tenant-bob', user_id: 'bob', created_at_ms: 1_700_000_000_000,
          provider_model: 'model@provider', total_tokens: 120,
        }] : [],
      }
    },
  })
  try {
    const manager = createAICCMgr({ useMock: false })
    const initialSummary = manager.getUsageSummary()
    const scopes: UsageScope[] = [{}, { zoneWide: true }, { zoneWide: true, userId: 'bob' }]
    for (const scope of scopes) {
      sent.length = 0
      const overview = await manager.queryUsageOverview(scope)
      assert(overview.summary.total_tokens === 120 && overview.summary.total_requests === 3, 'aggregate totals are preserved')
      assert(overview.summary.today_tokens === 120 && overview.summary.this_month_tokens === 120, 'short windows are queried in the same scope')
      assert(overview.users[0].user_id === 'bob', 'user breakdown uses user_id, not tenant_id')
      assert(overview.users[0].finance_totals.length === 2, 'per-user costs preserve separate currencies')
      assert(!overview.users[0].finance_complete, 'incomplete per-user costs remain marked incomplete')
      assert(overview.trend[0].tokens === 120, 'trend carries scoped aggregates')
      const page = await manager.queryUsageEvents({
        scope, timeRange: { startTimeMs: 0, endTimeMs: Date.now() }, limit: 10,
        filters: { providerInstanceNames: ['provider'] },
      })
      assert(page.events[0].user_id === 'bob' && page.events[0].tenant_id === 'tenant-bob', 'event ownership and tenant remain distinct')
      assert(sent.length === 8, 'all seven overview slices plus events are requested')
      for (const params of sent) {
        assert(params.zone_wide === (scope.zoneWide === true), 'every request carries the selected system/personal boundary')
        const filters = params.filters as Record<string, unknown>
        assert(JSON.stringify(filters.user_ids) === JSON.stringify(scope.userId ? [scope.userId] : undefined), 'every slice carries the selected user')
      }
      assert((sent.at(-1)?.filters as { provider_instance_names: string[] }).provider_instance_names[0] === 'provider', 'scope composes with existing detail filters')
      assert(manager.getUsageSummary() === initialSummary, 'viewing another user does not replace the cached personal summary')
    }
  } finally {
    sdk.getAccountInfo = originalAccountInfo
    sdk.getServiceRpcClient = originalClient
  }
})

Deno.test('usage overview rejects a failed slice instead of displaying an empty user breakdown', async () => {
  const sdk = buckyos as unknown as {
    getAccountInfo: () => Promise<unknown>
    getServiceRpcClient: (service: string) => unknown
  }
  const originalAccountInfo = sdk.getAccountInfo
  const originalClient = sdk.getServiceRpcClient
  sdk.getAccountInfo = async () => ({ session_token: 'test-session' })
  sdk.getServiceRpcClient = () => ({
    call: async (_method: string, params: Record<string, unknown>) => {
      if ((params.group_by as string[] | undefined)?.includes('user_id')) throw new Error('usage unavailable')
      return {}
    },
  })
  try {
    let error: unknown
    try {
      await createAICCMgr({ useMock: false }).queryUsageOverview({ zoneWide: true })
    } catch (caught) {
      error = caught
    }
    assert(error instanceof Error && error.message === 'usage unavailable', 'failed group query must remain visible to the dashboard')
  } finally {
    sdk.getAccountInfo = originalAccountInfo
    sdk.getServiceRpcClient = originalClient
  }
})

Deno.test('mock usage scopes separate personal, system and per-user totals and events', async () => {
  const originalLocation = Object.getOwnPropertyDescriptor(globalThis, 'location')
  Object.defineProperty(globalThis, 'location', { configurable: true, value: { search: '?aiccScenario=populated' } })
  try {
    const manager = createAICCMgr({ useMock: true })
    const all = await manager.queryUsageOverview({ zoneWide: true })
    const own = await manager.queryUsageOverview({})
    const bob = await manager.queryUsageOverview({ zoneWide: true, userId: 'bob' })
    assert(all.users.length === 3, 'the fixture has multiple owners')
    assert(own.users.length === 1 && own.users[0].user_id === 'alice', 'personal usage belongs to the current account')
    assert(bob.users.length === 1 && bob.users[0].user_id === 'bob', 'selecting bob excludes other owners')
    assert(all.summary.total_tokens > own.summary.total_tokens && all.summary.total_tokens > bob.summary.total_tokens, 'system totals exceed each selected user')
    assert(all.users.reduce((total, user) => total + user.total_tokens, 0) === all.summary.total_tokens, 'per-user breakdown reconciles to the system aggregate')
    for (const [scope, owner] of [[{}, 'alice'], [{ zoneWide: true, userId: 'bob' }, 'bob']] as const) {
      const page = await manager.queryUsageEvents({ scope, timeRange: { startTimeMs: 0, endTimeMs: Date.now() }, limit: 200 })
      assert(page.events.length > 0 && page.events.every((event) => event.user_id === owner), 'event owner matches the selected scope')
    }
  } finally {
    if (originalLocation) Object.defineProperty(globalThis, 'location', originalLocation)
    else Reflect.deleteProperty(globalThis, 'location')
  }
})
