/* Extension controls (notepads/done/aiworkspace-extension-p0-todo.md M5, p1-todo.md M5): a control made from a skeleton in
 * alice's home folder and built into the workspace, an instance initialized with a named binding, a source change
 * rebuilt into the instance, a broken source refused while the instance keeps the last build, the export to the library
 * and to a bundle folder, and the control added from the library into a second workspace; composite controls inserted,
 * laid out from their composition and rebuilt, copied (no mind map around a copied card, an instance through
 * `tree.clone`), packed by the service, and a selection defined as a control. The home folder is the standalone
 * nfs-server's `home` export (the Desktop reaches it through `/nfs/v1`, as through a Zone gateway). */

import { NfspClient } from 'buckyos/nfsp'
import { expect, fitAll, mainMenu, openCanvas, openSide, openWorkspacePage, test } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
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
  const rows = (await api.table(ALICE, ws.workspace_id, { source_id: 'tasks', limit: 500 })).rows.length
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
  // placed once its bindings are settled (on_bind)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).some((e: { type_id: string; def_id?: string }) => e.type_id === 'buckyos.cell' && e.def_id === defId)).toBe(true)
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

// ---- composite controls (notepads/done/aiworkspace-extension-p1-todo.md M5)

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
  // placed once its bindings are settled (on_bind)
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).some((e: J) => e.kind === 'composite')).toBe(true)
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

// ---- events, run mode, parent, upgrades, templates (notepads/done/aiworkspace-extension-p2p3-todo.md M2–M5)

const TASK = (status = 'open') => ({ op: 'entity.create', entity_id: 'task', type_id: 'buckyos.record', parent_id: 'data', order_key: 't', name: 'task',
  payload: { schema: { properties: [{ key: 'title', name: 'title', type: 'text' }, { key: 'status', name: 'status', type: 'text' }] }, props: { title: 'Write tests', status } } })
const DONE_COMPONENT = {
  manifest_version: 3, ident: { author_did: 'did:bns:alice', name: 'done-e2e' }, kind: 'html', host_api: 3, title: 'Done E2E', allow_no_source: true,
  default_size: { w: 240, h: 120 }, bindings: { task: { accepts: ['buckyos.record'] } },
  events: { completed: { title: 'Completed', params: { status: { type: 'string' } } } }, entry: { html: 'view.html', js: 'behavior.js' },
}
const doneBehavior = (status: string) => `document.getElementById('done').addEventListener('click', function () {
  aiws.emit('completed', { status: '${status}' }).then(function (r) { document.getElementById('out').textContent = (r.handled ? 'handled ' : 'not handled ') + aiws.version })
}); aiws.ready()`
const DONE_FILES = (status: string) => ({ 'component.json': DONE_COMPONENT, 'view.html': '<button id="done" type="button">Done</button><div id="out"></div>', 'behavior.js': doneBehavior(status) })
const WRITE_STATUS = { completed: { kind: 'write', target: 'task', what: 'recordSet', args: { props: { status: '{{event.status}}' } } } }
const BUTTON_COMPONENT = {
  manifest_version: 3, ident: { author_did: 'did:bns:alice', name: 'button-e2e' }, kind: 'declarative', title: 'Button E2E', allow_no_source: true,
  default_size: { w: 220, h: 110 }, bindings: { task: { accepts: ['buckyos.record'] } },
  events: { completed: { title: 'Completed', params: { status: { type: 'string' } } } }, entry: 'declarative.json',
}
const BUTTON_FILES = { 'component.json': BUTTON_COMPONENT, 'declarative.json': { layout: 'stack', items: [{ kind: 'text', text: 'Task' }, { kind: 'button', label: 'Done', emit: 'completed', params: { status: 'done' } }] } }

const status = async (api: import('./fixtures').Api, ws: string) => (await api.read(ALICE, ws, 'task') as J & { content: { props: J } }).content.props.status

async function runMode(page: import('@playwright/test').Page, on: boolean) {
  const toggle = page.getByTestId('aiws-run-toggle')
  if ((await toggle.getAttribute('data-interactive')) !== String(on)) await toggle.click()
  await expect(toggle).toHaveAttribute('data-interactive', String(on))
}

