import { expect, test } from '@playwright/test'

test('AI Center exposes the complete catalog and expands multi-currency totals', async ({ page }) => {
  await page.goto('/?scenario=normal&aiccScenario=populated')
  await page.getByTestId('desktop-app-ai-center').click()

  const costCard = page.getByRole('button', { name: /Est\. Cost/ })
  await expect(costCard).toContainText('3 currencies')
  const collapsedCost = await costCard.innerText()
  expect(['USD', 'EUR', 'CNY'].filter((currency) => collapsedCost.includes(currency))).toHaveLength(1)
  await costCard.click()
  await expect(costCard).toContainText('USD')
  await expect(costCard).toContainText('EUR')
  await expect(costCard).toContainText('CNY')

  await page.getByRole('button', { name: 'Providers', exact: true }).click()
  await page.getByRole('button', { name: 'Add Provider' }).click()
  for (const provider of ['OpenAI', 'Claude', 'Gemini', 'fal', 'OpenRouter', 'MiniMax', 'Kimi', 'GLM', 'DeepSeek', 'Doubao', 'Qwen']) {
    await expect(page.getByRole('button', { name: new RegExp(provider, 'i') })).toBeVisible()
  }
  await expect(page.getByRole('button', { name: /Custom Provider/ })).toBeVisible()

  await page.getByRole('button', { name: /Qwen/i }).click()
  await page.getByRole('button', { name: 'Next' }).click()
  await expect(page.getByText('Region', { exact: true })).toBeVisible()
  await expect(page.getByText('Workspace', { exact: true })).toBeVisible()
})
