import { expect, test, type Page } from '@playwright/test'

async function openAiCenter(page: Page, chinese = false) {
  await page.addInitScript((locale) => localStorage.setItem('buckyos.prototype.locale.v1', locale), chinese ? 'zh-CN' : 'en')
  await page.goto('/?aiccScenario=populated')
  await page.getByTestId('desktop-app-ai-center').click()
}

async function openPage(page: Page, name: string) {
  await page.getByRole('button', { name, exact: true }).click()
}

function directory(page: Page, path: string) {
  return page.locator(`[data-testid="routing-directory"][data-path="${path}"]`)
}

test('routing observation, manual and simple adjustments, events', async ({ page }, testInfo) => {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
  await openAiCenter(page)
  await openPage(page, 'Routing')

  const rows = page.getByTestId('routing-directory')
  await expect(rows.first()).toBeVisible()
  const availableCount = await rows.count()
  await expect(page.locator('[data-testid="routing-directory"][data-available="false"]')).toHaveCount(0)
  await page.getByRole('radio', { name: 'All', exact: true }).click()
  await expect.poll(() => rows.count()).toBeGreaterThan(availableCount)
  await expect(page.locator('[data-testid="routing-directory"][data-available="false"]').first()).toBeVisible()
  await page.getByRole('radio', { name: 'Specifications', exact: true }).click()
  await expect(page.locator('[data-testid="routing-directory"]:not([data-kind="spec"])')).toHaveCount(0)
  await page.getByRole('radio', { name: 'Use cases', exact: true }).click()
  await expect(page.locator('[data-testid="routing-directory"]:not([data-kind="task"])')).toHaveCount(0)

  await directory(page, 'llm.chat').click()
  const winner = page.getByTestId('routing-winner-model')
  await expect(winner).toHaveText('claude-sonnet-4.5')
  await expect(page.getByTestId('routing-winner-reason')).not.toBeEmpty()
  await expect(page.locator('[data-testid="routing-tree-row"][data-state="expanded"]').first()).toBeVisible()
  await expect(page.locator('[data-testid="routing-tree-row"][data-target="llm.gpt-mini"]')).toHaveCount(0)
  await page.getByLabel('Show skipped items').check()
  await expect(page.locator('[data-testid="routing-tree-row"][data-target="llm.gpt-mini"]')).toHaveAttribute('data-state', 'not_expanded')
  await page.screenshot({ path: testInfo.outputPath('routing-candidates.png'), fullPage: true })

  await page.getByRole('button', { name: 'Advanced mode' }).click()
  const gpt = page.locator('[data-testid="routing-item"][data-item="gpt-standard"]')
  await gpt.getByRole('button', { name: 'Prefer' }).click()
  await expect(winner).toHaveText('gpt-5.1')
  await expect(page.getByTestId('routing-adjustments')).toContainText('1')
  await expect(gpt).toContainText('3.0')
  await page.screenshot({ path: testInfo.outputPath('routing-advanced.png'), fullPage: true })

  await openPage(page, 'Models')
  const anthropic = page.getByRole('region', { name: 'Anthropic', exact: true })
  const vendorFactor = anthropic.getByTestId('weight-factor').first()
  await vendorFactor.getByRole('spinbutton').fill('2')
  await vendorFactor.getByRole('button', { name: 'Apply' }).click()
  await expect(vendorFactor.getByTestId('weight-factor-value')).toHaveText('× 2.0')

  await openPage(page, 'Routing')
  await page.getByRole('radio', { name: 'Use cases', exact: true }).click()
  await directory(page, 'llm.chat').click()
  await expect(winner).toHaveText('claude-sonnet-4.5')
  await page.getByTestId('routing-adjustments').getByRole('button').first().click()
  await expect(page.getByTestId('routing-adjustments')).toContainText('Vendor claude')

  await openPage(page, 'Home')
  const events = page.getByTestId('aicc-events')
  await expect(events).toContainText('Routing adjustments saved (2 active)')
  await page.screenshot({ path: testInfo.outputPath('home-events.png'), fullPage: true })
  expect(errors).toEqual([])
})

test('routing page is localized', async ({ page }) => {
  await openAiCenter(page, true)
  await openPage(page, '路由')
  await expect(page.getByRole('heading', { name: '路由', exact: true })).toBeVisible()
  await expect(page.getByRole('radio', { name: '可用', exact: true })).toHaveAttribute('aria-checked', 'true')
  await page.locator('[data-testid="routing-directory"][data-path="llm.chat"]').click()
  await expect(page.getByText('当前胜出模型')).toBeVisible()
  await expect(page.getByRole('button', { name: '高级模式' })).toBeVisible()
})