test('P2 run mode: a click on Done raises the event once — one Commit; a rebuilt behavior.js runs next; a reload replays nothing', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `events-e2e ${Date.now()}`)
  const built = await api.build(ALICE, ws.workspace_id, DONE_FILES('done'))
  expect((await api.commit(ALICE, ws, [TASK(), { op: 'entity.create', entity_id: 'card', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 260, h: 200 },
    payload: { view: { type: 'html' }, def_ref: { entity_id: built.def_entity_id }, bindings: { task: { entity_id: 'task' } }, on: WRITE_STATUS } }])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  // edit mode designs: the Block is its snapshot; ▶ runs the canvas, a click on the Block mounts it
  await expect(page.getByTestId('aiws-html-start-card')).toHaveCount(0)
  await runMode(page, true)
  await page.getByTestId('aiws-html-start-card').click()
  await expect(page.getByTestId('aiws-html-active-card')).toBeVisible()
  const frame = page.frameLocator('.aiws-html-frame')
  let head = await api.headSeq(ALICE, ws.workspace_id)
  await frame.locator('#done').click()
  await expect(frame.locator('#out')).toHaveText('handled 3')
  await expect.poll(() => status(api, ws.workspace_id)).toBe('done')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  const changes = await api.rpc(ALICE, 'doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq: head })
  expect((changes.changes as { idempotency_key: string }[]).map((c) => c.idempotency_key)[0]).toMatch(/^evt\/card\/completed\//)
  // a changed behavior.js, built: the Block remounts with it and the next click runs the new code — still one Commit
  await api.build(ALICE, ws.workspace_id, DONE_FILES('archived'), { expect: { def_rev: built.def_rev, dir_rev: built.dir_rev } })
  await expect(frame.locator('#out')).toHaveText('')
  head = await api.headSeq(ALICE, ws.workspace_id)
  await frame.locator('#done').click()
  await expect.poll(() => status(api, ws.workspace_id)).toBe('archived')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  // a reload mounts nothing by itself and raises nothing again
  head = await api.headSeq(ALICE, ws.workspace_id)
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-html-static-card')).toBeVisible()
  await page.waitForTimeout(1500)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
})

test('P2 design mode: a click on a declarative button selects the Block and notes the event; in run mode it writes once', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `design-e2e ${Date.now()}`)
  const built = await api.build(ALICE, ws.workspace_id, BUTTON_FILES)
  expect((await api.commit(ALICE, ws, [TASK(), { op: 'entity.create', entity_id: 'btn', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 240, h: 140 },
    payload: { view: { type: 'declarative' }, def_ref: { entity_id: built.def_entity_id }, bindings: { task: { entity_id: 'task' } }, on: WRITE_STATUS } }])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await runMode(page, false)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // in design mode the button is drawn, not enabled: the press goes to the canvas
  await expect(page.getByTestId('aiws-decl-button-completed')).toHaveAttribute('aria-disabled', 'true')
  await page.getByTestId('aiws-decl-button-completed').click({ force: true })
  await expect(page.getByTestId('aiws-canvas-block-btn')).toHaveClass(/is-selected/)
  await expect(page.getByTestId('aiws-event-note-btn')).toHaveAttribute('data-event', 'completed')
  await expect(page.getByTestId('aiws-event-note-btn')).toContainText('设计态，未执行')
  await page.waitForTimeout(500)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  expect(await status(api, ws.workspace_id)).toBe('open')
  // the events page of the instance shows what it binds
  await openSide(page, 'inspector')
  await page.getByTestId('aiws-instance-page-events').click()
  await expect(page.getByTestId('aiws-events-kind-completed-0')).toHaveValue('write')
  await expect(page.getByTestId('aiws-events-target-completed-0')).toHaveValue('task')
  await runMode(page, true)
  await page.getByTestId('aiws-decl-button-completed').click()
  await expect.poll(() => status(api, ws.workspace_id)).toBe('done')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
})

