/* Toolbar customization and App mode (UI 框架 PRD §4.6; K4) against the real backend: "视图 ▸ 自定义工具栏…" edits the
 * shared layer of this canvas (`structure`) or this Workspace (`manage`) — a scope the person may not change is shown
 * disabled with the reason —, its result shows for everyone who opens it, a locked zone is beyond the person's own
 * switches, and the layer travels with a Clone. App mode hides and locks every zone: whoever may change the layout keeps a
 * collapsed ⋯ to show them for a while or customize; anyone else has no way in; the alerts stay. Acceptance §15 #9. */

import type { Browser, Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { expect, mainMenu, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const PRESENTATION = resolve('../aiworkspace/fixtures/presentation/commits.json')
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function as(browser: Browser, token: string, workspaceId: string): Promise<{ page: Page; close: () => Promise<void> }> {
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const page = await context.newPage()
  await openWorkspacePage(page, token, workspaceId)
  return { page, close: () => context.close() }
}

async function customize(page: Page) {
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-customize-toolbar')
  await expect(page.getByTestId('aiws-customize-dialog')).toBeVisible()
  await expect(page.getByTestId('aiws-customize-zones')).toBeVisible()
}

async function save(page: Page) {
  await page.getByTestId('aiws-customize-save').click()
  await expect(page.getByTestId('aiws-customize-dialog')).toHaveCount(0)
}

/** The presentation fixture (a guide: its offer is a notice above the status bar for whoever opens it first). */
async function courseware(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(PRESENTATION, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [i, c] of fixture.commits.entries()) expect((await api.commit(ALICE, ws, c.operations, `customize/${i}`)).status).toBe('accepted')
  return ws
}

test('§15-9 画布配置隐藏并锁定工具选择区：所有打开的人都看不到，个人设置改不回来；App 模式只给有 structure 的人留退出入口，告警仍然显示', async ({ page, api, browser }) => {
  test.slow()
  const ws = await courseware(api, `customize 9 ${Date.now()}`)
  // bob edits content but may not change the structure: no layout of his own to set here
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read', 'update', 'append', 'comment'] }] })
  await openWorkspacePage(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-object-toolbar')).toBeVisible()

  // this canvas: the tool selection zone hidden and locked
  await customize(page)
  await expect(page.getByTestId('aiws-customize-scope-surface')).toBeChecked()
  const tools = page.getByTestId('aiws-customize-zone-tools')
  await tools.getByTestId('aiws-customize-zone-tools-shown').uncheck()
  await tools.getByTestId('aiws-customize-zone-tools-locked').check()
  await save(page)
  await expect(page.getByTestId('aiws-object-toolbar')).toHaveCount(0)
  const surfaceId = await page.getByTestId('aiws-canvas').getAttribute('data-surface-id')
  expect((await api.read(ALICE, ws.workspace_id, surfaceId!)).content.payload.config.toolbar).toEqual({ zones: { tools: { hidden: true, locked: true } } })
  // the person's own switch cannot bring it back: locked, said so
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-zones')
  await expect(page.getByTestId('aiws-menu-pref-object-toolbar')).toBeDisabled()
  await expect(page.getByTestId('aiws-menu-pref-object-toolbar')).toContainText('已由工作区或画布锁定')
  await page.keyboard.press('Escape')

  // bob sees it the same way; his own switch is locked too; the customize entry says why he may not
  const bob = await as(browser, BOB, ws.workspace_id)
  await expect(bob.page.getByTestId('aiws-canvas')).toBeVisible()
  await expect(bob.page.getByTestId('aiws-object-toolbar')).toHaveCount(0)
  await mainMenu(bob.page, 'aiws-menu-view', 'aiws-menu-zones')
  await expect(bob.page.getByTestId('aiws-menu-pref-object-toolbar')).toBeDisabled()
  await bob.page.keyboard.press('Escape')
  await mainMenu(bob.page, 'aiws-menu-view')
  await expect(bob.page.getByTestId('aiws-menu-customize-toolbar')).toBeDisabled()
  await bob.page.keyboard.press('Escape')
  // the guide's offer is a notice above the status bar (an alert stands for any)
  const offer = bob.page.getByTestId('aiws-notice').filter({ hasText: '使用引导' })
  await expect(offer).toBeVisible()

  // App mode on this canvas: every zone hidden and locked, at once for bob
  await customize(page)
  await page.getByTestId('aiws-customize-app-mode').click()
  await expect(page.getByTestId('aiws-customize-zone-title-shown')).not.toBeChecked()
  await save(page)
  for (const p of [page, bob.page]) {
    await expect(p.getByTestId('aiws-main-toolbar')).toHaveCount(0)
    await expect(p.getByTestId('aiws-view-toolbar')).toHaveCount(0)
    await expect(p.getByTestId('aiws-object-toolbar')).toHaveCount(0)
  }
  // nothing to change it with for bob: no ⋯, no status bar while all is saved; the alert stays
  await expect(bob.page.getByTestId('aiws-app-mode-entry')).toHaveCount(0)
  await expect(bob.page.getByTestId('aiws-status')).toHaveCount(0)
  await expect(offer).toBeVisible()
  // alice may structure the canvas: the collapsed ⋯ shows the zones in her window only
  await page.getByTestId('aiws-app-mode-entry').click()
  await page.getByTestId('aiws-menu-app-mode-reveal').click()
  await expect(page.getByTestId('aiws-main-toolbar')).toBeVisible()
  await expect(page.getByTestId('aiws-object-toolbar')).toBeVisible()
  await expect(bob.page.getByTestId('aiws-main-toolbar')).toHaveCount(0)
  // …hides them again from the view menu
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-app-mode-back')
  await expect(page.getByTestId('aiws-main-toolbar')).toHaveCount(0)
  // …and from the ⋯ customizes: the defaults again, for everyone
  await page.getByTestId('aiws-app-mode-entry').click()
  await page.getByTestId('aiws-menu-app-mode-customize').click()
  await expect(page.getByTestId('aiws-customize-zones')).toBeVisible()
  await page.getByTestId('aiws-customize-reset').click()
  await save(page)
  for (const p of [page, bob.page]) {
    await expect(p.getByTestId('aiws-main-toolbar')).toBeVisible()
    await expect(p.getByTestId('aiws-object-toolbar')).toBeVisible()
  }
  expect((await api.read(ALICE, ws.workspace_id, surfaceId!)).content.payload.config?.toolbar ?? null).toBeNull()
  await bob.close()
})

test('§4.6 工作区层：只有管理者能改；隐藏、添加带名称和图标的动作、拖动排序对所有画布生效；画布在其上继承；随 Clone 带走', async ({ page, api, browser }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `customize ws ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read', 'update', 'append', 'structure', 'comment'] }] })
  await openWorkspacePage(page, ALICE, ws.workspace_id)
  const collab = page.getByTestId('aiws-view-toolbar')
  await expect(collab.getByTestId('aiws-annotate-start')).toBeVisible()

  await customize(page)
  await page.getByTestId('aiws-customize-scope-workspace').check()
  const zone = page.getByTestId('aiws-customize-zone-collab')
  const item = (action: string) => zone.locator(`[data-testid="aiws-customize-item"][data-key="${action}"]`)
  // hide 评论; add "适应全部" as 「看全部」 with a star; drag it to the front
  await item('collab.comment').getByTestId('aiws-customize-item-shown').uncheck()
  await zone.getByTestId('aiws-customize-add-action-pick').selectOption('view.fit-all')
  await zone.getByTestId('aiws-customize-add-action-title').fill('看全部')
  await zone.getByTestId('aiws-customize-add-action-icon').selectOption('star')
  await zone.getByTestId('aiws-customize-add-action-confirm').click()
  const added = zone.getByTestId('aiws-customize-item').last()
  await expect(added).toContainText('看全部')
  await added.dragTo(item('collab.play'))
  await expect(zone.getByTestId('aiws-customize-item').first()).toContainText('看全部')
  await save(page)

  // the zone: the new item first, with its own name and icon; no 评论
  const tool = collab.locator('[data-action="view.fit-all"]')
  await expect(tool).toHaveAttribute('aria-label', '看全部')
  await expect(tool).toHaveAttribute('data-icon', 'star')
  await expect(collab.locator('[data-action]').first()).toHaveAttribute('data-action', 'view.fit-all')
  await expect(collab.getByTestId('aiws-annotate-start')).toHaveCount(0)
  const settings = (await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })).settings
  expect(settings.toolbar.zones.collab.relation).toBe('override')
  expect(settings.toolbar.zones.collab.items[0]).toMatchObject({ action: 'view.fit-all', title: '看全部', icon: 'star' })

  // bob may structure the canvas but does not manage the Workspace: the canvas scope only, the Workspace's said why
  const bob = await as(browser, BOB, ws.workspace_id)
  await expect(bob.page.getByTestId('aiws-view-toolbar').locator('[data-action="view.fit-all"]')).toHaveAttribute('aria-label', '看全部')
  await customize(bob.page)
  await expect(bob.page.getByTestId('aiws-customize-scope-surface')).toBeChecked()
  await expect(bob.page.getByTestId('aiws-customize-scope-workspace-reason')).toContainText('只有管理这个工作区的人')
  // the canvas layer starts from the Workspace's: the added item is there, 评论 unticked
  const bobZone = bob.page.getByTestId('aiws-customize-zone-collab')
  await expect(bobZone.getByTestId('aiws-customize-item').first()).toContainText('看全部')
  await expect(bobZone.locator('[data-testid="aiws-customize-item"][data-key="collab.comment"]').getByTestId('aiws-customize-item-shown')).not.toBeChecked()
  // on his canvas he hides the play button: written on the canvas, over the Workspace's
  await bobZone.locator('[data-testid="aiws-customize-item"][data-key="collab.play"]').getByTestId('aiws-customize-item-shown').uncheck()
  await save(bob.page)
  await expect(collab.getByTestId('aiws-collab-play')).toHaveCount(0)
  await expect(collab.locator('[data-action="view.fit-all"]')).toBeVisible()
  await bob.close()

  // the Workspace's settings and the canvas's toolbar are content: a Clone carries them
  const cloned = await api.rpc(ALICE, 'ws.clone', { workspace_id: ws.workspace_id })
  expect((await api.rpc(ALICE, 'ws.get_info', { workspace_id: cloned.workspace_id })).settings).toEqual(settings)
})
