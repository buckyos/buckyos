import { expect, test, type Page } from '@playwright/test'

const storageKey = 'buckyos.app-service.v4:prototype-zone:alice:admin'
async function snapshot(page: Page) {
  return page.evaluate(
    (key) =>
      JSON.parse(
        localStorage.getItem(key) ?? '{"tasks":{},"drafts":{},"services":[]}',
      ),
    storageKey,
  )
}
async function openPlan(
  page: Page,
  identifier = 'nextcloud',
  options?: unknown,
) {
  await page.goto(
    `/sysdlg/app_installer?${new URLSearchParams({ identifier, ...(options ? { options: JSON.stringify(options) } : {}) })}`,
  )
  await expect(
    page.getByTestId('app-installer-install-readiness'),
  ).toBeVisible()
  await page
    .getByRole('button', { name: 'Review installation plan', exact: true })
    .click()
  await expect(page.getByTestId('app-installer-submit')).toBeEnabled()
}
async function authorize(page: Page, password = 'prototype-admin') {
  await page.getByTestId('app-installer-submit').click()
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page
    .getByRole('button', { name: 'Authorize this plan', exact: true })
    .click()
}
async function taskId(page: Page) {
  await expect(page.getByTestId('app-installer-task-id')).toBeVisible()
  return (await page.getByTestId('app-installer-task-id').textContent())!
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    if (!sessionStorage.getItem('app22-test')) {
      localStorage.clear()
      sessionStorage.setItem('app22-test', '1')
    }
    localStorage.setItem('buckyos.prototype.locale.v1', 'en')
  })
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text())
  })
  page.on('request', (req) => {
    if (req.url().includes('/kapi/'))
      errors.push(`Unexpected backend call: ${req.url()}`)
  })
  test.info().annotations.push({
    type: 'console',
    description:
      'Browser exceptions, console errors and backend calls checked after each test.',
  })
  await page.exposeFunction('app22Errors', () => errors)
})
test.afterEach(async ({ page }) => {
  if (!page.isClosed())
    expect(
      await page.evaluate(() =>
        (
          window as unknown as { app22Errors: () => Promise<string[]> }
        ).app22Errors(),
      ),
    ).toEqual([])
})

test('inspect creates only a draft; submit creates one opaque task and refresh restores it', async ({
  page,
}) => {
  await openPlan(page, 'nextcloud', {
    target: { node_id: 'studio-pc', node_did: 'did:bns:studio-pc.alice' },
    install_params: { auto_start: false },
  })
  expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(0)
  await expect(page).toHaveURL(/identifier=/)
  await expect(page.getByTestId('app-installer-task-id')).toHaveCount(0)
  await expect(
    page.getByTestId('app-installer-approved-summary'),
  ).toContainText('Studio PC')
  await page.screenshot({
    path: 'test-results/app-service-beta22/desktop-plan.png',
    fullPage: true,
  })
  await authorize(page)
  const id = await taskId(page)
  expect(id).toMatch(/^t-[0-9a-f]{32}$/)
  await expect(page).toHaveURL(new RegExp(`task_id=${id}$`))
  await page.reload()
  await expect(page.getByTestId('app-installer-task-id')).toHaveText(id)
  await expect(
    page.getByRole('heading', { name: 'Installation configuration published' }),
  ).toBeVisible({ timeout: 10000 })
  await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
    'Not started',
  )
  expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(1)
  await page.getByRole('link', { name: 'Open in Task Center' }).click()
  await expect(page.getByText(id, { exact: true }).first()).toBeVisible()
  await expect(
    page.getByRole('heading', { name: 'Nextcloud', exact: true }).first(),
  ).toBeVisible()
})

test('draft refresh re-inspects and does not restore authorization', async ({
  page,
}) => {
  await openPlan(page)
  await page.getByTestId('app-installer-submit').click()
  await page
    .getByLabel('Password', { exact: true })
    .fill('do-not-persist-password')
  await page.reload()
  await expect(
    page.getByRole('button', { name: 'Review installation plan' }),
  ).toBeVisible()
  await expect(page.getByLabel('Password', { exact: true })).toHaveCount(0)
  const state = await snapshot(page)
  expect(Object.keys(state.tasks)).toHaveLength(0)
  expect(Object.keys(state.drafts)).toHaveLength(1)
  expect(JSON.stringify(state)).not.toContain('do-not-persist-password')
})

