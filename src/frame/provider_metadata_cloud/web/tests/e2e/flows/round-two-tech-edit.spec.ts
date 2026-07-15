import { expect, test } from '@playwright/test'

test('round two tech edit flow reaches publish preview diagnostics', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })

  await page.goto('/providers')
  await page.getByRole('button', { name: /Create provider/ }).click()
  await expect(page.getByTestId('provider-wizard-page')).toBeVisible()
  await expect(page.getByText(/provider-\d+/)).toBeVisible()
  await page.getByLabel(/Name/).fill('OpenRouter Flow')
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByText(/Original providers/)).toBeVisible()
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByText(/Model parameter rules/)).toBeVisible()
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByText(/Earlier patterns have higher match priority/)).toBeVisible()
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByText(/Variant and version rule drafts/).first()).toBeVisible()
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByText(/Edit selected variant\/version rule/)).toBeVisible()
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByRole('heading', { name: /Logical mounts/ })).toBeVisible()
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByText(/Rewrite preview/)).toBeVisible()
  await page.getByRole('button', { name: /Next/ }).click()
  await expect(page.getByText(/Risk checks/)).toBeVisible()
  const saveAndPreview = page.getByRole('button', { name: /Save and preview/ })
  if (await saveAndPreview.count()) {
    await saveAndPreview.click()
  }
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await expect(page.locator('body')).toContainText('Client driver metadata JSON')
  await expect(page.locator('body')).toContainText('provider_driver')
  await expect(page.locator('body')).toContainText('openai-compatible')

  await page.getByRole('link', { name: /Models/ }).click()
  await expect(page.getByTestId('models-page')).toBeVisible()
  await page.getByRole('row', { name: /exact/ }).first().click()
  await expect(page.getByText(/Delete impact/)).toBeVisible()
  await page.getByRole('button', { name: /Delete exact rule/ }).click()

  await page.getByRole('link', { name: /Nick Rules/ }).click()
  await expect(page.getByTestId('nick-rules-page')).toBeVisible()
  await page.getByRole('button', { name: /Create nick rule/ }).click()
  const nickKeyText = await page.locator('aside').getByText(/nick-rule-\d+/).textContent()
  const nickKey = nickKeyText?.match(/nick-rule-\d+/)?.[0] ?? ''
  expect(nickKey).not.toBe('')
  await page.getByLabel(/Published id/).fill('flow/{model}')
  await page.getByTestId('nick-rules-page').getByRole('button', { name: /Save draft/ }).click()
  await expect(page.getByRole('row', { name: new RegExp(nickKey) })).toBeVisible()

  await page.getByRole('button', { name: /Preview publish/ }).click()
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await expect(page.getByText(/Key field risk area/)).toBeVisible()

  expect(errors).toEqual([])
})

test('provider edit wizard backfills current provider values', async ({ page }) => {
  await page.goto('/providers')
  await page.getByRole('button', { name: /Enter edit/ }).click()
  await page.getByRole('button', { name: /Edit OpenAI/ }).click()

  await expect(page.getByTestId('provider-wizard-page')).toBeVisible()
  await expect(page.locator('body')).toContainText('openai')
  await expect(page.getByLabel(/Name/)).toHaveValue('OpenAI')
  await expect(page.getByLabel(/Base URL/)).toHaveValue('')
  await expect(page.getByLabel(/Driver/)).toHaveValue('openai')
  await expect(page.getByLabel(/Protocol family/)).toHaveValue('openai-compatible')

  await page.getByRole('button', { name: /^Models$/ }).click()
  const selectedRules = page.locator('section').filter({ has: page.getByRole('heading', { name: /Selected match rules/ }) })
  await expect(selectedRules.locator('button').first()).toBeVisible()

  await page.getByRole('button', { name: /^Preview$/ }).click()
  await expect(page.getByText(/Risk checks/)).toBeVisible()
  await page.getByRole('button', { name: /Save and preview/ }).click()
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await expect(page.locator('body')).toContainText('openai')
})
