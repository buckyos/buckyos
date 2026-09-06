import { expect, test, type Page } from '@playwright/test'

const storageKey = 'buckyos.app-service.v4:prototype-zone:alice:admin'
async function openAppService(page: Page) {
  const button = page.getByRole('button', { name: 'App Service', exact: true })
  if ((page.viewportSize()?.width ?? 1280) <= 767) {
    const box = await button.boundingBox()
    const x = (box?.x ?? 0) + (box?.width ?? 0) / 2
    const y = (box?.y ?? 0) + (box?.height ?? 0) / 2
    await button.dispatchEvent('pointerdown', {
      bubbles: true,
      clientX: x,
      clientY: y,
      pointerId: 21,
      pointerType: 'touch',
    })
    await page
      .locator('body')
      .dispatchEvent('pointerup', {
        bubbles: true,
        clientX: x + 5,
        clientY: y + 5,
        pointerId: 21,
        pointerType: 'touch',
      })
  } else await button.click()
  await expect(
    page.getByRole('heading', { name: 'App Service', exact: true }),
  ).toBeVisible()
}
async function openSource(page: Page) {
  await page.goto('/?scenario=normal')
  await openAppService(page)
  await page.getByRole('button', { name: 'Add app' }).click()
}
async function importPackage(page: Page, scenario = 'normal', content = true) {
  await page
    .getByTestId('app-service-pikg-upload')
    .setInputFiles({
      name: 'nextcloud.pikg',
      mimeType: 'application/octet-stream',
      buffer: Buffer.from(
        JSON.stringify({
          format: 'buckyos-pikg-mock-v1',
          app: 'nextcloud',
          scenario,
          content_available: content,
        }),
      ),
    })
  await expect(page.getByTestId('app-service-source-result')).toBeVisible()
  await page.getByTestId('app-service-source-next').click()
  await expect(
    page.getByTestId('app-installer-install-readiness'),
  ).toBeVisible()
}
async function authorize(page: Page) {
  await page.getByTestId('app-installer-submit').click()
  await page.getByLabel('Password', { exact: true }).fill('prototype-admin')
  await page
    .getByRole('button', { name: 'Authorize this plan', exact: true })
    .click()
}
async function state(page: Page) {
  return page.evaluate(
    (key) =>
      JSON.parse(localStorage.getItem(key) ?? '{"tasks":{},"drafts":{}}'),
    storageKey,
  )
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    if (!sessionStorage.getItem('app22-test')) {
      localStorage.clear()
      sessionStorage.setItem('app22-test', '1')
    }
    if (!localStorage.getItem('buckyos.prototype.locale.v1'))
      localStorage.setItem('buckyos.prototype.locale.v1', 'en')
  })
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text())
  })
  page.on('request', (req) => {
    if (req.url().includes('/kapi/')) errors.push(req.url())
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

test('home preserves three layers, owner identity and truthful settings and runtime controls', async ({
  page,
}) => {
  await page.goto('/?scenario=normal')
  await openAppService(page)
  await expect(
    page.getByRole('heading', { name: 'Applications', exact: true }),
  ).toBeVisible()
  await expect(
    page.getByRole('heading', { name: 'System Services' }),
  ).toBeVisible()
  await expect(
    page.getByRole('heading', { name: 'Kernel', exact: true }),
  ).toBeVisible()
  await expect(
    page.getByTestId('app-service-card-nostr-relay.buckyos.bns.did@alice'),
  ).toContainText('alice')
  await expect(
    page.getByTestId('app-service-card-nostr-relay.buckyos.bns.did@bob'),
  ).toContainText('bob')
  await page.screenshot({
    path: 'test-results/app-service-beta22/desktop-home.png',
    fullPage: true,
  })
  await page
    .getByTestId('app-service-card-home-assistant.buckyos.bns.did@alice')
    .click()
  await expect(page.getByText('Docker Engine')).toBeVisible()
  await expect(
    page.getByText('Settings are read only in this prototype.', {
      exact: false,
    }),
  ).toBeVisible()
  await expect(page.getByRole('button', { name: 'Save settings' })).toHaveCount(
    0,
  )
  await page.getByRole('button', { name: 'Start', exact: true }).click()
  await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
    'Installed · starting',
  )
  await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
    'Running normally',
  )
  await page.getByRole('button', { name: 'Stop', exact: true }).click()
  await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
    'Stopping',
  )
  await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
    'Not started',
  )
  await page.getByRole('button', { name: 'Open log' }).click()
  await expect(page.getByTestId('app-service-runtime-log')).toContainText(
    'No runtime log entries',
  )
})

