import { expect, test, type Page } from '@playwright/test'
import type { AppDeploymentIdentity, AppInstallationStatus, AppSummary } from '../../../src/api/app_mgr'
import type { AppServiceItem } from '../../../src/app/app-service/types'

const NOW = 1_800_000_000_000
const SECONDS = NOW / 1000
const INSTANCE = 'systest@alice'

function summary(overrides: Partial<AppSummary> = {}): AppSummary {
  return {
    app_id: 'systest' as AppSummary['app_id'],
    app_instance_id: INSTANCE as AppSummary['app_instance_id'],
    app_did: 'did:bns:systest.buckyos',
    runtime_type: 'dapp',
    owner_user_id: 'alice',
    availability_match: null,
    show_name: 'System Test',
    version: '0.7.0',
    app_icon_url: null,
    author: 'did:bns:buckyos',
    app_index: 1,
    enable: true,
    state: 'new',
    expected_instance_count: 1,
    spec_path: 'users/alice/apps/systest',
    web_hosts: ['systest'],
    ...overrides,
  }
}

function deployment(overrides: Partial<AppDeploymentIdentity> = {}): AppDeploymentIdentity {
  return {
    app_instance_id: INSTANCE,
    task_id: 'install-2',
    app_doc_object_id: 'appdoc-2',
    spec_generation: 2,
    pikg_digest: 'sha256:new-package',
    ...overrides,
  }
}

type Report = NonNullable<AppInstallationStatus['runtime_instances']>[number]
type WebEvidence = NonNullable<AppInstallationStatus['static_web_evidence']>[number]

function report(overrides: Partial<Report> = {}): Report {
  return {
    deployment: deployment(),
    node_id: 'ood1',
    state: 'started',
    health: 'healthy',
    observed_at: SECONDS - 10,
    expires_at: SECONDS + 60,
    ...overrides,
  }
}

function webEvidence(overrides: Partial<WebEvidence> = {}): WebEvidence {
  return {
    deployment: deployment(),
    node_id: 'ood1',
    materialized_at: SECONDS - 30,
    gateway_ready_at: SECONDS - 20,
    gateway_config_generation: 'gateway-2',
    observed_at: SECONDS - 10,
    expires_at: SECONDS + 60,
    ...overrides,
  }
}

function snapshot(overrides: Partial<AppInstallationStatus> = {}): AppInstallationStatus {
  return {
    app_instance_id: INSTANCE,
    desired_deployment: deployment(),
    desired_spec: {
      enable: true,
      state: 'new',
      packages: [{ sub_pkg_name: 'script' }],
      app_doc: { app_type: 'dapp', pkg_list: { script: {} } },
    },
    desired_instance_count: 1,
    runtime_instances: [],
    static_web_evidence: [],
    ...overrides,
  }
}

function webSnapshot(evidence: WebEvidence): AppInstallationStatus {
  return snapshot({
    desired_spec: { enable: true, state: 'new', packages: [{ sub_pkg_name: 'web' }], app_doc: { app_type: 'web', pkg_list: { web: {} } } },
    static_web_evidence: [evidence],
  })
}

async function map(page: Page, app: AppSummary, status?: AppInstallationStatus): Promise<AppServiceItem> {
  return page.evaluate(async ({ app, status, now }) => {
    const modulePath = '/src/app/app-service/realApps.ts'
    const { appServiceItemFromSummary } = await import(modulePath)
    return appServiceItemFromSummary(app, { zoneHost: 'test.buckyos.io' }, status, now)
  }, { app, status, now: NOW })
}