test('wrong password, cancellation, expiry and retry preserve the confirmation boundary', async ({
  page,
}) => {
  await openPlan(page)
  await authorize(page, 'wrong-password')
  await expect(page.getByRole('alert')).toContainText('Incorrect password')
  expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(0)
  await page.getByRole('button', { name: 'Cancel', exact: true }).click()
  await expect(page.getByTestId('app-installer-submit')).toBeEnabled()
  await authorize(page, 'prototype-expired')
  await expect(page.getByRole('alert')).toContainText('authorization expired')
  expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(0)
  await authorize(page)
  await taskId(page)
  expect(JSON.stringify(await snapshot(page))).not.toMatch(
    /wrong-password|prototype-expired|prototype-admin|sudo-/,
  )
})

test('edits invalidate the plan; required declarations and sensitive variables are protected', async ({
  page,
}) => {
  await openPlan(page)
  await page.getByLabel('LANG · Required').fill('zh_CN.UTF-8')
  await page
    .getByLabel('API_KEY', { exact: true })
    .fill('private-environment-secret')
  await expect(page.getByTestId('app-installer-submit')).toBeDisabled()
  await page.getByTestId('app-installer-recheck').click()
  await expect(page.getByTestId('app-installer-submit')).toBeDisabled()
  await expect(page.getByTestId('app-installer-submit')).toBeEnabled()
  await expect(
    page.getByTestId('app-installer-approved-summary'),
  ).toContainText('zh_CN.UTF-8')
  await expect(
    page.getByTestId('app-installer-approved-summary'),
  ).not.toContainText('private-environment-secret')
  await expect(
    page.getByLabel('main · Required', { exact: true }),
  ).toBeDisabled()
  await expect(page.getByLabel('www · http:80 · Required')).toBeDisabled()
  await page
    .getByTestId('app-installer-access-settings')
    .getByLabel('Allow guests')
    .first()
    .check()
  await page.getByTestId('app-installer-recheck').click()
  await expect(page.getByRole('alert')).toContainText(
    'Guest access requires public exposure',
  )
  await expect(page.getByTestId('app-installer-submit')).toBeDisabled()
  await page
    .getByTestId('app-installer-access-settings')
    .getByLabel('Allow guests')
    .first()
    .uncheck()
  await page.getByTestId('app-installer-recheck').click()
  await expect(page.getByTestId('app-installer-submit')).toBeEnabled()
  await authorize(page)
  await taskId(page)
  expect(JSON.stringify(await snapshot(page))).not.toContain(
    'private-environment-secret',
  )
})

test('PlanStale requires fresh inspection and new approval', async ({
  page,
}) => {
  await openPlan(page, 'nextcloud-stale')
  const first = await page
    .getByTestId('app-installer-approved-summary')
    .textContent()
  await authorize(page)
  await expect(page.getByRole('alert')).toContainText(
    'Application information or installation conditions changed',
  )
  await expect(page.getByTestId('app-installer-submit')).toBeDisabled()
  expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(0)
  await page.getByTestId('app-installer-recheck').click()
  await expect(page.getByTestId('app-installer-submit')).toBeEnabled()
  expect(
    await page.getByTestId('app-installer-approved-summary').textContent(),
  ).not.toEqual(first)
  await authorize(page)
  await taskId(page)
})

for (const [identifier, code] of [
  ['nextcloud-trust-pending', 'TRUST_RESOLUTION_REQUIRED'],
  ['nextcloud-signature-fail', 'SIGNATURE_INVALID'],
  ['nextcloud-owner-fail', 'OWNER_MISMATCH'],
  ['nextcloud-document-fail', 'INVALID_APPDOC'],
  ['nextcloud-revoked', 'IDENTITY_REVOKED'],
  ['nextcloud-unsupported', 'UNSUPPORTED_TARGET'],
  ['nextcloud-config-invalid', 'CONFIG_CONFLICT'],
])
  test(`check classifies ${code}`, async ({ page }) => {
    await page.goto(`/sysdlg/app_installer?identifier=${identifier}`)
    await expect(
      page.getByTestId('app-installer-blocking-reason'),
    ).toContainText(code)
    expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(0)
  })

