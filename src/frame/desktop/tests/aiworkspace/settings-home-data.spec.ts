/* The UI framework's data layer against the real backend (doc/workspace/BuckyOS AI Workspace UI 框架PRD.md §4.6, §9,
 * §12; K4, K6, K7): the Workspace's shared settings (written by managers, carried by a Clone), personal preferences
 * that follow a person from one Workspace origin to another (§15 #10), the home page's fields of `ws.list` (the open
 * record comes from opening the Workspace page), and the public lecture catalogue listing a room the caller never
 * joined but may open (§15 #29). The interfaces are not on screen yet: the assertions go through the service and the
 * Desktop's own preference store, loaded on the Workspace pages. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import { expect, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const PRESENTATION = resolve('../aiworkspace/fixtures/presentation/commits.json')
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const TOOLBAR = { zones: { collab: { items: [{ action: 'show.start' }, { action: 'macro.run', args: { wish: '~w_today' }, title: '今天的日记' }] }, tools: { hidden: true, locked: true } } }

async function setSettings(api: Api, token: string, ws: { workspace_id: string; epoch: string }, props: Json): Promise<Json> {
  return api.commit(token, ws, [{ op: 'entity.set_props', entity_id: 'root', key: 'config', props }])
}

function card(list: Json, workspaceId: string): Json {
  return (list.workspaces as Json[]).find((w) => w.workspace_id === workspaceId)
}

/** The Desktop's preference store (state/prefs.ts) on a Workspace page, over that page's own transport: the dev server
 * serves the app's modules, so the page loads the very store the shell uses. */
async function prefsOnPage(page: Page, change?: Record<string, Json>): Promise<{ prefs: Record<string, Json>; state: string }> {
  await page.addScriptTag({ type: 'module', content: `
    import { resolveTransport } from '/src/app/aiworkspace/api/transport.ts'
    import { userPrefsStore } from '/src/app/aiworkspace/state/prefs.ts'
    window.__aiwsPrefs = async (change) => {
      const available = resolveTransport()
      if (!available.ok) throw new Error(available.reason)
      const store = userPrefsStore(available.transport)
      await store.reload()
      if (change) await store.set(change)
      const snap = store.snapshot()
      return { prefs: snap.prefs, state: snap.state }
    }` })
  await page.waitForFunction(() => typeof (window as Json).__aiwsPrefs === 'function')
  return page.evaluate((change) => (window as Json).__aiwsPrefs(change ?? null), change)
}

test('§4.6 K4 the Workspace settings: the owner writes them, a non-manager cannot, a Clone carries them', async ({ api }) => {
  const ws = await api.rpc(ALICE, 'ws.create', { title: '共享设置' }) as { workspace_id: string; epoch: string }
  expect((await setSettings(api, ALICE, ws, { toolbar: TOOLBAR })).status).toBe('accepted')
  // bob edits the whole Workspace but does not manage it
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read', 'update', 'structure', 'export'] }] })
  const info = await api.rpc(BOB, 'ws.get_info', { workspace_id: ws.workspace_id })
  expect(info.settings).toEqual({ toolbar: TOOLBAR })
  const refused = await setSettings(api, BOB, ws, { toolbar: null })
  expect(refused.status).toBe('rejected')
  expect(refused.code).toBe('PERMISSION_DENIED')
  // a canvas's toolbar is `structure` on it: bob has it, so his canvas layer goes through
  const canvas = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'c1-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'c1', title: 'C1' } },
    { op: 'entity.create', entity_id: 'c1', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'C1', content_folder_id: 'c1-content' } },
  ])
  expect(canvas.status).toBe('accepted')
  const surfaceToolbar = await api.commit(BOB, ws, [{ op: 'entity.set_props', entity_id: 'c1', key: 'config', props: { toolbar: { zones: { tools: { hidden: true } } } } }])
  expect(surfaceToolbar.status, JSON.stringify(surfaceToolbar)).toBe('accepted')
  // the settings are content: a Clone (by bob, who may export) starts with them, and with the canvas's toolbar
  const cloned = await api.rpc(BOB, 'ws.clone', { workspace_id: ws.workspace_id })
  expect(cloned.ok, JSON.stringify(cloned)).toBe(true)
  const copy = await api.rpc(BOB, 'ws.get_info', { workspace_id: cloned.workspace_id })
  expect(copy.settings).toEqual({ toolbar: TOOLBAR })
  expect((await api.read(BOB, cloned.workspace_id, 'c1')).content.payload.config.toolbar).toEqual({ zones: { tools: { hidden: true } } })
  // the clone is bob's: there he manages, and his change stays there
  const clone = { workspace_id: cloned.workspace_id as string, epoch: copy.epoch as string }
  expect((await setSettings(api, BOB, clone, { toolbar: null })).status).toBe('accepted')
  expect((await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })).settings).toEqual({ toolbar: TOOLBAR })
})

