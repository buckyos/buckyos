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

async function readPackages(request: APIRequestContext) {
  const response = await request.get('/open-source/index.json')
  expect(response.ok()).toBeTruthy()
  const manifest = await response.json() as { packages: SoftwarePackage[] }
  return manifest.packages
}

async function readPackage(request: APIRequestContext, name: string, ecosystem: string) {
  const packages = await readPackages(request)
  const pkg = packages.find((item) => item.name === name && item.ecosystem === ecosystem)
  expect(pkg, `${name} must be included in the generated software inventory`).toBeDefined()
  return pkg!
}

function projectRow(page: Page, panel: Locator, name: string) {
  return panel.getByTestId('settings-open-source-project')
    .filter({ has: page.getByText(name, { exact: true }) })
}

function componentRow(page: Page, dialog: Locator, pkg: SoftwarePackage) {
  return dialog.getByTestId('settings-open-source-component')
    .filter({ has: page.getByText(pkg.name, { exact: true }) })
    .filter({ hasText: pkg.version })
}

async function openPanel(page: Page) {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const panel = page.getByTestId('settings-open-source')
  await expect(panel).toBeVisible()
  await expect(panel.getByTestId('settings-open-source-project').first()).toBeVisible()
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

  test('projects retain component versions, licenses and license files', async ({ page, request }) => {
    const panel = await openPanel(page)
    const search = panel.getByRole('searchbox', { name: 'Search open-source software' })
    const packages = await readPackages(request)
    for (const [name, ecosystem] of [['react', 'npm'], ['serde', 'cargo']]) {
      await search.fill(`  ${name.toUpperCase()}  `)
      const row = projectRow(page, panel, name)
      await expect(row).toHaveCount(1)
      await row.click()
      const dialog = page.getByRole('dialog')
      await expect(dialog.getByRole('heading', { name, exact: true })).toBeVisible()
      const components = packages.filter((pkg) => pkg.ecosystem === ecosystem && (pkg.name === name || (name === 'react' && pkg.name === 'react-dom')))
      expect(components.length).toBeGreaterThan(0)
      for (const pkg of components) {
        const component = componentRow(page, dialog, pkg)
        await expect(component).toHaveCount(1)
        await expect(component).toContainText(pkg.license!)
        await component.click()
        await expectPackageDetails(page, request, pkg)
        await dialog.getByRole('button', { name: 'Back to group', exact: true }).click()
        await expect(componentRow(page, dialog, pkg)).toBeVisible()
      }
      await page.keyboard.press('Escape')
      await expect(page.getByRole('dialog')).toHaveCount(0)
      await expect(row).toBeFocused()
    }
  })

  test('proc-macro components appear as one group even when searching for one component', async ({ page, request }) => {
    const packages = await readPackages(request)
    const expectedNames = ['proc-macro-crate', 'proc-macro-error-attr3', 'proc-macro-error3', 'proc-macro2', 'proc-macro2-diagnostics']
    const components = packages.filter((pkg) => pkg.ecosystem === 'cargo' && expectedNames.includes(pkg.name))
    expect(new Set(components.map((pkg) => pkg.name))).toEqual(new Set(expectedNames))
    const panel = await openPanel(page)
    await panel.getByRole('searchbox', { name: 'Search open-source software' }).fill('proc-macro2')
    await expect(panel.getByTestId('settings-open-source-project')).toHaveCount(1)
    await projectRow(page, panel, 'proc-macro').click()
    const dialog = page.getByRole('dialog')
    for (const pkg of components) {
      await expect(componentRow(page, dialog, pkg)).toBeVisible()
      await componentRow(page, dialog, pkg).click()
      await expectPackageDetails(page, request, pkg)
      await dialog.getByRole('button', { name: 'Back to group', exact: true }).click()
    }
    await expect(dialog.getByTestId('settings-open-source-component')).toHaveCount(components.length)
    await dialog.getByRole('button', { name: 'Close', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    await expect(panel.getByRole('searchbox', { name: 'Search open-source software' })).toHaveValue('proc-macro2')
  })

  test('similar names share a family while component search retains precise license details', async ({ page, request }, testInfo) => {
    const panel = await openPanel(page)
    const search = panel.getByRole('searchbox', { name: 'Search open-source software' })
    const packages = await readPackages(request)
    for (const [name, ecosystem, familyName] of [['react-hook-form', 'npm', 'react'], ['serde_yaml', 'cargo', 'serde']]) {
      const pkg = packages.find((item) => item.name === name && item.ecosystem === ecosystem)
      expect(pkg).toBeDefined()
      const baseComponents = packages.filter((item) => item.name === familyName && item.ecosystem === ecosystem)
      expect(baseComponents.length).toBeGreaterThan(0)
      await search.fill(name)
      await expect(panel.getByTestId('settings-open-source-project')).toHaveCount(1)
      await projectRow(page, panel, familyName).click()
      const dialog = page.getByRole('dialog')
      await expect(dialog.getByRole('heading', { name: familyName, exact: true })).toBeVisible()
      const rows = dialog.getByTestId('settings-open-source-component')
      const totalComponents = await rows.count()
      expect(totalComponents).toBeGreaterThan(baseComponents.length)
      for (const base of baseComponents) await expect(componentRow(page, dialog, base)).toHaveCount(1)
      await expect(componentRow(page, dialog, pkg!)).toHaveCount(1)
      const componentSearch = dialog.getByRole('searchbox', { name: 'Search components' })
      await componentSearch.fill(name)
      await expect(componentRow(page, dialog, pkg!)).toBeVisible()
      await expect(rows).toHaveCount(packages.filter((item) => item.name.includes(name) && item.ecosystem === ecosystem).length)
      await componentRow(page, dialog, pkg!).click()
      await expectPackageDetails(page, request, pkg!)
      await dialog.getByRole('button', { name: 'Back to group', exact: true }).click()
      await expect(componentSearch).toHaveValue(name)
      await expect(componentRow(page, dialog, pkg!)).toBeVisible()
      await componentSearch.fill('no-such-family-component-xyz')
      await expect(rows).toHaveCount(0)
      await expect(dialog.getByRole('status', { name: '' })).toHaveText('No matching components')
      await componentSearch.clear()
      await expect(rows).toHaveCount(totalComponents)
      if (familyName === 'react') {
        await page.screenshot({ path: testInfo.outputPath('open-source-name-family.png'), animations: 'disabled' })
      }
      await dialog.getByRole('button', { name: 'Close', exact: true }).click()
      await expect(dialog).toHaveCount(0)
      await expect(search).toHaveValue(name)
    }
  })

  test('search, pagination and an empty result can be reset without losing the list', async ({ page }, testInfo) => {
    const panel = await openPanel(page)
    const search = panel.getByRole('searchbox', { name: 'Search open-source software' })
    const rows = panel.getByTestId('settings-open-source-project')
    const firstPageName = await rows.first().textContent()
    expect(await rows.count()).toBeLessThanOrEqual(50)
    await panel.getByRole('button', { name: 'Next page', exact: true }).click()
    await expect(rows.first()).not.toHaveText(firstPageName!)
    await search.fill('react')
    await expect(projectRow(page, panel, 'react')).toBeVisible()
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
    await projectRow(page, panel, 'react').click()
    const componentQuery = `${pkg.name} ${pkg.version}`
    await page.getByRole('dialog').getByRole('searchbox', { name: '搜索组件' }).fill(componentQuery)
    await componentRow(page, page.getByRole('dialog'), pkg).click()
    const dialog = await expectPackageDetails(page, request, pkg)
    await expect(dialog.getByText('版本', { exact: true })).toBeVisible()
    await expect(dialog.getByText('许可证', { exact: true })).toBeVisible()
    await expect.poll(() => dialog.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1)
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(1)
    await page.screenshot({ path: testInfo.outputPath('open-source-mobile-detail.png'), animations: 'disabled' })
    await dialog.getByRole('button', { name: '返回分组', exact: true }).click()
    await expect(dialog.getByRole('searchbox', { name: '搜索组件' })).toHaveValue(componentQuery)
    await expect(componentRow(page, dialog, pkg)).toBeVisible()
    await dialog.getByRole('button', { name: '关闭', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    await expect(panel.getByRole('searchbox', { name: '搜索开源软件' })).toHaveValue('react')
  })
})