test('offline mode blocks missing content and never fetches a URL', async ({
  page,
}) => {
  await page.goto(
    `/sysdlg/app_installer?${new URLSearchParams({ identifier: 'nextcloud', options: JSON.stringify({ offline: true }) })}`,
  )
  await expect(page.getByTestId('app-installer-blocking-reason')).toContainText(
    'OFFLINE_CONTENT_UNAVAILABLE',
  )
  await page.goto(
    '/sysdlg/app_installer?identifier=https%3A%2F%2Fexample.com%2Fapp.pikg',
  )
  await expect(page.getByTestId('app-installer-launch-error')).toContainText(
    'URL_IMPORT_REQUIRED',
  )
})

test('download retry uses returned new identity and retains the old attempt', async ({
  page,
}) => {
  await openPlan(page, 'nextcloud-fail-download')
  await authorize(page)
  const previous = await taskId(page)
  await expect(
    page.getByRole('heading', { name: 'Installation stopped' }),
  ).toBeVisible()
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByTestId('app-installer-task-id')).not.toHaveText(
    previous,
  )
  const next = await taskId(page)
  await expect(page).toHaveURL(new RegExp(`task_id=${next}$`))
  await expect(
    page.getByRole('heading', { name: 'Installation configuration published' }),
  ).toBeVisible({ timeout: 10000 })
  const state = await snapshot(page)
  expect(state.tasks[next].retry_of).toBe(previous)
  expect(state.tasks[previous].outcome).toBe('Failed')
  expect(
    state.services.filter(
      (s: { app_instance_id: string }) =>
        s.app_instance_id === state.tasks[next].app_instance_id,
    ),
  ).toHaveLength(1)
})

test('paused tasks resume under their original ID', async ({ page }) => {
  await openPlan(page, 'nextcloud-paused')
  await authorize(page)
  const id = await taskId(page)
  await expect(
    page.getByRole('heading', { name: 'Waiting to continue' }),
  ).toBeVisible()
  await page.getByRole('button', { name: 'Resume', exact: true }).click()
  await expect(page.getByTestId('app-installer-task-id')).toHaveText(id)
  await expect(
    page.getByRole('heading', { name: 'Installation configuration published' }),
  ).toBeVisible({ timeout: 10000 })
})

test('cancel is explicit before commit; close keeps submitted tasks in the background', async ({
  page,
}) => {
  await openPlan(page)
  await authorize(page)
  const id = await taskId(page)
  await page.getByRole('button', { name: 'Cancel installation' }).click()
  await expect(
    page.getByRole('heading', { name: 'Installation canceled' }),
  ).toBeVisible()
  expect((await snapshot(page)).tasks[id].outcome).toBe('Canceled')
  await openPlan(page, 'paperless')
  await authorize(page)
  const backgroundId = await taskId(page)
  await page.getByRole('button', { name: 'Run in background' }).click()
  await page.goto(`/sysdlg/app_installer?task_id=${backgroundId}`)
  await expect(page.getByTestId('app-installer-commit-boundary')).toBeVisible({
    timeout: 10000,
  })
  await expect(
    page.getByRole('button', { name: 'Cancel installation' }),
  ).toHaveCount(0)
})

test('failure after commit only resumes scheduling and cannot cancel', async ({
  page,
}) => {
  await openPlan(page, 'nextcloud-fail-committed')
  await authorize(page)
  const id = await taskId(page)
  await expect(page.getByRole('alert')).toContainText('SCHEDULING_FAILED', {
    timeout: 10000,
  })
  await expect(page.getByRole('button', { name: 'Change source' })).toHaveCount(
    0,
  )
  await expect(
    page.getByRole('button', { name: 'Cancel installation' }),
  ).toHaveCount(0)
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(
    page.getByRole('heading', { name: 'Installation configuration published' }),
  ).toBeVisible({ timeout: 10000 })
  await expect(page.getByTestId('app-installer-task-id')).toHaveText(id)
})

