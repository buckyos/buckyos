/* The Workspace's entries that take one file (notepads/sysdlg-select-adoption-todo.md §4.1 A2, A3, A6, A8): each
 * opens the system file picker, whose main source is the Zone; what was chosen becomes the Workspace's content. */

import { chooseFromDevice, chooseFromZone, mainMenu, openCanvas, test, expect, type Api } from './fixtures'
import { closeSelect, selectDialog, selectRow } from '../e2e/fixtures/selectDialog'

const ALICE = 'tok-alice'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')

async function assetBytes(api: Api, workspaceId: string, sourceId: string): Promise<Buffer> {
  const id = (await api.read(ALICE, workspaceId, sourceId)).content.payload.object_id
  const response = await fetch(`${api.base}/asset/${workspaceId}/${id}`, { headers: { authorization: `Bearer ${ALICE}` } })
  return Buffer.from(await response.arrayBuffer())
}
const assetCells = async (api: Api, workspaceId: string) => (await api.outline(ALICE, workspaceId)).filter((e: Json) => e.view_type === 'asset' && e.parent_id === 'free' && !e.deleted)

test('FP-01 a file object from the toolbar and from the catalog, then a replacement: chosen from the Zone', async ({ page, api }) => {
  test.setTimeout(90_000)
  const ws = await api.freeCanvas(ALICE, `fp-01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)

  // the toolbar's file object: the picker first, then a click places it
  await chooseFromZone(page, () => page.getByTestId('aiws-tool-insert-asset').click(), { name: '照片.png', mimeType: 'image/png', buffer: PNG })
  await expect(page.getByTestId('aiws-tool-hint')).toBeVisible()
  const area = (await page.locator('.aiws-canvas-main').boundingBox())!
  // away from the middle, where the catalog inserts next
  await page.mouse.click(area.x + area.width / 2 - 400, area.y + area.height / 2 + 200)
  await expect.poll(async () => (await assetCells(api, ws.workspace_id)).length).toBe(1)
  const [placed] = await assetCells(api, ws.workspace_id)
  expect(await assetBytes(api, ws.workspace_id, placed.source_id)).toEqual(PNG)

  // the catalog's file field: a button and the chosen name instead of a native input
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-search').fill('文件')
  await page.getByTestId('aiws-catalog-entry-block-asset').click()
  await expect(page.getByTestId('aiws-catalog-insert')).toBeDisabled()
  // the picker over the catalog has the focus: Escape closes the picker, not the catalog under it
  await page.getByTestId('aiws-catalog-file').click()
  await expect.poll(() => page.evaluate(() => Boolean(document.activeElement?.closest('[data-testid="select-dialog"]')))).toBe(true)
  await page.keyboard.press('Escape')
  await expect(selectDialog(page)).toHaveCount(0)
  await expect(page.getByTestId('aiws-catalog')).toBeVisible()
  await chooseFromZone(page, () => page.getByTestId('aiws-catalog-file').click(), { name: '纪要.txt', mimeType: 'text/plain', buffer: Buffer.from('会议纪要') })
  await expect(page.getByTestId('aiws-catalog-file-name')).toHaveText('纪要.txt')
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await assetCells(api, ws.workspace_id)).length).toBe(2)
  const minutes = (await assetCells(api, ws.workspace_id)).find((e: Json) => e.entity_id !== placed.entity_id)
  expect((await assetBytes(api, ws.workspace_id, minutes.source_id)).toString()).toBe('会议纪要')

  // replace the picture's content: only pictures can be chosen, the new bytes become the asset's
  await page.getByTestId(`aiws-canvas-block-${placed.entity_id}`).click()
  const replacement = Buffer.concat([PNG, Buffer.from([0])])
  await chooseFromZone(page, () => page.getByTestId('aiws-near-asset-replace').click(), { name: '新照片.png', mimeType: 'image/png', buffer: replacement })
  await expect.poll(async () => assetBytes(api, ws.workspace_id, placed.source_id)).toEqual(replacement)
})

test('FP-02 an SVG shape read from a Zone file; a .pikg import only offers packages; a local file as well', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `fp-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-search').fill('SVG')
  await page.getByTestId('aiws-catalog-entry-block-shape:svg').click()
  await chooseFromZone(page, () => page.getByTestId('aiws-catalog-svg-file').click(), { name: '圆.svg', mimeType: 'image/svg+xml', buffer: Buffer.from('<svg viewBox="0 0 10 10" width="80" height="80"><circle cx="5" cy="5" r="4" fill="#1a7f37"/></svg>') })
  await expect(page.getByTestId('aiws-catalog-svg-text')).toHaveValue(/circle/)
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.view_type === 'shape' && !e.deleted).length).toBe(1)

  // the same entry from this device
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-search').fill('SVG')
  await page.getByTestId('aiws-catalog-entry-block-shape:svg').click()
  await chooseFromDevice(page, () => page.getByTestId('aiws-catalog-svg-file').click(), { name: 'square.svg', mimeType: 'image/svg+xml', buffer: Buffer.from('<svg viewBox="0 0 10 10" width="60" height="60"><rect width="8" height="8" fill="#0969da"/></svg>') })
  await expect(page.getByTestId('aiws-catalog-svg-text')).toHaveValue(/rect/)
  await page.getByTestId('aiws-catalog').getByRole('button', { name: '关闭', exact: true }).click()
  await expect(page.getByTestId('aiws-catalog')).toHaveCount(0)

  // the extension library's import: a .pikg can be chosen, another file cannot
  await page.evaluate(([dir]) => {
    const hook = (window as unknown as { __mockZoneFiles: { put(path: string, base64: string): void } }).__mockZoneFiles
    hook.put(`${dir}/controls.pikg`, btoa('pikg'))
    hook.put(`${dir}/readme.txt`, btoa('text'))
  }, ['/home/Documents/aiws-pikg'])
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-extensions')
  await page.getByTestId('aiws-ext-import-pikg').click()
  const dialog = selectDialog(page)
  await dialog.getByTestId('select-locations').getByRole('button', { name: '我的文件', exact: true }).click()
  await selectRow(dialog, 'Documents').click()
  await selectRow(dialog, 'aiws-pikg').click()
  await expect(selectRow(dialog, 'controls.pikg')).not.toHaveAttribute('aria-disabled', 'true')
  await expect(selectRow(dialog, 'readme.txt')).toHaveAttribute('aria-disabled', 'true')
  await closeSelect(page)
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('aiws-ext-dialog')).toBeVisible()
})
