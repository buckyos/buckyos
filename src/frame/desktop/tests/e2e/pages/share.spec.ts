import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

/** ShareContentMgr (notepads/share-content-mgr-tech-design-todo.md §6): system share dialog + My Content, mock data. */

async function openFiles(page: Page) {
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto('/?scenario=normal&fbData=mock')
  await page.getByTestId('desktop-app-files').click()
  const win = page.getByTestId('window-files')
  await expect(win.getByRole('cell', { name: 'Documents', exact: true })).toBeVisible()
  return win
}

async function openShare(page: Page, folder: string, file: string) {
  const win = page.getByTestId('window-files')
  await win.getByRole('cell', { name: folder, exact: true }).dblclick()
  await win.getByRole('cell', { name: file, exact: true }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Share…', exact: true }).click()
  const dialog = page.getByTestId('share-dialog')
  await expect(dialog.getByTestId('share-dialog-link-form')).toBeVisible()
  return dialog
}

async function myContentRows(page: Page) {
  await openFromAllApps(page, 'my-content')
  const win = page.getByTestId('window-my-content')
  await expect(win.getByTestId('mc-list')).toBeVisible()
  return win.getByTestId('mc-row')
}

test('T01: opening and closing the share dialog creates nothing', async ({ page }) => {
  await openFiles(page)
  const dialog = await openShare(page, 'Documents', 'Friend Contract Draft.pdf')
  await expect(dialog.getByTestId('share-dialog-name')).toHaveText('Friend Contract Draft.pdf')
  await expect(dialog.getByTestId('share-dialog-summary')).toContainText(/\d+(\.\d)? (KB|MB)/)
  await dialog.getByTestId('share-dialog-close').click()
  await expect(dialog).toBeHidden()
  // The mock starts with six shares; nothing was added.
  await expect(await myContentRows(page)).toHaveCount(6)
})

test('T02: seven days by default; choosing no expiry keeps the access type; three actions to a copied link', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await openFiles(page)
  const dialog = await openShare(page, 'Documents', 'Kyoto Trip Plan.md')
  await expect(dialog.getByTestId('share-access-holder')).toBeChecked()
  await expect(dialog.getByTestId('share-expiry')).toHaveValue('7d')
  await expect(dialog.getByTestId('share-expiry-preview')).not.toContainText('Does not expire')

  await dialog.getByTestId('share-access-password').check()
  await expect(dialog.getByTestId('share-dialog-create')).toBeDisabled()
  await dialog.getByTestId('share-expiry').selectOption('never')
  await expect(dialog.getByTestId('share-access-password')).toBeChecked()
  await expect(dialog.getByTestId('share-expiry-preview')).toContainText('Does not expire')
  await dialog.getByTestId('share-access-holder').check()

  // Create (1) → Copy (2), after opening the dialog.
  await dialog.getByTestId('share-dialog-create').click()
  await expect(dialog.getByTestId('share-dialog-created')).toBeVisible()
  const url = await dialog.getByTestId('share-dialog-copy-url').inputValue()
  expect(url).toMatch(/^https:\/\/share\.example\.zone\/s\//)
  await dialog.getByTestId('share-dialog-copy').click()
  await expect(dialog.getByTestId('share-dialog-copy')).toContainText('Copied')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(url)
  await dialog.getByTestId('share-dialog-done').click()

  // The new link is listed first in My Content, without expiry.
  const rows = await myContentRows(page)
  await expect(rows.first()).toContainText('Kyoto Trip Plan.md')
  await rows.first().click()
  const detail = page.getByTestId('mc-detail')
  await expect(detail.getByTestId('mc-detail-url')).toHaveValue(url)
  await expect(detail.getByTestId('mc-detail-expiry')).toContainText('No expiry')
})

test('app targets: HomeStation takes a PDF as a non-expiring link, MessageHub a file as an attachment', async ({ page }) => {
  await openFiles(page)
  const dialog = await openShare(page, 'Documents', 'Friend Contract Draft.pdf')
  await dialog.getByTestId('share-dialog-homestation').click()
  await expect(dialog.getByTestId('share-dialog-target-link')).toContainText('does not expire')
  await expect(dialog.getByTestId('share-dialog-target-link')).toContainText('posts only take photos')
  await dialog.getByTestId('share-dialog-target-back').click()
  await dialog.getByTestId('share-dialog-messagehub').click()
  await expect(dialog.getByTestId('share-dialog-target-messagehub')).toContainText('will be sent as an attachment')
  await dialog.getByTestId('share-dialog-target-back').click()
  await dialog.getByTestId('share-dialog-close').click()
})

test('My Content: overview, detail, password change and stop', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto('/?scenario=normal&shareData=mock')
  const rows = await myContentRows(page)
  const win = page.getByTestId('window-my-content')
  await expect(win.getByTestId('mc-tile-views')).not.toHaveText('—')
  await expect(win.getByTestId('mc-overview-chart')).toBeVisible()
  await win.getByTestId('mc-overview-chart-hit').hover()
  await expect(win.getByTestId('mc-overview-chart-tooltip')).toBeVisible()

  await win.getByTestId('mc-tab-links').click()
  await expect(rows.first()).toContainText('Link')
  await win.getByTestId('mc-search').fill('Q3')
  await expect(rows).toHaveCount(1)
  await rows.first().click()
  const detail = win.getByTestId('mc-detail')
  await expect(detail.getByTestId('mc-detail-chart')).toBeVisible()
  await detail.getByTestId('mc-password-edit').click()
  await detail.getByTestId('mc-password-input').fill('s3cret')
  await detail.getByTestId('mc-password-save').click()
  await expect(detail.getByTestId('mc-detail-password')).toContainText('On')
  await expect(detail.getByTestId('mc-detail-audit')).toContainText('Changed')

  await detail.getByTestId('mc-stop').click()
  await expect(detail.getByTestId('mc-stop-confirm')).toContainText('no longer be able to open it')
  await detail.getByTestId('mc-stop-confirm-button').click()
  await expect(detail.getByTestId('mc-status')).toHaveText('Stopped')
  await expect(detail.getByTestId('mc-stop')).toHaveCount(0)
  await detail.getByTestId('mc-detail-close').click()
  await expect(rows.first().getByTestId('mc-status')).toHaveText('Stopped')

  // App deliveries are not counted; their state is shown as reported.
  await win.getByTestId('mc-search').fill('')
  await win.getByTestId('mc-tab-apps').click()
  await rows.filter({ hasText: 'contract.pdf' }).click()
  await expect(win.getByTestId('mc-detail-delivery')).toContainText('Not confirmed')
  await expect(win.getByTestId('mc-detail')).toContainText('Statistics are not available')
  await win.getByTestId('mc-detail-close').click()

  // Public: broken links are flagged.
  await win.getByTestId('mc-tab-public').click()
  await expect(win.getByTestId('mc-public-broken')).toHaveCount(1)
})