test.beforeEach(async ({ page }) => {
  await page.route('**/runtime-mapping-harness', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>Runtime mapping</title>' }))
  await page.goto('/runtime-mapping-harness')
})

test('a native script uses its process evidence without acquiring Docker state', async ({ page }) => {
  const item = await map(page, summary(), snapshot({ runtime_instances: [report()] }))
  expect(item.status).toBe('running')
  expect(item.runtime).toMatchObject({ type: 'script', status: 'running', process_health: 'READY', docker: null })
  expect(item.docker).toBeNull()
  expect(item.runtime.evidence).toEqual({ deployment: deployment(), observed_at: NOW - 10_000, valid_until: NOW + 60_000 })
  expect(item.serviceInfo.endpoint).toBe('https://systest.test.buckyos.io')
})

test('fresh gateway evidence makes a static web app running while its desired state is new', async ({ page }) => {
  const item = await map(page, summary({ runtime_type: 'web', state: 'new' }), webSnapshot(webEvidence()))
  expect(item.status).toBe('running')
  expect(item.runtime).toMatchObject({ type: 'web', web_accessible: 'READY', process_health: 'UNKNOWN', docker: null })
})

const invalidWebEvidence: Array<[string, Partial<WebEvidence>]> = [
  ['expired', { expires_at: SECONDS }],
  ['future observation', { observed_at: SECONDS + 1 }],
  ['invalid lifetime', { expires_at: SECONDS - 20 }],
  ['other app instance', { deployment: deployment({ app_instance_id: 'systest@bob' }) }],
  ['previous task', { deployment: deployment({ task_id: 'install-1' }) }],
  ['previous document', { deployment: deployment({ app_doc_object_id: 'appdoc-1' }) }],
  ['previous generation', { deployment: deployment({ spec_generation: 1 }) }],
  ['other package digest', { deployment: deployment({ pikg_digest: 'sha256:other-package' }) }],
  ['missing package digest', { deployment: deployment({ pikg_digest: undefined }) }],
  ['no materialized content', { materialized_at: 0 }],
  ['no gateway generation', { gateway_config_generation: '' }],
  ['gateway predates content', { gateway_ready_at: SECONDS - 31 }],
  ['future content and gateway', { materialized_at: SECONDS + 1, gateway_ready_at: SECONDS + 2 }],
  ['future gateway', { gateway_ready_at: SECONDS + 1 }],
]

for (const [name, evidence] of invalidWebEvidence) {
  test(`static web rejects ${name} evidence`, async ({ page }) => {
    const item = await map(page, summary({ runtime_type: 'web', state: 'running' }), webSnapshot(webEvidence(evidence)))
    expect(item.status).not.toBe('running')
    expect(item.runtime.web_accessible).not.toBe('READY')
  })
}

test('one healthy replica does not make a two-replica service ready', async ({ page }) => {
  const item = await map(page, summary({ expected_instance_count: 2 }), snapshot({
    desired_instance_count: 2,
    runtime_instances: [report(), report({ node_id: 'node2', state: 'deploying', health: 'unknown' })],
  }))
  expect(item.status).not.toBe('running')
  expect(item.runtime.process_health).toBe('NOT_READY')
})

test('stale process reports and reports for an older deployment cannot make a script ready', async ({ page }) => {
  const item = await map(page, summary({ state: 'running' }), snapshot({ runtime_instances: [
    report({ expires_at: SECONDS }),
    report({ node_id: 'node2', deployment: deployment({ pikg_digest: 'sha256:old-package' }) }),
  ] }))
  expect(item.status).toBe('unknown')
  expect(item.runtime.process_health).toBe('UNKNOWN')
  expect(item.runtime.evidence).toBeNull()
})

test('two matching healthy replicas make the service ready until the first evidence expires', async ({ page }) => {
  const item = await map(page, summary({ expected_instance_count: 2 }), snapshot({
    desired_instance_count: 2,
    runtime_instances: [report(), report({ node_id: 'node2', observed_at: SECONDS - 5, expires_at: SECONDS + 30 })],
  }))
  expect(item.status).toBe('running')
  expect(item.runtime.evidence).toMatchObject({ observed_at: NOW - 5_000, valid_until: NOW + 30_000 })
})

test('mixed script and Docker packages remain an unknown runtime', async ({ page }) => {
  const status = snapshot({ runtime_instances: [report()] })
  status.desired_spec.packages = [{ sub_pkg_name: 'script' }, { sub_pkg_name: 'docker' }]
  const item = await map(page, summary(), status)
  expect(item.runtime.type).toBe('unknown')
  expect(item.runtime.docker).toBeNull()
})

test('deployment failures are errors and retain their diagnostic reason', async ({ page }) => {
  const item = await map(page, summary(), snapshot({ runtime_instances: [report({
    state: 'exited', health: 'unhealthy', deployment_error: { code: 'port_conflict', message: 'www port is occupied' },
  })] }))
  expect(item.status).toBe('error')
  expect(item.runtime).toMatchObject({ process_health: 'NOT_READY', reason: 'port_conflict' })
})

test('a disabled service is stopped only after matching stopped evidence', async ({ page }) => {
  const status = snapshot({ runtime_instances: [report({ state: 'stopped', health: 'unknown' })] })
  status.desired_spec.enable = false
  status.desired_spec.state = 'stopped'
  const stopped = await map(page, summary({ enable: false, state: 'stopped' }), status)
  expect(stopped.status).toBe('stopped')
  expect(stopped.runtime.process_health).toBe('NOT_READY')
  status.runtime_instances = []
  const pending = await map(page, summary({ enable: false, state: 'stopped' }), status)
  expect(pending.status).toBe('unknown')
})

test('a disabled service with a still-running process is stopping rather than starting', async ({ page }) => {
  const status = snapshot({ runtime_instances: [report()] })
  status.desired_spec.enable = false
  status.desired_spec.state = 'stopped'
  const item = await map(page, summary({ enable: false, state: 'stopped' }), status)
  expect(item.status).toBe('stopping')
  expect(item.runtime.process_health).toBe('NOT_READY')
})

test('a running summary without apps.status never invents live runtime evidence', async ({ page }) => {
  for (const runtime_type of ['dapp', 'web', 'agent'] as const) {
    const item = await map(page, summary({ runtime_type, state: 'running' }))
    expect(item.status).toBe('unknown')
    expect(item.runtime.evidence).toBeNull()
    expect(item.runtime.expected_deployment).toBeNull()
    expect(item.runtime.process_health).toBe('UNKNOWN')
    expect(item.runtime.web_accessible).toBe('UNKNOWN')
    expect(item.runtime.environment_ready).toBe('UNKNOWN')
    expect(item.runtime.agent_bindings).toBeNull()
  }
})

test('status for another installation cannot supply runtime evidence', async ({ page }) => {
  const item = await map(page, summary({ state: 'running' }), snapshot({
    app_instance_id: 'systest@bob', runtime_instances: [report()],
  }))
  expect(item.status).toBe('unknown')
  expect(item.runtime.evidence).toBeNull()
})
