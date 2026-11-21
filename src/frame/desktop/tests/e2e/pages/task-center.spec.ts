import { expect, test, type Page } from '@playwright/test'

function trackConsoleErrors(page: Page): string[] {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(error.message))
  return errors
}

const row = (page: Page, title: string) => page.getByRole('treeitem').filter({ hasText: title })
const panel = (page: Page) => page.getByTestId('task-detail-panel')

async function openTasks(page: Page) {
  await page.goto('/taskcenter')
  await page.getByRole('button', { name: 'Tasks', exact: true }).click()
  await expect(page.getByTestId('task-tree')).toBeVisible()
}

test.describe('desktop', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('the task list is a tree: collapsed by default, a toggle shows the subtasks', async ({ page }) => {
    const errors = trackConsoleErrors(page)
    await openTasks(page)
    const install = row(page, 'Install BuckyOS System Update')
    await expect(install).toHaveAttribute('aria-expanded', 'false')
    await expect(row(page, 'Apply system patches')).toHaveCount(0)

    await install.getByRole('button', { name: 'Show subtasks' }).click()
    await expect(install).toHaveAttribute('aria-expanded', 'true')
    await expect(row(page, 'Apply system patches')).toHaveAttribute('aria-level', '2')
    // The toggle does not open the task.
    await expect(panel(page)).toHaveCount(0)

    await install.getByRole('button', { name: 'Hide subtasks' }).click()
    await expect(row(page, 'Apply system patches')).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('a click opens the detail beside the list, and another click switches it without moving the list', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 640 })
    await openTasks(page)
    await row(page, 'Sync Photos to Cloud Backup').click()
    await expect(panel(page).getByRole('heading', { name: 'Sync Photos to Cloud Backup' })).toBeVisible()
    await expect(row(page, 'Sync Photos to Cloud Backup')).toHaveAttribute('aria-selected', 'true')

    const main = page.locator('main main')
    await main.evaluate((node) => node.scrollTo({ top: 200 }))
    await row(page, 'Download Large Dataset').click()
    await expect(panel(page).getByRole('heading', { name: 'Download Large Dataset' })).toBeVisible()
    expect(await main.evaluate((node) => node.scrollTop)).toBe(200)
    await expect(row(page, 'Sync Photos to Cloud Backup')).toHaveAttribute('aria-selected', 'false')

    await page.getByRole('button', { name: 'Close details' }).click()
    await expect(panel(page)).toHaveCount(0)
  })

  test('subtasks in the detail open in place; the tree follows and "Part of" goes back up', async ({ page }) => {
    await openTasks(page)
    await row(page, 'Install BuckyOS System Update').click()
    await panel(page).getByTestId('task-step').filter({ hasText: 'Apply system patches' }).click()
    await expect(panel(page).getByRole('heading', { name: 'Apply system patches' })).toBeVisible()
    await expect(row(page, 'Install BuckyOS System Update')).toHaveAttribute('aria-expanded', 'true')
    await expect(row(page, 'Apply system patches')).toHaveAttribute('aria-selected', 'true')

    await panel(page).getByTestId('task-parent').click()
    await expect(panel(page).getByRole('heading', { name: 'Install BuckyOS System Update v2.4.1' })).toBeVisible()
  })

  test('arrow keys move through the tree and the open detail follows', async ({ page }) => {
    await openTasks(page)
    await row(page, 'Install BuckyOS System Update').click()
    await page.keyboard.press('ArrowRight')
    await expect(row(page, 'Install BuckyOS System Update')).toHaveAttribute('aria-expanded', 'true')
    await page.keyboard.press('ArrowDown')
    await expect(panel(page).getByRole('heading', { name: 'Download update package' })).toBeVisible()
    await page.keyboard.press('ArrowLeft')
    await expect(panel(page).getByRole('heading', { name: 'Install BuckyOS System Update v2.4.1' })).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(panel(page)).toHaveCount(0)
  })

  test('a search that hits a subtask keeps its parent and opens it', async ({ page }) => {
    await openTasks(page)
    await page.getByRole('searchbox', { name: 'Search tasks' }).fill('epoch')
    await expect(page.getByRole('treeitem')).toHaveCount(2)
    await expect(row(page, 'AI Model Fine-tuning Job')).toHaveAttribute('aria-expanded', 'true')
    await expect(row(page, 'Model training - Epoch 2/15')).toBeVisible()
  })

  test('a deep link to a subtask opens the tree to it', async ({ page }) => {
    await page.goto('/taskcenter?taskid=task-003-2')
    await expect(row(page, 'Model training - Epoch 2/15')).toHaveAttribute('aria-selected', 'true')
    await expect(panel(page).getByTestId('task-parent')).toContainText('AI Model Fine-tuning Job')
  })

  test('home, scheduled tasks and events open the same side panel', async ({ page }) => {
    await page.goto('/taskcenter')
    await page.getByRole('button', { name: /Sync Photos to Cloud Backup/ }).click()
    await expect(panel(page).getByRole('heading', { name: 'Sync Photos to Cloud Backup' })).toBeVisible()

    await page.getByRole('button', { name: 'Scheduled tasks', exact: true }).click()
    await expect(panel(page)).toHaveCount(0)
    await page.getByTestId('schedule-row').filter({ hasText: 'cleanup-temp-files' }).click()
    await expect(panel(page).getByRole('heading', { name: 'workflow/schedule/cleanup-temp-files' })).toBeVisible()

    await page.getByRole('button', { name: 'System events', exact: true }).click()
    await page.getByRole('button', { name: /Package downloaded, verifying next/ }).click()
    await expect(panel(page).getByRole('heading', { name: 'Download update package' })).toBeVisible()
  })

  test('in a narrow window the panel covers the list instead of squeezing it', async ({ page }) => {
    await page.setViewportSize({ width: 860, height: 700 })
    await page.goto('/taskcenter?taskid=task-001')
    await expect(panel(page)).toHaveAttribute('data-overlay', '')
    await page.setViewportSize({ width: 1280, height: 700 })
    await expect(panel(page)).not.toHaveAttribute('data-overlay', '')
  })
})

test.describe('phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

  test('a tap opens the detail page; back returns to the tree as it was', async ({ page }) => {
    await openTasks(page)
    await row(page, 'Install BuckyOS System Update').getByRole('button', { name: 'Show subtasks' }).click()
    await row(page, 'Verify package integrity').click()
    await expect(page.getByRole('heading', { name: 'Verify package integrity' })).toBeVisible()
    await expect(page.getByTestId('task-tree')).toBeHidden()

    await page.getByRole('button', { name: 'Tasks', exact: true }).last().click()
    await expect(page.getByTestId('task-tree')).toBeVisible()
    await expect(row(page, 'Verify package integrity')).toBeVisible()
  })
})

test('an old active task updates through Waiting and completion and is removed when access is revoked', async ({ page }) => {
  await page.goto('/taskcenter')
  const result = await page.evaluate(async () => {
    const path = '/tests/e2e/fixtures/task-center-rpc.ts'
    const { pollOlderTask } = await import(path)
    return pollOlderTask()
  })
  expect(result).toEqual({ states: ['Running', 'Running', 'Waiting', 'Terminal', null], shared: true, sharedReads: 1 })
})
