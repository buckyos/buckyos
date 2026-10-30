/* Extension controls (notepads/aiworkspace-extension-p0-todo.md M5): a control made from a skeleton in alice's home
 * folder and built into the workspace, an instance initialized with a named binding, a source change rebuilt into the
 * instance, a broken source refused while the instance keeps the last build, the export to the library and to a
 * bundle folder, and the control added from the library into a second workspace. The home folder is the standalone
 * nfs-server's `home` export (the Desktop reaches it through `/nfs/v1`, as through a Zone gateway). */

import { NfspClient } from 'buckyos/nfsp'
import { expect, mainMenu, openCanvas, test } from './fixtures'

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