test('local PIKG import, inspection, plan, sudo, task and independent runtime result', async ({
  page,
}) => {
  await openSource(page)
  await page.screenshot({
    path: 'test-results/app-service-beta22/desktop-source.png',
    fullPage: true,
  })
  await importPackage(page)
  const installer = page.getByTestId('app-installer-dialog')
  await expect(installer).not.toContainText('Highly trusted')
  await expect(installer.getByText('Free', { exact: true })).toHaveCount(0)
  await expect(installer.getByText('Latest', { exact: true })).toHaveCount(0)
  expect(Object.keys((await state(page)).tasks)).toHaveLength(0)
  await installer
    .getByRole('button', { name: 'Review installation plan' })
    .click()
  await authorize(page)
  await expect(page.getByTestId('app-installer-task-id')).toBeVisible()
  await expect(
    page.getByRole('heading', { name: 'Installation configuration published' }),
  ).toBeVisible({ timeout: 10000 })
  await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
    'Installed · starting',
  )
  await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
    'Running normally',
  )
  await page.getByRole('button', { name: 'View application' }).click()
  await expect(
    page.getByRole('heading', { name: 'Nextcloud', exact: true }),
  ).toBeVisible()
  await page.screenshot({
    path: 'test-results/app-service-beta22/desktop-detail.png',
    fullPage: true,
  })
})

test('local development is explicit and preserves unpublished authority', async ({
  page,
}) => {
  await openSource(page)
  await importPackage(page, 'developer')
  await expect(page.getByTestId('app-installer-blocking-reason')).toContainText(
    'LOCAL_DEVELOPER_REQUIRED',
  )
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await expect(page.getByTestId('app-installer-submit')).toBeDisabled()
  await page
    .getByLabel('Installation policy', { exact: true })
    .selectOption('LOCAL_DEVELOPER')
  await page.getByTestId('app-installer-recheck').click()
  await expect(page.getByTestId('app-installer-submit')).toBeEnabled()
  await expect(
    page.getByTestId('app-installer-developer-authority'),
  ).toContainText('does not establish public publication')
  await page
    .getByTestId('app-installer-trust-evidence')
    .locator('summary')
    .click()
  await expect(
    page.getByText('Not publicly published', { exact: true }),
  ).toBeVisible()
  await authorize(page)
  await expect(page.getByTestId('app-installer-task-id')).toBeVisible()
})
for (const scenario of ['revoked', 'migrated', 'tombstoned'])
  test(`local development cannot override ${scenario}`, async ({ page }) => {
    await openSource(page)
    await importPackage(page, scenario)
    await page.getByRole('button', { name: 'Review installation plan' }).click()
    await page
      .getByLabel('Installation policy', { exact: true })
      .selectOption('LOCAL_DEVELOPER')
    await page.getByTestId('app-installer-recheck').click()
    await expect(
      page.getByTestId('app-installer-blocking-reason'),
    ).toContainText(`IDENTITY_${scenario.toUpperCase()}`)
    await expect(page.getByTestId('app-installer-submit')).toBeDisabled()
  })

test('file validation, import failure, cancel and reference expiration are distinct', async ({
  page,
}) => {
  await openSource(page)
  await page
    .getByTestId('app-service-pikg-upload')
    .setInputFiles({
      name: 'fake.pikg',
      mimeType: 'application/octet-stream',
      buffer: Buffer.from('not-a-package'),
    })
  await expect(page.getByTestId('app-service-source-error')).toContainText(
    'file content is not a valid',
  )
  await page
    .getByTestId('app-service-pikg-upload')
    .setInputFiles({
      name: 'broken.pikg',
      mimeType: 'application/octet-stream',
      buffer: Buffer.from(
        JSON.stringify({
          format: 'buckyos-pikg-mock-v1',
          app: 'nextcloud',
          scenario: 'import-fail',
        }),
      ),
    })
  await expect(page.getByTestId('app-service-source-error')).toContainText(
    'could not be imported',
  )
  await page
    .getByRole('button', { name: 'Choose from Personal Server' })
    .click()
  await page
    .getByRole('dialog', { name: 'Choose from Personal Server' })
    .getByRole('button', { name: 'Cancel', exact: true })
    .click()
  await expect(page.getByTestId('app-service-source-error')).toContainText(
    'preparation was canceled',
  )
  await importPackage(page, 'expired-reference')
  await expect(page.getByTestId('app-installer-blocking-reason')).toContainText(
    'REFERENCE_EXPIRED',
  )
})