test('P2 view mode runs: a collaborator with update writes through a button, a reader is refused', async ({ browser, api }) => {
  const ws = await api.freeCanvas(ALICE, `view-run-e2e ${Date.now()}`)
  const built = await api.build(ALICE, ws.workspace_id, BUTTON_FILES)
  expect((await api.commit(ALICE, ws, [TASK(), { op: 'entity.create', entity_id: 'btn', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 240, h: 140 },
    payload: { view: { type: 'declarative' }, def_ref: { entity_id: built.def_entity_id }, bindings: { task: { entity_id: 'task' } }, on: WRITE_STATUS } }])).status).toBe('accepted')
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  await api.rpc(BOB, 'ws.set_user_state', { workspace_id: ws.workspace_id, entries: { 'canvas:mode': 'view' } })
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const page = await context.newPage()
  await openCanvas(page, BOB, ws.workspace_id)
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view')
  await expect(page.getByTestId('aiws-run-toggle')).toHaveAttribute('data-interactive', 'true')
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-decl-button-completed').click()
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '不能修改数据 task' }).last()).toBeVisible()
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update'] })
  await openCanvas(page, BOB, ws.workspace_id)
  await page.getByTestId('aiws-decl-button-completed').click()
  await expect.poll(() => status(api, ws.workspace_id)).toBe('done')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  await context.close()
})

test('P2 a guest of a public link views and its HTML control runs by itself: a click on the canvas does not stop it; its write is refused (owner 11-12)', async ({ browser, api }) => {
  const ws = await api.freeCanvas(ALICE, `guest-run-e2e ${Date.now()}`)
  const built = await api.build(ALICE, ws.workspace_id, DONE_FILES('done'))
  expect((await api.commit(ALICE, ws, [TASK(), { op: 'entity.create', entity_id: 'card', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 260, h: 200 },
    payload: { view: { type: 'html' }, def_ref: { entity_id: built.def_entity_id }, bindings: { task: { entity_id: 'task' } }, on: WRITE_STATUS } }])).status).toBe('accepted')
  await api.rpc(ALICE, 'ws.create_entry', { workspace_id: ws.workspace_id, capabilities: ['read'], methods: { guest: true, password: false, self_register: false, did_login: false } })
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const page = await context.newPage()
  await openWorkspacePage(page, { guest: true }, ws.workspace_id)
  await expect(page.getByTestId('aiws-principal')).toHaveAttribute('data-guest', 'true')
  if (await page.getByTestId('aiws-top-canvas').isVisible().catch(() => false)) await page.getByTestId('aiws-top-canvas').click()
  // nothing chosen: a guest views, and view mode runs
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-run-toggle')).toHaveAttribute('data-interactive', 'true')
  await expect(page.getByTestId('aiws-html-active-card')).toBeVisible()
  const frame = page.frameLocator('.aiws-html-frame')
  await expect(frame.locator('#done')).toBeVisible()
  await frame.locator('#out').evaluate((el) => { el.setAttribute('data-mark', 'same-runtime') })
  // clicks on the empty canvas and on another place: the same runtime keeps running
  await page.mouse.click(1200, 700)
  await page.mouse.click(900, 500)
  await page.waitForTimeout(500)
  await expect(page.getByTestId('aiws-html-active-card')).toBeVisible()
  await expect(frame.locator('#out')).toHaveAttribute('data-mark', 'same-runtime')
  // it runs for the guest, but what it writes needs update
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await frame.locator('#done').click()
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '不能修改数据 task' }).last()).toBeVisible()
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  await context.close()
})

test('P2 a composite re-raises its card\'s event: hello-card Done → hello-board card_done → the board\'s notice', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `bubble-e2e ${Date.now()}`)
  const card = await api.build(ALICE, ws.workspace_id, 'hello-card')
  const board = await api.build(ALICE, ws.workspace_id, 'hello-board')
  expect((await api.commit(ALICE, ws, [TASK(),
    { op: 'entity.create', entity_id: 'board', type_id: 'buckyos.container', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 'auto', h: 'auto' },
      payload: { kind: 'composite', def_ref: { entity_id: board.def_entity_id }, on: { card_done: { kind: 'notify', text: 'Card {{event.title}} done' } } } },
    { op: 'entity.create', entity_id: 'hc', type_id: 'buckyos.cell', parent_id: 'board', order_key: 'a', placement: { x: 'auto', y: 'auto', w: 260, h: 200 },
      payload: { view: { type: 'html' }, def_ref: { entity_id: card.def_entity_id }, slot: 'cards', bindings: { task: { entity_id: 'task' } } } },
  ])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await runMode(page, true)
  await page.getByTestId('aiws-html-start-hc').click()
  const frame = page.frameLocator('.aiws-html-frame')
  await expect(frame.locator('#title')).toHaveText('Write tests')
  await frame.locator('#done').click()
  await expect(page.getByTestId('aiws-notice').filter({ hasText: 'Card Write tests done' }).last()).toBeVisible()
  await expect.poll(() => status(api, ws.workspace_id)).toBe('done')
})