for (const [scenario, expected] of [
  ['normal', 'Running normally'],
  ['activation-fail', 'Installed · startup failed'],
  ['offline-node', 'Runtime status unknown'],
  ['runtime-timeout', 'Runtime status unknown'],
  ['runtime-unknown', 'Runtime status unknown'],
  ['stale-evidence', 'Runtime status unknown'],
])
  test(`completed task keeps independent runtime state: ${scenario}`, async ({
    page,
  }) => {
    await openPlan(
      page,
      scenario === 'normal' ? 'nextcloud' : `nextcloud-${scenario}`,
    )
    await authorize(page)
    await expect(
      page.getByRole('heading', {
        name: 'Installation configuration published',
      }),
    ).toBeVisible({ timeout: 10000 })
    await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
      'Configuration submitted · deploying',
    )
    await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
      expected,
      { timeout: 6000 },
    )
    if (expected === 'Runtime status unknown') {
      await expect(
        page.getByTestId('app-service-runtime-summary').locator('dd'),
      ).toHaveText([
        'Unknown / unavailable',
        'Unknown / unavailable',
        'Unknown / unavailable',
      ])
    }
  })

test('same content is satisfied without task; upgrade confirms impact and downgrade is rejected', async ({
  page,
}) => {
  await openPlan(page)
  await authorize(page)
  await expect(
    page.getByRole('heading', { name: 'Installation configuration published' }),
  ).toBeVisible({ timeout: 10000 })
  const count = Object.keys((await snapshot(page)).tasks).length
  await page.goto('/sysdlg/app_installer?identifier=nextcloud')
  await expect(page.getByTestId('app-installer-satisfied')).toBeVisible()
  expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(count)
  await page.goto('/sysdlg/app_installer?identifier=nextcloud-upgrade')
  await expect(page.getByTestId('app-installer-upgrade')).toContainText(
    '28.0.2 → 29.0.0',
  )
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await expect(page.getByTestId('app-installer-submit')).toHaveText(
    'Confirm and upgrade',
  )
  await authorize(page)
  await expect(
    page.getByRole('heading', { name: 'Installation configuration published' }),
  ).toBeVisible({ timeout: 10000 })
  await page.goto('/sysdlg/app_installer?identifier=nextcloud-downgrade')
  await expect(page.getByTestId('app-installer-blocking-reason')).toContainText(
    'DOWNGRADE_NOT_SUPPORTED',
  )
})

test('public query rejects ambiguous input and missing or forbidden tasks never create substitutes', async ({
  page,
}) => {
  for (const [query, code] of [
    ['identifier=nextcloud&auto_confirm=true', 'unknown_parameter'],
    ['identifier=nextcloud&identifier=files', 'duplicate_parameter'],
    ['task_id=t-one&identifier=nextcloud', 'conflicting_parameters'],
    ['task_id=', 'invalid_task_id'],
    [
      'identifier=nextcloud&options=%7B%22target%22%3A%7B%22node_id%22%3A%22missing%22%7D%7D',
      'invalid_target',
    ],
    ['task_id=t-forbidden', 'TASK_FORBIDDEN'],
    ['task_id=t-does-not-exist', 'TASK_NOT_FOUND'],
    ['draft_id=internal', 'unknown_parameter'],
  ]) {
    await page.goto(`/sysdlg/app_installer?${query}`)
    await expect(page.getByTestId('app-installer-launch-error')).toContainText(
      code,
    )
  }
  expect(Object.keys((await snapshot(page)).tasks)).toHaveLength(0)
})

test.describe('mobile installer', () => {
  test.use({
    viewport: { width: 375, height: 812 },
    hasTouch: true,
    isMobile: true,
  })
  test('check, form, authorization and result fit at 375px', async ({
    page,
  }) => {
    await openPlan(page, 'opendan-upgrade')
    await page.screenshot({
      path: 'test-results/app-service-beta22/mobile-plan.png',
      fullPage: true,
    })
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBeTruthy()
    await authorize(page)
    await expect(
      page.getByRole('heading', {
        name: 'Installation configuration published',
      }),
    ).toBeVisible({ timeout: 10000 })
    await expect(page.getByTestId('app-service-runtime-summary')).toContainText(
      'no Agent binding yet',
    )
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBeTruthy()
    await page.screenshot({
      path: 'test-results/app-service-beta22/mobile-result.png',
      fullPage: true,
    })
  })
})

