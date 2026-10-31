/* Extension controls (notepads/aiworkspace-extension-p0-todo.md M5, p1-todo.md M5): a control made from a skeleton in
 * alice's home folder and built into the workspace, an instance initialized with a named binding, a source change
 * rebuilt into the instance, a broken source refused while the instance keeps the last build, the export to the library
 * and to a bundle folder, and the control added from the library into a second workspace; composite controls inserted,
 * laid out from their composition and rebuilt, copied (no mind map around a copied card, an instance through
 * `tree.clone`), packed by the service, and a selection defined as a control. The home folder is the standalone
 * nfs-server's `home` export (the Desktop reaches it through `/nfs/v1`, as through a Zone gateway). */

import { NfspClient } from 'buckyos/nfsp'
import { expect, fitAll, mainMenu, openCanvas, openWorkspacePage, test } from './fixtures'

const ALICE = 'tok-alice'
const NAME = 'kpi-e2e'

function home(): NfspClient {
  return new NfspClient({ baseUrl: `http://127.0.0.1:${process.env.AIWS_E2E_NDM_PORT}` })
}

/** Replace one file of the control's source folder (what an editor saving it does). */
async function save(dir: string, name: string, text: string) {
  const nfsp = home()
  await nfsp.hello()
  const folder = await nfsp.stat(dir)
  await nfsp.uploadFile(folder.ref, name, new TextEncoder().encode(text), { overwrite: true })
}

