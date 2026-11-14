/* The home (UI framework PRD §9; acceptance §15 #28–#30) against the real backend: three categories, "Recent" first when
 * there is any and the category kept once picked, search that filters as it is typed, filter settings kept over a reload
 * (the person's preference across Workspaces, K4), the card fields of K6 (owner, last change, my role, public, live),
 * cards opened by a click, the card menu (details, delete with a confirmation), every public class I may open (K7) going
 * in or replaying by state and marking the ones I present, a package exported into the Zone by default and imported
 * from it, and the phone layout. The test accounts open workspaces all through the suite: "Recent" is what the service
 * recorded, so the specs decide what the home starts on from the service. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { devices, type Page } from '@playwright/test'
import { selectRow } from '../e2e/fixtures/selectDialog'
import { asUser, cardMenu, chooseFromZone, closeWorkspace, expect, openedPicker, test, workspaceOrigin, zoneGoto, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const PRESENTATION = resolve('../aiworkspace/fixtures/presentation/commits.json')
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const tagOf = (n: number) => `h${n}${Date.now().toString(36)}`
const card = (page: Page, id: string) => page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${id}"]`)
const listed = (list: Json, id: string): Json => (list.workspaces as Json[]).find((w) => w.workspace_id === id)
const tab = (page: Page, category: 'recent' | 'all' | 'sessions') => page.getByTestId(`aiws-home-tab-${category}`)
const sessionCard = (page: Page, id: string) => page.getByTestId(`aiws-my-session-${id}`)

/** The app in a Desktop window, on its home — without the specs' usual switch to "All workspaces" (fixtures
 * `backToList`): this spec looks at where the home starts. A workspace the start restored is closed first. */
async function openHome(page: Page, token: string) {
  await page.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), token)
  await page.goto('/?scenario=normal', { waitUntil: 'networkidle' })
  await startApp(page)
  await page.getByTestId('window-aiworkspace').getByRole('button', { name: /^(Maximize|最大化)$/ }).click()
}

async function startApp(page: Page) {
  await page.getByTestId('desktop-app-aiworkspace').click()
  await expect(page.getByTestId('aiws-list').or(page.getByTestId('aiws-workspace'))).toBeVisible({ timeout: 30_000 })
  if (await page.getByTestId('aiws-workspace').isVisible()) await closeWorkspace(page)
  await expect(page.getByTestId('aiws-list')).toHaveAttribute('data-state', 'ready', { timeout: 30_000 })
}