test('P3 aiws.createBlock { parent, slot }: an HTML control puts a Block into a composite\'s slot; without the slot it is refused', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `parent-e2e ${Date.now()}`)
  const holder = await api.build(ALICE, ws.workspace_id, {
    'component.json': { manifest_version: 3, ident: { author_did: 'did:bns:alice', name: 'holder-e2e' }, kind: 'composite', title: 'Holder', entry: 'composition.json' },
    'composition.json': { layout: { algorithm: 'grid', columns: 2, gap: 6, padding: 8 }, slots: { items: { accepts: ['shape'] } }, chrome: { empty: 'Nothing yet' } },
  })
  const maker = await api.build(ALICE, ws.workspace_id, {
    'component.json': { manifest_version: 3, ident: { author_did: 'did:bns:alice', name: 'maker-e2e' }, kind: 'html', host_api: 3, title: 'Maker', allow_no_source: true,
      config_schema: { type: 'object', properties: { parent: { type: 'string' } } }, entry: { html: 'view.html', js: 'behavior.js' } },
    'view.html': '<button id="make" type="button">Make</button><button id="bad" type="button">Bad</button><div id="out"></div><div id="err"></div>',
    'behavior.js': `var parentId = aiws.context.config.parent
      document.getElementById('make').addEventListener('click', function () { aiws.createBlock({ renderer: 'shape', parent: parentId, slot: 'items', title: 'made', config: { shape: 'rect' }, placement: { w: 120, h: 60 } }).then(function (r) { document.getElementById('out').textContent = r.entity_id }) })
      document.getElementById('bad').addEventListener('click', function () { aiws.createBlock({ renderer: 'shape', parent: parentId, title: 'bad' }).catch(function (e) { document.getElementById('err').textContent = String(e.message) }) })
      aiws.ready()`,
  })
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'holder', type_id: 'buckyos.container', parent_id: 'free', order_key: 'a', placement: { x: 400, y: 40, w: 'auto', h: 'auto' }, payload: { kind: 'composite', def_ref: { entity_id: holder.def_entity_id } } },
    { op: 'entity.create', entity_id: 'maker', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'b', placement: { x: 40, y: 40, w: 300, h: 200 },
      payload: { view: { type: 'html' }, def_ref: { entity_id: maker.def_entity_id }, config: { parent: 'holder' } } },
  ])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-maker').click()
  await page.getByTestId('aiws-near-run').click()
  const frame = page.frameLocator('.aiws-html-frame')
  await frame.locator('#make').click()
  await expect(frame.locator('#out')).not.toHaveText('')
  const made = await frame.locator('#out').textContent()
  const child = (await api.outline(ALICE, ws.workspace_id)).find((e: J) => e.entity_id === made) as J
  expect([child.parent_id, child.slot, child.title]).toEqual(['holder', 'items', 'made'])
  expect((child.placement as J).x).toBe('auto')
  await expect(page.getByTestId(`aiws-canvas-block-${made}`)).toBeVisible()
  await frame.locator('#bad').click()
  await expect(frame.locator('#err')).toContainText('必须给出 slot')
})

