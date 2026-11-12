import { expect, test, type APIRequestContext, type Locator, type Page } from '@playwright/test'

interface SoftwarePackage {
  id: string
  name: string
  version: string
  ecosystem: string
  license: string | null
  description: string
  homepage: string | null
  repository: string | null
  licenseFileCount: number
}

async function readPackage(request: APIRequestContext, name: string, ecosystem: string) {
  const response = await request.get('/open-source/index.json')
  expect(response.ok()).toBeTruthy()
  const manifest = await response.json() as { packages: SoftwarePackage[] }
  const pkg = manifest.packages.find((item) => item.name === name && item.ecosystem === ecosystem)
  expect(pkg, `${name} must be included in the generated software inventory`).toBeDefined()
  return pkg!
}

function packageRow(page: Page, panel: Locator, pkg: SoftwarePackage) {
  return panel.getByTestId('settings-open-source-package')
    .filter({ has: page.getByText(pkg.name, { exact: true }) })
    .filter({ hasText: pkg.version })
}

async function openPanel(page: Page) {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const panel = page.getByTestId('settings-open-source')
  await expect(panel).toBeVisible()
  await expect(panel.getByTestId('settings-open-source-package').first()).toBeVisible()
  return panel
}

async function expectPackageDetails(page: Page, request: APIRequestContext, pkg: SoftwarePackage) {
  const dialog = page.getByRole('dialog')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('heading', { name: pkg.name, exact: true })).toBeVisible()
  await expect(dialog).toContainText(pkg.version)
  expect(pkg.license).toBeTruthy()
  await expect(dialog).toContainText(pkg.license!)
  if (pkg.description) await expect(dialog).toContainText(pkg.description)
  const links = [pkg.homepage, pkg.repository].filter((url): url is string => Boolean(url))
  for (const url of links) {
    await expect.poll(() => dialog.locator('a').evaluateAll((elements) => elements.map((element) => (element as HTMLAnchorElement).href))).toContain(new URL(url).href)
  }
  expect(pkg.licenseFileCount).toBeGreaterThan(0)
  const response = await request.get(`/open-source/licenses/${pkg.id[0]}.json`)
  expect(response.ok()).toBeTruthy()
  const licenses = await response.json() as Record<string, { files: Array<{ name: string; text: string }> }>
  expect(licenses[pkg.id].files.length).toBe(pkg.licenseFileCount)
  await expect(dialog.getByTestId('settings-open-source-license-text').locator('pre'))
    .toHaveText(licenses[pkg.id].files.map((file) => file.text))
  return dialog
}

test.describe('settings: open-source software', () => {
  test.use({ viewport: { width: 1280, height: 860 }, locale: 'en-US' })

  test('web and Rust dependencies show their resolved versions, licenses and license files', async ({ page, request }) => {
    const panel = await openPanel(page)
    const search = panel.getByRole('searchbox', { name: 'Search open-source software' })
    for (const [name, ecosystem] of [['react', 'npm'], ['serde', 'cargo']]) {
      const pkg = await readPackage(request, name, ecosystem)
      await search.fill(`  ${name.toUpperCase()}  `)
      const row = packageRow(page, panel, pkg)
      await expect(row).toContainText(pkg.license!)
      await row.click()
      await expectPackageDetails(page, request, pkg)
      await page.keyboard.press('Escape')
      await expect(page.getByRole('dialog')).toHaveCount(0)
      await expect(row).toBeFocused()
    }
  })

  test('search, pagination and an empty result can be reset without losing the list', async ({ page, request }, testInfo) => {
    const panel = await openPanel(page)
    const search = panel.getByRole('searchbox', { name: 'Search open-source software' })
    const rows = panel.getByTestId('settings-open-source-package')
    const firstPageName = await rows.first().textContent()
    expect(await rows.count()).toBeLessThanOrEqual(50)
    await panel.getByRole('button', { name: 'Next page', exact: true }).click()
    await expect(rows.first()).not.toHaveText(firstPageName!)
    const pkg = await readPackage(request, 'react', 'npm')
    await search.fill('react')
    await expect(packageRow(page, panel, pkg)).toBeVisible()
    await search.fill('no-such-open-source-package-xyz')
    await expect(rows).toHaveCount(0)
    await expect(panel.getByRole('status').filter({ hasText: 'No matching software' })).toBeVisible()
    await search.clear()
    await expect(rows.first()).toHaveText(firstPageName!)
    await expect(panel.getByRole('button', { name: 'Previous page', exact: true })).toBeDisabled()
    await panel.evaluate((element) => element.scrollIntoView({ block: 'start' }))
    await page.screenshot({ path: testInfo.outputPath('open-source-list.png'), animations: 'disabled' })
  })
})

test.describe('open-source software on a phone', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true, locale: 'zh-CN' })

  test('Chinese labels and long license content fit the phone viewport', async ({ page, request }, testInfo) => {
    await page.addInitScript(() => localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
    await page.goto('/?scenario=normal')
    const launcher = page.getByRole('button', { name: '设置', exact: true })
    await expect(launcher).toBeVisible()
    const box = (await launcher.boundingBox())!
    const pointer = { clientX: box.x + box.width / 2, clientY: box.y + box.height / 2, pointerId: 1, pointerType: 'touch', bubbles: true }
    await launcher.dispatchEvent('pointerdown', pointer)
    await page.locator('body').dispatchEvent('pointerup', pointer)
    await page.getByRole('button', { name: '通用', exact: true }).click()
    const panel = page.getByTestId('settings-open-source')
    await expect(panel.getByRole('heading', { name: '开源软件', exact: true })).toBeVisible()
    const pkg = await readPackage(request, 'react', 'npm')
    await panel.getByRole('searchbox', { name: '搜索开源软件' }).fill('react')
    await expect.poll(() => panel.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1)
    await packageRow(page, panel, pkg).click()
    const dialog = await expectPackageDetails(page, request, pkg)
    await expect(dialog.getByText('版本', { exact: true })).toBeVisible()
    await expect(dialog.getByText('许可证', { exact: true })).toBeVisible()
    await expect.poll(() => dialog.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1)
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(1)
    await page.screenshot({ path: testInfo.outputPath('open-source-mobile-detail.png'), animations: 'disabled' })
    await dialog.getByRole('button', { name: '关闭', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    await expect(panel.getByRole('searchbox', { name: '搜索开源软件' })).toHaveValue('react')
  })
})
