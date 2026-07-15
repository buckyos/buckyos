import { expect, test } from '@playwright/test'

test('round five operations source sync and overlay edit workflow', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })

  await page.goto('/')
  await page.getByRole('button', { name: /Operations parameters/ }).click()

  await page.goto('/tech-source')
  await expect(page.getByTestId('tech-source-page')).toBeVisible()
  await page.getByLabel(/Technical parameter service URL/).fill('https://metadata.ops-source.mock/kapi/provider-metadata-tech-service')
  await page.getByRole('button', { name: /Save draft/ }).click()
  await expect(page.locator('body')).toContainText('metadata.ops-source.mock')
  await page.getByRole('button', { name: /Test connection/ }).click()
  await page.getByRole('button', { name: /Manual sync/ }).click()
  await expect(page.locator('body')).toContainText('Cache usable')

  await page.goto('/providers')
  await page.getByRole('button', { name: /Operations parameters/ }).click()
  await expect(page.getByTestId('ops-providers-page')).toBeVisible()
  await expect(page.getByRole('button', { name: /Create provider/ })).toHaveCount(0)
  await expect(page.getByRole('button', { name: /Save draft/ })).toHaveCount(0)
  await page.getByRole('row', { name: /OpenRouter/ }).click()
  await expect(page.locator('body')).toContainText('Providers are read-only for operations parameters')
  await expect(page.locator('body')).toContainText('Operations Providers')

  await page.goto('/models')
  await page.getByRole('button', { name: /Operations parameters/ }).click()
  await expect(page.getByTestId('ops-models-page')).toBeVisible()
  await expect(page.getByRole('button', { name: /Create exact rule/ })).toHaveCount(0)
  await page.getByRole('row').nth(1).click()
  await page.getByLabel(/Input price/).fill('0.000002')
  await page.getByLabel(/Output price/).fill('0.000006')
  await page.getByLabel(/Routing weight/).fill('72')
  await page.getByLabel(/Cost class/).selectOption('medium')
  await page.getByLabel(/Latency class/).selectOption('fast')
  await page.getByLabel(/Quality score/).fill('92')
  await page.getByLabel(/Rollout strategy/).selectOption('canary')
  await page.getByLabel(/Operations note/).fill('Canary rollout with price override')
  await page.getByRole('button', { name: /Save draft/ }).click()
  await expect(page.locator('body')).toContainText('Operations Models')

  await page.goto('/resolver-rules')
  await page.getByRole('button', { name: /Operations parameters/ }).click()
  await expect(page.getByTestId('ops-resolver-page')).toBeVisible()
  await expect(page.getByRole('button', { name: /Save draft/ })).toHaveCount(0)
  await page.getByRole('row').nth(1).click()
  await expect(page.locator('body')).toContainText('Variants and version rules are read-only for operations parameters')
  await expect(page.locator('body')).toContainText('Resolver Operations Overlay')

  expect(errors).toEqual([])
})