test('P2 upgrade: a control imported at 0.1.0 is upgraded to 0.2.0 from the library; the instance keeps its overrides', async ({ page, api }) => {
  const src = await api.freeCanvas(ALICE, `upgrade-src ${Date.now()}`)
  const UP = { manifest_version: 3, ident: { author_did: 'did:bns:alice', name: 'up-e2e' }, kind: 'declarative', title: 'Up E2E', allow_no_source: true,
    config_schema: { type: 'object', properties: { label: { type: 'string', default: 'A' } } }, entry: 'declarative.json' }
  const pkg = `up-e2e-${Date.now()}`
  const v1 = await api.build(ALICE, src.workspace_id, { 'component.json': UP, 'declarative.json': { items: [{ kind: 'text', text: 'version one' }] } })
  await api.rpc(ALICE, 'ext.install', { from: { workspace_id: src.workspace_id, entity_ids: [v1.def_entity_id] }, package: { name: pkg, version: '0.1.0' }, mode: 'dev' })
  const ws = await api.freeCanvas(ALICE, `upgrade-e2e ${Date.now()}`)
  const imported = await api.rpc(ALICE, 'ext.import', { workspace_id: ws.workspace_id, items: [{ pkg_id: pkg, ident: 'did:bns:alice#up-e2e' }] })
  const defId = imported.controls[0].def_entity_id as string
  expect((await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'inst', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 240, h: 120 },
    payload: { view: { type: 'declarative' }, def_ref: { entity_id: defId }, config: { label: 'Mine' }, title: 'my card' } }])).status).toBe('accepted')
  // a new version in the library
  await api.build(ALICE, src.workspace_id, { 'component.json': UP, 'declarative.json': { items: [{ kind: 'text', text: 'version two' }] } }, { expect: { def_rev: v1.def_rev, dir_rev: v1.dir_rev } })
  await api.rpc(ALICE, 'ext.install', { from: { workspace_id: src.workspace_id, entity_ids: [v1.def_entity_id] }, package: { name: pkg, version: '0.2.0' }, mode: 'dev' })
  const before = await api.read(ALICE, ws.workspace_id, 'inst') as J & { content: { payload: J } }
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-decl-inst')).toContainText('version one')
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  const dialog = page.getByTestId('aiws-ext-dialog')
  await dialog.getByTestId(`aiws-ext-row-${defId}`).click()
  await expect(dialog.getByTestId('aiws-ext-new-version')).toHaveAttribute('data-version', '0.2.0')
  await dialog.getByTestId('aiws-ext-upgrade').click()
  const form = dialog.getByTestId('aiws-ext-upgrade-form')
  await expect(form.getByTestId('aiws-ext-upgrade-versions')).toContainText('0.1.0 → 0.2.0')
  await expect(form.getByTestId('aiws-ext-upgrade-compatible')).toBeVisible()
  await expect(form.getByTestId('aiws-ext-upgrade-checked')).toBeVisible({ timeout: 30_000 })
  await form.getByTestId('aiws-ext-upgrade-run').click()
  await expect(dialog.getByTestId('aiws-ext-upgrade-form')).toBeHidden({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-decl-inst')).toContainText('version two')
  const def = await api.read(ALICE, ws.workspace_id, defId) as J & { content: { payload: J } }
  expect((def.content.payload.provenance as J).version).toBe('0.2.0')
  const after = await api.read(ALICE, ws.workspace_id, 'inst') as J & { content: { payload: J } }
  expect(after.content.payload).toEqual(before.content.payload)
  // the library offers the upgrade for an identity that is here already, instead of a second definition
  await dialog.getByTestId('aiws-ext-library').click()
  await expect(dialog.getByTestId('aiws-ext-library-upgrade-up-e2e')).toBeVisible()
})