test('Personal Server uses internal references and closing preparation releases them', async ({
  page,
}) => {
  await openSource(page)
  await page
    .getByRole('button', { name: 'Choose from Personal Server' })
    .click()
  const picker = page.getByRole('dialog', {
    name: 'Choose from Personal Server',
  })
  await picker.getByRole('button', { name: /paperless-2.9.0/ }).click()
  await picker.getByRole('button', { name: 'Choose package' }).click()
  await expect(page.getByTestId('app-service-source-result')).toContainText(
    'Personal Server package',
  )
  await page.getByTestId('app-service-source-next').click()
  await expect(
    page.getByTestId('app-installer-install-readiness'),
  ).toBeVisible()
  await expect(page.getByTestId('app-installer-dialog')).not.toContainText(
    'staging_handle',
  )
  expect(page.url()).not.toMatch(/draft-|source-|pikg-stage-/)
  await page
    .getByTestId('app-installer-dialog')
    .getByRole('button', { name: 'Close', exact: true })
    .click()
  expect(Object.keys((await state(page)).drafts)).toHaveLength(0)
  expect(Object.keys((await state(page)).tasks)).toHaveLength(0)
})

test('local package does not imply offline readiness', async ({ page }) => {
  await openSource(page)
  await importPackage(page, 'content-missing', false)
  await expect(
    page.getByTestId('app-installer-install-readiness'),
  ).toContainText('Download required')
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await page.getByLabel('Offline mode: prohibit network acquisition').check()
  await page.getByTestId('app-installer-recheck').click()
  await expect(page.getByTestId('app-installer-blocking-reason')).toContainText(
    'OFFLINE_CONTENT_UNAVAILABLE',
  )
  await expect(page.getByTestId('app-installer-submit')).toBeDisabled()
})

for (const [fixture, label] of [
  ['backup-script', 'Process health'],
  ['home-dashboard', 'Deployment accessibility'],
  ['opendan', 'no Agent binding yet'],
])
  test(`${fixture} renders its own runtime dependencies`, async ({ page }) => {
    await page.goto('/?scenario=normal')
    await openAppService(page)
    await page
      .getByTestId(`app-service-card-${fixture}.buckyos.bns.did@alice`)
      .click()
    await expect(page.getByTestId('app-service-runtime-summary')).toContainText(
      label,
    )
    await expect(
      page.getByText('This service runs directly on the node'),
    ).toHaveCount(0)
  })
