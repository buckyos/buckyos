/**
 * File Browser × real nfs_server integration (NFSP adapter stage).
 *
 * Opt-in: needs a running nfs_server and the dev server proxying to it —
 *
 *   nfs_server --listen 127.0.0.1:3260 --data-dir <tmp>/data \
 *              --export home=<tmp>/exports/home --export public=<tmp>/exports/public --named-store <tmp>/named-store
 *   VITE_NFS_PROXY=http://127.0.0.1:3260 pnpm run dev --host 127.0.0.1 --port 4173
 *   FB_NFSP_E2E=1 npx playwright test tests/e2e/pages/filebrowser.nfsp.spec.ts
 *
 * Or start the same standalone fixtures automatically (prebuilt nfs_server):
 *   FB_NFSP_AUTO=1 VITE_NFS_PROXY=http://127.0.0.1:3262 pnpm exec playwright test tests/e2e/pages/filebrowser.nfsp.spec.ts
 * Set CARGO_TARGET_DIR when the binary is outside buckyos/src/target.
 *
 * The fixture tree must contain /home/Documents (with files) and /home/Pictures.
 * The shell stays mocked (VITE_CP_USE_MOCK); `?fbData=nfsp` switches only the
 * File Browser data layer onto the NFSP adapter.
 */

import { expect, test } from '../fixtures/nfs-copy'
import type { Page, Request } from '@playwright/test'
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

async function readNfspText(page: Page, path: string) {
  return page.evaluate(async path => {
    const hello = await fetch('/nfs/v1/hello', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ args: {} }) }).then(response => response.json())
    const stat = await fetch('/nfs/v1/stat', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ session: hello.result.session, at: { path }, args: {}, want: ['ref'] }) }).then(response => response.json())
    const response = await fetch(`/nfs/v1/read/${encodeURIComponent(stat.result.ref.node_id)}`)
    if (!response.ok) throw new Error(`Read failed: ${response.status}`)
    return response.text()
  }, path)
}

test.describe('File browser on real nfs_server', () => {
  test.skip(!process.env.FB_NFSP_E2E && !process.env.FB_NFSP_AUTO, 'set FB_NFSP_E2E=1 with a running nfs_server + proxy')

  test('list, mkdir, rename, upload, collection, search, delete against NFSP', async ({
    page,
  }) => {
    let legacyUploads = 0
    const dataUploads: Request[] = []
    // Chromium omits Blob request bodies in postDataBuffer/sizes. TUS offsets
    // measure bytes accepted by the real server for each data request.
    const uploadedBytes = async () => (await Promise.all(dataUploads.map(async request => {
      const response = await request.response()
      return Number(await response?.headerValue('Upload-Offset')) - Number(await request.headerValue('Upload-Offset'))
    }))).reduce((total, bytes) => total + bytes, 0)
    page.on('request', request => {
      if (/\/nfs\/v1\/(uploads|probe|open_write)(\/|$)/.test(new URL(request.url()).pathname)) legacyUploads += 1
      if (request.method() === 'PATCH' && new URL(request.url()).pathname.startsWith('/ndm/v1/uploads/')) dataUploads.push(request)
    })
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
    await win.getByRole('button', { name: 'New', exact: true }).click()
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

    // Upload runs SDK import/finalization and NFSP local object extraction.
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

    const payload = `uploaded against nfs_server ${stamp}`
    expect(await readNfspText(page, `/home/Documents/${uploadName}`)).toBe(payload)
    const firstUploadBytes = await uploadedBytes()
    expect(firstUploadBytes).toBe(Buffer.byteLength(payload))
    const duplicateChooser = page.waitForEvent('filechooser')
    await win.getByRole('button', { name: 'Upload', exact: true }).click()
    await (await duplicateChooser).setFiles({ name: `${uploadName}.copy`, mimeType: 'text/plain', buffer: Buffer.from(payload) })
    await expect(page.getByTestId('transfer-success')).toHaveCount(2, { timeout: 20000 })
    expect(await readNfspText(page, `/home/Documents/${uploadName}.copy`)).toBe(payload)
    expect(await uploadedBytes()).toBe(firstUploadBytes)

    const source = await mkdtemp(join(tmpdir(), 'nfsp-folder-'))
    const rootName = `folder-${stamp}`
    const root = join(source, rootName)
    await mkdir(join(root, 'nested'), { recursive: true })
    await writeFile(join(root, 'same.txt'), 'root content')
    await writeFile(join(root, 'nested', 'same.txt'), 'nested content')
    await writeFile(join(root, 'zero.txt'), '')
    try {
      const folderChooser = page.waitForEvent('filechooser')
      await win.getByRole('button', { name: 'New', exact: true }).click()
      await page.getByRole('menuitem', { name: 'Upload folder', exact: true }).click()
      await (await folderChooser).setFiles(root)
      await expect(page.getByTestId('transfer-success')).toHaveCount(3, { timeout: 30000 })
      await expect(win.getByRole('cell', { name: new RegExp(`^${rootName}`) })).toBeVisible()
      await win.getByRole('cell', { name: new RegExp(`^${rootName}`) }).dblclick()
      await expect(win.getByRole('cell', { name: /^same.txt/ })).toBeVisible()
      await expect(win.getByRole('cell', { name: /^zero.txt/ })).toBeVisible()
      await win.getByRole('cell', { name: /^nested/ }).dblclick()
      await expect(win.getByRole('cell', { name: /^same.txt/ })).toBeVisible()
      expect(await readNfspText(page, `/home/Documents/${rootName}/same.txt`)).toBe('root content')
      expect(await readNfspText(page, `/home/Documents/${rootName}/nested/same.txt`)).toBe('nested content')
      expect(await readNfspText(page, `/home/Documents/${rootName}/zero.txt`)).toBe('')
      expect(legacyUploads).toBe(0)
      expect(await uploadedBytes()).toBe(Buffer.byteLength(payload) + Buffer.byteLength('root contentnested content'))
      await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    } finally { await rm(source, { recursive: true, force: true }) }

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
    await win.getByRole('combobox', { name: 'Search in' }).selectOption('all')
    await expect(page.getByText('Search results')).toBeVisible()
    await expect(page.getByText(/Traditional matches/)).toBeVisible({ timeout: 15000 })
    // Blank input leaves search and restores the listing (§4.4 idle).
    await page.getByPlaceholder(/Search across files/).fill('')

    // Destroy-semantics delete (confirm dialog) removes the created folder.
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    await win
      .getByRole('cell', { name: new RegExp(`^${folderName}-v2`) })
      .click({ button: 'right' })
    await page.getByRole('menuitem', { name: /^Delete/ }).click()
    await page.getByTestId('delete-dialog').getByRole('button', { name: 'Permanently delete', exact: true }).click()
    await expect(win.getByRole('cell', { name: new RegExp(`^${folderName}-v2`) })).toHaveCount(
      0,
      { timeout: 15000 },
    )
  })
})
