/* The Workspace's cover (UI 框架 PRD §9.2, Q37) against the real backend: the user sets it — nothing is taken from the
 * canvases. From a home card's ⋯ "设置封面…": a picture chosen in the system file picker (the Zone, mocked), previewed,
 * saved — the card shows it — and removed again; what is no picture or too large is refused with a reason; a reader
 * who does not manage the workspace has no such entry. Inside the workspace "工作区 ▸ 设置封面…" (the action registry)
 * does the same as one undoable settings Commit, disabled with the reason for a non-manager, and on a phone (Pixel 7)
 * it is in the phone's main menu. */

import { devices, type Page } from '@playwright/test'
import { cardMenu, chooseFromZone, expect, mainMenu, openApp, openCanvas, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** A 16×9 PNG that decodes; bytes after its end make distinct objects of it. */
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAFElEQVR42mPQiDpBEmIY1TAoNAAAk1q5oVLxFXwAAAAASUVORK5CYII=', 'base64')
const png = (tag: string) => Buffer.concat([PNG, Buffer.from(tag)])
const tagOf = (n: number) => `cv${n}${Date.now().toString(36)}`
const card = (page: Page, id: string) => page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${id}"]`)

async function coverOf(api: Api, token: string, workspaceId: string): Promise<Json> {
  return ((await api.rpc(token, 'ws.list', {})).workspaces as Json[]).find((w) => w.workspace_id === workspaceId)?.cover
}

async function coverBytes(api: Api, token: string, workspaceId: string): Promise<Buffer | null> {
  const cover = await coverOf(api, token, workspaceId)
  if (!cover) return null
  const response = await fetch(`${api.base}/asset/${cover.workspace_id}/${cover.object_id}`, { headers: { authorization: `Bearer ${token}` } })
  return response.ok ? Buffer.from(await response.arrayBuffer()) : null
}

async function homeWith(page: Page, tag: string) {
  await openApp(page, ALICE)
  await page.getByTestId('aiws-home-search').fill(tag)
}

test('§9.2 Q37 设置封面… from a home card\'s ⋯: chosen from the Zone, previewed, saved — the card shows it —; no picture and too large are refused with a reason; removed again', async ({ page, api }) => {
  test.setTimeout(150_000)
  const tag = tagOf(1)
  const ws = await api.freeCanvas(ALICE, `${tag} 封面`)
  await homeWith(page, tag)
  await expect(card(page, ws.workspace_id).getByTestId('aiws-home-cover')).toHaveAttribute('data-cover', 'none')

  await cardMenu(page, ws.workspace_id, 'aiws-home-cover-set')
  const dialog = page.getByTestId('aiws-cover-dialog')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('aiws-cover-preview')).toHaveAttribute('data-state', 'none')
  await expect(dialog.getByTestId('aiws-cover-save')).toBeDisabled()
  await expect(dialog.getByTestId('aiws-cover-remove')).toHaveCount(0)

  // too large: told at once, nothing is uploaded
  await chooseFromZone(page, () => dialog.getByTestId('aiws-cover-pick').click(), { name: '大图.png', mimeType: 'image/png', buffer: Buffer.concat([PNG, Buffer.alloc(8 * 1024 * 1024)]) })
  await expect(dialog.getByTestId('aiws-cover-error')).toContainText('8 MiB')
  await expect(dialog.getByTestId('aiws-cover-save')).toBeDisabled()
  // named like a picture but not one: the service refuses it, the dialog says why and stays
  await chooseFromZone(page, () => dialog.getByTestId('aiws-cover-pick').click(), { name: '假图.png', mimeType: 'image/png', buffer: Buffer.from('not a picture') })
  await expect(dialog.getByTestId('aiws-cover-preview')).toHaveAttribute('data-state', 'picked')
  await dialog.getByTestId('aiws-cover-save').click()
  await expect(dialog.getByTestId('aiws-cover-error')).toContainText('PNG')
  expect(await coverOf(api, ALICE, ws.workspace_id)).toBeNull()

  // a picture from the Zone: previewed, then saved
  const picture = png(`${tag}-zone`)
  await chooseFromZone(page, () => dialog.getByTestId('aiws-cover-pick').click(), { name: '封面.png', mimeType: 'image/png', buffer: picture })
  await expect(dialog.getByTestId('aiws-cover-preview')).toHaveAttribute('data-state', 'picked')
  await expect.poll(() => dialog.getByTestId('aiws-cover-preview-picture').evaluate((el) => (el as HTMLImageElement).naturalWidth)).toBe(16)
  await expect(dialog.getByTestId('aiws-cover-caption')).toContainText('封面.png')
  await dialog.getByTestId('aiws-cover-save').click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('aiws-list-message')).toContainText('封面已设置')
  await expect.poll(() => coverBytes(api, ALICE, ws.workspace_id)).toEqual(picture)
  const cover = card(page, ws.workspace_id).getByTestId('aiws-home-cover')
  await expect(cover).toHaveAttribute('data-cover', 'shown')
  await expect.poll(() => cover.getByTestId('aiws-home-cover-picture').evaluate((el) => (el as HTMLImageElement).naturalWidth)).toBe(16)

  // again: the current cover, and removing it
  await cardMenu(page, ws.workspace_id, 'aiws-home-cover-set')
  await expect(dialog.getByTestId('aiws-cover-preview')).toHaveAttribute('data-state', 'current')
  await expect(dialog.getByTestId('aiws-cover-preview').getByTestId('aiws-home-cover-picture')).toBeVisible()
  await dialog.getByTestId('aiws-cover-remove').click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('aiws-list-message')).toContainText('封面已移除')
  await expect.poll(() => coverOf(api, ALICE, ws.workspace_id)).toBeNull()
  await expect(cover).toHaveAttribute('data-cover', 'none')
})

