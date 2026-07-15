import { expect, test } from '@playwright/test'

test('round six operations bulk warnings and stale publish workflow', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })

  await page.goto('/bulk-operations')
  await expect(page.getByTestId('bulk-operations-page')).toBeVisible()
  await page.getByLabel(/Model id pattern/).fill('gpt-*')
  await page.getByLabel(/API type/).selectOption('llm')
  await page.getByLabel(/Bulk action/).selectOption('adjust_price_percent')
  await page.getByLabel(/Price percent/).fill('15')
  await expect(page.locator('body')).toContainText('Matched samples')
  await page.getByRole('button', { name: /Apply bulk operation/ }).click()
  await expect(page.locator('body')).toContainText('Bulk operation added to pending changes')

  await page.goto('/warnings')
  await expect(page.getByTestId('warnings-page')).toBeVisible()
  await page.getByLabel(/Search/).fill('overlay')
  await expect(page.locator('body')).toContainText(/Operations overlay|overlay/i)
  await page.getByRole('button', { name: /Locate/ }).first().click()
  await expect(page).toHaveURL(/\/providers|\/models|\/resolver-rules|\/tech-source/)

  await page.goto('/publish')
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await expect(page.locator('body')).toContainText('Operations publish counts')
  await expect(page.locator('body')).toContainText('Client driver metadata JSON')
  await page.getByRole('button', { name: /Simulate stale source/ }).click()
  await expect(page.locator('body')).toContainText('Stale')
  await page.getByLabel(/Publish note/).fill('Publish operations bulk pricing update')
  await page.getByLabel(/reviewed key field risks/i).check()
  await page.getByLabel(/writes a mock change log/i).check()
  await page.getByRole('button', { name: /^Publish$/ }).click()
  await expect(page.locator('body')).toContainText('Confirm stale publish')
  await page.getByLabel(/reviewed stale source status/i).check()
  await page.getByRole('button', { name: /^Publish$/ }).click()

  await expect(page.getByTestId('change-logs-page')).toBeVisible()
  await expect(page.locator('body')).toContainText('Publish operations bulk pricing update')

  expect(errors).toEqual([])
})
