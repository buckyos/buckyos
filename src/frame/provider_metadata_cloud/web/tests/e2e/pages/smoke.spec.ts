import { expect, test } from '@playwright/test'

test('renders round one pages without console errors', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })

  await page.goto('/')
  await expect(page.getByTestId('dashboard-page')).toBeVisible()
  await page.getByRole('link', { name: /Providers/ }).click()
  await expect(page.getByTestId('providers-page')).toBeVisible()
  await page.getByRole('link', { name: /Models/ }).click()
  await expect(page.getByTestId('models-page')).toBeVisible()
  await page.getByRole('link', { name: /Import Plan/ }).click()
  await expect(page.getByTestId('import-plan-page')).toBeVisible()
  await page.getByRole('link', { name: /Publish Preview/ }).click()
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await page.getByRole('link', { name: /Change Logs/ }).click()
  await expect(page.getByTestId('change-logs-page')).toBeVisible()

  expect(errors).toEqual([])
})

test('happy path enters publish preview from edit action', async ({ page }) => {
  await page.goto('/providers')
  await page.getByRole('button', { name: /Enter edit/ }).click()
  await expect(page.getByText(/Edit/).first()).toBeVisible()
  await page.getByRole('button', { name: /Preview publish/ }).click()
  await expect(page.getByTestId('publish-page')).toBeVisible()
  await expect(page.getByText(/Edit action ready for publish preview/)).toBeVisible()
})

test('mobile single column browse shell renders', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 })
  await page.goto('/')
  await expect(page.getByTestId('dashboard-page')).toBeVisible()
  await expect(page.getByRole('navigation')).toBeVisible()
})
