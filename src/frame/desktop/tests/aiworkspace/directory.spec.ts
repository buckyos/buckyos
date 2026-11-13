import type { Locator, Page } from '@playwright/test'
import { chooseFromDevice, expect, openWorkspace, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')

async function directoryWorkspace(api: Api) {
  const ws = await api.freeCanvas(ALICE, `directory ${Date.now()}`)
  const photo = await api.uploadAsset(ALICE, ws.workspace_id, PNG)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'empty-dir', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'a', name: 'Empty directory', payload: { members: {} } },
    { op: 'entity.create', entity_id: 'photos', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'b', name: 'Photos', payload: { members: {
      'photo.png': { kind: 'file', object_id: photo.object_id },
      'nested': { kind: 'dir', entity_id: 'empty-dir' },
    } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return { ws, photo }
}

async function openDirectory(page: Page, token: string, workspaceId: string, entityId = 'photos') {
  await openWorkspace(page, token, workspaceId)
  if (await page.getByTestId('aiws-top-sources').isVisible()) await page.getByTestId('aiws-top-sources').click()
  if (!await page.getByTestId(`aiws-detail-${entityId}`).isVisible()) await page.getByTestId(`aiws-tree-${entityId}`).click()
  await expect(page.getByTestId('aiws-directory')).toBeVisible()
}

function member(page: Page, name: string): Locator {
  return page.locator(`[data-testid="aiws-directory-member"][data-name="${name}"]`)
}

test('directory data shows members, uploads, renames and removes files persistently', async ({ page, api }) => {
  const { ws, photo } = await directoryWorkspace(api)
  await openDirectory(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-directory-member')).toHaveCount(2)
  await expect(member(page, 'photo.png')).toContainText('image/png')
  await expect(member(page, 'nested')).toHaveAttribute('data-kind', 'dir')

  await member(page, 'photo.png').getByTestId('aiws-directory-open').click()
  await expect(page.getByTestId('aiws-directory-preview')).toBeVisible()

  await member(page, 'photo.png').getByTestId('aiws-directory-rename').click()
  await page.getByTestId('aiws-directory-name').fill('renamed.png')
  await page.getByTestId('aiws-directory-save').click()
  await expect(member(page, 'renamed.png')).toBeVisible()
  const renamed = (await api.read(ALICE, ws.workspace_id, 'photos')).content.payload.members
  expect(renamed['photo.png']).toBeUndefined()
  expect(renamed['renamed.png'].object_id).toBe(photo.object_id)

  await chooseFromDevice(page, () => page.getByTestId('aiws-directory-upload').click(), { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('Directory upload content') })
  await expect(member(page, 'notes.txt')).toBeVisible()
  const uploaded = (await api.read(ALICE, ws.workspace_id, 'photos')).content.payload.members['notes.txt']
  const bytes = await fetch(`${api.base}/asset/${ws.workspace_id}/${uploaded.object_id}`, { headers: { authorization: `Bearer ${ALICE}` } })
  expect(bytes.ok).toBe(true)
  expect(await bytes.text()).toBe('Directory upload content')

  await openDirectory(page, ALICE, ws.workspace_id)
  await expect(member(page, 'renamed.png')).toBeVisible()
  await expect(member(page, 'notes.txt')).toBeVisible()
  await member(page, 'renamed.png').getByTestId('aiws-directory-remove').click()
  await expect(member(page, 'renamed.png')).toHaveCount(0)
  expect((await api.read(ALICE, ws.workspace_id, 'photos')).content.payload.members['renamed.png']).toBeUndefined()
  await openDirectory(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-directory-member')).toHaveCount(2)
  await expect(member(page, 'renamed.png')).toHaveCount(0)
})

test('directory references open their contents and a reader cannot change members', async ({ page, browser, api }) => {
  const { ws } = await directoryWorkspace(api)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  await openDirectory(page, ALICE, ws.workspace_id)
  await member(page, 'nested').getByTestId('aiws-directory-open').click()
  await expect(page.getByTestId('aiws-detail-empty-dir')).toBeVisible()
  await expect(page.getByTestId('aiws-directory-empty')).toBeVisible()
  await expect(page.getByTestId('aiws-directory-upload')).toBeVisible()

  await page.getByTestId('aiws-new-kind').selectOption('dir')
  await page.getByTestId('aiws-datatree-actions').getByRole('textbox', { name: '标题', exact: true }).fill('Created directory')
  await page.getByTestId('aiws-new-data').click()
  await expect(page.locator('.aiws-detail-head')).toContainText('Created directory')
  await expect(page.getByTestId('aiws-directory-empty')).toBeVisible()
  const created = (await api.outline(ALICE, ws.workspace_id)).find((entity: { name: string }) => entity.name === 'Created directory')
  expect(created?.type_id).toBe('buckyos.dir')
  await expect(page.getByTestId(`aiws-detail-${created.entity_id}`)).toBeVisible()

  const bobContext = await browser.newContext()
  const bob = await bobContext.newPage()
  await openDirectory(bob, BOB, ws.workspace_id)
  await expect(bob.getByTestId('aiws-directory-member')).toHaveCount(2)
  await expect(bob.getByTestId('aiws-directory-upload')).toHaveCount(0)
  await expect(bob.getByTestId('aiws-directory-rename')).toHaveCount(0)
  await expect(bob.getByTestId('aiws-directory-remove')).toHaveCount(0)
  await expect(bob.getByTestId('aiws-directory-add-dir')).toHaveCount(0)
  await member(bob, 'nested').getByTestId('aiws-directory-open').click()
  await expect(bob.getByTestId('aiws-directory-empty')).toBeVisible()
  await bobContext.close()
})

test('known data without a dedicated editor displays its payload instead of a blank panel', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `fallback data ${Date.now()}`)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'viewpoint', type_id: 'buckyos.viewport', parent_id: 'shows', order_key: 'v', payload: { title: 'Overview', surface_ref: { entity_id: 'free' }, center: { x: 123, y: 456 }, zoom: 1.25 } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="shows"]').getByRole('button', { name: '展开', exact: true }).click()
  await page.getByTestId('aiws-tree-viewpoint').click()
  const detail = page.getByTestId('aiws-detail-viewpoint')
  await expect(detail.getByTestId('aiws-raw-detail')).toContainText('"zoom": 1.25')
  await expect(detail.getByTestId('aiws-raw-detail')).toContainText('"x": 123')
})
