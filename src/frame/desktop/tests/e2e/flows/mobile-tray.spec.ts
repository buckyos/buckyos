import { expect, test, type Page } from '@playwright/test'

test.use({
  viewport: { width: 390, height: 844 },
  hasTouch: true,
  isMobile: true,
})

function trackConsoleErrors(page: Page): string[] {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(error.message))
  return errors
}

test('message tray lists unread conversations, hides their content on request and opens the session', async ({ page }) => {
  const errors = trackConsoleErrors(page)
  await page.goto('/?scenario=normal')

  const button = page.getByTestId('status-tray-messages-button')
  const badge = page.getByTestId('status-tray-messages-badge')
  await expect(badge).toBeVisible()
  const unread = Number(await badge.textContent())
  expect(unread).toBeGreaterThan(0)
  // The red count of messages, not the blue one of notifications.
  expect(await badge.evaluate((node) => getComputedStyle(node).backgroundColor)).not.toBe(
    await page.getByTestId('status-tray-notifications-badge').evaluate((node) => getComputedStyle(node).backgroundColor),
  )

  await button.click()
  const panel = page.getByTestId('status-tray-messages-panel')
  await expect(panel).toBeVisible()
  const alice = panel.getByTestId('tray-message-row').filter({ hasText: 'Alice Chen' })
  await expect(alice.getByTestId('tray-row-text')).toHaveText('And the brief for Friday.')
  // System notifications are counted by the bell, not here.
  await expect(panel.getByText('System notifications')).toHaveCount(0)

  // Content hidden: who wrote and how many, nothing more; kept across reloads.
  await panel.getByTestId('tray-preview-toggle').click()
  await expect(alice.getByTestId('tray-row-text')).toHaveText('1 new message')
  await page.reload()
  await page.getByTestId('status-tray-messages-button').click()
  const reloadedAlice = page.getByTestId('status-tray-messages-panel').getByTestId('tray-message-row').filter({ hasText: 'Alice Chen' })
  await expect(reloadedAlice.getByTestId('tray-row-text')).toHaveText('1 new message')

  const sessionId = await reloadedAlice.getAttribute('data-session-id')
  await reloadedAlice.click()
  await expect(page).toHaveURL(new RegExp(`/messagehub\\?entityId=.+&sessionId=${sessionId}`))
  await expect(page.getByText('And the brief for Friday.').first()).toBeVisible()
  expect(errors).toEqual([])
})

test('the message preview setting lives in Settings > Appearance', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await page.evaluate(() => window.localStorage.setItem('buckyos.desktop.messagePreview', 'hidden'))
  await page.reload()
  await page.getByTestId('status-tray-messages-button').click()
  const alice = page.getByTestId('tray-message-row').filter({ hasText: 'Alice Chen' })
  await expect(alice.getByTestId('tray-row-text')).toHaveText('1 new message')
  await page.keyboard.press('Escape')

  await page.getByRole('button', { name: 'BuckyOS' }).first().click()
  await page.getByTestId('mobile-taskbar').getByTestId('taskbar-app-settings').click()
  await page.getByText('Appearance', { exact: true }).first().click()
  const toggle = page.getByRole('switch', { name: 'Show message content in the status bar' })
  await expect(toggle).not.toBeChecked()
  await toggle.check()
  expect(await page.evaluate(() => window.localStorage.getItem('buckyos.desktop.messagePreview'))).toBe('shown')
})

test('the bell lists open notifications and opens their MessageHub card', async ({ page }) => {
  const errors = trackConsoleErrors(page)
  await page.goto('/?scenario=normal')

  const badge = page.getByTestId('status-tray-notifications-badge')
  await expect(badge).toBeVisible()
  await page.getByTestId('status-tray-notifications-button').click()
  const panel = page.getByTestId('status-tray-notifications-panel')
  const rows = panel.getByTestId('tray-notify-row')
  await expect(rows.first()).toBeVisible()
  await expect(badge).toHaveText(String(await rows.count()))
  // Closed notifications (dismissed, answered, expired) are not listed.
  await expect(panel.getByText('Disk space is low')).toHaveCount(0)
  await expect(panel.getByText('Needs your answer').first()).toBeVisible()

  const backup = rows.filter({ hasText: 'Backup finished' })
  await expect(backup).toHaveAttribute('data-notify-id', 'nt-app-backup')
  await backup.click()
  await expect(page).toHaveURL(/\/messagehub\?notify=nt-app-backup/)
  await expect(page.getByText('Backup finished').first()).toBeVisible()
  expect(errors.filter((text) => !text.includes('flushSync'))).toEqual([])
})

test('a download started in the Task Center shows its progress in the bell until dismissed', async ({ page }) => {
  test.setTimeout(60_000)
  const errors = trackConsoleErrors(page)
  await page.goto('/?scenario=normal')
  const badge = page.getByTestId('status-tray-notifications-badge')
  await expect(badge).toBeVisible()
  const before = Number(await badge.textContent())

  await page.goto('/taskcenter')
  await page.getByTestId('task-center-new-download').click()
  const submit = page.getByTestId('task-download-submit')
  await page.getByTestId('task-download-url').fill('ftp://example.com/a.iso')
  await expect(submit).toBeDisabled()
  await page.getByTestId('task-download-url').fill('https://example.com/files/release-notes.pdf')
  await submit.click()
  await expect(page.getByTestId('task-download-info')).toContainText('https://example.com/files/release-notes.pdf')

  await page.goto('/?scenario=normal')
  await expect(badge).toHaveText(String(before + 1))
  await page.getByTestId('status-tray-notifications-button').click()
  const row = page.getByTestId('tray-task-row').filter({ hasText: 'Download release-notes.pdf' })
  await expect(row).toHaveAttribute('data-state', 'running')
  await expect(row.getByRole('progressbar')).toBeVisible()

  await expect(row).toHaveAttribute('data-state', 'completed', { timeout: 20_000 })
  await expect(row.getByTestId('tray-row-text')).toHaveText('Completed')
  await expect(badge).toHaveText(String(before + 1))

  await row.getByRole('button', { name: 'Dismiss' }).click()
  await expect(row).toHaveCount(0)
  await expect(badge).toHaveText(String(before))
  expect(errors).toEqual([])
})

test('a running download opens its Task Center page, which can cancel it', async ({ page }) => {
  await page.goto('/taskcenter')
  await page.getByTestId('task-center-new-download').click()
  await page.getByTestId('task-download-url').fill('https://example.com/big/dataset.tar.gz')
  await page.getByTestId('task-download-submit').click()
  await expect(page.getByTestId('task-download-info')).toBeVisible()

  await page.goto('/?scenario=normal')
  await page.getByTestId('status-tray-notifications-button').click()
  const row = page.getByTestId('tray-task-row').filter({ hasText: 'Download dataset.tar.gz' })
  const taskId = await row.getAttribute('data-task-id')
  await row.getByRole('button').first().click()
  await expect(page).toHaveURL(new RegExp(`/taskcenter\\?taskid=${taskId}`))

  await page.getByTestId('task-cancel').click()
  await expect(page.getByText('cancelled', { exact: true }).first()).toBeVisible()
  await page.goto('/?scenario=normal')
  await page.getByTestId('status-tray-notifications-button').click()
  await expect(page.getByTestId('tray-task-row').filter({ hasText: 'Download dataset.tar.gz' })).toHaveAttribute('data-state', 'canceled')
})
