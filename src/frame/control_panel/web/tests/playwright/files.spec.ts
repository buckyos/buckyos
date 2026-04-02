import { expect, test } from '@playwright/test'

test('files standalone route smoke works in mock mode', async ({ page }) => {
  await page.goto('/files')

  await expect(page.getByTestId('files-root')).toBeVisible()
  await expect(page.getByTestId('files-tab-files')).toBeVisible()
  await expect(page.getByTestId('files-scope-browse')).toBeVisible()
  await expect(page.getByText('Documents')).toBeVisible()

  await page.getByTestId('files-search-input').fill('welcome.md')
  await page.getByTestId('files-search-button').click()
  await expect(page.getByText('Found 1 result(s) in /.')).toBeVisible()
  await expect(page.getByText('Welcome.md')).toBeVisible()

  await page.getByTestId('files-search-clear').click()
  await page.getByTestId('files-scope-recent').click()
  await expect(page.getByText('Welcome.md')).toBeVisible()

  await page.getByTestId('files-scope-starred').click()
  await expect(page.getByText('nebula.svg')).toBeVisible()

  await page.getByTestId('files-scope-trash').click()
  await expect(page.getByText('Draft.md')).toBeVisible()
})

test('public share route smoke works in mock mode', async ({ page }) => {
  await page.goto('/share/share-welcome')

  await expect(page.getByTestId('public-share-root')).toBeVisible()
  await expect(page.getByText('Shared with you')).toBeVisible()
  await expect(page.getByText('Share ID: share-welcome')).toBeVisible()
  await expect(page.getByText('/Documents/Welcome.md')).toBeVisible()
  await expect(page.getByText('This is the mock Files workspace for control_panel.')).toBeVisible()
})
