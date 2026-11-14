/* UI 框架 PRD §10 (Block 相关的界面) and §6.2 (流式页): one menu for a Block wherever it is opened, the Block toolbar
 * (primary actions, quick properties, the common zone, "more"), a flow page's title row and the floating properties
 * panel. Real backend; the desktop half of §15 scenario 17 and scenarios 31–34. */

import { fitAll, openCanvas, openWorkspace, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'

// The backend's JSON results are inspected ad hoc.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** The quarterly demo plus a note and a shape on the analysis canvas (as objects.spec). */
async function objectsWorkspace(api: Api, title: string) {
  const ws = await api.demo(ALICE, 'quarterly', title)
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'note-1', type_id: 'buckyos.annotation', parent_id: 'sf-analysis-content', order_key: 'x1', payload: { kind: 'note', body: '记得核对华南的成本', style: { color: '#fff2cc' } } },
    { op: 'entity.create', entity_id: 'blk-note', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'x1', placement: { x: 40, y: 640, w: 220, h: 160 }, payload: { view: { type: 'note' }, source_ref: { entity_id: 'note-1' } } },
    { op: 'entity.create', entity_id: 'blk-shape', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'x2', placement: { x: 300, y: 640, w: 160, h: 120 }, payload: { view: { type: 'shape' }, title: '流程', config: { shape: 'ellipse', fill: '#e8f0fe', stroke: '#4f8df7' } } },
  ])
  return ws
}

const configOf = async (api: Api, ws: { workspace_id: string }, id: string): Promise<Json> => (await api.read(ALICE, ws.workspace_id, id)).content.payload.config ?? {}
const panel = (page: import('@playwright/test').Page) => page.getByTestId('aiws-props-panel')

