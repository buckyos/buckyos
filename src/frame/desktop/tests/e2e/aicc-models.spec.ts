import { expect, test, type Page } from '@playwright/test'

async function openModels(page: Page, scenario = 'populated', chinese = false) {
  await page.addInitScript((locale) => localStorage.setItem('buckyos.prototype.locale.v1', locale), chinese ? 'zh-CN' : 'en')
  await page.goto(`/?aiccScenario=${scenario}`)
  if ((page.viewportSize()?.width ?? 1440) < 768) await page.getByTestId('desktop-app-ai-center').tap()
  else await page.getByTestId('desktop-app-ai-center').click()
  await page.getByRole('button', { name: chinese ? '模型' : 'Models', exact: true }).click()
}

test('catalog cards, specifications, details and combined filters', async ({ page }, testInfo) => {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
  await openModels(page)
  await expect(page.getByTestId('model-card')).toHaveCount(7)
  await expect(page.locator('[data-model="gpt-5.6"]')).toHaveAttribute('data-available', 'false')
  await expect(page.locator('[data-model="gpt-5.1"]')).toHaveAttribute('data-available', 'true')
  await expect(page.locator('[data-model="qwen2.5-coder-32b"]')).toHaveAttribute('data-local', 'true')
  await page.locator('summary').filter({ hasText: 'gpt-standard' }).click()
  const spec = page.locator('details').filter({ has: page.locator('summary').filter({ hasText: 'gpt-standard' }) })
  await expect(spec.getByText('Weight', { exact: true })).toBeVisible()
  await expect(spec.getByText('Default', { exact: true })).toBeVisible()
  await spec.getByRole('button').filter({ hasText: 'gpt-5.6' }).click()
  await expect(page.getByRole('dialog')).toContainText('No provider is connected')
  await page.getByText('All Model Driver metadata', { exact: true }).click()
  await expect(page.getByRole('dialog').locator('pre')).toContainText('max_context_tokens')
  await page.keyboard.press('Escape')
  await page.getByRole('checkbox', { name: 'Locally deployable only', exact: true }).check()
  await expect(page.getByTestId('model-card')).toHaveCount(2)
  await page.getByRole('checkbox', { name: 'Available only', exact: true }).check()
  await page.getByRole('checkbox', { name: 'Deployed locally only', exact: true }).check()
  await expect(page.getByTestId('model-card')).toHaveCount(1)
  await page.getByRole('button', { name: 'Clear filters' }).click()
  await page.getByRole('textbox').fill('QWEN3.5')
  await expect(page.getByTestId('model-card')).toHaveCount(1)
  await page.getByTestId('model-card').click()
  await expect(page.getByRole('button', { name: 'Deploy locally', exact: true })).toBeDisabled()
  await expect(page.getByRole('dialog')).toContainText('future release')
  await page.keyboard.press('Escape')
  await page.getByRole('textbox').fill('no-such-model')
  await expect(page.getByText('No matching models', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Clear filters' }).click()
  await page.screenshot({ path: testInfo.outputPath('models-desktop.png') })
  expect(errors).toEqual([])
})

test('catalog remains browsable without providers', async ({ page }) => {
  await openModels(page, 'empty')
  await expect(page.getByTestId('model-card')).toHaveCount(7)
  await expect(page.locator('[data-available="true"]')).toHaveCount(0)
  await page.locator('summary').filter({ hasText: 'qwen-code' }).filter({ hasNotText: 'qwen-coder' }).click()
  await expect(page.getByText('No known models in this specification yet.')).toBeVisible()
  await page.getByRole('checkbox', { name: 'Available only', exact: true }).check()
  await expect(page.getByTestId('model-card')).toHaveCount(0)
})

test('refresh lights the local indicator only after local inventory appears', async ({ page }) => {
  await page.route('**/src/app/ai-center/mock/model-catalog.ts*', (route) => route.fulfill({
    contentType: 'application/javascript',
    body: `export function mockModelCatalog() {
      return { revision: 1, vendors: [{ id: 'qwen', revision: 1, specs: [], models: [{
        id: 'qwen3.5-27b', metadata: {local_deployable: true},
        providers: globalThis.localProviderReady ? [{id: 'local-engine', local: true, exact_models: ['alias@local-engine']}] : []
      }] }] };
    }`,
  }))
  await openModels(page)
  const card = page.getByTestId('model-card')
  await expect(card).toHaveAttribute('data-local', 'false')
  await expect(card).toHaveAttribute('data-available', 'false')
  await page.evaluate(() => { (globalThis as unknown as { localProviderReady: boolean }).localProviderReady = true })
  await page.getByRole('button', { name: 'Refresh model catalog', exact: true }).click()
  await expect(card).toHaveAttribute('data-local', 'true')
  await expect(card).toHaveAttribute('data-available', 'true')
  await page.getByRole('checkbox', { name: 'Deployed locally only', exact: true }).check()
  await expect(card).toHaveCount(1)
})

test.describe('touch viewport', () => {
  test.use({ hasTouch: true })

  test('mobile Chinese catalog and details fit the viewport', async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 375, height: 812 })
    await openModels(page, 'populated', true)
    await expect(page.getByTestId('model-card')).toHaveCount(7)
    const main = page.locator('main').filter({ has: page.getByTestId('model-card') }).last()
    expect(await main.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await page.getByRole('checkbox', { name: '只查看已经本地部署的模型', exact: true }).check()
    await page.getByTestId('model-card').click()
    await expect(page.getByRole('dialog')).toContainText('已本地部署')
    const box = await page.getByRole('dialog').boundingBox()
    expect(box!.x).toBeGreaterThanOrEqual(0)
    expect(box!.x + box!.width).toBeLessThanOrEqual(375)
    await expect(page.locator('.MuiDialog-container')).toHaveCSS('opacity', '1')
    await page.screenshot({ path: testInfo.outputPath('models-mobile-detail.png'), animations: 'disabled' })
  })

})

test('loading, error retry and empty catalog states', async ({ page }) => {
  await page.route('**/src/app/ai-center/mock/model-catalog.ts*', (route) => route.fulfill({
    contentType: 'application/javascript',
    body: `let attempts = 0; export async function mockModelCatalog() {
      if (attempts++ === 0) return await new Promise((_, reject) => { globalThis.rejectCatalog = () => reject(new Error('offline')); });
      return { revision: 1, vendors: [] };
    }`,
  }))
  await openModels(page)
  await expect(page.getByRole('status')).toContainText('Loading model catalog')
  await page.evaluate(() => (globalThis as unknown as { rejectCatalog: () => void }).rejectCatalog())
  await expect(page.getByRole('alert')).toContainText('Could not refresh')
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByText('No model metadata available', { exact: true })).toBeVisible()
})
