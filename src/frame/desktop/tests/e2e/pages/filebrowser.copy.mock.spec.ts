import { test, expect, type Page } from '@playwright/test'

async function setup(page: Page) {
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto('/?scenario=normal&fbData=mock')
  await page.getByTestId('desktop-app-files').click()
  const win = page.getByTestId('window-files')
  await win.locator('aside').first().getByRole('button', { name: 'Documents', exact: true }).click()
  return win
}

test('copy clipboard survives success and same-directory conflicts in mock mode', async ({ page }) => {
  const win = await setup(page)
  await win.getByRole('cell', { name: 'Kyoto Trip Plan.md', exact: true }).click()
  await win.getByRole('button', { name: 'Copy files', exact: true }).click()
  await win.getByRole('button', { name: 'Paste', exact: true }).click()
  await page.getByTestId('conflict-dialog').getByRole('button', { name: 'Keep both', exact: true }).click()
  await expect(win.getByTestId('batch-results')).toContainText('1 succeeded')
  await win.getByTestId('batch-results').getByRole('button', { name: 'Close', exact: true }).click()
  await win.getByRole('button', { name: 'Paste', exact: true }).click()
  await page.getByTestId('conflict-dialog').getByRole('button', { name: 'Keep both', exact: true }).click()
  await expect(win.getByRole('cell', { name: 'Kyoto Trip Plan (3).md', exact: true })).toBeVisible()
  const identities = await page.evaluate(async () => {
    const { mockEntryByPath } = await import('/src/app/filebrowser/mock/' + 'data.ts')
    return ['Kyoto Trip Plan.md', 'Kyoto Trip Plan (2).md', 'Kyoto Trip Plan (3).md'].map((name) => mockEntryByPath(`/home/Documents/${name}`).id)
  })
  expect(new Set(identities).size).toBe(3)
})

test('mock recursive failure discloses children and retry reuses completed copies', async ({ page }) => {
  const win = await setup(page)
  await page.evaluate(async () => {
    const { mockAddEntry } = await import('/src/app/filebrowser/mock/' + 'data.ts')
    const { invalidateMockPath } = await import('/src/app/filebrowser/data/' + 'mockReader.ts')
    for (const [id, path, kind] of [['tree', '/home/Documents/copy-tree', 'folder'], ['a', '/home/Documents/copy-tree/A.txt', 'document'], ['b', '/home/Documents/copy-tree/fail-once-B.txt', 'document']]) mockAddEntry({ id, path, name: path.split('/').pop(), kind, modifiedAt: '2026-09-07T00:00:00Z' })
    invalidateMockPath('/home/Documents')
  })
  await win.getByRole('cell', { name: 'copy-tree', exact: true }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Copy to…', exact: true }).click()
  const target = page.getByTestId('target-dialog')
  await target.getByRole('textbox', { name: 'Destination' }).fill('/home/Pictures')
  await target.getByRole('button', { name: 'Go', exact: true }).click()
  await target.getByRole('button', { name: 'Copy here', exact: true }).click()
  await expect(win.getByTestId('batch-results')).toContainText('2 failed')
  const before = await page.evaluate(async () => (await import('/src/app/filebrowser/mock/' + 'data.ts')).mockEntryByPath('/home/Pictures/copy-tree/A.txt').id)
  await win.getByTestId('batch-results').getByRole('button', { name: 'Retry failed items' }).click()
  await expect(win.getByTestId('batch-results')).toContainText('3 succeeded · 0 failed')
  const after = await page.evaluate(async () => (await import('/src/app/filebrowser/mock/' + 'data.ts')).mockEntryByPath('/home/Pictures/copy-tree/A.txt').id)
  expect(after).toBe(before)
  await expect(win.getByRole('cell', { name: 'copy-tree', exact: true })).toBeVisible()
})