test('§15-10 K4 personal preferences follow a person from one Workspace origin to another; another account is not affected', async ({ api, browser }) => {
  const a = await api.rpc(ALICE, 'ws.create', { title: '偏好 A' })
  const b = await api.rpc(ALICE, 'ws.create', { title: '偏好 B' })
  await api.rpc(ALICE, 'acl.set', { workspace_id: b.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read'] }] })
  const alice = await browser.newContext()
  const pageA = await alice.newPage()
  await openWorkspacePage(pageA, ALICE, a.workspace_id)
  const written = await prefsOnPage(pageA, { inspector_dock: 'right' })
  expect(written).toEqual({ prefs: expect.objectContaining({ inspector_dock: 'right' }), state: 'synced' })
  // another Workspace is another origin: its browser storage is not A's, the service's copy is the same
  const pageB = await alice.newPage()
  await openWorkspacePage(pageB, ALICE, b.workspace_id)
  expect((await prefsOnPage(pageB)).prefs.inspector_dock).toBe('right')
  // bob, in the same Workspace B, has his own
  const bobContext = await browser.newContext()
  const bobPage = await bobContext.newPage()
  await openWorkspacePage(bobPage, BOB, b.workspace_id)
  expect((await prefsOnPage(bobPage)).prefs.inspector_dock).toBeUndefined()
  await prefsOnPage(bobPage, { inspector_dock: 'left' })
  expect((await api.rpc(ALICE, 'user.get_prefs', {})).prefs.inspector_dock).toBe('right')
  expect((await api.rpc(BOB, 'user.get_prefs', {})).prefs.inspector_dock).toBe('left')
  await alice.close()
  await bobContext.close()
})

test('§9.2 K6 ws.list carries the card fields; opening the Workspace page records the caller\'s own open', async ({ api, page }) => {
  const ws = await api.rpc(ALICE, 'ws.create', { title: '首页卡片' })
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read', 'comment'] }] })
  const before = card(await api.rpc(BOB, 'ws.list', {}), ws.workspace_id)
  expect(before).toMatchObject({ owner_principal: 'alice', opened_at: null, public: false, live_sessions: 0, cover: null, capabilities: ['read', 'comment'] })
  expect(typeof before.updated_at).toBe('string')
  await openWorkspacePage(page, BOB, ws.workspace_id)
  await expect.poll(async () => card(await api.rpc(BOB, 'ws.list', {}), ws.workspace_id).opened_at, { timeout: 10_000 }).not.toBeNull()
  expect(card(await api.rpc(ALICE, 'ws.list', {}), ws.workspace_id).opened_at).toBeNull()
  // published by path: public
  const made = await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'note', type_id: 'buckyos.richtext', parent_id: 'data', order_key: 'a', name: '公告', payload: {} }])
  expect(made.status).toBe('accepted')
  expect((await api.rpc(ALICE, 'ws.publish', { workspace_id: ws.workspace_id, entity_id: 'note' })).ok).toBe(true)
  const after = card(await api.rpc(ALICE, 'ws.list', {}), ws.workspace_id)
  expect(after).toMatchObject({ public: true, published: true })
  expect(after.updated_at > before.updated_at).toBe(true)
})

test('§15-29 K7 the public lecture catalogue lists a room bob never joined but may open; the presenter\'s is marked', async ({ api }) => {
  const ws = await api.rpc(ALICE, 'ws.create', { title: '课件目录' }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(PRESENTATION, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [i, c] of fixture.commits.entries()) expect((await api.commit(ALICE, ws, c.operations, `catalogue/${i}`)).status).toBe('accepted')
  const lecture = (await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro', title: '第一讲' })).session
  const bobPresents = (await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro', title: '第二讲', presenter: 'bob' })).session
  // the backend is shared by the specs: only this courseware's sessions count
  const ids = (list: Json) => (list.sessions as Json[]).filter((s) => s.source_workspace_id === ws.workspace_id).map((s) => s.session_id)
  // bob presents the second one; the first is not his to open yet
  const first = await api.rpc(BOB, 'session.list_open', {})
  expect(ids(first)).toEqual([bobPresents.session_id])
  expect((first.sessions as Json[]).find((s) => s.session_id === bobPresents.session_id)).toMatchObject({ my_role: 'presenter', presenting: true })
  // as a reader of the courseware he may open the first without a link: listed, never joined, not his own
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read'] }] })
  const open = await api.rpc(BOB, 'session.list_open', {})
  expect(new Set(ids(open))).toEqual(new Set([lecture.session_id, bobPresents.session_id]))
  const offered = (open.sessions as Json[]).find((s) => s.session_id === lecture.session_id)
  expect(offered).toMatchObject({ my_role: 'member', presenting: false, opened_at: null, cover: null, state: 'prepared', title: '第一讲' })
  expect(String(offered.url)).toContain(`?session=${lecture.session_id}`)
  expect(ids(await api.rpc(BOB, 'session.list_mine', {}))).not.toContain(lecture.session_id)
  // the service opens it for him without a link (the room page's first call), as audience
  const got = await api.rpc(BOB, 'session.get', { session_id: lecture.session_id })
  expect(got).toMatchObject({ ok: true, role: 'audience' })
  // a room closed to new audience is no longer offered to him
  const closed = await api.rpc(ALICE, 'session.set_access', { session_id: lecture.session_id, access: 'closed' })
  expect(closed.ok).toBe(true)
  expect(ids(await api.rpc(BOB, 'session.list_open', {}))).toEqual([bobPresents.session_id])
})