test('a control from home: build, an instance with a binding, rebuild, a refused build, export and the library', async ({ page, api }) => {
  const ws = await api.sample(ALICE, '控件开发 E2E')
  const rows = (await api.rpc(ALICE, 'doc.query', { workspace_id: ws.workspace_id, source_id: 'tasks', limit: 500 })).rows.length
  await openCanvas(page, ALICE, ws.workspace_id)

  // new control: a folder in alice's home with a working sample, built at once
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  const dialog = page.getByTestId('aiws-ext-dialog')
  await expect(dialog).toBeVisible()
  await dialog.getByTestId('aiws-ext-new').click()
  await dialog.getByTestId('aiws-ext-new-name').fill(NAME)
  await dialog.getByTestId('aiws-ext-new-title').fill('任务计数')
  await dialog.getByTestId('aiws-ext-new-create').click()
  await expect(dialog.getByTestId('aiws-ext-ident')).toHaveText(`did:bns:alice#${NAME}`, { timeout: 30_000 })
  const dir = await dialog.getByTestId('aiws-ext-dir').inputValue()
  expect(dir).toBe(`/home/alice/AI Workspace/控件开发 E2E/controls/${NAME}`)
  const defId = await dialog.getByTestId('aiws-ext-detail').getAttribute('data-def')
  expect(defId).toBeTruthy()
  const outline = await api.outline(ALICE, ws.workspace_id)
  expect(outline.find((e: { entity_id: string }) => e.entity_id === defId).ident).toBe(`did:bns:alice#${NAME}`)

  // an instance: the catalog opens at the control and asks for its binding
  await dialog.getByTestId('aiws-ext-insert').click()
  await expect(dialog).toBeHidden()
  const catalog = page.getByTestId('aiws-catalog')
  await expect(catalog.getByTestId('aiws-catalog-detail')).toHaveAttribute('data-entry', `def:${defId}`)
  await expect(catalog.getByTestId('aiws-instance-form')).toBeVisible()
  await catalog.getByTestId('aiws-instance-binding-data').selectOption('tasks')
  await catalog.getByTestId('aiws-instance-config-label').fill('数量')
  await catalog.getByTestId('aiws-catalog-insert').click()
  await expect(catalog).toBeHidden()
  const cell = (await api.outline(ALICE, ws.workspace_id)).find((e: { type_id: string; def_id?: string }) => e.type_id === 'buckyos.cell' && e.def_id === defId)
  expect(cell).toBeTruthy()
  const cellRead = await api.read(ALICE, ws.workspace_id, cell.entity_id)
  expect(cellRead.content.payload.bindings).toEqual({ data: { entity_id: 'tasks' } })
  expect(cellRead.content.payload.config).toEqual({ label: '数量' })
  const instance = page.getByTestId(`aiws-decl-${cell.entity_id}`)
  await expect(instance.getByTestId('aiws-decl-metric')).toContainText(String(rows))

  // a saved change builds into the instance
  await save(dir, 'declarative.json', JSON.stringify({ layout: 'stack', items: [{ kind: 'metric', label: '全部任务', bind: 'data.count', format: 'number' }] }))
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  await dialog.getByTestId(`aiws-ext-row-${defId}`).click()
  await dialog.getByTestId('aiws-ext-build').click()
  await expect(dialog.getByTestId('aiws-ext-build-ok')).toBeVisible({ timeout: 30_000 })
  await expect(instance).toContainText('全部任务')

  // a broken component.json: refused with its line, the instance keeps the last build
  await save(dir, 'component.json', '{\n  "manifest_version": 1,\n  oops\n}\n')
  await dialog.getByTestId('aiws-ext-build').click()
  await expect(dialog.getByTestId('aiws-ext-build-failed')).toBeVisible({ timeout: 30_000 })
  await expect(dialog.getByTestId('aiws-ext-diagnostics')).toContainText('component.json:3')
  await expect(instance).toContainText('全部任务')
  const def = await api.read(ALICE, ws.workspace_id, defId!)
  expect(def.content.payload.version).toBe(2)

  // export: into alice's library, and as a bundle folder in her home
  await dialog.getByTestId(`aiws-ext-check-${defId}`).check()
  await dialog.getByTestId('aiws-ext-export').click()
  await dialog.getByTestId('aiws-ext-export-name').fill('e2e-pack')
  await dialog.getByTestId('aiws-ext-export-library').click()
  await expect(dialog.getByTestId('aiws-ext-export-done')).toContainText('e2e-pack')
  await dialog.getByTestId('aiws-ext-export-bundle').click()
  await expect(dialog.getByTestId('aiws-ext-export-done')).toContainText('/home/alice/AI Workspace/exports/e2e-pack/aiws')
  const nfsp = home()
  await nfsp.hello()
  const bundle = await nfsp.list('/home/alice/AI Workspace/exports/e2e-pack/aiws', {}, ['base'])
  expect(bundle.entries.map((e) => e.name).sort()).toEqual(['aiws-manifest.json', 'controls'])
  const library = await api.rpc(ALICE, 'ext.list_library', {})
  expect(library.packages.find((p: { pkg_id: string }) => p.pkg_id === 'e2e-pack').controls[0].ident).toBe(`did:bns:alice#${NAME}`)
  await dialog.getByTestId('aiws-ext-close').click()
  await expect(dialog).toBeHidden()

  // a second workspace adds it from the library
  const ws2 = await api.sample(ALICE, '控件使用 E2E')
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-back').click()
  await page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${ws2.workspace_id}"]`).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws2.workspace_id)
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  await dialog.getByTestId('aiws-ext-library').click()
  await dialog.getByTestId(`aiws-ext-library-pick-${NAME}`).check()
  await dialog.getByTestId('aiws-ext-library-add').click()
  await expect(dialog.getByTestId('aiws-ext-list')).toContainText('任务计数')
  const imported = (await api.outline(ALICE, ws2.workspace_id)).find((e: { ident?: string }) => e.ident === `did:bns:alice#${NAME}`)
  expect(imported.provenance_pkg).toBe('e2e-pack')
})

// ---- composite controls (notepads/aiworkspace-extension-p1-todo.md M5)

type J = Record<string, unknown>
const b64 = (text: string) => Buffer.from(text, 'utf8').toString('base64')
const files = (map: Record<string, unknown>) => Object.entries(map).map(([name, v]) => ({ name, base64: b64(typeof v === 'string' ? v : JSON.stringify(v)) }))

