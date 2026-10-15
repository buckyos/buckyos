import { expect, test, type Page } from '@playwright/test'

async function openFiles(page: Page) {
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto('/?scenario=normal&fbData=mock')
  await page.getByTestId('desktop-app-files').click()
  const win = page.getByTestId('window-files')
  await expect(win.getByRole('cell', { name: 'Projects', exact: true })).toBeVisible()
  return win
}

async function openPermissions(page: Page, cell: string) {
  await page.getByTestId('window-files').getByRole('cell', { name: cell, exact: true }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Permissions…', exact: true }).click()
  const dialog = page.getByTestId('permissions-dialog')
  await expect(dialog.getByTestId('acl-table')).toBeVisible()
  return dialog
}

test('FP-01: edit who has access to a folder with the ACL dialog', async ({ page }) => {
  await openFiles(page)
  let dialog = await openPermissions(page, 'Projects')
  await expect(dialog.getByTestId('permissions-own-access')).toContainText('Full control')
  await expect(dialog.getByTestId('acl-builtin-user:alice')).toContainText('Zone owner')
  await expect(dialog.getByTestId('acl-level-user:bob')).toHaveValue('write')
  await expect(dialog.getByTestId('acl-level-user:carol')).toHaveValue('read')
  await expect(dialog.getByTestId('acl-save')).toBeDisabled()

  await dialog.getByTestId('acl-level-user:carol').selectOption('write')
  await dialog.getByTestId('acl-add-principal').fill('Public (anyone, no sign-in)')
  await expect(dialog.getByTestId('acl-add-level')).toBeDisabled()
  await dialog.getByTestId('acl-add').click()
  await expect(dialog.getByTestId('acl-row-public')).toContainText('Public')
  await expect(dialog.getByTestId('acl-level-public').locator('option')).toHaveCount(1)
  await expect(dialog.getByTestId('acl-public-warning')).toBeVisible()
  // A user id outside the directory can still be typed in.
  await dialog.getByTestId('acl-add-principal').fill('dave')
  await dialog.getByTestId('acl-add').click()
  await expect(dialog.getByTestId('acl-row-user:dave')).toContainText('dave')
  await dialog.getByTestId('acl-remove-user:bob').click()
  await dialog.getByTestId('acl-save').click()
  await expect(dialog).toHaveCount(0)

  dialog = await openPermissions(page, 'Projects')
  await expect(dialog.getByTestId('acl-level-user:carol')).toHaveValue('write')
  await expect(dialog.getByTestId('acl-row-public')).toBeVisible()
  await expect(dialog.getByTestId('acl-row-user:bob')).toHaveCount(0)
  await expect(dialog.getByTestId('acl-level-user:dave')).toHaveValue('read')
})

test('FP-02: inherited grants are shown with their source and can be cut off', async ({ page }) => {
  const win = await openFiles(page)
  await win.getByRole('cell', { name: 'Projects', exact: true }).dblclick()
  await expect(win.getByRole('cell', { name: 'buckyos', exact: true })).toBeVisible()
  const dialog = await openPermissions(page, 'buckyos')
  await expect(dialog.getByTestId('acl-inherited-user:bob')).toContainText('Inherited from /home/Projects')
  await expect(dialog.getByTestId('acl-inherited-user:carol')).toContainText('Read')
  await dialog.getByTestId('acl-inherit').locator('input').uncheck()
  await expect(dialog.getByTestId('acl-inherited-user:bob')).toHaveCount(0)
  await expect(dialog.getByText('Only the entries below apply here')).toBeVisible()
  await dialog.getByTestId('acl-add-principal').fill('carol')
  await dialog.getByTestId('acl-add-level').selectOption('manage')
  await dialog.getByTestId('acl-add').click()
  await dialog.getByTestId('acl-save').click()
  await expect(dialog).toHaveCount(0)

  // The folder itself is reachable from the blank-area menu too.
  await win
    .locator('div.overflow-y-auto')
    .filter({ has: page.locator('[role="table"]') })
    .click({ button: 'right', position: { x: 420, y: 420 } })
  await page.getByRole('menuitem', { name: 'Folder permissions…', exact: true }).click()
  await expect(page.getByTestId('permissions-dialog')).toContainText('/home/Projects')
  await expect(page.getByTestId('acl-level-user:bob')).toHaveValue('write')
})

test('FP-03: settings lists every permission record and opens the same editor', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 860 })
  await page.goto('/?scenario=normal&fbData=mock')
  await page.getByTestId('desktop-app-settings').click()
  const win = page.getByTestId('window-settings')
  await expect(win.getByTestId('settings-general-page')).toBeVisible()
  await win.getByRole('button', { name: 'File Permissions', exact: true }).click()
  const pageRoot = win.getByTestId('settings-file-permissions-page')
  await expect(pageRoot).toContainText('Everything shared in this zone, by every user.')
  const shared = pageRoot.getByTestId('file-permissions-group-shared')
  await expect(shared.getByTestId('file-permission-/home/Projects')).toContainText('Bob · Read & write')
  await expect(shared.getByTestId('file-permission-/public')).toContainText('Public (anyone, no sign-in) · Read')
  await expect(shared.getByTestId('file-permission-/shared')).toContainText('All zone users · Read & write')

  await pageRoot.getByRole('textbox', { name: 'Filter by path or person' }).fill('carol')
  await expect(pageRoot.getByTestId(/^file-permission-\//)).toHaveCount(1)

  await pageRoot.getByTestId('file-permission-edit-/home/Projects').click()
  const dialog = page.getByTestId('permissions-dialog')
  await expect(dialog.getByTestId('acl-level-user:carol')).toHaveValue('read')
  await dialog.getByTestId('acl-level-user:carol').selectOption('manage')
  await dialog.getByTestId('acl-save').click()
  await expect(dialog).toHaveCount(0)
  await expect(pageRoot.getByTestId('file-permission-/home/Projects')).toContainText('Carol · Full control')
})