for (const [scenario, message] of [
  ['operation-fail', 'runtime operation failed'],
  ['operation-timeout', 'did not converge before timeout'],
])
  test(`runtime controls report ${scenario}`, async ({ page }) => {
    await page.goto(`/?scenario=normal&appServiceScenario=${scenario}`)
    await openAppService(page)
    await page
      .getByTestId('app-service-card-home-assistant.buckyos.bns.did@alice')
      .click()
    await page.getByRole('button', { name: 'Start', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText(message)
    await expect(page.getByTestId('app-service-runtime-status')).toHaveText(
      'Runtime status unknown',
    )
  })

test('multiple tasks persist independently and switching users or Zones cannot expose another cache', async ({
  page,
}) => {
  await openSource(page)
  await importPackage(page)
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await authorize(page)
  await expect(page.getByTestId('app-installer-task-id')).toBeVisible()
  const first = await page.getByTestId('app-installer-task-id').textContent()
  await page.getByRole('button', { name: 'Run in background' }).click()
  await page.getByRole('button', { name: 'Add app' }).click()
  await page
    .getByLabel('Enter a link or application identifier')
    .fill('paperless')
  await page.getByRole('button', { name: 'Check source', exact: true }).click()
  await expect(page.getByTestId('app-service-source-next')).toBeEnabled()
  await page.getByTestId('app-service-source-next').click()
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await authorize(page)
  await expect(page.getByTestId('app-installer-task-id')).toBeVisible()
  expect(Object.keys((await state(page)).tasks)).toHaveLength(2)
  await page.evaluate(() =>
    localStorage.setItem(
      'buckyos.app-service.context.v4',
      JSON.stringify({
        zone_id: 'prototype-zone',
        user_id: 'bob',
        role: 'user',
      }),
    ),
  )
  await page.goto(`/sysdlg/app_installer?task_id=${first}`)
  await expect(page.getByTestId('app-installer-launch-error')).toContainText(
    'TASK_NOT_FOUND',
  )
  await page.goto('/?scenario=normal')
  await openAppService(page)
  await expect(
    page.getByTestId('app-service-card-nostr-relay.buckyos.bns.did@alice'),
  ).toHaveCount(0)
  await expect(
    page.getByTestId('app-service-card-nostr-relay.buckyos.bns.did@bob'),
  ).toBeVisible()
  await expect(page.getByTestId('app-service-active-task')).toHaveCount(0)
  await page.evaluate(() =>
    localStorage.setItem(
      'buckyos.app-service.context.v4',
      JSON.stringify({
        zone_id: 'other-zone',
        user_id: 'alice',
        role: 'admin',
      }),
    ),
  )
  await page.goto(`/sysdlg/app_installer?task_id=${first}`)
  await expect(page.getByTestId('app-installer-launch-error')).toContainText(
    'TASK_NOT_FOUND',
  )
})

test('loading, empty and recoverable list errors remain explicit', async ({
  page,
}) => {
  await page.goto('/?scenario=normal&appServiceScenario=loading')
  await openAppService(page)
  await expect(page.getByLabel('Loading application services')).toBeVisible()
  await expect(
    page.getByRole('heading', { name: 'Applications', exact: true }),
  ).toBeVisible()
  await page.goto('/?scenario=normal&appServiceScenario=empty')
  await openAppService(page)
  await expect(
    page.getByRole('heading', { name: 'No applications are installed' }),
  ).toBeVisible()
  await page.goto('/?scenario=normal&appServiceScenario=error')
  await openAppService(page)
  await expect(
    page.getByRole('heading', { name: 'App Service data is unavailable' }),
  ).toBeVisible()
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(
    page.getByRole('heading', { name: 'Applications', exact: true }),
  ).toBeVisible()
})

test('Chinese source, verification, plan and authorization are translated', async ({
  page,
}) => {
  await page.goto('/')
  await page.evaluate(() =>
    localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'),
  )
  await page.reload()
  await page.getByRole('button', { name: '应用服务', exact: true }).click()
  await page.getByRole('button', { name: '添加应用' }).click()
  await expect(page.getByRole('heading', { name: '选择应用包' })).toBeVisible()
  await page.getByLabel('输入链接或应用标识').fill('nextcloud')
  await page.getByRole('button', { name: '检查来源', exact: true }).click()
  await expect(page.getByTestId('app-service-source-next')).toBeEnabled()
  await page.getByTestId('app-service-source-next').click()
  await page.getByRole('button', { name: '查看安装计划' }).click()
  await expect(page.getByTestId('app-installer-submit')).toHaveText(
    '确认并安装',
  )
  await page.getByTestId('app-installer-submit').click()
  await expect(page.getByLabel('密码', { exact: true })).toBeVisible()
})

test.describe('mobile App Service', () => {
  test.use({
    viewport: { width: 375, height: 812 },
    hasTouch: true,
    isMobile: true,
  })
  test('source and nested installer remain within the screen', async ({
    page,
  }) => {
    await openSource(page)
    await page.screenshot({
      path: 'test-results/app-service-beta22/mobile-source.png',
      fullPage: true,
    })
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBeTruthy()
    await importPackage(page)
    await expect(page.getByTestId('app-installer-dialog')).toBeVisible()
    expect(
      await page
        .getByTestId('app-installer-dialog')
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBeTruthy()
    await page.screenshot({
      path: 'test-results/app-service-beta22/mobile-check.png',
      fullPage: true,
    })
  })
})
