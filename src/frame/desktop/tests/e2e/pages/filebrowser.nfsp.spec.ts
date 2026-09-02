/**
 * File Browser × real nfs_server integration (NFSP adapter stage).
 *
 * Opt-in: needs a running nfs_server and the dev server proxying to it —
 *
 *   nfs_server --listen 127.0.0.1:3260 --data-dir <tmp>/data \
 *              --export home=<tmp>/exports/home --export public=<tmp>/exports/public
 *   VITE_NFS_PROXY=http://127.0.0.1:3260 pnpm run dev --host 127.0.0.1 --port 4173
 *   FB_NFSP_E2E=1 npx playwright test tests/e2e/pages/filebrowser.nfsp.spec.ts
 *
 * The fixture tree must contain /home/Documents (with files) and /home/Pictures.
 * The shell stays mocked (VITE_CP_USE_MOCK); `?fbData=nfsp` switches only the
 * File Browser data layer onto the NFSP adapter.
 */

import { expect, test } from '@playwright/test'

test.describe('File browser on real nfs_server', () => {
  test.skip(!process.env.FB_NFSP_E2E, 'set FB_NFSP_E2E=1 with a running nfs_server + proxy')

  test('list, mkdir, rename, upload, collection, search, delete against NFSP', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal&fbData=nfsp')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')
    const sidebar = win.locator('aside').first()
    const stamp = Date.now().toString(36)

    // Real backend listing: export roots reach the sidebar, /home lists dirs.
    await expect(win.getByRole('cell', { name: /^Documents(\s|$)/ })).toBeVisible({
      timeout: 15000,
    })
    await expect(win.getByRole('cell', { name: /^Pictures(\s|$)/ })).toBeVisible()

    // Navigate into Documents from the sidebar tree (loaded from the server).
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    await expect(win.getByText('notes.txt')).toBeVisible()

    // mkdir through the schema dialog → server mkdir → listing invalidates.
    const dialog = page.getByTestId('name-prompt-dialog')
    const folderName = `e2e-folder-${stamp}`
    await win
      .locator('div.overflow-y-auto')
      .filter({ has: page.locator('[role="table"]') })
      .click({ button: 'right', position: { x: 420, y: 420 } })
    await page.getByRole('menuitem', { name: 'New folder' }).click()
    await dialog.getByRole('textbox').fill(folderName)
    await dialog.getByRole('button', { name: 'Create' }).click()
    await expect(win.getByRole('cell', { name: new RegExp(`^${folderName}`) })).toBeVisible({
      timeout: 15000,
    })

    // Rename goes through NFSP move.
    await win.getByRole('cell', { name: new RegExp(`^${folderName}`) }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Rename…' }).click()
    await dialog.getByRole('textbox').fill(`${folderName}-v2`)
    await dialog.getByRole('button', { name: 'Rename' }).click()
    await expect(
      win.getByRole('cell', { name: new RegExp(`^${folderName}-v2`) }),
    ).toBeVisible({ timeout: 15000 })

    // Upload runs the real probe → tus → commit pipeline.
    const uploadName = `e2e-upload-${stamp}.txt`
    const chooser = page.waitForEvent('filechooser')
    await win.getByRole('button', { name: 'Upload', exact: true }).click()
    await (await chooser).setFiles({
      name: uploadName,
      mimeType: 'text/plain',
      buffer: Buffer.from(`uploaded against nfs_server ${stamp}`),
    })
    await expect(page.getByTestId('transfer-success')).toBeVisible({ timeout: 20000 })
    await expect(win.getByRole('cell', { name: new RegExp(uploadName) })).toBeVisible({
      timeout: 15000,
    })

    // Server-owned collection: create, add a reference, verify membership.
    const collectionTitle = `E2E List ${stamp}`
    await sidebar.getByRole('button', { name: 'New collection…' }).click()
    await dialog.getByRole('textbox').fill(collectionTitle)
    await dialog.getByRole('button', { name: 'Create' }).click()
    await expect(win.getByText(`Collection: ${collectionTitle}`)).toBeVisible({
      timeout: 15000,
    })
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    await win.getByRole('cell', { name: /notes\.txt/ }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Add to Collection' }).click()
    await page.getByRole('menuitem', { name: collectionTitle }).click()
    await sidebar.getByRole('button', { name: new RegExp(collectionTitle) }).click()
    await expect(
      win.locator('[role="row"]').filter({ hasText: 'notes.txt' }),
    ).toBeVisible({ timeout: 15000 })

    // Search rides the server's name mode.
    await win.getByRole('button', { name: 'Search', exact: true }).click()
    await page.getByPlaceholder(/Search across files/).fill('notes')
    await expect(page.getByText('Search results')).toBeVisible()
    await expect(page.getByText(/Traditional matches/)).toBeVisible({ timeout: 15000 })
    // Blank input leaves search and restores the listing (§4.4 idle).
    await page.getByPlaceholder(/Search across files/).fill('')

    // Destroy-semantics delete (confirm dialog) removes the created folder.
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    page.once('dialog', (confirm) => void confirm.accept())
    await win
      .getByRole('cell', { name: new RegExp(`^${folderName}-v2`) })
      .click({ button: 'right' })
    await page.getByRole('menuitem', { name: /^Delete/ }).click()
    await expect(win.getByRole('cell', { name: new RegExp(`^${folderName}-v2`) })).toHaveCount(
      0,
      { timeout: 15000 },
    )
  })
})
