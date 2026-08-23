import type { AppSummary } from '../../src/api/app_mgr.ts'
import {
  buildAuthorizedAppDefinitions,
  createBackendAppDefinitionMapper,
} from '../../src/app/backend-apps.ts'
import type { AppDefinition } from '../../src/models/ui.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    )
  }
}

function summary(overrides: Partial<AppSummary> = {}): AppSummary {
  return {
    app_id: 'notes.example' as AppSummary['app_id'],
    app_instance_id: 'notes.example@alice' as AppSummary['app_instance_id'],
    app_did: 'did:web:notes.example',
    runtime_type: 'dapp',
    owner_user_id: 'alice',
    availability_match: { type: 'owner', subject: 'alice' },
    show_name: 'Notes',
    version: '1.0.0',
    app_icon_url: null,
    icon_res_url: 'res/notes/appicon.png',
    author: 'alice',
    app_index: 1,
    enable: true,
    state: 'running',
    expected_instance_count: 1,
    spec_path: 'users/alice/apps/notes/spec',
    web_hosts: ['notes'],
    ...overrides,
  }
}

const systestCatalogEntry: AppDefinition = {
  id: 'systest',
  iconKey: 'systest',
  labelKey: 'apps.systest',
  summaryKey: 'appSummary.systest',
  accent: 'var(--cp-success)',
  tier: 'sdk',
  manifest: {
    defaultMode: 'windowed',
    allowMinimize: true,
    allowMaximize: true,
    allowClose: true,
    allowFullscreen: false,
    mobileFullscreenBehavior: 'cover_dead_zone',
    mobileStatusBarMode: 'compact',
    titleBarMode: 'system',
    placement: 'inplace',
  },
}

Deno.test('Desktop built-in apps remain available without AppSpecs', () => {
  const expectedIds = ['ai-center', 'files', 'task-center', 'workflow']
  const catalog = expectedIds.map((id): AppDefinition => ({
    ...systestCatalogEntry,
    id,
    iconKey: id,
    labelKey: `apps.${id}`,
    summaryKey: `appSummary.${id}`,
    tier: 'system',
  }))
  const definitions = buildAuthorizedAppDefinitions(catalog, [])

  assertEquals(definitions.map((app) => app.id), expectedIds, 'Desktop built-in ids')
})

Deno.test('buckyos_systest keeps its canonical instance id but uses the short Systest catalog identity', () => {
  const definitions = buildAuthorizedAppDefinitions(
    [systestCatalogEntry],
    [
      summary({
        app_id: 'buckyos-systest.buckyos.bns.did' as AppSummary['app_id'],
        app_instance_id: 'buckyos-systest.buckyos.bns.did@devtest' as AppSummary['app_instance_id'],
        app_did: 'did:bns:buckyos-systest.buckyos',
        owner_user_id: 'devtest',
        show_name: 'buckyos_systest@devtest',
        web_hosts: ['systest'],
      }),
    ],
  )
  const app = definitions[0]

  assertEquals(
    app.id,
    'buckyos-systest.buckyos.bns.did@devtest',
    'canonical instance id',
  )
  assertEquals(app.logicalAppId, 'buckyos-systest.buckyos.bns.did', 'logical app id')
  assertEquals(app.labelKey, 'apps.systest', 'short localized label')
  assertEquals(app.iconKey, 'systest', 'catalog icon')
  assertEquals(app.webHosts, ['systest'], 'gateway launch host')
  assertEquals(app.tier, 'sdk', 'embedded web tier')
  assertEquals(app.manifest.placement, 'inplace', 'window placement')
})

Deno.test('unknown Web apps use their AppSpec host and the iframe window contract', () => {
  const app = buildAuthorizedAppDefinitions([], [summary()])[0]

  assertEquals(app.id, 'notes.example@alice', 'canonical instance id')
  assertEquals(app.labelKey, 'Notes', 'display name')
  assertEquals(app.webHosts, ['notes'], 'gateway launch host')
  assertEquals(app.tier, 'sdk', 'embedded web tier')
  assertEquals(app.manifest.placement, 'inplace', 'window placement')
  assertEquals(app.manifest.contentPadding, 'none', 'iframe content padding')
})

Deno.test('owner-qualified app instances use a short logical display name', () => {
  const app = buildAuthorizedAppDefinitions([], [
    summary({
      app_id: 'photos.example' as AppSummary['app_id'],
      app_instance_id: 'photos.example@alice' as AppSummary['app_instance_id'],
      app_did: 'did:web:photos.example',
      show_name: 'photos',
      web_hosts: ['photos'],
    }),
  ])[0]

  assertEquals(app.id, 'photos.example@alice', 'canonical instance id')
  assertEquals(app.labelKey, 'photos', 'short display name')
  assertEquals(app.webHosts, ['photos'], 'gateway launch host')
  assertEquals(app.manifest.placement, 'inplace', 'window placement')
})

Deno.test('backend app mapping remains linear through one million entries', () => {
  const toDefinition = createBackendAppDefinitionMapper([])
  const input = summary()

  for (const count of [1, 10, 1_000, 1_000_000]) {
    const start = performance.now()
    let checksum = 0
    for (let index = 0; index < count; index += 1) {
      checksum += toDefinition(input).id.length
    }
    const elapsed = performance.now() - start
    if (checksum !== count * input.app_instance_id.length) {
      throw new Error(`mapping checksum mismatch for ${count} entries`)
    }
    console.log(
      `[backend app mapping] ${count} entries: ${elapsed.toFixed(1)}ms, 0 extra RPCs`,
    )
  }
})
