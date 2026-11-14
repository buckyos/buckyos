/* Presence, the work log and the personal layout on screen (UI 框架 PRD §4.2, §4.5, §4.6, §12; K2, K3, K4) against the
 * real backend: two accounts in one Workspace each see themselves first and the other on their canvas, groups only
 * where the service shows them; the status bar counts who is online and its panel lists the work log, filtered, paged,
 * and locates an entry; the tool zones' switches and the status bar follow a person from one Workspace to another and
 * leave another account alone. Acceptance scenarios §15 #7, #8 (the log), #10 (the personal layout). */

import type { Browser, Page } from '@playwright/test'
import { core, expect, mainMenu, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** alice's Workspace with two canvases; bob is in the logical group 编辑组, which may edit. */
async function shared(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.freeCanvas(ALICE, title)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'second-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'b', payload: { kind: 'folder', system: 'surface_content', surface_id: 'second', title: 'Second' } },
    { op: 'entity.create', entity_id: 'second', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'b', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Second', content_folder_id: 'second-content' } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  expect((await api.rpc(ALICE, 'acl.set_group', { workspace_id: ws.workspace_id, group_id: 'editors', title: '编辑组' })).ok).toBe(true)
  expect((await api.rpc(ALICE, 'acl.members', { workspace_id: ws.workspace_id, group_id: 'editors', add: ['bob'] })).ok).toBe(true)
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: '@editors', allow: ['read', 'update', 'append', 'structure'] }] })).ok).toBe(true)
  return ws
}

async function as(browser: Browser, token: string, workspaceId: string, surfaceId?: string): Promise<{ page: Page; close: () => Promise<void> }> {
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const page = await context.newPage()
  await openWorkspacePage(page, token, workspaceId, surfaceId)
  return { page, close: () => context.close() }
}

const row = (page: Page, principal: string) => page.locator(`[data-testid="aiws-member-row"][data-principal="${principal}"]`)

test('§15-7 两个账号同时在线：各自的头像排在第一个；成员面板显示对方所在的画布，点击跳过去；管理者能看到组别；状态栏有在线人数', async ({ page, api, browser }) => {
  const ws = await shared(api, `activity 7 ${Date.now()}`)
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'free')
  const bob = await as(browser, BOB, ws.workspace_id, 'second')

  // the window registers as soon as the Workspace is open: the stacks show both, me first
  const stack = page.getByTestId('aiws-collab-avatars')
  await expect(stack).toHaveAttribute('data-count', '2', { timeout: 25_000 })
  await expect(stack.locator('button').first()).toHaveAttribute('data-testid', 'aiws-principal')
  await expect(stack.getByTestId('aiws-principal')).toHaveAttribute('data-principal', 'alice')
  await expect(stack.getByTestId('aiws-avatar-other')).toHaveAttribute('data-principal', 'bob')
  const bobStack = bob.page.getByTestId('aiws-collab-avatars')
  await expect(bobStack).toHaveAttribute('data-count', '2', { timeout: 25_000 })
  await expect(bobStack.getByTestId('aiws-principal')).toHaveAttribute('data-principal', 'bob')
  await expect(bobStack.getByTestId('aiws-avatar-other')).toHaveAttribute('data-principal', 'alice')
  // the status bar counts who is online
  await expect(page.getByTestId('aiws-status-online')).toHaveText('在线 2')

  // alice manages: bob's group and his canvas; her own row says who she is
  await stack.getByTestId('aiws-principal').click()
  const panel = page.getByTestId('aiws-identity-panel')
  await expect(panel.getByTestId('aiws-member-me').getByTestId('aiws-member-group')).toHaveText('所有者')
  await expect(row(page, 'bob').getByTestId('aiws-member-group')).toHaveText('编辑组')
  await expect(row(page, 'bob').getByTestId('aiws-member-surface')).toHaveText('Second')
  await expect(panel.getByTestId('aiws-members-permissions')).toBeVisible()
  await expect(panel.getByTestId('aiws-presence-unavailable')).toHaveCount(0)
  // a click on his canvas goes there
  await row(page, 'bob').getByTestId('aiws-member-surface').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'second')
  await expect(panel).toHaveCount(0)

  // bob does not manage: his own group, not alice's; no "成员与权限…"
  await bob.page.getByTestId('aiws-status-online').click()
  const bobPanel = bob.page.getByTestId('aiws-members-popover')
  await expect(bobPanel.getByTestId('aiws-member-me').getByTestId('aiws-member-group')).toHaveText('编辑组')
  await expect(row(bob.page, 'alice')).toBeVisible()
  await expect(row(bob.page, 'alice').getByTestId('aiws-member-group')).toHaveCount(0)
  // alice moved to Second: bob sees it at her next report
  await expect(row(bob.page, 'alice').getByTestId('aiws-member-surface')).toHaveText('Second', { timeout: 25_000 })
  await expect(bobPanel.getByTestId('aiws-members-permissions')).toHaveCount(0)
  await bob.page.keyboard.press('Escape')

  // bob leaves: alice is alone again, and the count says so
  await bob.close()
  await expect(stack).toHaveAttribute('data-count', '1', { timeout: 45_000 })
  await expect(page.getByTestId('aiws-status-online')).toHaveText('在线 1')
})

