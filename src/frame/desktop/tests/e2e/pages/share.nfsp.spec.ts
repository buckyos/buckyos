/**
 * Share dialog and My Content against a real nfs_server (share module, named store).
 *
 * Opt-in: needs a running nfs_server with a named store and the dev server proxying to it —
 *
 *   nfs_server --listen 127.0.0.1:3263 --data-dir <tmp>/data --named-store <tmp>/store \
 *              --share-base-url http://127.0.0.1:3263 \
 *              --export home=<tmp>/exports/home --export srv=<tmp>/exports/srv
 *   VITE_NFS_PROXY=http://127.0.0.1:3263 pnpm run dev --host 127.0.0.1 --port 4173
 *   SHARE_NFSP_E2E=1 npx playwright test tests/e2e/pages/share.nfsp.spec.ts
 *
 * The fixture tree must contain /home/Documents/notes.txt ("hello from a real share\n").
 * The shell stays mocked; `?fbData=nfsp` puts Files and the share service on NFSP.
 */

import { expect, test } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

test.describe('Sharing on real nfs_server', () => {
  test.skip(!process.env.SHARE_NFSP_E2E, 'set SHARE_NFSP_E2E=1 with a running nfs_server + proxy')

  test('create a password link from Files, open it as a visitor, see it counted and stop it in My Content', async ({ page, request }) => {
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto('/?scenario=normal&fbData=nfsp')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')
    await expect(win.getByRole('cell', { name: /^Documents(\s|$)/ })).toBeVisible({ timeout: 15000 })
    await win.getByRole('cell', { name: /^Documents(\s|$)/ }).dblclick()
    await win.getByRole('cell', { name: /^notes\.txt/ }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Share…', exact: true }).click()

    const dialog = page.getByTestId('share-dialog')
    await expect(dialog.getByTestId('share-dialog-summary')).toContainText('24 B')
    await dialog.getByTestId('share-access-password').check()
    await dialog.getByTestId('share-password').fill('s3cret')
    await dialog.getByTestId('share-dialog-create').click()
    await expect(dialog.getByTestId('share-dialog-created')).toBeVisible({ timeout: 15000 })
    const url = await dialog.getByTestId('share-dialog-copy-url').inputValue()
    expect(url).toMatch(/\/s\/[A-Za-z0-9_-]{43}$/)
    await dialog.getByTestId('share-dialog-done').click()

    // Visitor: locked until the password, then the snapshot.
    expect((await request.get(`${url}/d`)).status()).toBe(401)
    const visitor = await page.context().newPage()
    await visitor.goto(url)
    await visitor.getByPlaceholder('Password').fill('s3cret')
    await visitor.getByRole('button', { name: 'Open' }).click()
    await expect(visitor.locator('pre')).toHaveText('hello from a real share\n')
    await visitor.close()

    // The share is listed with its visit; the plan shows it as an existing link.
    await openFromAllApps(page, 'my-content')
    const mc = page.getByTestId('window-my-content')
    // Visits reach the statistics within the 5 s flush.
    await expect.poll(async () => {
      await mc.getByTestId('mc-refresh').click()
      return (await mc.getByTestId('mc-tile-views').textContent()) ?? ''
    }, { timeout: 20000 }).not.toMatch(/^(0|—)$/)
    // Newest first: the link just created.
    await mc.getByTestId('mc-row').filter({ hasText: 'notes.txt' }).first().click()
    const detail = mc.getByTestId('mc-detail')
    await expect(detail.getByTestId('mc-detail-url')).toHaveValue(url)
    await expect(detail.getByTestId('mc-detail-password')).toContainText('On')
    await expect(detail.getByTestId('mc-detail-objid')).toContainText('cyfile:')
    await expect(detail).toContainText('Views 1')
    await detail.getByTestId('mc-stop').click()
    await detail.getByTestId('mc-stop-confirm-button').click()
    await expect(detail.getByTestId('mc-status')).toHaveText('Stopped')
    expect((await request.get(url)).status()).toBe(404)
  })
})