test('concurrent submission and replay produce exactly one task with one card', async ({
  page,
}) => {
  await openPlan(page)
  const result = await page.evaluate(async () => {
    const path = '/src/app/app-service/hooks/use-app-service-store.ts'
    const { getSharedAppServiceStore } = await import(path)
    const store = getSharedAppServiceStore()
    const draft = Object.values(store.drafts)[0] as {
      draft_id: string
      plan: { plan_fingerprint: string }
    }
    await store.inspectDraft(draft.draft_id)
    const grant = await store.authorize(
      {
        username: 'alice',
        appid: 'control-panel',
        password: 'prototype-admin',
      },
      draft.draft_id,
    )
    const fp = draft.plan.plan_fingerprint
    const [first, second] = await Promise.all([
      store.submitDraft(draft.draft_id, fp, 'same-request', grant),
      store.submitDraft(draft.draft_id, fp, 'same-request', grant),
    ])
    const replay = await store.submitDraft(
      draft.draft_id,
      fp,
      'same-request',
      grant,
    )
    let mismatch = ''
    try {
      await store.submitDraft(
        draft.draft_id,
        'different-fingerprint',
        'same-request',
        grant,
      )
    } catch (e) {
      mismatch = (e as Error).message
    }
    return {
      first,
      second,
      replay,
      mismatch,
      count: store.getTasks().length,
      cardCount: store
        .getByLayer('app')
        .filter(
          (s: { app_did: string }) => s.app_did === 'did:bns:nextcloud.buckyos',
        ).length,
    }
  })
  expect(result.first.task_id).toEqual(result.second.task_id)
  expect(result.first.task_id).toEqual(result.replay.task_id)
  expect(result.mismatch).toBe('PLAN_STALE')
  expect(result.count).toBe(1)
  expect(result.cardCount).toBe(1)
})

test('schema rejects undeclared grants, system overrides, missing required data and port conflicts', async ({
  page,
}) => {
  await page.goto('/sysdlg/app_installer?identifier=nextcloud')
  const results = await page.evaluate(async () => {
    const schemaPath = '/src/app/app-service/schemas.ts'
    const fixturePath = '/src/app/app-service/mock/fixtures.ts'
    const { createInstallInputSchema, appDocCandidateSchema } = await import(
      schemaPath
    )
    const { appDocument, defaultInput, targets } = await import(fixturePath)
    const app = appDocument('nextcloud')
    const schema = createInstallInputSchema(app, targets, true)
    const valid = defaultInput(app)
    const missingComponent = structuredClone(valid)
    missingComponent.install_params.selected_components = ['worker']
    const permission = structuredClone(valid)
    permission.install_params.permissions[0].actions = ['read']
    const mounts = structuredClone(valid)
    mounts.install_params.data_mount_points = {}
    const env = structuredClone(valid)
    env.install_params.bash_envs.BUCKYOS_APP_INSTANCE_ID = 'spoofed'
    const target = structuredClone(valid)
    target.target_node_id = 'invented-ood'
    const traversal = structuredClone(valid)
    traversal.install_params.external_mount_points = {
      '/app/import': { target_path: '/../secrets', access: 'read_only' },
    }
    const duplicate = structuredClone(valid)
    duplicate.install_params.selected_components = ['main', 'main']
    const multiPortApp = structuredClone(app)
    multiPortApp.endpoints.push({
      name: 'extra',
      protocol: 'tcp',
      inner_port: 9091,
      required: false,
      route: 'port',
    })
    const ports = defaultInput(multiPortApp)
    ports.install_params.service_settings.services.metrics.enabled = true
    ports.install_params.service_settings.services.extra.enabled = true
    ports.install_params.service_settings.services.extra.expose.route.expose_port = 9090
    const document = {
      schema_version: 1,
      doc_type: 'app',
      did: 'did:bns:example.buckyos',
      owner: 'did:bns:buckyos',
      controller: 'did:bns:buckyos',
      author: 'did:bns:buckyos',
      create_time: 1788566400,
      last_update_time: 1788566400,
      exp: 1893456000,
      version: '1.0.0',
      app_type: 'web',
      show_name: 'Example',
      selector_type: 'static',
      pkg_list: {
        web: {
          pkg_id: 'example#1.0.0',
          pkg_objid: `pkg:${'a1'.repeat(32)}`,
          required: true,
        },
      },
      service_config_tips: {},
    }
    return {
      valid: schema.safeParse(valid).success,
      invalid: [
        missingComponent,
        permission,
        mounts,
        env,
        target,
        traversal,
        duplicate,
      ].map((v) => schema.safeParse(v).success),
      ports: createInstallInputSchema(multiPortApp, targets, true).safeParse(
        ports,
      ).success,
      doc: appDocCandidateSchema.safeParse(document).success,
      legacyDoc: appDocCandidateSchema.safeParse({
        ...document,
        doc_type: 'APPDOC',
        id: document.did,
      }).success,
    }
  })
  expect(results.valid).toBeTruthy()
  expect(results.invalid.every((value: boolean) => !value)).toBeTruthy()
  expect(results.ports).toBeFalsy()
  expect(results.doc).toBeTruthy()
  expect(results.legacyDoc).toBeFalsy()
})