test('§9.2 Q37 a reader who does not manage the workspace has no 设置封面… on its card, and sees the cover its owner chose', async ({ page, api }) => {
  const tag = tagOf(2)
  const ws = await api.freeCanvas(BOB, `${tag} 别人的`)
  expect((await api.rpc(BOB, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'alice', resource: 'free', allow: ['read'] }] })).ok).not.toBe(false)
  const up = await api.uploadAsset(BOB, ws.workspace_id, png(`${tag}-bob`))
  expect((await api.commit(BOB, ws, [{ op: 'entity.set_props', entity_id: 'root', key: 'config', props: { cover: { object_id: up.object_id } } }])).status).toBe('accepted')
  await homeWith(page, tag)
  await expect(card(page, ws.workspace_id).getByTestId('aiws-home-cover')).toHaveAttribute('data-cover', 'shown')
  await card(page, ws.workspace_id).getByTestId('aiws-home-card-more').click()
  await expect(card(page, ws.workspace_id).getByTestId('aiws-home-open-tab')).toBeVisible()
  await expect(card(page, ws.workspace_id).getByTestId('aiws-home-cover-set')).toHaveCount(0)
})

test('§9.2 Q37 工作区 ▸ 设置封面… inside the workspace: one undoable settings Commit; disabled with the reason for a non-manager', async ({ page, api, browser }) => {
  test.setTimeout(120_000)
  const tag = tagOf(3)
  const ws = await api.freeCanvas(ALICE, `${tag} 菜单`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await mainMenu(page, 'aiws-menu-workspace', 'aiws-menu-cover')
  const dialog = page.getByTestId('aiws-cover-dialog')
  await expect(dialog.getByTestId('aiws-cover-preview')).toHaveAttribute('data-state', 'none')
  const picture = png(`${tag}-menu`)
  await chooseFromZone(page, () => dialog.getByTestId('aiws-cover-pick').click(), { name: '菜单封面.png', mimeType: 'image/png', buffer: picture })
  await dialog.getByTestId('aiws-cover-save').click()
  await expect(dialog).toHaveCount(0)
  await expect.poll(() => coverBytes(api, ALICE, ws.workspace_id)).toEqual(picture)
  // the dialog shows it now
  await mainMenu(page, 'aiws-menu-workspace', 'aiws-menu-cover')
  await expect(dialog.getByTestId('aiws-cover-preview')).toHaveAttribute('data-state', 'current')
  await expect(dialog.getByTestId('aiws-cover-preview').getByTestId('aiws-home-cover')).toHaveAttribute('data-cover', 'shown')
  await dialog.getByTestId('aiws-cover-close').click()
  // undone like any setting
  await mainMenu(page, 'aiws-menu-edit', 'aiws-menu-undo')
  await expect.poll(() => coverOf(api, ALICE, ws.workspace_id)).toBeNull()

  // bob edits the canvas but does not manage the workspace: the entry says why
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', allow: ['read', 'update', 'structure'] }] })).ok).not.toBe(false)
  const context = await browser.newContext()
  const bobPage = await context.newPage()
  await openWorkspacePage(bobPage, BOB, ws.workspace_id)
  await mainMenu(bobPage, 'aiws-menu-workspace')
  const entry = bobPage.getByTestId('aiws-menu-cover')
  await expect(entry).toBeDisabled()
  await expect(entry).toHaveAttribute('title', '只有管理这个工作区的人可以设置封面')
  await context.close()
})

test.describe('phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('§9.2 Q37 on a phone: 工作区 ▸ 设置封面… in the phone\'s main menu, the picture from the Zone, the dialog within the screen', async ({ page, api }) => {
    test.setTimeout(120_000)
    const tag = tagOf(4)
    const ws = await api.freeCanvas(ALICE, `${tag} 手机`)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    await page.getByTestId('aiws-main-menu').tap()
    const menu = page.getByTestId('aiws-main-menu-list')
    await menu.getByTestId('aiws-menu-workspace').tap()
    await menu.getByTestId('aiws-menu-cover').tap()
    const dialog = page.getByTestId('aiws-cover-dialog')
    await expect(dialog).toBeVisible()
    // within the screen: nothing scrolls sideways
    const width = page.viewportSize()!.width
    const box = (await dialog.boundingBox())!
    expect(box.x).toBeGreaterThanOrEqual(0)
    expect(box.x + box.width).toBeLessThanOrEqual(width + 1)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    const picture = png(`${tag}-phone`)
    await chooseFromZone(page, () => dialog.getByTestId('aiws-cover-pick').tap(), { name: '手机封面.png', mimeType: 'image/png', buffer: picture })
    await expect(dialog.getByTestId('aiws-cover-preview')).toHaveAttribute('data-state', 'picked')
    await dialog.getByTestId('aiws-cover-save').tap()
    await expect(dialog).toHaveCount(0)
    await expect.poll(() => coverBytes(api, ALICE, ws.workspace_id)).toEqual(picture)
  })
})
