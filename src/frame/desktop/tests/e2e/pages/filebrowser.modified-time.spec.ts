import { expect, test } from '@playwright/test'
import type { Entry } from '../../../src/api/nfsp_client'

test.use({ timezoneId: 'Asia/Shanghai', locale: 'en-US', viewport: { width: 1440, height: 1000 } })

function entry(path: string, kind: Entry['target']['kind'], binding: Entry['binding'], mtime?: number): Entry {
  return {
    name: path.split('/').pop()!,
    binding,
    canonical_path: `cyfs://${path}`,
    target: { ref: { type: 'live', node_id: path }, kind, attrs: { mtime, size: 1 } },
  }
}

for (const [timezoneId, epochLabel, currentLabel] of [
  ['Asia/Shanghai', '01/01/1970, 08:00', '10/28/2026, 08:00'],
  ['UTC', '01/01/1970, 00:00', '10/28/2026, 00:00'],
  ['America/Los_Angeles', '12/31/1969, 16:00', '10/27/2026, 17:00'],
]) {
  test.describe(timezoneId, () => {
    test.use({ timezoneId })
    test('NFSP timestamps preserve real epoch dates and reject unknown or invalid values', async ({ page }) => {
      await page.goto('/?scenario=normal&fbData=mock')
      const result = await page.evaluate(async () => {
        const { unixToIso } = await import('/src/app/filebrowser/data/nfsp/' + 'mapping.ts')
        const { formatDate } = await import('/src/app/filebrowser/' + 'fileDisplay.tsx')
        return {
          unknown: [undefined, null, Number.NaN, Infinity, -Infinity, 8.64e12 + 1].map(unixToIso),
          valid: [0, -1, 1793145600].map(unixToIso),
          unknownLabels: [formatDate('', 'en'), formatDate('', 'zh-CN')],
          epochLabel: formatDate(unixToIso(0), 'en'),
          currentLabel: formatDate(unixToIso(1793145600), 'en'),
        }
      })
      expect(result.unknown).toEqual(['', '', '', '', '', ''])
      expect(result.valid).toEqual(['1970-01-01T00:00:00.000Z', '1969-12-31T23:59:59.000Z', '2026-10-28T00:00:00.000Z'])
      expect(result.unknownLabels).toEqual(['Unknown', '未知'])
      expect(result.epochLabel).toBe(epochLabel)
      expect(result.currentLabel).toBe(currentLabel)
    })
  })
}

test('Files shows unknown projection dates in lists and details without hiding real epoch dates', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('buckyos.prototype.locale.v1', 'en')
    localStorage.setItem('files.listPreferences', JSON.stringify({ fullColumns: true }))
  })
  await page.goto('/?scenario=normal&fbData=mock')
  const root = '/home/time-check'
  const entries = [
    entry(root, 'dir', 'native', 1793145600),
    entry(`${root}/Apps`, 'dir', 'projection', 0),
    entry(`${root}/Collected`, 'dir', 'projection', 0),
    entry(`${root}/Apps/Calendar`, 'dir', 'projection', 0),
    entry(`${root}/Apps/Calendar/assets`, 'dir', 'projection', 0),
    entry(`${root}/Apps/Calendar/assets/photos`, 'dir', 'projection', 0),
    entry(`${root}/Collected/Bookmarks`, 'dir', 'projection', 0),
    entry(`${root}/Timed projection`, 'dir', 'projection', 1793145600),
    entry(`${root}/Epoch folder`, 'dir', 'native', 0),
    entry(`${root}/epoch.txt`, 'file', 'native', 0),
    entry(`${root}/projected-epoch.txt`, 'file', 'projection', 0),
    entry(`${root}/current.txt`, 'file', 'native', 1793145600),
    entry(`${root}/missing.txt`, 'file', 'native'),
  ]
  await page.evaluate(async (entries) => {
    const { mapEntryToItem } = await import('/src/app/filebrowser/data/nfsp/' + 'mapping.ts')
    const { mockAddEntry } = await import('/src/app/filebrowser/mock/' + 'data.ts')
    for (const wire of entries) mockAddEntry(mapEntryToItem(wire, {}).entry)
  }, entries)
  await page.getByTestId('desktop-app-files').click()
  const win = page.getByTestId('window-files')
  await win.getByRole('cell', { name: 'time-check', exact: true }).dblclick()
  const row = (name: string) => win.getByRole('row').filter({ has: page.getByRole('cell', { name, exact: true }) })
  const unknown = (name: string) => expect(row(name).getByRole('cell').nth(3)).toHaveText('Unknown')

  await unknown('Apps')
  await unknown('Collected')
  await unknown('missing.txt')
  for (const name of ['Epoch folder', 'epoch.txt', 'projected-epoch.txt']) {
    await expect(row(name).getByRole('cell').nth(3)).toHaveText('01/01/1970, 08:00')
  }
  await expect(row('current.txt').getByRole('cell').nth(3)).toHaveText('10/28/2026, 08:00')
  await expect(row('Timed projection').getByRole('cell').nth(3)).toHaveText('10/28/2026, 08:00')

  await win.getByRole('cell', { name: 'Apps', exact: true }).click()
  await win.getByRole('button', { name: 'Details', exact: true }).click()
  const modifiedDetail = win.getByText('Modified', { exact: true }).last().locator('..')
  await expect(modifiedDetail).toHaveText('ModifiedUnknown')
  await win.getByRole('cell', { name: 'epoch.txt', exact: true }).click()
  await expect(modifiedDetail).toHaveText('Modified01/01/1970, 08:00')
  await win.getByRole('cell', { name: 'missing.txt', exact: true }).click()
  await expect(modifiedDetail).toHaveText('ModifiedUnknown')

  for (const [parent, child] of [['Apps', 'Calendar'], ['Calendar', 'assets'], ['assets', 'photos']]) {
    await win.getByRole('cell', { name: parent, exact: true }).dblclick()
    await unknown(child)
  }
  for (let depth = 0; depth < 3; depth++) await win.getByRole('button', { name: 'back', exact: true }).click()
  await win.getByRole('cell', { name: 'Collected', exact: true }).dblclick()
  await unknown('Bookmarks')
})
