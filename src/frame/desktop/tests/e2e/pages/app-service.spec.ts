import { expect, test, type Page } from '@playwright/test'

const installTaskStorageKey = 'buckyos.app-service.install-task.v2'

async function openAppService(page: Page) {
  const button = page.getByRole('button', { name: 'App Service' })
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
    await page.locator('body').dispatchEvent('pointerup', {
      bubbles: true,
      clientX: x + 5,
      clientY: y + 5,
      pointerId: 21,
      pointerType: 'touch',
    })
  } else {
    await button.click()
  }
  await expect(page.getByRole('heading', { name: 'App Service' })).toBeVisible()
  await expect(page.getByTestId('app-service-root')).toBeVisible()
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript((storageKey) => {
    window.localStorage.removeItem(storageKey)
    if (!window.localStorage.getItem('buckyos.prototype.locale.v1')) {
      window.localStorage.setItem('buckyos.prototype.locale.v1', 'en')
    }
  }, installTaskStorageKey)
})

test('App Service opens details, edits settings, controls runtime, and shows logs', async ({ page }) => {
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  await page.goto('/?scenario=normal')
  await openAppService(page)

  await page.getByTestId('app-service-card-app-home-assistant').click()
  await expect(page.getByRole('heading', { name: 'Home Assistant' })).toBeVisible()
  await expect(page.getByText('Docker Engine')).toBeVisible()

  await page.getByRole('button', { name: 'Edit settings' }).click()
  await page.getByLabel('timezone').fill('America/New_York')
  await page.getByRole('button', { name: 'Save settings' }).click()
  await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  await expect(page.getByText('America/New_York', { exact: true })).toBeVisible()

  await page.getByRole('button', { name: 'Start', exact: true }).click()
  await expect(page.getByText('Starting', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('Running', { exact: true }).first()).toBeVisible({ timeout: 5_000 })

  await page.getByRole('button', { name: 'Open log' }).click()
  await expect(page.getByTestId('app-service-runtime-log')).toContainText('Application started by administrator')
  expect(consoleErrors).toEqual([])
})

test('App Service normalizes a URL and completes the shared Installer task flow', async ({ page }) => {
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  await page.goto('/?scenario=normal')
  await openAppService(page)
  await page.getByRole('button', { name: 'Add app' }).click()

  const source = page.getByLabel('Installation source')
  await source.fill('https://apps.buckyos.ai/nextcloud/app-meta.jwt')
  await expect(page.getByTestId('app-service-source-result')).toContainText('App Meta URL')
  const next = page.getByTestId('app-service-source-next')
  await expect(next).toBeEnabled()
  await next.click()

  const installer = page.getByTestId('app-installer-dialog')
  await expect(installer).toContainText('System App Installer')
  await expect(installer).toContainText('Nextcloud')
  await expect(installer).toContainText('Authoritative publication')
  await installer.getByRole('button', { name: 'Review installation plan' }).click()

  await page.getByLabel('Administrator password').fill('prototype-admin')
  await page.getByRole('button', { name: 'Authorize and install' }).click()
  await expect(page.getByRole('heading', { name: 'Installing Nextcloud' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Installation complete' })).toBeVisible({ timeout: 10_000 })

  await page.getByRole('button', { name: 'View application' }).click()
  await expect(page.getByRole('heading', { name: 'Nextcloud' })).toBeVisible()
  await expect(page.getByText('nextcloud:28.0.2')).toBeVisible()
  expect(consoleErrors).toEqual([])
})

test('Installer explains a download failure and retries the same task', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await openAppService(page)
  await page.getByRole('button', { name: 'Add app' }).click()
  await page.getByLabel('Installation source').fill('https://apps.buckyos.ai/fail-download/app-meta.jwt')
  await expect(page.getByTestId('app-service-source-result')).toBeVisible()
  await page.getByTestId('app-service-source-next').click()
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await page.getByLabel('Administrator password').fill('prototype-admin')
  await page.getByRole('button', { name: 'Authorize and install' }).click()

  await expect(page.getByRole('heading', { name: 'Installation stopped' })).toBeVisible({ timeout: 5_000 })
  await expect(page.getByText('DOWNLOAD_FAILED')).toBeVisible()
  const taskId = await page.getByText(/^install_/).first().textContent()
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Installation complete' })).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText(taskId ?? '', { exact: true }).first()).toBeVisible()
})

test('Installer blocks trust resolution without mislabeling it as a download', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await openAppService(page)
  await page.getByRole('button', { name: 'Add app' }).click()
  await page.getByLabel('Installation source').fill('https://apps.buckyos.ai/trust-pending/app-meta.jwt')
  await expect(page.getByTestId('app-service-source-result')).toBeVisible()
  await page.getByTestId('app-service-source-next').click()

  await expect(page.getByTestId('app-installer-blocking-reason')).toContainText('TRUST_RESOLUTION_REQUIRED')
  await expect(page.getByRole('button', { name: 'Review installation plan' })).toBeDisabled()
  await expect(page.getByTestId('app-installer-blocking-reason')).not.toContainText('Download required')
})

test('source entry handles local and Personal Server PIKG references', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await openAppService(page)
  await page.getByRole('button', { name: 'Add app' }).click()

  await page.getByTestId('app-service-pikg-upload').setInputFiles({
    name: 'local-notes-1.0.0.pikg',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('mock-pikg'),
  })
  await expect(page.getByTestId('app-service-source-result')).toContainText('Local PIKG package')

  await page.getByRole('button', { name: 'Choose .pikg from Personal Server' }).click()
  const picker = page.getByRole('dialog', { name: 'Choose from Personal Server' })
  await picker.getByRole('button', { name: /paperless-2.9.0-aarch64.pikg/ }).click()
  await picker.getByRole('button', { name: 'Choose package' }).click()
  await expect(page.getByTestId('app-service-source-result')).toContainText('Personal Server PIKG package')
  await expect(page.getByTestId('app-service-source-next')).toBeEnabled()
})

test('Installer preserves success when automatic activation fails', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await openAppService(page)
  await page.getByRole('button', { name: 'Add app' }).click()
  await page.getByLabel('Installation source').fill('https://apps.buckyos.ai/activation-fail/app-meta.jwt')
  await expect(page.getByTestId('app-service-source-result')).toBeVisible()
  await page.getByTestId('app-service-source-next').click()
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await page.getByLabel('Administrator password').fill('prototype-admin')
  await page.getByRole('button', { name: 'Authorize and install' }).click()

  await expect(page.getByRole('heading', { name: 'Installed, but startup failed' })).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText('28.0.2', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'View application' }).click()
  await expect(page.getByText('Installed · start failed', { exact: true }).first()).toBeVisible()
})

