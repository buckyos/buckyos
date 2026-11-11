import { expect, test, type Page } from '@playwright/test'
import { pickLocal, pickZone, putZoneFile, selectDialog } from '../fixtures/selectDialog'

const definition = JSON.stringify({ schema_version: '0.2.0', id: 'zone_pipeline', name: 'zone_pipeline', trigger: { type: 'manual' }, steps: [], edges: [] })

async function openImport(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.locator('.swiper-pagination-bullet').nth(1).click()
  await page.locator('.swiper-slide-active').getByTestId('desktop-app-workflow').click()
  const win = page.getByTestId('window-workflow')
  await win.getByTitle('Import Workflow').click()
  await win.getByRole('button', { name: 'File', exact: true }).click()
  return win
}

test.describe('Workflow import', () => {
  test('a definition file is chosen in the Zone through the system picker', async ({ page }) => {
    const win = await openImport(page)
    await putZoneFile(page, '/home/Documents/zone_pipeline.json', Buffer.from(definition), 'application/json')
    await win.getByTestId('workflow-import-choose-file').click()
    await pickZone(selectDialog(page), '/home/Documents/zone_pipeline.json')
    await expect(win.getByTestId('workflow-import-choose-file')).toContainText(`Loaded zone_pipeline.json (${definition.length} chars)`)
    await win.getByRole('button', { name: 'Paste text', exact: true }).click()
    await expect(win.locator('textarea')).toHaveValue(definition)
  })

  test('a definition file can also come from this device', async ({ page }) => {
    const win = await openImport(page)
    await win.getByTestId('workflow-import-choose-file').click()
    await pickLocal(page, { name: 'local.json', mimeType: 'application/json', buffer: Buffer.from(definition) })
    await expect(win.getByTestId('workflow-import-choose-file')).toContainText('Loaded local.json')
  })
})
