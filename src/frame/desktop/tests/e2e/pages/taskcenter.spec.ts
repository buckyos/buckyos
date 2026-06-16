import { expect, test, type Page } from '@playwright/test'

function trackConsoleErrors(page: Page) {
  const errors: string[] = []

  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text())
    }
  })

  page.on('pageerror', (error) => {
    errors.push(error.message)
  })

  return errors
}

test('taskcenter standalone route renders dashboard and handles notifications', async ({ page }) => {
  const consoleErrors = trackConsoleErrors(page)

  await page.goto('/taskcenter')

  await expect(page.getByRole('button', { name: /Home/ })).toBeVisible()
  await expect(page.getByRole('heading', { name: /Running Tasks \(\d+\)/ })).toBeVisible()
  await expect(page.getByText('Install BuckyOS System Update v2.4.1')).toBeVisible()
  await expect(page.getByText('Agent Authorization Required')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'System Notifications (3)' })).toBeVisible()

  await page.getByRole('button', { name: 'Approve' }).click()

  await expect(page.getByText('Agent Authorization Required')).toBeHidden()
  await expect(page.getByRole('heading', { name: 'System Notifications (2)' })).toBeVisible()
  expect(consoleErrors).toEqual([])
})

test('taskcenter scheduled tasks page exposes schedule state and task details', async ({ page }) => {
  const consoleErrors = trackConsoleErrors(page)

  await page.goto('/taskcenter')
  await page.getByRole('button', { name: /Scheduled Tasks/ }).click()

  await expect(page.getByRole('button', { name: /weekly-full-backup Enabled/ })).toBeVisible()
  await expect(page.getByText('sch-weekly-full-backup')).toBeVisible()
  await expect(page.getByRole('button', { name: /scan-new-images Enabled/ })).toBeVisible()
  await expect(page.getByRole('button', { name: /cleanup-temp-files Error/ })).toBeVisible()
  await expect(page.getByText('3 failures', { exact: true })).toBeVisible()

  await page.getByPlaceholder('Search schedules...').fill('cleanup')
  await expect(page.getByRole('button', { name: /cleanup-temp-files Error/ })).toBeVisible()
  await expect(page.getByRole('button', { name: /weekly-full-backup Enabled/ })).toBeHidden()

  await page.getByPlaceholder('Search schedules...').fill('')
  await page.getByRole('button', { name: /weekly-full-backup/ }).click()

  await expect(page.getByRole('button', { name: 'Back to Scheduled Tasks' })).toBeVisible()
  await expect(page.getByText('Task ID', { exact: true })).toBeVisible()
  await expect(page.getByText('task-006', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('sch-weekly-full-backup')).toBeVisible()

  await page.getByRole('button', { name: 'Back to Scheduled Tasks' }).click()
  await expect(page.getByRole('button', { name: /weekly-full-backup Enabled/ })).toBeVisible()
  expect(consoleErrors).toEqual([])
})

test('taskcenter task detail supports taskid deep link', async ({ page }) => {
  const consoleErrors = trackConsoleErrors(page)

  await page.goto('/taskcenter?taskid=task-006')

  await expect(page.getByRole('button', { name: 'Back to Tasks' })).toBeVisible()
  await expect(page.getByText('workflow/schedule/weekly-full-backup')).toBeVisible()
  await expect(page.getByText('Root Task ID')).toBeVisible()
  await expect(page.getByText('task-006', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('Extended Data')).toBeVisible()

  await page.getByRole('button', { name: 'Back to Tasks' }).click()
  await expect(page.getByPlaceholder('Search tasks...')).toBeVisible()
  expect(consoleErrors).toEqual([])
})

test('taskcenter task detail renders approval schema actions', async ({ page }) => {
  const consoleErrors = trackConsoleErrors(page)

  await page.goto('/taskcenter?taskid=task-008')

  await expect(page.getByRole('heading', { name: 'Agent Authorization' })).toBeVisible()
  await expect(
    page.getByText('DataBot wants to continue the pipeline with access to /private/documents.', { exact: true }),
  ).toBeVisible()

  await page.getByRole('button', { name: 'Approve access' }).click()

  await expect(page.getByText(/^Submitted\b/)).toBeVisible()
  await expect(page.getByRole('button', { name: 'Approve access' })).toBeDisabled()
  await expect(page.getByText('"kind": "approve"')).toBeVisible()
  expect(consoleErrors).toEqual([])
})

test('taskcenter task detail renders comment schema interaction', async ({ page }) => {
  const consoleErrors = trackConsoleErrors(page)

  await page.goto('/taskcenter?taskid=task-012')

  await expect(page.getByRole('heading', { name: 'Review Request' })).toBeVisible()
  await page.getByLabel('Response').fill('Looks good. Run it during the maintenance window.')
  await page.getByRole('button', { name: 'Submit response' }).click()

  await expect(page.getByText(/^Submitted\b/)).toBeVisible()
  await expect(page.getByRole('button', { name: 'Submit response' })).toBeDisabled()
  await expect(page.getByText('"kind": "submit_output"')).toBeVisible()
  await expect(page.getByText('"comment": "Looks good. Run it during the maintenance window."')).toBeVisible()
  expect(consoleErrors).toEqual([])
})
