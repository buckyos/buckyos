import { expect, test } from '@playwright/test'

const installTaskStorageKey = 'buckyos.app-service.install-task.v2'

test.beforeEach(async ({ page }) => {
  await page.addInitScript((storageKey) => {
    if (!window.sessionStorage.getItem('app-installer-test-ready')) {
      window.localStorage.removeItem(storageKey)
      window.sessionStorage.setItem('app-installer-test-ready', 'true')
    }
    window.localStorage.setItem('buckyos.prototype.locale.v1', 'en')
  }, installTaskStorageKey)
})

test('direct identifier launch creates a task, normalizes the URL, and resumes after reload', async ({ page }) => {
  const options = JSON.stringify({
    target: {
      node_id: 'ood-backup',
      node_did: 'did:cyfs:device:ood-backup',
    },
    install_params: { storage_profile: 'family', retention_days: 30 },
    offline: false,
  })
  const search = new URLSearchParams({
    identifier: 'did:bns:filebrowser.buckyos',
    ref: 'did:bns:store.buckyos',
    options,
  })

  await page.goto(`/sysdlg/app_installer?${search.toString()}`)

  await expect(page.getByTestId('app-installer-dialog')).toBeVisible()
  await expect(page).toHaveURL(/\/sysdlg\/app_installer\?task_id=[1-9]\d*$/)
  const sourceIdentity = page.getByTestId('app-installer-source-identity')
  await expect(sourceIdentity).not.toHaveAttribute('open', '')
  await sourceIdentity.locator('summary').click()
  await expect(page.getByText('did:bns:store.buckyos', { exact: true })).toBeVisible()

  const normalizedUrl = page.url()
  await page.getByRole('button', { name: 'Review installation plan' }).click()
  await expect(page.getByLabel('Target node')).toHaveCount(0)
  await expect(page.getByText('Components', { exact: true })).toHaveCount(0)
  await expect(page.getByText('Technical impact', { exact: true })).toHaveCount(0)
  await expect(page.getByTestId('app-installer-access-settings')).toBeVisible()
  await expect(page.getByTestId('app-installer-mount-settings')).toBeVisible()
  await expect(page.getByTestId('app-installer-environment-settings')).toBeVisible()
  await expect(page.getByTestId('app-installer-risky-params')).toContainText('High risk')
  await expect(page.getByTestId('app-installer-permissions')).toHaveAttribute('open', '')
  await page.getByLabel('Shortcut domain').selectOption('www')
  await page.getByLabel('Exposure route').selectOption('port')
  await expect(page.getByLabel('Exposed port')).toHaveValue('80')
  await page.getByRole('button', { name: 'Add mapping' }).click()
  await expect(page.getByText('Custom mapping', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Add variable' }).click()
  await page.getByLabel('Name', { exact: true }).fill('CUSTOM_VALUE')
  await page.getByLabel('Value', { exact: true }).last().fill('enabled')
  await expect(page.getByLabel('Permission granted by you').first()).toHaveValue('default')
  await expect(page.getByText('Caller-provided suggestions')).toBeVisible()
  await page.getByText('Caller-provided suggestions').click()
  await expect(page.getByText(/"storage_profile": "family"/).last()).toBeVisible()

  await page.reload()
  await expect(page).toHaveURL(normalizedUrl)
  await expect(page.getByTestId('app-installer-dialog')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Install application' })).toBeVisible()
})

test('direct launch rejects unknown, duplicate, and conflicting parameters', async ({ page }) => {
  await page.goto('/sysdlg/app_installer?identifier=nextcloud&auto_confirm=true')
  await expect(page.getByTestId('app-installer-launch-error')).toContainText('does not accept')

  await page.goto('/sysdlg/app_installer?identifier=nextcloud&identifier=files')
  await expect(page.getByTestId('app-installer-launch-error')).toContainText('more than once')

  await page.goto('/sysdlg/app_installer?task_id=123&identifier=nextcloud')
  await expect(page.getByTestId('app-installer-launch-error')).toContainText('exactly one')
})

test('offline direct launch blocks missing network content', async ({ page }) => {
  const search = new URLSearchParams({
    identifier: 'nextcloud',
    options: JSON.stringify({ offline: true }),
  })
  await page.goto(`/sysdlg/app_installer?${search.toString()}`)

  const blockingReason = page.getByTestId('app-installer-blocking-reason')
  await expect(blockingReason).toContainText('This application cannot be installed')
  await expect(blockingReason).toContainText('Content unavailable offline')
  await expect(page.getByRole('button', { name: 'Review installation plan' })).toHaveCount(0)
  await page.getByRole('button', { name: 'End', exact: true }).click()
  await expect(page).toHaveURL(/\/$/)
})

test.describe('mobile direct App Installer', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('remains usable without horizontal overflow', async ({ page }) => {
    await page.goto('/sysdlg/app_installer?identifier=nextcloud')
    await expect(page.getByTestId('app-installer-dialog')).toBeVisible()
    const hasOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)
    expect(hasOverflow).toBeFalsy()
  })
})