/** The third phase's presentation fixture (two Surfaces, the path `path-intro`) as a courseware of `token`. */
async function courseware(api: Api, token: string, title: string): Promise<{ workspace_id: string }> {
  const ws = await api.rpc(token, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(PRESENTATION, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [index, commit] of fixture.commits.entries()) expect((await api.commit(token, ws, commit.operations, `home-fx/${index + 1}`)).status).toBe('accepted')
  return ws
}

/** Whether the service has anything this account opened (the home then starts on "Recent", §9.1). */
async function hasRecent(api: Api, token: string): Promise<boolean> {
  const list = await api.rpc(token, 'ws.list', {})
  const open = await api.rpc(token, 'session.list_open', {})
  return (list.workspaces as Json[]).some((w) => w.opened_at) || (open.sessions as Json[]).some((s) => s.opened_at)
}

async function session(api: Api, token: string, workspaceId: string, title: string): Promise<string> {
  const created = await api.rpc(token, 'session.create', { workspace_id: workspaceId, path_id: 'path-intro', title })
  expect(created.ok, JSON.stringify(created)).toBe(true)
  return created.session.session_id as string
}

test('§15-28 three categories, Recent first once there is any, the category kept once picked; search filters at once; the filter settings are kept over a reload; the card fields; the ⋯ menu', async ({ page, api }) => {
  test.setTimeout(150_000)
  const tag = tagOf(28)
  const first = await api.rpc(ALICE, 'ws.create', { title: `${tag} 甲` })
  const second = await api.rpc(ALICE, 'ws.create', { title: `${tag} 乙` })
  const shared = await api.rpc(BOB, 'ws.create', { title: `${tag} 共享` })
  expect((await api.rpc(BOB, 'acl.set', { workspace_id: shared.workspace_id, changes: [{ subject: 'alice', allow: ['read'] }] })).ok).not.toBe(false)

  const recentBefore = await hasRecent(api, ALICE)
  await openHome(page, ALICE)
  // no category kept: "Recent" when the service has anything I opened, else "All workspaces"
  for (const c of ['recent', 'all', 'sessions'] as const) await expect(tab(page, c)).toBeVisible()
  await expect(tab(page, recentBefore ? 'recent' : 'all')).toHaveAttribute('aria-selected', 'true')
  await tab(page, 'all').click()
  await expect.poll(async () => (await api.rpc(ALICE, 'user.get_prefs', {})).prefs.home?.category).toBe('all')
  // the first card of "All workspaces" is "New workspace"
  await expect(page.getByTestId('aiws-home-grid-all').locator(':scope > *').first()).toHaveAttribute('data-testid', 'aiws-home-new-card')
  // the card front: title, the owner when it is not me, the last change, my role; no ID, no commit sequence, no row of buttons
  await expect(card(page, shared.workspace_id).getByTestId('aiws-home-role')).toHaveText('只读')
  await expect(card(page, shared.workspace_id).getByTestId('aiws-home-owner')).toHaveText('所有者 bob')
  await expect(card(page, first.workspace_id).getByTestId('aiws-home-role')).toHaveText('所有者')
  await expect(card(page, first.workspace_id).getByTestId('aiws-home-owner')).toHaveCount(0)
  await expect(card(page, first.workspace_id).getByTestId('aiws-home-modified')).toContainText('修改于')
  await expect(card(page, first.workspace_id)).not.toContainText(first.workspace_id)
  await expect(card(page, first.workspace_id).getByRole('button')).toHaveCount(2)
  // "last modified" first (§9.1): the newest of the three leads
  const order = await page.getByTestId('aiws-home-grid-all').getByTestId('aiws-workspace-card').evaluateAll((els) => els.map((el) => el.getAttribute('data-workspace-id')))
  expect(order.indexOf(shared.workspace_id)).toBeLessThan(order.indexOf(first.workspace_id))

  // search: as it is typed, in the current category
  const search = page.getByTestId('aiws-home-search')
  await search.fill(`${tag} 甲`)
  await expect(card(page, first.workspace_id)).toBeVisible()
  await expect(card(page, second.workspace_id)).toHaveCount(0)
  await expect(card(page, shared.workspace_id)).toHaveCount(0)
  await search.fill(tag)
  await expect(page.getByTestId('aiws-workspace-card')).toHaveCount(3)
  await search.fill(`${tag} 没有这个`)
  await expect(page.getByTestId('aiws-home-no-match')).toBeVisible()
  await page.getByTestId('aiws-home-clear').click()
  await expect(search).toHaveValue('')

  // filter settings: ownership (K6) — shared with me only, mine only; then my permission → read only
  await page.getByTestId('aiws-home-filter').click()
  const panel = page.getByTestId('aiws-home-filter-panel')
  await expect(panel.getByTestId('aiws-home-filter-sort')).toHaveValue('modified')
  await panel.getByTestId('aiws-home-filter-owner').selectOption('shared')
  await expect(card(page, shared.workspace_id)).toBeVisible()
  await expect(card(page, first.workspace_id)).toHaveCount(0)
  await panel.getByTestId('aiws-home-filter-owner').selectOption('mine')
  await expect(card(page, shared.workspace_id)).toHaveCount(0)
  await expect(card(page, first.workspace_id)).toBeVisible()
  await panel.getByTestId('aiws-home-filter-owner').selectOption('all')
  await panel.getByTestId('aiws-home-filter-permission').selectOption('read')
  await page.keyboard.press('Escape')
  await expect(panel).toHaveCount(0)
  await expect(page.getByTestId('aiws-home-filter')).toHaveAttribute('data-active', 'true')
  await expect(card(page, shared.workspace_id)).toBeVisible()
  await expect(card(page, first.workspace_id)).toHaveCount(0)

  // a click anywhere on a card opens it (here on its title); back on the home, the category kept: All workspaces
  await card(page, shared.workspace_id).locator('.aiws-home-card-title').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', shared.workspace_id, { timeout: 30_000 })
  await closeWorkspace(page)
  await expect(tab(page, 'all')).toHaveAttribute('aria-selected', 'true')
  // Recent: the service recorded the open (K6), newest first
  await tab(page, 'recent').click()
  await expect(page.getByTestId('aiws-home-grid-recent').locator(':scope > *').first()).toHaveAttribute('data-workspace-id', shared.workspace_id)
  await expect(card(page, shared.workspace_id)).toContainText('上次打开')
  expect(listed(await api.rpc(ALICE, 'ws.list', {}), shared.workspace_id)?.opened_at ?? null).not.toBeNull()

  // a reload: the start reopens that workspace (UI improvement §4, unchanged); the home starts on the category kept
  // (Recent) and the filter is still set — on the service, not in this browser
  await page.reload({ waitUntil: 'networkidle' })
  await startApp(page)
  await expect(tab(page, 'recent')).toHaveAttribute('aria-selected', 'true')
  expect((await api.rpc(ALICE, 'user.get_prefs', {})).prefs.home).toMatchObject({ category: 'recent', workspaces: { permission: 'read' } })
  await tab(page, 'all').click()
  await expect(page.getByTestId('aiws-home-filter')).toHaveAttribute('data-active', 'true')
  await expect(card(page, first.workspace_id)).toHaveCount(0)
  await page.getByTestId('aiws-home-filter').click()
  await expect(panel.getByTestId('aiws-home-filter-permission')).toHaveValue('read')
  await panel.getByTestId('aiws-home-filter-reset').click()
  await page.keyboard.press('Escape')
  await expect(card(page, first.workspace_id)).toBeVisible()

  // ⋯ → open in a new tab: the workspace's page on its own origin
  const [opened] = await Promise.all([page.context().waitForEvent('page'), cardMenu(page, first.workspace_id, 'aiws-home-open-tab')])
  await expect.poll(() => new URL(opened.url()).origin, { timeout: 30_000 }).toBe(workspaceOrigin(first.workspace_id))
  await opened.close()
  // ⋯ → details: the ID and the commit sequence the card front no longer shows
  const info = await api.rpc(ALICE, 'ws.get_info', { workspace_id: first.workspace_id })
  await cardMenu(page, first.workspace_id, 'aiws-home-details')
  const details = page.getByTestId('aiws-home-details-dialog')
  await expect(details.getByTestId('aiws-home-details-id')).toContainText(first.workspace_id)
  await expect(details.getByTestId('aiws-home-details-seq')).toContainText(String(info.head_seq))
  await details.getByRole('button', { name: '关闭', exact: true }).click()
  // ⋯ → delete asks first; cancelling keeps it
  await cardMenu(page, second.workspace_id, 'aiws-delete')
  const confirm = page.getByTestId('aiws-home-delete-dialog')
  await expect(confirm).toContainText(`${tag} 乙`)
  await confirm.getByRole('button', { name: '取消' }).click()
  await expect(card(page, second.workspace_id)).toBeVisible()
  await cardMenu(page, second.workspace_id, 'aiws-delete')
  await confirm.getByTestId('aiws-delete-confirm').click()
  await expect(card(page, second.workspace_id)).toHaveCount(0)
  await expect(page.getByTestId('aiws-list-message')).toContainText('已删除')
  expect((await api.rpc(ALICE, 'ws.list', {})).workspaces.some((w: { workspace_id: string }) => w.workspace_id === second.workspace_id)).toBe(false)
})

test('§15-29 a card opens its workspace; every public class I may open is listed (K7) — one I never joined too, as a reader of the canvas it presents (Q19), never one whose object I cannot read — going in or replaying by state, marking the ones I present; Recent mixes both', async ({ page, api, context }) => {
  test.setTimeout(180_000)
  const tag = tagOf(29)
  const mine = await courseware(api, ALICE, `${tag} 课件`)
  const live = await session(api, ALICE, mine.workspace_id, `${tag} 直播课`)
  expect((await api.rpc(ALICE, 'session.start', { session_id: live })).ok).toBe(true)
  const ended = await session(api, ALICE, mine.workspace_id, `${tag} 回放课`)
  expect((await api.rpc(ALICE, 'session.start', { session_id: ended })).ok).toBe(true)
  expect((await api.rpc(ALICE, 'session.end', { session_id: ended })).ok).toBe(true)
  // bob's courseware, of which alice reads only the canvas the path starts on (发布会, Q19): one class she attended through
  // its link, one she never joined (its permission object is that canvas, the default), one whose object is the other
  // canvas (总结), which she cannot read
  const theirs = await courseware(api, BOB, `${tag} 别人的课件`)
  expect((await api.rpc(BOB, 'acl.set', { workspace_id: theirs.workspace_id, changes: [{ subject: 'alice', resource: '/surfaces/发布会', allow: ['read'] }] })).ok).not.toBe(false)
  const attended = await session(api, BOB, theirs.workspace_id, `${tag} 听过的课`)
  const at = (await api.rpc(BOB, 'session.invite', { session_id: attended, kind: 'at' })).invite.token as string
  expect((await api.rpc(ALICE, 'session.join', { session_id: attended, link: at, name: 'alice' })).ok).toBe(true)
  const unseen = await session(api, BOB, theirs.workspace_id, `${tag} 没参加的课`)
  const elsewhere = (await api.rpc(BOB, 'session.create', { workspace_id: theirs.workspace_id, path_id: 'path-intro', title: `${tag} 别的画布的课`, readers_of: '/surfaces/总结' })).session.session_id as string

  await openHome(page, ALICE)
  await tab(page, 'all').click()
  // the courseware's card marks its live class (`live_sessions`, K6)
  await expect(card(page, mine.workspace_id).getByTestId('aiws-home-live-chip')).toBeVisible()
  await expect(card(page, theirs.workspace_id).getByTestId('aiws-home-live-chip')).toHaveCount(0)
  // a click on the card opens the workspace
  await card(page, mine.workspace_id).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', mine.workspace_id, { timeout: 30_000 })
  await closeWorkspace(page)

  // All public classes: live first; what a click does follows the state; "我主讲" on the ones I present
  await tab(page, 'sessions').click()
  const grid = page.getByTestId('aiws-home-grid-sessions')
  await expect(grid.locator('[data-session-id]').first()).toHaveAttribute('data-session-id', live)
  await expect(sessionCard(page, live)).toHaveAttribute('data-action', 'enter')
  await expect(sessionCard(page, live).getByTestId('aiws-home-presenting')).toHaveText('我主讲')
  await expect(sessionCard(page, ended)).toHaveAttribute('data-action', 'replay')
  await expect(sessionCard(page, ended).getByTestId('aiws-home-presenting')).toBeVisible()
  await expect(sessionCard(page, attended)).toHaveAttribute('data-role', 'audience')
  await expect(sessionCard(page, attended).getByTestId('aiws-home-presenting')).toHaveCount(0)
  // §15 #29: a public class I never joined but may open (I read the canvas it presents, its room is open) is listed too
  // (K7, Q19); one whose permission object is a canvas I cannot read is not
  await expect(sessionCard(page, unseen)).toBeVisible()
  await expect(sessionCard(page, unseen)).toHaveAttribute('data-role', 'member')
  await expect(sessionCard(page, unseen)).toHaveAttribute('data-action', 'enter')
  await expect(sessionCard(page, unseen).getByTestId('aiws-home-presenting')).toHaveCount(0)
  await expect(sessionCard(page, elsewhere)).toHaveCount(0)

  // the filters of public classes: status, only those I present
  await page.getByTestId('aiws-home-filter').click()
  const panel = page.getByTestId('aiws-home-filter-panel')
  await panel.getByTestId('aiws-home-filter-state').selectOption('replay')
  await expect(sessionCard(page, ended)).toBeVisible()
  await expect(sessionCard(page, live)).toHaveCount(0)
  await panel.getByTestId('aiws-home-filter-state').selectOption('all')
  await panel.getByTestId('aiws-home-filter-mine').check()
  await expect(sessionCard(page, attended)).toHaveCount(0)
  await expect(sessionCard(page, live)).toBeVisible()
  await panel.getByTestId('aiws-home-filter-reset').click()
  await page.keyboard.press('Escape')

  // a click on the class I may open as a reader of its canvas goes into its room (as audience), in a tab of its own on
  // its courseware's origin; so does one on the live class
  const [their] = await Promise.all([context.waitForEvent('page'), sessionCard(page, unseen).getByTestId('aiws-my-session-open').click()])
  await expect.poll(() => their.url()).toContain(`session=${unseen}`)
  expect(new URL(their.url()).host).toBe(new URL(workspaceOrigin(theirs.workspace_id)).host)
  await their.close()
  expect((await api.rpc(ALICE, 'session.get', { session_id: unseen })).role).toBe('audience')
  const [room] = await Promise.all([context.waitForEvent('page'), sessionCard(page, live).getByTestId('aiws-my-session-open').click()])
  await expect.poll(() => room.url()).toContain(`session=${live}`)
  expect(new URL(room.url()).host).toBe(new URL(workspaceOrigin(mine.workspace_id)).host)
  await room.close()

  // Recent mixes workspaces and public classes, newest first
  await tab(page, 'recent').click()
  const recent = page.getByTestId('aiws-home-grid-recent')
  await expect(recent.locator(':scope > *').first()).toHaveAttribute('data-session-id', live)
  await expect(recent.locator(`[data-workspace-id="${mine.workspace_id}"]`)).toBeVisible()
})

test('§9.3 export saves the package into the Zone by default (the system file picker), or downloads it; Import takes it from the Zone', async ({ page, api }) => {
  test.setTimeout(150_000)
  const tag = tagOf(93)
  const title = `${tag} 导出`
  const ws = await api.sample(ALICE, title)
  await openHome(page, ALICE)
  await tab(page, 'all').click()
  await cardMenu(page, ws.workspace_id, 'aiws-home-export')
  const dialog = page.getByTestId('aiws-export-dialog')
  // "save to the Zone" is the main action; downloading to this device is the other one
  await expect(dialog.getByTestId('aiws-export-zone')).toHaveClass(/is-primary/)
  await expect(dialog.getByTestId('aiws-export')).toBeVisible()
  await dialog.getByTestId('aiws-export-zone').click()
  const save = (await openedPicker(page)).dialog
  await zoneGoto(save, ['Documents'])
  const fileName = `${title}-share.zip`
  await expect(save.getByTestId('select-name')).toHaveValue(fileName)
  await save.getByTestId('select-confirm').click()
  await expect(dialog.getByTestId('aiws-export-message')).toContainText(`/home/Documents/${fileName}`, { timeout: 30_000 })
  await expect(dialog.getByTestId('aiws-export-manifest')).toContainText('分享')
  await dialog.getByRole('button', { name: '关闭', exact: true }).click()

  // the home's Import: the system file picker, then what the package becomes
  await page.getByTestId('aiws-home-import').click()
  const pick = (await openedPicker(page)).dialog
  await zoneGoto(pick, ['Documents'])
  await selectRow(pick, fileName).click()
  await pick.getByTestId('select-confirm').click()
  const form = page.getByTestId('aiws-import-dialog').getByTestId('aiws-import')
  await expect(form.getByTestId('aiws-import-file-name')).toHaveText(fileName)
  await form.getByLabel('作为新的工作区').check()
  await form.getByTestId('aiws-import-submit').click()
  await expect(page.getByTestId('aiws-import-done')).toContainText('已导入为新的工作区')
  const imported = (await page.getByTestId('aiws-import-done').innerText()).match(/ws_[a-z0-9]+/)?.[0] as string
  expect((await api.rpc(ALICE, 'doc.checkpoint', { workspace_id: imported })).content_root).toBe((await api.rpc(ALICE, 'doc.checkpoint', { workspace_id: ws.workspace_id })).content_root)
  // the list has it, and "Open" goes there
  await page.getByTestId('aiws-import-open').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', imported, { timeout: 30_000 })
})

test.describe('the home on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('§15-30 one column, no horizontal scroll, five cards a screen; fixed categories, search behind an icon, filters from the bottom; floating New and Import (the Zone first)', async ({ page, api }) => {
    test.setTimeout(150_000)
    const tag = tagOf(30)
    const made: string[] = []
    for (let i = 1; i <= 9; i += 1) made.push((await api.rpc(ALICE, 'ws.create', { title: `${tag} 第${i}个` })).workspace_id)
    const pkg = await api.rpc(ALICE, 'doc.export', { workspace_id: made[0], mode: 'share', self_contained: true })
    const bytes = Buffer.from(await api.download(pkg.download))

    await asUser(page, ALICE)
    await page.goto('/workspace')
    const list = page.getByTestId('aiws-list')
    await expect(list).toHaveAttribute('data-state', 'ready', { timeout: 30_000 })
    const viewport = page.viewportSize()!
    // search behind an icon: it takes the whole top bar
    await page.getByTestId('aiws-home-search-toggle').tap()
    const search = page.getByTestId('aiws-home-search')
    await expect(search).toBeFocused()
    expect((await search.boundingBox())!.width).toBeGreaterThan(viewport.width * 0.7)
    await search.fill(tag)
    await page.getByTestId('aiws-home-tab-all').tap()
    await expect(page.getByTestId('aiws-workspace-card')).toHaveCount(9)

    // one column of cards as wide as the screen allows, nothing scrolls sideways, at least five on one screen
    const boxes = await page.getByTestId('aiws-workspace-card').evaluateAll((cards) => cards.map((c) => { const r = c.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, bottom: r.bottom } }))
    expect(new Set(boxes.map((b) => Math.round(b.x))).size).toBe(1)
    expect(boxes[0].w).toBeGreaterThan(viewport.width - 40)
    expect(boxes.filter((b) => b.y >= 0 && b.bottom <= viewport.height).length).toBeGreaterThanOrEqual(5)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    expect(await list.locator('.aiws-home-scroll').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true)
    // the categories stay at the top while the cards scroll
    const tabs = page.getByTestId('aiws-home-tabs')
    const before = (await tabs.boundingBox())!.y
    await list.locator('.aiws-home-scroll').evaluate((el) => { el.scrollTop = el.scrollHeight })
    await expect.poll(() => list.locator('.aiws-home-scroll').evaluate((el) => el.scrollTop)).toBeGreaterThan(0)
    expect((await tabs.boundingBox())!.y).toBe(before)
    await page.getByTestId('aiws-home-search-close').tap()
    await expect(search).toHaveCount(0)

    // the filter settings: a panel from the bottom
    await page.getByTestId('aiws-home-filter').tap()
    const sheet = page.getByTestId('aiws-home-filter-sheet')
    await expect(sheet).toBeVisible()
    expect(Math.abs((await sheet.boundingBox())!.y + (await sheet.boundingBox())!.height - viewport.height)).toBeLessThan(2)
    await sheet.getByTestId('aiws-home-filter-close').tap()
    await expect(sheet).toHaveCount(0)

    // floating New: the New dialog
    const fab = page.getByTestId('aiws-new')
    const fabBox = (await fab.boundingBox())!
    expect(fabBox.x + fabBox.width).toBeGreaterThan(viewport.width - 40)
    expect(fabBox.y + fabBox.height).toBeGreaterThan(viewport.height - 120)
    await fab.tap()
    await expect(page.getByTestId('aiws-new-dialog')).toBeVisible()
    await page.getByTestId('aiws-new-dialog').getByRole('button', { name: '关闭', exact: true }).tap()
    // floating Import: the system file picker opens on the Zone; the package chosen there is imported
    await chooseFromZone(page, async () => {
      await page.getByTestId('aiws-home-import').tap()
      const { dialog } = await openedPicker(page)
      await expect(dialog.getByTestId('select-locations')).toBeVisible()
    }, { name: 'phone.zip', mimeType: 'application/zip', buffer: bytes })
    const form = page.getByTestId('aiws-import-dialog').getByTestId('aiws-import')
    await expect(form.getByTestId('aiws-import-file-name')).toHaveText('phone.zip')
    await form.getByLabel('作为新的工作区').check()
    await form.getByTestId('aiws-import-submit').tap()
    await expect(page.getByTestId('aiws-import-done')).toContainText('已导入为新的工作区')
  })
})
