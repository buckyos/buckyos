/* Who may come in to a public class without a link (UI 框架 PRD §16.3 Q19, §15 #29): the readers of the session's
 * permission object — by default the canvas of the presentation path's first step. The creation panel names it and
 * changes it with the permission editor's object picker; the presenter's links panel shows and changes it on the
 * service; someone who reads only that canvas comes in as audience without a link. On a phone the picker is a bottom
 * sheet. The third phase's presentation fixture is the courseware (path-intro starts on 发布会; 总结 is the other canvas). */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { devices, type Browser, type BrowserContextOptions, type Page } from '@playwright/test'
import { asUser, expect, openCanvas, test, workspaceOrigin, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const FIXTURE = resolve('../aiworkspace/fixtures/presentation/commits.json')
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
const { defaultBrowserType, ...pixel } = devices['Pixel 7']

/** The presentation fixture without its guide (播放 then opens "开始放映" at once). */
async function courseware(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [index, commit] of fixture.commits.entries()) {
    const operations = commit.operations.filter((op) => op.entity_id !== 'path-guide')
    if (operations.length === 0) continue
    expect((await api.commit(ALICE, ws, operations, `readers-fx/${index + 1}`)).status).toBe('accepted')
  }
  return ws
}

function roomUrl(workspaceId: string, sessionId: string): string {
  return `${workspaceOrigin(workspaceId)}/w?session=${encodeURIComponent(sessionId)}`
}

async function openRoom(browser: Browser, token: string, url: string, options: BrowserContextOptions = { viewport: { width: 1280, height: 800 } }): Promise<Page> {
  const context = await browser.newContext(options)
  const page = await context.newPage()
  await asUser(page, token)
  await page.goto(url)
  return page
}

/** Pick an object in the permission editor's picker (the tree opens on `surfaces`). */
async function pick(page: Page, entityId: string) {
  const picker = page.getByTestId('aiws-perm-picker')
  await expect(picker).toBeVisible()
  await picker.locator(`[data-testid="aiws-perm-picker-item"][data-entity-id="${entityId}"]`).click()
  await expect(picker).toHaveCount(0)
}

test('§15-29 Q19 the creation panel says who may come in without a link — readers of the first step\'s canvas — and changes it; the links panel changes it on the service; a reader of that canvas comes in as audience', async ({ page, api, browser }) => {
  test.setTimeout(180_000)
  const ws = await courseware(api, `readers ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-collab-play').click()
  await expect(page.getByTestId('aiws-start-show')).toBeVisible()
  await page.getByTestId('aiws-start-kind-session').click()
  const panel = page.getByTestId('aiws-session-create')
  const readers = panel.getByTestId('aiws-session-readers')
  // the default: the canvas the path starts on
  await expect(readers).toHaveAttribute('data-entity-id', 's-launch')
  await expect(readers.getByTestId('aiws-session-readers-name')).toHaveText('能读「发布会」的人')
  // another canvas, from the object picker
  await readers.getByTestId('aiws-session-readers-change').click()
  await pick(page, 's-work')
  await expect(readers).toHaveAttribute('data-entity-id', 's-work')
  await expect(readers.getByTestId('aiws-session-readers-name')).toHaveText('能读「总结」的人')
  await panel.getByTestId('aiws-session-create-title').fill('读者课')
  // the room opens in a tab of its own; it is looked at below in a window of the presenter's
  const [room] = await Promise.all([page.context().waitForEvent('page'), panel.getByTestId('aiws-session-create-go').click()])
  await expect.poll(() => room.url()).toContain('session=')
  await room.waitForLoadState('domcontentloaded')
  await room.close()
  const sessions = (await api.rpc(ALICE, 'session.list', { workspace_id: ws.workspace_id })).sessions as Json[]
  const session = sessions.find((s) => s.title === '读者课')
  expect(session?.policy?.readers_of).toBe('s-work')
  const sid = session.session_id as string

  // the presenter's links panel: the object as the service names it, changed back to the first canvas
  const presenter = await openRoom(browser, ALICE, roomUrl(ws.workspace_id, sid))
  await expect(presenter.getByTestId('aiws-session-presenter')).toHaveAttribute('data-state', 'prepared', { timeout: 30_000 })
  await presenter.getByTestId('aiws-session-open-links').click()
  const row = presenter.getByTestId('aiws-session-links-panel').getByTestId('aiws-session-readers')
  await expect(row.getByTestId('aiws-session-readers-name')).toHaveText('能读「总结」的人')
  await expect(row.getByTestId('aiws-session-readers-name')).toHaveAttribute('title', '/surfaces/总结')
  await row.getByTestId('aiws-session-readers-change').click()
  await pick(presenter, 's-launch')
  await expect(row).toHaveAttribute('data-entity-id', 's-launch')
  await expect(row.getByTestId('aiws-session-readers-name')).toHaveText('能读「发布会」的人')
  expect((await api.rpc(ALICE, 'session.get', { session_id: sid })).readers_of).toMatchObject({ entity_id: 's-launch', alive: true })
  // opened again, the panel says what the service has now
  const links = presenter.getByTestId('aiws-session-links-panel')
  await links.getByRole('button', { name: '关闭' }).click()
  await expect(links).toHaveCount(0)
  await presenter.getByTestId('aiws-session-open-links').click()
  await expect(row.getByTestId('aiws-session-readers-name')).toHaveText('能读「发布会」的人')

  // bob reads only that canvas: he opens the room without a link and joins as audience
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/surfaces/发布会', allow: ['read'] }] })).ok).toBe(true)
  const bob = await openRoom(browser, BOB, roomUrl(ws.workspace_id, sid))
  await expect(bob.getByTestId('aiws-session-gate')).toBeVisible({ timeout: 30_000 })
  await bob.getByTestId('aiws-session-name').fill('小明')
  await bob.getByTestId('aiws-session-join').click()
  await expect(bob.getByTestId('aiws-session-audience')).toBeVisible({ timeout: 30_000 })
  await expect(bob.getByTestId('aiws-session-readers')).toHaveCount(0)
  await presenter.context().close()
  await bob.context().close()
})

test('Q19 on a phone the presenter changes the permission object in a bottom sheet', async ({ api, browser }) => {
  test.setTimeout(120_000)
  const ws = await courseware(api, `readers phone ${Date.now()}`)
  const created = await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro', title: '手机上的课' })
  const sid = created.session.session_id as string
  const phone = await openRoom(browser, ALICE, roomUrl(ws.workspace_id, sid), pixel)
  await expect(phone.getByTestId('aiws-session-presenter')).toHaveAttribute('data-state', 'prepared', { timeout: 30_000 })
  await phone.getByTestId('aiws-session-open-links').click()
  const row = phone.getByTestId('aiws-session-links-panel').getByTestId('aiws-session-readers')
  await expect(row.getByTestId('aiws-session-readers-name')).toHaveText('能读「发布会」的人')
  await row.getByTestId('aiws-session-readers-change').tap()
  await expect(phone.locator('.aiws-perm-sheet[data-testid="aiws-perm-picker"]')).toBeVisible()
  await phone.getByTestId('aiws-perm-picker').locator('[data-testid="aiws-perm-picker-item"][data-entity-id="s-work"]').tap()
  await expect(row.getByTestId('aiws-session-readers-name')).toHaveText('能读「总结」的人')
  expect(await phone.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  expect((await api.rpc(ALICE, 'session.get', { session_id: sid })).readers_of.entity_id).toBe('s-work')
  await phone.context().close()
})