test('home provides explicit loading, empty, and recoverable error states', async ({ page }) => {
  await page.goto('/?scenario=normal&appServiceScenario=loading')
  await openAppService(page)
  await expect(page.getByLabel('Loading application services')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Applications' })).toBeVisible()

  await page.goto('/?scenario=normal&appServiceScenario=empty')
  await openAppService(page)
  await expect(page.getByRole('heading', { name: 'No applications are installed' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'System Services' })).toBeVisible()

  await page.goto('/?scenario=normal&appServiceScenario=error')
  await openAppService(page)
  await expect(page.getByRole('heading', { name: 'App Service data is unavailable' })).toBeVisible()
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Applications' })).toBeVisible()
})

test('App Service renders the source flow in zh-CN', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await page.evaluate(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
  await page.reload()
  await page.getByRole('button', { name: '应用服务' }).click()
  await expect(page.getByRole('heading', { name: '应用服务', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '添加应用' }).click()
  await expect(page.getByRole('heading', { name: '添加应用', exact: true })).toBeVisible()
  await expect(page.getByLabel('安装来源')).toBeVisible()
})

test.describe('mobile App Service', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('keeps the source flow usable without horizontal overflow', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })

    await page.goto('/?scenario=normal')
    await openAppService(page)
    await page.getByRole('button', { name: 'Add app' }).click()
    await expect(page.getByLabel('Installation source')).toBeVisible()
    await expect(page.getByTestId('app-service-source-next')).toBeDisabled()

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)
    expect(overflow).toBeFalsy()
    expect(consoleErrors).toEqual([])
  })
})
