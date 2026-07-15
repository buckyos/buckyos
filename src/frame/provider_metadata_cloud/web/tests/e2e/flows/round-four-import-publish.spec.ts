import { expect, test } from '@playwright/test'

test('round four import plan draft recovery and publish workflow', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })

  await page.goto('/import-plan')
  await expect(page.getByTestId('import-plan-page')).toBeVisible()
  await page.getByRole('button', { name: /Save draft/ }).click()
  await expect(page.locator('body')).toContainText('Draft saved')
  await page.getByLabel(/YAML or Markdown/).fill(`# Unsupported plan
actions:
  - action: unsupported_future_action`)
  await page.getByRole('button', { name: /Import update plan/ }).click()
  await expect(page.locator('body')).toContainText('Unsupported action')
  await page.getByRole('button', { name: /Restore draft/ }).click()
  await expect(page.getByLabel(/YAML or Markdown/)).toHaveValue(/upsert_provider/)

  await page.getByRole('button', { name: /Import update plan/ }).click()
  await expect(page.locator('body')).toContainText('Actions were added to pending changes')
  await expect(page.getByRole('row', { name: /upsert_provider/ })).toBeVisible()
  await expect(page.getByRole('row', { name: /include_models/ })).toContainText('gpt-*')
  await expect(page.getByRole('row', { name: /delete_model_param_rule/ })).toContainText(/fall back/)
  await expect(page.getByRole('row', { name: /delete_api_type/ })).toContainText('embedding.text')
  await expect(page.getByRole('row', { name: /delete_capability/ })).toContainText('vision')
  await expect(page.getByRole('row', { name: /move_logical_directory/ })).toContainText('/llm/image')
  await expect(page.getByRole('row', { name: /set_capabilities/ })).toContainText('plan_cached_tokens')
  await expect(page.locator('body')).toContainText('19/19')

  await page.getByTestId('import-plan-page').getByRole('button', { name: /Preview publish/ }).click()
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await expect(page.getByText(/Publish Wizard/)).toBeVisible()
  await expect(page.locator('body')).toContainText('Client driver metadata JSON')
  await expect(page.locator('body')).toContainText('schema_version')
  await expect(page.locator('body')).toContainText('plan/gpt')
  await expect(page.locator('body')).toContainText('plan.chat')
  await page.getByRole('button', { name: /Simulate revision conflict/ }).click()
  await expect(page.getByTestId('revision-conflict-banner')).toBeVisible()
  await page.getByRole('button', { name: /Refresh preview/ }).click()
  await expect(page.getByTestId('revision-conflict-banner')).toHaveCount(0)
  await page.getByLabel(/Publish note/).fill('Publish imported provider metadata update')
  await page.getByLabel(/reviewed key field risks/i).check()
  await page.getByLabel(/writes a mock change log/i).check()
  await page.getByRole('button', { name: /^Publish$/ }).click()

  await expect(page.getByTestId('change-logs-page')).toBeVisible()
  await expect(page.locator('body')).toContainText('Publish imported provider metadata update')

  expect(errors).toEqual([])
})