test('§15-17 (desktop) a flow page: an asset Block\'s title row has its fit and preview; its ⋯ menu has comment, lock and delete', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `blockui-17 ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const asset = page.getByTestId('aiws-cell-frame-cell-diagram')
  await expect(asset).toBeVisible()
  // an untitled type: a thin row while it is hovered, focused or selected
  const row = page.getByTestId('aiws-flow-row-cell-diagram')
  await expect(row).toBeHidden()
  await asset.hover()
  await expect(row).toBeVisible()
  await page.mouse.move(5, 500)
  await expect(row).toBeHidden()
  await asset.focus()
  await expect(row).toBeVisible()
  await expect(row.getByTestId('aiws-near-asset-preview')).toBeVisible()
  await expect(row.getByTestId('aiws-near-asset-fit')).toBeVisible()
  // the fit is a quick property: changed from the row (one commit)
  await row.getByTestId('aiws-near-asset-fit').click()
  await row.getByTestId('aiws-near-asset-fit-cover').click()
  await expect.poll(async () => (await configOf(api, ws, 'cell-diagram')).fit).toBe('cover')
  // a titled type carries its tools on its heading line
  await expect(page.getByTestId('aiws-flow-row-cell-info').getByTestId('aiws-cell-title-cell-info')).toBeVisible()
  await expect(page.getByTestId('aiws-flow-more-cell-info')).toBeAttached()

  // ⋯ selects it and opens its whole menu (the same list as a free canvas's context menu)
  await asset.focus()
  await row.getByTestId('aiws-flow-more-cell-diagram').click()
  const menu = page.getByTestId('aiws-flow-block-menu')
  await expect(menu).toBeVisible()
  await expect(asset).toHaveClass(/is-selected/)
  await expect(menu.getByTestId('aiws-menu-asset-preview')).toBeVisible()
  await expect(menu.getByTestId('aiws-menu-annotate')).toBeVisible()
  await expect(menu.getByTestId('aiws-inspect-cell-diagram')).toBeVisible()
  await expect(menu.getByTestId('aiws-delete-block-cell-diagram')).toBeVisible()
  // more than twelve items: "arrange" and "links and permissions" are submenus
  await expect(menu.getByTestId('aiws-menu-group-relate')).toBeVisible()
  await menu.getByTestId('aiws-menu-group-arrange').click()
  await expect(menu.getByTestId('aiws-menu-lock')).toBeVisible()
  await menu.getByTestId('aiws-menu-lock').click()
  await expect(menu).toHaveCount(0)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'cell-diagram').locked).toBe(true)
  // a locked Block offers reading only: no type tools on its row
  await expect(row).toBeVisible()
  await expect(row.getByTestId('aiws-near-asset-fit')).toHaveCount(0)

  // a right click on the Block (outside an editor) opens the same menu; unlock from it
  await page.getByTestId('aiws-flow-row-cell-diagram').click({ button: 'right', position: { x: 4, y: 10 } })
  await expect(menu).toBeVisible()
  // locked, it offers fewer than twelve items: nothing folds
  await expect(menu.getByTestId('aiws-menu-group-arrange')).toHaveCount(0)
  await menu.getByTestId('aiws-menu-unlock').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'cell-diagram').locked ?? false).toBe(false)
  // Shift+F10 opens it for the selected Block; "properties" there shows the same fit
  await asset.focus()
  await page.keyboard.press('Shift+F10')
  await expect(menu).toBeVisible()
  await menu.getByTestId('aiws-inspect-cell-diagram').click()
  await expect(panel(page)).toHaveAttribute('data-subject', 'cell-diagram')
  await expect(panel(page).getByTestId('aiws-asset-field-fit')).toHaveValue('cover')
})

test('§15-31 selecting a Block never opens the properties panel; "properties" does; unpinned it closes, pinned it follows the selection', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `blockui-31 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await expect(page.getByTestId('aiws-near-toolbar')).toBeVisible()
  await expect(panel(page)).toHaveCount(0)
  // the Block toolbar's "more" → properties: beside the Block, not over it
  await page.getByTestId('aiws-near-more').click()
  await page.getByTestId('aiws-near-inspector').click()
  await expect(panel(page)).toHaveAttribute('data-subject', 'blk-shape')
  await expect(panel(page)).toHaveAttribute('data-pinned', 'false')
  const box = (await panel(page).boundingBox())!
  const block = (await page.getByTestId('aiws-canvas-block-blk-shape').boundingBox())!
  expect(box.x >= block.x + block.width || box.x + box.width <= block.x || box.y >= block.y + block.height || box.y + box.height <= block.y).toBe(true)
  // the right side panel has no "properties" tab any more
  await expect(page.getByTestId('aiws-side-tab-inspector')).toHaveCount(0)
  // another Block selected: the unpinned panel closes and does not come back by itself
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  await expect(panel(page)).toHaveCount(0)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await expect(panel(page)).toHaveCount(0)
  // the context menu's "properties"
  await page.getByTestId('aiws-canvas-block-blk-note').click({ button: 'right' })
  await page.getByTestId('aiws-context-menu').getByTestId('aiws-menu-inspector').click()
  await expect(panel(page)).toHaveAttribute('data-subject', 'blk-note')
  // pinned: it stays, against the right edge, and shows what is selected — the page when nothing is
  await page.getByTestId('aiws-props-pin').click()
  await expect(panel(page)).toHaveAttribute('data-pinned', 'true')
  const area = (await page.getByTestId('aiws-canvas-body').boundingBox())!
  const pinned = (await panel(page).boundingBox())!
  expect(area.x + area.width - (pinned.x + pinned.width)).toBeLessThan(40)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await expect(panel(page)).toHaveAttribute('data-subject', 'blk-shape')
  await page.keyboard.press('Escape')
  await expect(panel(page)).toHaveAttribute('data-subject', 'page')
  await expect(panel(page).getByTestId('aiws-props-page-title')).toBeVisible()
  // closed by the user; pinned is remembered (user state) for the next opening
  await page.getByTestId('aiws-props-close').click()
  await expect(panel(page)).toHaveCount(0)
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  await page.keyboard.press('F2')
  await expect(panel(page)).toHaveAttribute('data-pinned', 'true')
  await page.getByTestId('aiws-props-pin').click()
  await expect(panel(page)).toHaveAttribute('data-pinned', 'false')
  // the blank spot's menu has "page settings": the page's properties
  await page.getByTestId('aiws-props-close').click()
  // a blank spot: the fit leaves a margin above the bottom tool area
  const main = (await page.locator('.aiws-canvas-main').boundingBox())!
  await page.mouse.click(main.x + main.width / 2, main.y + main.height - 70, { button: 'right' })
  await page.getByTestId('aiws-context-menu').getByTestId('aiws-menu-page-settings').click()
  await expect(panel(page)).toHaveAttribute('data-subject', 'page')
})