test('§15-8 工作日志：状态栏展开后是最近的日志（时间、操作者、做了什么），筛选“我的”，加载更早的；点一条定位到画布上的 Block 或数据模式里的对象', async ({ page, api }) => {
  test.slow()
  const ws = await shared(api, `activity 8 ${Date.now()}`)
  const say = async (token: string, message: string, operations: Json[]) => {
    const r = await api.rpc(token, 'doc.commit', { protocol_version: '1.0', workspace_id: ws.workspace_id, epoch: ws.epoch, idempotency_key: `say/${Math.random().toString(36).slice(2)}`, session_id: 'test-api', message, operations })
    expect(r.status, JSON.stringify(r)).toBe('accepted')
  }
  let key: string | undefined
  const folder = (id: string, title: string) => {
    key = core().order_key_between(key, undefined)
    return { op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: 'data', order_key: key, payload: { kind: 'folder', title } }
  }
  // enough history for more than one page (50)
  for (let i = 0; i < 52; i += 1) await say(ALICE, `整理第 ${i} 份`, [folder(`f-${i}`, `第 ${i} 份`)])
  await say(BOB, 'bob 的数据', [folder('bob-data', 'bob 的数据')])
  await say(ALICE, '放到第二张画布', [
    { op: 'entity.create', entity_id: 'note-2', type_id: 'buckyos.richtext', parent_id: 'second-content', order_key: 'a', name: '第二张的笔记', payload: {} },
    { op: 'entity.create', entity_id: 'blk-note-2', type_id: 'buckyos.cell', parent_id: 'second', order_key: 'a', placement: { x: 80, y: 80, w: 320, h: 200 }, payload: { view: { type: 'richtext' }, source_ref: { entity_id: 'note-2' } } },
  ])
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'free')

  await page.getByTestId('aiws-status').click()
  const log = page.getByTestId('aiws-status-log')
  await expect(log).toBeVisible()
  const entries = log.getByTestId('aiws-status-log-entry')
  // newest first: what alice said last, with who and when
  await expect(entries.first().getByTestId('aiws-status-log-text')).toHaveText('放到第二张画布', { timeout: 20_000 })
  await expect(entries.first()).toContainText('alice')
  await expect(entries.first().locator('.aiws-worklog-time')).toHaveText(/\d/)
  await expect(log.locator('[data-testid="aiws-status-log-text"]', { hasText: 'bob 的数据' })).toBeVisible()
  // 50 at a time, older ones on request
  await expect(entries).toHaveCount(50)
  await log.getByTestId('aiws-status-log-more').click()
  await expect.poll(() => entries.count()).toBeGreaterThan(50)
  await expect(log.locator('[data-testid="aiws-status-log-text"]', { hasText: '整理第 0 份' })).toBeVisible()

  // 我的: nothing of bob's; Agent 与宏: none here
  await log.getByTestId('aiws-status-log-filter-mine').click()
  await expect(log).toHaveAttribute('data-filter', 'mine')
  await expect(entries.first().getByTestId('aiws-status-log-text')).toHaveText('放到第二张画布')
  await expect(log.locator('[data-testid="aiws-status-log-text"]', { hasText: 'bob 的数据' })).toHaveCount(0)
  await log.getByTestId('aiws-status-log-filter-automation').click()
  await expect(log.getByTestId('aiws-status-log-empty')).toBeVisible()
  await log.getByTestId('aiws-status-log-filter-all').click()

  // a click locates: the Block on the second canvas…
  await entries.first().click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'second')
  // …and an object on no canvas in data mode (the panel stays open for the next one)
  await expect(log).toBeVisible()
  await log.locator('[data-testid="aiws-status-log-entry"]', { hasText: 'bob 的数据' }).click()
  await expect(page.getByTestId('aiws-sources')).toBeVisible()
})

test('§15-10 个人布局跨工作区：在一个工作区隐藏协作区、展开状态栏，换一个工作区仍然如此；另一个账号不受影响；恢复默认布局', async ({ page, api, browser }) => {
  const a = await api.freeCanvas(ALICE, `layout A ${Date.now()}`)
  const b = await api.freeCanvas(ALICE, `layout B ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: b.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read', 'update', 'append', 'structure'] }] })
  await openWorkspacePage(page, ALICE, a.workspace_id, 'free')
  await expect(page.getByTestId('aiws-view-toolbar')).toBeVisible()
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-zones', 'aiws-menu-pref-view-toolbar')
  await expect(page.getByTestId('aiws-view-toolbar')).toHaveCount(0)
  await page.getByTestId('aiws-status').click()
  await expect(page.getByTestId('aiws-status-detail')).toBeVisible()
  await expect.poll(async () => (await api.rpc(ALICE, 'user.get_prefs', {})).prefs).toMatchObject({ zones: { collab: { hidden: true } }, 'status.expanded': true })

  // another Workspace is another origin: the same layout
  await openWorkspacePage(page, ALICE, b.workspace_id, 'free')
  await expect(page.getByTestId('aiws-canvas')).toBeVisible()
  await expect(page.getByTestId('aiws-view-toolbar')).toHaveCount(0)
  await expect(page.getByTestId('aiws-status-detail')).toBeVisible()
  await expect(page.getByTestId('aiws-object-toolbar')).toBeVisible()

  // bob, in the same Workspace, keeps his own
  const bob = await as(browser, BOB, b.workspace_id, 'free')
  await expect(bob.page.getByTestId('aiws-view-toolbar')).toBeVisible()
  await expect(bob.page.getByTestId('aiws-status-detail')).toHaveCount(0)
  await bob.close()

  // "恢复默认布局" puts the person's layout back, everywhere
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-reset-layout')
  await expect(page.getByTestId('aiws-view-toolbar')).toBeVisible()
  await expect(page.getByTestId('aiws-status-detail')).toHaveCount(0)
  await expect.poll(async () => (await api.rpc(ALICE, 'user.get_prefs', {})).prefs.zones ?? null).toBeNull()
})
