import { expect, test, type Page } from '@playwright/test'

function collectConsoleErrors(page: Page) {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })
  return errors
}

test('round seven mobile browse mode hides edit and publish actions', async ({ page }) => {
  const errors = collectConsoleErrors(page)
  await page.setViewportSize({ width: 375, height: 812 })
  await page.goto('/')

  await expect(page.getByTestId('dashboard-page')).toBeVisible()
  await expect(page.getByText(/Mobile browse only/)).toBeVisible()
  await expect(page.getByRole('button', { name: /Enter edit/ })).toHaveCount(0)
  await expect(page.getByRole('button', { name: /Preview publish/ })).toHaveCount(0)
  await expect(page.getByText(/Publishing is available on desktop only/)).toBeVisible()

  await page.getByRole('link', { name: /Providers/ }).click()
  await expect(page.getByTestId('providers-page')).toBeVisible()
  await expect(page.getByRole('button', { name: /Create provider/ })).toHaveCount(0)
  await expect(page.getByRole('button', { name: /From template/ })).toHaveCount(0)

  const hasHorizontalOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)
  expect(hasHorizontalOverflow).toBe(false)
  expect(errors).toEqual([])
})

test('round seven loading, error retry, empty, and stale states render', async ({ page }) => {
  const errors = collectConsoleErrors(page)
  const errorKey = `round-seven-${Date.now()}`

  await page.goto(`/?mockState=error-once&mockErrorKey=${errorKey}`, { waitUntil: 'domcontentloaded' })
  await expect(page.getByText(/Loading metadata workspace/)).toBeVisible()
  await expect(page.getByText(/Unable to load mock metadata/)).toBeVisible()
  await page.getByRole('button', { name: /Retry/ }).click()
  await expect(page.getByTestId('dashboard-page')).toBeVisible()

  await page.goto('/providers')
  await page.getByLabel(/Search/).fill('no-provider-matches-this-filter')
  await expect(page.getByText(/No records match the current filters/)).toBeVisible()

  await page.goto('/?mockState=stale')
  await expect(page.getByTestId('workspace-status-banner')).toBeVisible()
  await expect(page.getByText(/Technical source cache is stale/)).toBeVisible()

  expect(errors).toEqual([])
})

test('round seven i18n switch keeps filters and selected workspace state', async ({ page }) => {
  const errors = collectConsoleErrors(page)

  await page.goto('/providers')
  await page.getByLabel(/Search/).fill('openrouter')
  await expect(page.locator('tbody').getByText(/OpenRouter/).first()).toBeVisible()
  await page.getByLabel(/Language/).selectOption('zh-CN')

  await expect(page.getByRole('heading', { name: 'Providers' })).toBeVisible()
  await expect(page.getByLabel('搜索')).toHaveValue('openrouter')
  await expect(page.locator('tbody').getByText(/OpenRouter/).first()).toBeVisible()

  expect(errors).toEqual([])
})