test('a changed draft invalidates an in-flight inspection and old authorization', async ({
  page,
}) => {
  await openPlan(page)
  const result = await page.evaluate(async () => {
    const path = '/src/app/app-service/hooks/use-app-service-store.ts'
    const { getSharedAppServiceStore } = await import(path)
    const store = getSharedAppServiceStore()
    const draft = Object.values(store.drafts)[0] as {
      draft_id: string
      input: { install_params: { bash_envs: Record<string, string> } }
      plan: { plan_fingerprint: string }
    }
    await store.inspectDraft(draft.draft_id)
    const old = draft.plan.plan_fingerprint
    const grant = await store.authorize(
      {
        username: 'alice',
        appid: 'control-panel',
        password: 'prototype-admin',
      },
      draft.draft_id,
    )
    const first = store.inspectDraft(draft.draft_id)
    const input = structuredClone(draft.input)
    input.install_params.bash_envs.LANG = 'fr_FR.UTF-8'
    store.editDraft(draft.draft_id, input)
    const second = store.inspectDraft(draft.draft_id)
    await Promise.all([first, second])
    let error = ''
    try {
      await store.submitDraft(draft.draft_id, old, 'old-plan', grant)
    } catch (e) {
      error = (e as Error).message
    }
    return {
      error,
      language: store.getDraft(draft.draft_id).plan.input.install_params
        .bash_envs.LANG,
      count: store.getTasks().length,
    }
  })
  expect(result.error).toBe('AUTH_EXPIRED')
  expect(result.language).toBe('fr_FR.UTF-8')
  expect(result.count).toBe(0)
})

test('unknown task phase and stage fall back safely without actions', async ({
  page,
}) => {
  await openPlan(page)
  await authorize(page)
  const id = await taskId(page)
  await page.evaluate(
    ({ key, id }) => {
      const state = JSON.parse(localStorage.getItem(key)!)
      state.tasks[id].phase = 'FuturePhase'
      state.tasks[id].stage = 'future_stage'
      localStorage.setItem(key, JSON.stringify(state))
    },
    { key: storageKey, id },
  )
  await page.reload()
  await expect(
    page.getByRole('heading', { name: 'Unknown / unavailable' }),
  ).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Cancel installation' }),
  ).toHaveCount(0)
})

test('competing plans cannot overwrite a newer committed deployment', async ({
  page,
}) => {
  await openPlan(page)
  const ids = await page.evaluate(async () => {
    const path = '/src/app/app-service/hooks/use-app-service-store.ts'
    const { getSharedAppServiceStore } = await import(path)
    const store = getSharedAppServiceStore()
    const firstDraft = Object.values(store.drafts)[0] as {
      draft_id: string
      source: unknown
      plan: { plan_fingerprint: string }
    }
    const submit = async (id: string) => {
      await store.inspectDraft(id)
      const grant = await store.authorize(
        {
          username: 'alice',
          appid: 'control-panel',
          password: 'prototype-admin',
        },
        id,
      )
      return store.submitDraft(
        id,
        store.getDraft(id).plan.plan_fingerprint,
        id,
        grant,
      )
    }
    const first = await submit(firstDraft.draft_id)
    const second = await submit(store.createDraft(firstDraft.source))
    return { first: first.task_id, second: second.task_id }
  })
  await expect
    .poll(async () => (await snapshot(page)).tasks[ids.second].error?.code, {
      timeout: 10000,
    })
    .toBe('PLAN_STALE')
  const stored = await snapshot(page)
  const record = stored.services.find(
    (service: { app_instance_id: string }) =>
      service.app_instance_id === 'nextcloud.buckyos.bns.did@alice',
  ).record
  expect(record.deployment.spec_generation).toBe(1)
  expect(record.task_id).toBe(ids.first)
  expect(stored.tasks[ids.second].desired_state_committed).toBeFalsy()
  expect(stored.tasks[ids.second].available_actions).toEqual(['inspect'])
})