for (const theme of ['light', 'dark'] as const) {
  test(`share dialog and My Content in the ${theme} theme`, async ({ page }) => {
    await page.addInitScript((value) => window.localStorage.setItem('buckyos.prototype.theme.v1', value), theme)
    await openFiles(page)
    const dialog = await openShare(page, 'Documents', 'Friend Contract Draft.pdf')
    await dialog.getByTestId('share-access-password').check()
    await page.screenshot({ path: `test-results/share-dialog-${theme}.png` })
    await dialog.getByTestId('share-dialog-close').click()
    await myContentRows(page)
    const win = page.getByTestId('window-my-content')
    await win.getByTestId('mc-overview-chart-hit').hover({ position: { x: 300, y: 40 } })
    await page.screenshot({ path: `test-results/my-content-${theme}.png` })
    await win.getByTestId('mc-row').filter({ hasText: 'Q3-report' }).click()
    await expect(win.getByTestId('mc-detail-chart')).toBeVisible()
    await page.screenshot({ path: `test-results/my-content-detail-${theme}.png` })
  })
}

test('T06 HomeStation: a photo opens a pre-filled post; publishing reports the delivery', async ({ page }) => {
  await openFiles(page)
  const win = page.getByTestId('window-files')
  await win.getByRole('cell', { name: 'Pictures', exact: true }).dblclick()
  const dialog = await openShare(page, 'sketches', 'design-sketch.png')
  await dialog.getByTestId('share-dialog-homestation').click()
  await expect(dialog.getByTestId('share-dialog-target-homestation')).toContainText('added to a new post')
  await dialog.getByTestId('share-dialog-target-continue').click()
  await expect(dialog).toBeHidden()
  const hs = page.getByTestId('window-homestation')
  await expect(hs.getByTestId('hs-publish-attachments')).toContainText('design-sketch.png')
  await hs.getByTestId('hs-publish-submit').click()
  await expect(hs.getByTestId('hs-publish-attachments')).toHaveCount(0)

  const rows = await myContentRows(page)
  await rows.filter({ hasText: 'design-sketch.png' }).first().click()
  await expect(page.getByTestId('mc-detail-delivery')).toContainText('Delivered')
})

async function handToMessageHub(page: Page, file: string) {
  await openFiles(page)
  const dialog = await openShare(page, 'Documents', file)
  await dialog.getByTestId('share-dialog-messagehub').click()
  await dialog.getByTestId('share-dialog-target-continue').click()
  const picker = page.getByTestId('forward-dialog')
  await expect(picker).toBeVisible()
  await expect(picker.getByTestId('forward-preview')).toContainText(file)
  return picker
}

async function deliveryState(page: Page, file: string) {
  const rows = await myContentRows(page)
  await page.getByTestId('window-my-content').getByTestId('mc-tab-apps').click()
  await rows.filter({ hasText: file }).first().click()
  return page.getByTestId('mc-detail-delivery')
}

test('T06 MessageHub: closing the picker leaves the delivery unsent', async ({ page }) => {
  const picker = await handToMessageHub(page, 'Kyoto Trip Plan.md')
  await picker.getByRole('button', { name: 'Cancel' }).click()
  await expect(picker).toBeHidden()
  await expect(await deliveryState(page, 'Kyoto Trip Plan.md')).toContainText('Not sent')
})

test('T06 MessageHub: sending to a conversation confirms the delivery', async ({ page }) => {
  const picker = await handToMessageHub(page, 'Friend Contract Draft.pdf')
  await picker.getByTestId('forward-targets').getByRole('button').first().click()
  await expect(picker).toBeHidden()
  await expect(await deliveryState(page, 'Friend Contract Draft.pdf')).toContainText('Delivered')
})