const CARD = {
  manifest_version: 2, ident: { author_did: 'did:bns:alice', name: 'card-e2e' }, kind: 'declarative', title: 'Card E2E', allow_no_source: true,
  default_size: { w: 200, h: 80 }, entry: 'declarative.json',
}
const BOARD = {
  manifest_version: 2, ident: { author_did: 'did:bns:alice', name: 'board-e2e' }, kind: 'composite', title: 'Board E2E', entry: 'composition.json',
  config_schema: { type: 'object', properties: { title: { type: 'string', default: 'Today' }, gap: { type: 'integer', default: 8 } } },
  deps: ['did:bns:alice#card-e2e'],
}
const composition = (direction: 'column' | 'row', spacing: number | string) => ({
  layout: { algorithm: 'stack', direction, spacing, padding: 10 },
  slots: { cards: { accepts: ['cell:did:bns:alice#card-e2e'] } },
  chrome: { background: '#f6f7fb', radius: 12, title: { text: '{{config.title}}' }, empty: 'Drop cards' },
})

/** A workspace with a free canvas, the card and the board built into it. */
async function compositeWorkspace(api: import('./fixtures').Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const surface = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'free-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'free', title: 'Free' } },
    { op: 'entity.create', entity_id: 'free', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Free', content_folder_id: 'free-content' } },
  ]) as J
  expect(surface.status, JSON.stringify(surface)).toBe('accepted')
  const card = await api.rpc(ALICE, 'ext.build', { workspace_id: ws.workspace_id, files: files({ 'component.json': CARD, 'declarative.json': { layout: 'stack', items: [{ kind: 'text', text: 'card' }] } }) }) as J
  expect(card.status, JSON.stringify(card)).toBe('accepted')
  const board = await api.rpc(ALICE, 'ext.build', { workspace_id: ws.workspace_id, files: files({ 'component.json': BOARD, 'composition.json': composition('column', '{{config.gap}}') }) }) as J
  expect(board.status, JSON.stringify(board)).toBe('accepted')
  return { ws, card: String(card.def_entity_id), board, boardId: String(board.def_entity_id) }
}

async function rect(page: import('@playwright/test').Page, id: string) {
  const el = page.getByTestId(`aiws-canvas-block-${id}`)
  await expect(el).toBeVisible()
  const r = await el.boundingBox()
  if (!r) throw new Error(`${id} is not on screen`)
  return r
}