test('§15-32 one panel at a time, at most 560 px and scrolling inside; folded sections stay folded; layout numbers write one undoable step', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `blockui-32 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.keyboard.press('F2')
  await expect(panel(page)).toHaveAttribute('data-subject', 'blk-shape')
  await page.getByTestId('aiws-canvas-block-blk-note').click()
  await page.keyboard.press('F2')
  await expect(panel(page)).toHaveCount(1)
  await expect(panel(page)).toHaveAttribute('data-subject', 'blk-note')
  // the shape has many properties: the panel stops at 560 px and its body scrolls under a fixed title
  await page.getByTestId('aiws-props-close').click()
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.keyboard.press('F2')
  const height = (await panel(page).boundingBox())!.height
  expect(height).toBeLessThanOrEqual(561)
  const body = panel(page).getByTestId('aiws-inspector')
  expect(await body.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true)
  await body.evaluate((el) => { el.scrollTop = el.scrollHeight })
  await expect(panel(page).getByTestId('aiws-props-head')).toBeInViewport()
  // the sections in their order; diagnostics folded at first
  const sections = await panel(page).locator('[data-testid^="aiws-props-section-"]').evaluateAll((els) => els.map((el) => el.getAttribute('data-testid')!.replace('aiws-props-section-', '')))
  expect(sections).toEqual(['common', 'type-look', 'type-text', 'drawn', 'layout', 'interaction', 'presentation', 'diagnostics'])
  await expect(panel(page).getByTestId('aiws-props-section-diagnostics')).toHaveAttribute('data-collapsed', 'true')
  // fold the text section: closed and opened again, and in a new window, it is still folded (per type, user state)
  await panel(page).getByTestId('aiws-props-toggle-type-text').click()
  await expect(panel(page).getByTestId('aiws-props-section-type-text')).toHaveAttribute('data-collapsed', 'true')
  await page.getByTestId('aiws-props-close').click()
  await page.keyboard.press('F2')
  await expect(panel(page).getByTestId('aiws-props-section-type-text')).toHaveAttribute('data-collapsed', 'true')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.keyboard.press('F2')
  await expect(panel(page).getByTestId('aiws-props-section-type-text')).toHaveAttribute('data-collapsed', 'true')
  await expect(panel(page).getByTestId('aiws-props-section-type-look')).toHaveAttribute('data-collapsed', 'false')
  // layout: the position as numbers (new), one commit, undone in one step
  const x = panel(page).getByTestId('aiws-props-layout-x')
  await x.scrollIntoViewIfNeeded()
  await expect(x).toHaveValue('300')
  await x.fill('340')
  await x.press('Enter')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'blk-shape').placement.x).toBe(340)
  await panel(page).getByTestId('aiws-props-head').click()
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).find((e: Json) => e.entity_id === 'blk-shape').placement.x).toBe(300)
})

test('§15-33 an extension control\'s declared action is on its Block toolbar and in its context menu', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `blockui-33 ${Date.now()}`)
  const built = await api.build(ALICE, ws.workspace_id, {
    'component.json': { manifest_version: 4, ident: { author_did: 'did:bns:alice', name: 'stamp-e2e' }, kind: 'declarative', title: 'Stamp', allow_no_source: true, entry: 'declarative.json',
      config_schema: { type: 'object', properties: { note: { type: 'string', default: '' } } },
      actions: [{ id: 'stamp', title: 'Stamp it', icon: 'check', modes: ['edit', 'view'], needs: ['update'], run: { kind: 'config', props: { note: 'stamped' } } }] },
    'declarative.json': { items: [{ kind: 'text', text: 'card' }] },
  })
  expect((await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'card', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 60, y: 60, w: 240, h: 120 },
    payload: { view: { type: 'declarative' }, def_ref: { entity_id: built.def_entity_id } } }])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-card').click()
  // a primary action: an icon button of the toolbar (its icon name is one the host knows)
  const tool = page.getByTestId('aiws-near-toolbar').getByTestId('aiws-near-ext-action-stamp')
  await expect(tool).toBeVisible()
  await expect(tool.locator('svg')).toHaveCount(1)
  // and in the context menu, with the type's other actions
  await page.getByTestId('aiws-canvas-block-card').click({ button: 'right' })
  const menu = page.getByTestId('aiws-context-menu')
  await expect(menu.getByTestId('aiws-menu-ext-action-stamp')).toBeVisible()
  await menu.getByTestId('aiws-menu-ext-action-stamp').click()
  await expect.poll(async () => (await configOf(api, ws, 'card')).note).toBe('stamped')
  // its properties are in the panel's type section (the same form as before), its definition under "data"
  await page.getByTestId('aiws-canvas-block-card').click()
  await page.keyboard.press('F2')
  await expect(panel(page).getByTestId('aiws-instance-config-note')).toHaveValue('stamped')
  await expect(panel(page).getByTestId('aiws-props-section-data').getByTestId('aiws-instance-open-def')).toBeVisible()
})

test('§15-34 one quick property, two places: changed in the toolbar it shows in the panel, changed in the panel it shows in the toolbar', async ({ page, api }) => {
  const ws = await objectsWorkspace(api, `blockui-34 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-shape').click()
  await page.keyboard.press('F2')
  const fillField = panel(page).getByTestId('aiws-shape-field-fill')
  await expect(fillField.getByTestId('aiws-color-e8f0fe')).toHaveCount(0)
  // toolbar → panel
  await page.getByTestId('aiws-near-shape-fill').click()
  await page.getByTestId('aiws-palette-shape-fill').getByTestId('aiws-color-dbe8ff').click()
  await expect.poll(async () => (await configOf(api, ws, 'blk-shape')).fill).toBe('#dbe8ff')
  await expect(fillField.getByTestId('aiws-color-dbe8ff')).toHaveAttribute('aria-pressed', 'true')
  // panel → toolbar
  await fillField.getByTestId('aiws-color-ffd9d9').click()
  await expect.poll(async () => (await configOf(api, ws, 'blk-shape')).fill).toBe('#ffd9d9')
  await expect(page.getByTestId('aiws-near-shape-fill').locator('.aiws-swatch')).toHaveCSS('background-color', 'rgb(255, 217, 217)')
  await expect(fillField.getByTestId('aiws-color-dbe8ff')).toHaveAttribute('aria-pressed', 'false')
  // a toggle: bold from the panel is pressed in the toolbar
  const bold = panel(page).getByTestId('aiws-shape-field-bold')
  await bold.scrollIntoViewIfNeeded()
  await bold.click()
  await expect.poll(async () => (await configOf(api, ws, 'blk-shape')).bold).toBe(true)
  await expect(page.getByTestId('aiws-near-shape-bold')).toHaveAttribute('aria-pressed', 'true')
  // each change is one undoable step (Ctrl+Z from outside a field)
  await panel(page).getByTestId('aiws-props-head').click()
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await configOf(api, ws, 'blk-shape')).bold ?? null).toBeNull()
  await expect(bold).not.toBeChecked()
  await expect(page.getByTestId('aiws-near-shape-bold')).toHaveAttribute('aria-pressed', 'false')
})
