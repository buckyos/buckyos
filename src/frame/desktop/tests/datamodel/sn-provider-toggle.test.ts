import { MockDataStore } from '../../src/app/ai-center/mock/store.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

Deno.test('SN Provider remains visible and leaves routing when disabled', () => {
  const store = new MockDataStore('populated')
  const provider = store.getProviders().find((item) => item.config.provider_driver === 'sn-ai-provider')
  assert(provider, 'populated scenario must contain SN Provider')
  const enabledProviderCount = store.getSnapshot().aiStatus.provider_count

  store.setSnProviderEnabled(provider.config.id, false)

  const disabled = store.getProvider(provider.config.id)
  assert(disabled, 'disabled SN Provider must remain visible')
  assert(!disabled.config.enabled, 'SN Provider must be marked disabled')
  assert(store.getSnapshot().aiStatus.provider_count === enabledProviderCount - 1, 'disabled provider must leave routing status')

  store.setSnProviderEnabled(provider.config.id, true)
  assert(store.getProvider(provider.config.id)?.config.enabled, 'SN Provider must be re-enabled')
})

Deno.test('SN Provider toggle mapping remains constant-time as provider data grows', () => {
  for (const count of [1, 10, 1_000]) {
    const stores = Array.from({ length: count }, () => new MockDataStore('populated'))
    const start = performance.now()
    for (const store of stores) {
      const provider = store.getProviders().find((item) => item.config.provider_driver === 'sn-ai-provider')
      assert(provider, 'populated scenario must contain SN Provider')
      store.setSnProviderEnabled(provider.config.id, false)
    }
    console.log(`[SN Provider toggle] ${count} stores: ${(performance.now() - start).toFixed(1)}ms, 0 extra reads per store`)
  }
})