test('composite controls: inserted from the catalogue, laid out from the composition, rebuilt, copied, packed', async ({ page, api }) => {
  const { ws, card, board, boardId } = await compositeWorkspace(api, `composite-e2e ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // an instance: the container from the extension catalogue, no data asked for
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  const dialog = page.getByTestId('aiws-ext-dialog')
  await dialog.getByTestId(`aiws-ext-row-${boardId}`).click()
  await expect(dialog.getByTestId('aiws-ext-deps')).toHaveText('did:bns:alice#card-e2e')
  await dialog.getByTestId('aiws-ext-insert').click()
  const catalog = page.getByTestId('aiws-catalog')
  await expect(catalog.getByTestId('aiws-catalog-detail')).toHaveAttribute('data-entry', `def:${boardId}`)
  await catalog.getByTestId('aiws-instance-config-title').fill('Sprint')
  await catalog.getByTestId('aiws-catalog-insert').click()
  await expect(catalog).toBeHidden()
  const instance = (await api.outline(ALICE, ws.workspace_id)).find((e: J) => e.kind === 'composite') as J
  expect(instance.def_id).toBe(boardId)
  const id = String(instance.entity_id)
  await expect(page.getByTestId(`aiws-composite-title-${id}`)).toHaveText('Sprint')
  await expect(page.getByTestId(`aiws-composite-chrome-${id}`)).toContainText('Drop cards')

  // two cards in the slot: stacked under the title, 8 px apart (the definition's default gap)
  const put = await api.commit(ALICE, ws, ['c1', 'c2'].map((c, i) => ({ op: 'entity.create', entity_id: c, type_id: 'buckyos.cell', parent_id: id, order_key: ['a', 'b'][i],
    placement: { x: 'auto', y: 'auto', w: 200, h: 80 }, payload: { view: { type: 'declarative' }, def_ref: { entity_id: card }, slot: 'cards', title: c } }))) as J
  expect(put.status, JSON.stringify(put)).toBe('accepted')
  await fitAll(page)
  const [a, b] = [await rect(page, 'c1'), await rect(page, 'c2')]
  expect(Math.abs(a.x - b.x)).toBeLessThan(2)
  expect(b.y).toBeGreaterThan(a.y + a.height)
  const zoom = a.width / 200
  expect(Math.round((b.y - a.y - a.height) / zoom)).toBe(8)

  // the composition changes: a row 24 px apart; the cards are the same Blocks with the same content
  const before = await api.read(ALICE, ws.workspace_id, 'c1') as J
  const rebuilt = await api.rpc(ALICE, 'ext.build', { workspace_id: ws.workspace_id, expect: { def_rev: board.def_rev, dir_rev: board.dir_rev },
    files: files({ 'component.json': BOARD, 'composition.json': composition('row', 24) }) }) as J
  expect(rebuilt.action, JSON.stringify(rebuilt)).toBe('rebuilt')
  await expect.poll(async () => { const [x, y] = [await rect(page, 'c1'), await rect(page, 'c2')]; return Math.abs(x.y - y.y) < 2 && y.x > x.x + x.width }).toBe(true)
  expect((await api.read(ALICE, ws.workspace_id, 'c1') as J).content).toEqual(before.content)

  // a card copied onto the canvas is an ordinary Block (no mind map around it)
  await page.getByTestId('aiws-canvas-block-c1').click()
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' }).last()).toBeVisible()
  await page.mouse.click(300, 900)
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: J) => e.def_id === card && e.parent_id === 'free').length).toBe(1)
  expect((await api.outline(ALICE, ws.workspace_id)).some((e: J) => e.kind === 'mindmap')).toBe(false)

  // the instance copied: the core clones it with its cards, the definition shared
  await page.keyboard.press('Escape')
  const band = await page.getByTestId(`aiws-composite-title-${id}`).boundingBox()
  await page.mouse.click(band!.x + 6, band!.y + band!.height / 2)
  await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toHaveClass(/is-selected/)
  const notices = await page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' }).count()
  await page.keyboard.press('Control+c')
  await expect.poll(() => page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' }).count()).toBeGreaterThan(notices)
  await page.mouse.click(300, 900)
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: J) => e.kind === 'composite').length).toBe(2)
  const all = await api.outline(ALICE, ws.workspace_id)
  const copy = all.find((e: J) => e.kind === 'composite' && e.entity_id !== id) as J
  expect(copy.def_id).toBe(boardId)
  expect(all.filter((e: J) => e.parent_id === copy.entity_id).map((e: J) => e.slot)).toEqual(['cards', 'cards'])

  // a .pikg made by the service, then put into the library
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  await dialog.getByTestId(`aiws-ext-check-${boardId}`).check()
  await dialog.getByTestId('aiws-ext-export').click()
  await dialog.getByTestId('aiws-ext-export-name').fill('boards-e2e')
  await dialog.getByTestId('aiws-ext-export-pikg').click()
  await expect(dialog.getByTestId('aiws-ext-export-done')).toContainText('boards-e2e-0.1.0.pikg')
  await dialog.getByTestId('aiws-ext-pikg-install').click()
  await expect(dialog.getByTestId('aiws-ext-export-done')).toContainText('boards-e2e.alice.bns.did')
  const library = await api.rpc(ALICE, 'ext.list_library', {}) as { packages: { pkg_id: string; controls: { ident: string; deps?: string[] }[] }[] }
  const pkg = library.packages.find((p) => p.pkg_id === 'boards-e2e.alice.bns.did')!
  expect(pkg.controls.map((c) => c.ident).sort()).toEqual(['did:bns:alice#board-e2e', 'did:bns:alice#card-e2e'])
})

test('define as control: the selection becomes a composite in its place', async ({ page, api }) => {
  const { ws } = await compositeWorkspace(api, `define-e2e ${Date.now()}`)
  const put = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 's1', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'm', placement: { x: 100, y: 100, w: 160, h: 80 }, payload: { view: { type: 'shape' }, title: 'one', config: { shape: 'rect' } } },
    { op: 'entity.create', entity_id: 's2', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'n', placement: { x: 300, y: 100, w: 160, h: 80 }, payload: { view: { type: 'shape' }, title: 'two', config: { shape: 'rect' } } },
  ]) as J
  expect(put.status, JSON.stringify(put)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-s1').click()
  await page.getByTestId('aiws-canvas-block-s2').click({ modifiers: ['Shift'] })
  await page.getByTestId('aiws-near-more').click()
  await page.getByTestId('aiws-near-define-control').click()
  const define = page.getByTestId('aiws-define-dialog')
  await define.getByTestId('aiws-define-name').fill('pair-e2e')
  await define.getByTestId('aiws-define-title').fill('Pair')
  await define.getByTestId('aiws-define-create').click()
  await expect(define).toBeHidden({ timeout: 30_000 })
  const outline = await api.outline(ALICE, ws.workspace_id)
  const def = outline.find((e: J) => e.ident === 'did:bns:alice#pair-e2e') as J
  expect(def.def_kind).toBe('composite')
  const instance = outline.find((e: J) => e.kind === 'composite' && e.def_id === def.entity_id) as J
  const kids = outline.filter((e: J) => e.parent_id === instance.entity_id && !e.deleted)
  expect(kids.map((e: J) => e.title).sort()).toEqual(['one', 'two'])
  expect(kids.every((e: J) => typeof e.slot === 'string')).toBe(true)
  expect(outline.some((e: J) => e.entity_id === 's1' && !e.deleted)).toBe(false)
  // the fixed layout keeps their places: still side by side
  const [a, b] = [await rect(page, String(kids[0].entity_id)), await rect(page, String(kids[1].entity_id))]
  expect(Math.abs(a.y - b.y)).toBeLessThan(2)
})

test('Q24: on a workspace\'s own host the home folder is reached too: a new control is created and built there', async ({ page, api }) => {
  const ws = await api.rpc(ALICE, 'ws.create', { title: `own-host ${Date.now()}` }) as { workspace_id: string; epoch: string }
  const surface = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'free-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'free', title: 'Free' } },
    { op: 'entity.create', entity_id: 'free', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Free', content_folder_id: 'free-content' } },
  ]) as J
  expect(surface.status, JSON.stringify(surface)).toBe('accepted')
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'free')
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  const dialog = page.getByTestId('aiws-ext-dialog')
  await dialog.getByTestId('aiws-ext-new').click()
  await dialog.getByTestId('aiws-ext-new-name').fill('own-host-kpi')
  await dialog.getByTestId('aiws-ext-new-create').click()
  await expect(dialog.getByTestId('aiws-ext-ident')).toHaveText('did:bns:alice#own-host-kpi', { timeout: 30_000 })
  await expect(dialog.getByTestId('aiws-ext-build')).toBeEnabled()
})

test('aiws.asset: an HTML control takes a file of its assets as a blob URL (hello-card\'s check.svg)', async ({ page, api }) => {
  const { readFileSync, readdirSync } = await import('node:fs')
  const dir = '../../../doc/workspace/hello-controls/controls/hello-card'
  const ws = await api.rpc(ALICE, 'ws.create', { title: `asset ${Date.now()}` }) as { workspace_id: string; epoch: string }
  const built = await api.rpc(ALICE, 'ext.build', { workspace_id: ws.workspace_id, files: readdirSync(dir).map((name) => ({ name, base64: readFileSync(`${dir}/${name}`).toString('base64') })) }) as J
  expect(built.status, JSON.stringify(built)).toBe('accepted')
  const put = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'free-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'free', title: 'Free' } },
    { op: 'entity.create', entity_id: 'free', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Free', content_folder_id: 'free-content' } },
    { op: 'entity.create', entity_id: 'card', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 240, h: 120 }, payload: { view: { type: 'html' }, def_ref: { entity_id: built.def_entity_id } } },
  ]) as J
  expect(put.status, JSON.stringify(put)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-card').click()
  await page.getByTestId('aiws-near-run').click()
  const done = page.frameLocator('.aiws-html-frame').locator('#done')
  await expect(done).toBeVisible()
  await expect.poll(() => done.evaluate((el) => (el as HTMLElement).style.background)).toContain('blob:')
  // a name the definition does not list is refused
  const refused = await page.frameLocator('.aiws-html-frame').locator('body').evaluate(async () => {
    try { await (window as unknown as { aiws: { asset: (n: string) => Promise<string> } }).aiws.asset('behavior.js'); return 'served' } catch (e) { return String(e) }
  })
  expect(refused).toContain('behavior.js')
})
