import { expect, test } from '@playwright/test'

test('round three resolver, directory, and dictionary flows stay mock-first', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })

  await page.goto('/resolver-rules')
  await expect(page.getByTestId('resolver-rules-page')).toBeVisible()
  await expect(page.locator('body')).toContainText(/Variants and version rules/)
  await page.getByRole('button', { name: /Enter edit/ }).click()
  await expect(page.getByLabel(/Variant key/)).toHaveCount(0)
  await page.getByRole('button', { name: /Add variant/ }).click()
  await page.getByLabel(/Model selector/).first().fill('gpt-*')
  await page.getByTestId('resolver-rules-page').getByRole('button', { name: /Save draft/ }).first().click()
  await expect(page.getByTestId('resolver-rules-page').getByRole('button', { name: /Save draft/ })).toHaveCount(0)
  await page.getByRole('button', { name: /Version rules/ }).click()
  await expect(page.getByLabel(/Version rule key/)).toHaveCount(0)
  await page.getByRole('button', { name: /Add version rule/ }).click()
  await page.getByLabel(/Model selector/).fill('gpt-*')
  await page.getByTestId('resolver-rules-page').getByRole('button', { name: /Save draft/ }).first().click()
  await expect(page.getByTestId('resolver-rules-page').getByRole('button', { name: /Save draft/ })).toHaveCount(0)
  await page.getByRole('button', { name: /Preview publish/ }).click()
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await expect(page.locator('body')).toContainText(/Pending changes: 4/)

  await page.getByRole('link', { name: /Logical Directory/ }).click()
  await expect(page.getByTestId('logical-directory-page')).toBeVisible()
  await page.getByLabel(/Search/).fill('gpt')
  await expect(page.getByText(/Search result mode is active/)).toBeVisible()
  await page.getByRole('button', { name: /Path browse/ }).click()
  await expect(page.getByText(/Directory tree/)).toBeVisible()
  await page.getByRole('button', { name: /Create directory/ }).click()
  await page.getByLabel(/Directory key/).fill('round3-empty')
  await page.getByLabel(/Path/).fill('/round3-empty')
  await page.getByLabel(/^Title$/).fill('Round 3 Empty')
  await page.getByTestId('logical-directory-page').getByRole('button', { name: /Save draft/ }).click()
  await expect(page.getByText(/Logical directory has no mounted model/)).toBeVisible()

  await page.getByRole('link', { name: /Dictionaries/ }).click()
  await expect(page.getByTestId('dictionaries-page')).toBeVisible()
  await expect(page.locator('option[value="streamming"]')).toHaveCount(0)
  await page.getByLabel(/Dictionary key/).last().selectOption('streaming')
  await page.getByRole('button', { name: /Apply selected key/ }).click()
  await expect(page.locator('body')).toContainText('Pending changes: 5')

  expect(errors).toEqual([])
})