test('P3 templates: canvases saved as a template into the library, a new workspace made from it — the KPI binds the new table', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `template-src ${Date.now()}`)
  const kpi = await api.build(ALICE, ws.workspace_id, {
    'component.json': { manifest_version: 3, ident: { author_did: 'did:bns:alice', name: 'kpi-tpl-e2e' }, kind: 'declarative', title: 'KPI', entry: 'declarative.json',
      accepts: ['buckyos.table-source'], allow_no_source: true, bindings: { data: { accepts: ['buckyos.table-source'], required: true } } },
    'declarative.json': { items: [{ kind: 'metric', label: 'Tasks', bind: 'data.count', format: 'number' }] },
  })
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'tasks', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 't', name: 'tasks', payload: { fields: [{ field_id: 'title', name: 'title', type: 'text' }] } },
    { op: 'table.insert_records', source_id: 'tasks', records: [{ record_id: 'r1', values: { title: 'one' } }, { record_id: 'r2', values: { title: 'two' } }] },
    { op: 'entity.create', entity_id: 'kpi', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 240, h: 120 },
      payload: { view: { type: 'declarative' }, def_ref: { entity_id: kpi.def_entity_id }, bindings: { data: { entity_id: 'tasks' } }, title: '{{params.team_name}} KPI' } },
  ])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await mainMenu(page, 'aiws-menu-export-menu', 'aiws-menu-save-template')
  const save = page.getByTestId('aiws-save-template')
  await expect(save.getByTestId('aiws-template-surface-free')).toBeChecked()
  await save.getByTestId('aiws-template-title').fill('Team E2E')
  await save.getByTestId('aiws-template-id').fill('team-e2e')
  await save.getByTestId('aiws-template-param-add').click()
  await save.getByTestId('aiws-template-param-key-0').fill('team_name')
  await save.getByTestId('aiws-template-param-default-0').fill('Team')
  await save.getByTestId('aiws-template-reset-tasks').check()
  const pkg = `team-tpl-${Date.now()}`
  await save.getByTestId('aiws-template-package').fill(pkg)
  await save.getByTestId('aiws-template-to-library').click()
  await expect(save.getByTestId('aiws-template-done')).toContainText(pkg, { timeout: 30_000 })
  await save.getByTestId('aiws-save-template-close').click()
  // installing created no workspace; the New dialog lists the template beside the built-in ones
  const listed = (await api.rpc(ALICE, 'ws.list', {})).workspaces.length
  await mainMenu(page, 'aiws-menu-new')
  await page.getByTestId('aiws-new-tab-template').click()
  await page.getByTestId(`aiws-template-lib-${pkg}-team-e2e`).click()
  await page.getByTestId('aiws-template-param-team_name').fill('Alpha')
  await page.getByTestId('aiws-template-workspace-title').fill(`from template ${Date.now()}`)
  await page.getByTestId('aiws-create-template').click()
  await expect.poll(async () => page.getByTestId('aiws-workspace').getAttribute('data-workspace-id'), { timeout: 30_000 }).not.toBe(ws.workspace_id)
  const made = await page.getByTestId('aiws-workspace').getAttribute('data-workspace-id')
  expect((await api.rpc(ALICE, 'ws.list', {})).workspaces.length).toBe(listed + 1)
  const outline = await api.outline(ALICE, made!)
  const cell = outline.find((e: J) => e.type_id === 'buckyos.cell' && e.def_id && outline.find((d: J) => d.entity_id === e.def_id)?.ident === 'did:bns:alice#kpi-tpl-e2e') as J
  expect(cell.title).toBe('Alpha KPI')
  const read = await api.read(ALICE, made!, String(cell.entity_id)) as J & { content: { payload: J } }
  const table = ((read.content.payload.bindings as J).data as J).entity_id as string
  expect(outline.find((e: J) => e.entity_id === table)?.type_id).toBe('buckyos.table-source')
  expect((await api.table(ALICE, made!, { source_id: table, limit: 10 })).rows).toHaveLength(0)
  // the source workspace still has its rows
  expect((await api.table(ALICE, ws.workspace_id, { source_id: 'tasks', limit: 10 })).rows).toHaveLength(2)
})

test('Q33 a host older than the definition: a layout it does not ship is a marked group, an HTML API above it shows the snapshot fallback', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `fallback-e2e ${Date.now()}`)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'd-radial', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'd1',
      payload: { def_id: 'radial-e2e', kind: 'composite', title: 'Radial', composition: { layout: { algorithm: 'radial' }, slots: { items: {} } } } },
    { op: 'entity.create', entity_id: 'd-future', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'd2',
      payload: { def_id: 'future-e2e', kind: 'html', title: 'Future', allow_no_source: true, html: { html: '<div/>', js: 'aiws.ready()', api_version: 7 } } },
    { op: 'entity.create', entity_id: 'ring', type_id: 'buckyos.container', parent_id: 'free', order_key: 'a', placement: { x: 40, y: 40, w: 'auto', h: 'auto' }, payload: { kind: 'composite', def_ref: { entity_id: 'd-radial' } } },
    { op: 'entity.create', entity_id: 'r1', type_id: 'buckyos.cell', parent_id: 'ring', order_key: 'a', placement: { x: 20, y: 30, w: 160, h: 80 }, payload: { view: { type: 'shape' }, slot: 'items', title: 'one', config: { shape: 'rect' } } },
    { op: 'entity.create', entity_id: 'future', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'b', placement: { x: 500, y: 40, w: 240, h: 140 }, payload: { view: { type: 'html' }, def_ref: { entity_id: 'd-future' } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-composite-unknown-ring')).toContainText('radial')
  await expect(page.getByTestId('aiws-canvas-block-r1')).toBeVisible()
  await expect(page.getByTestId('aiws-block-fallback-future')).toHaveAttribute('data-reason', 'unsupported_api')
  await expect(page.getByTestId('aiws-block-fallback-future')).toContainText('需要宿主 API 7')
  await runMode(page, true)
  await expect(page.locator('.aiws-html-frame')).toHaveCount(0)
})
