/* UI 框架 PRD §4.2, §4.5, K2, K3 (§15 scenarios 6, 7, 8): presence and the work log through the Desktop client — the
 * services behind `usePresence` / `useWorkLog`, reached through the e2e hooks (`window.__aiwsTestHooks.activity`) —
 * against the real backend (what the shell shows of them: activity.spec.ts). The shell registers every open window
 * (FrameworkHost), so presence is live from the start. Two accounts have the same Workspace open:
 * each is first in its own list, sees where the other is, and only the manager sees groups; the log has the Commits
 * with their messages and a wish run's start and end, newest first, filtered and paged. */

import type { Page } from '@playwright/test'
import { expect, openCanvas, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

interface Member { principal: string; self: boolean; guest: boolean; name: string | null; surface_id: string | null; devices: number; owner?: boolean; groups?: { group_id: string; title: string }[] }
interface Presence { members: Member[]; live: boolean; online: number }
interface Entry { id: string; at: string; kind: string; phase?: string; actor: string; actor_kind: string; mine: boolean; message?: string | null; stage?: string; state?: string; run_id?: string | null; target: { entity_id: string; surface_id?: string | null } | null }
interface LogState { entries: Entry[]; loading: boolean; more: boolean; error: string | null }
interface Hooks {
  activity?: {
    presence: { retain(): void; release(): void; beat(): Promise<void>; snapshot(): Presence; surface(): string | null; select(surfaceId: string): void; labels(): { name: string; group: string | null }[] }
    workLog(filter: string, limit?: number): { refresh(): Promise<void>; loadMore(): Promise<void>; snapshot(): LogState; describe(): string[] }
  }
}
type Win = { __aiwsTestHooks?: Hooks }

const retain = (page: Page) => page.evaluate(() => (window as unknown as Win).__aiwsTestHooks!.activity!.presence.retain())
/** A heartbeat now, then what `usePresence` returns. */
const presence = (page: Page) => page.evaluate(async () => { const p = (window as unknown as Win).__aiwsTestHooks!.activity!.presence; await p.beat(); return p.snapshot() })
const labels = (page: Page) => page.evaluate(() => (window as unknown as Win).__aiwsTestHooks!.activity!.presence.labels())
const member = (p: Presence, principal: string) => p.members.find((m) => m.principal === principal)
/** What the canvas selector does: the user state's active Surface. */
const switchCanvas = (page: Page, surfaceId: string) => page.evaluate((id) => (window as unknown as Win).__aiwsTestHooks!.activity!.presence.select(id), surfaceId)

async function workLog(page: Page, filter: string, limit?: number): Promise<{ state: LogState; text: string[] }> {
  return page.evaluate(async ({ filter, limit }) => {
    const feed = (window as unknown as Win).__aiwsTestHooks!.activity!.workLog(filter, limit)
    await feed.refresh()
    return { state: feed.snapshot(), text: feed.describe() }
  }, { filter, limit })
}

const keys = (entries: Entry[]) => entries.map((e) => `${e.at}|${e.id}`)

/** alice's Workspace with two canvases; bob is in the logical group 编辑组, which may edit. */
async function shared(api: Api): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.freeCanvas(ALICE, `presence ${Date.now()}`)
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

test('§15-7 两个账号同时在线：各自排在第一个，看到对方所在的画布，只有管理者看到组别；换画布、离开随之更新', async ({ page, api, browser }) => {
  const ws = await shared(api)
  const bobContext = await browser.newContext()
  const bobPage = await bobContext.newPage()
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'free')
  await openWorkspacePage(bobPage, BOB, ws.workspace_id, 'second')
  // each window reports the canvas it shows (the link's canvas becomes the active one)
  await expect.poll(() => bobPage.evaluate(() => (window as unknown as Win).__aiwsTestHooks!.activity!.presence.surface())).toBe('second')
  await expect.poll(() => page.evaluate(() => (window as unknown as Win).__aiwsTestHooks!.activity!.presence.surface())).toBe('free')
  // the shell registers each window as soon as the Workspace is open; this window's person is always first
  await expect.poll(async () => (await presence(page)).live, { timeout: 20_000 }).toBe(true)
  expect((await presence(page)).members[0].principal).toBe('alice')
  await retain(page)
  await retain(bobPage)

  // alice (the owner, a manager): herself first, bob on his canvas with his group
  await expect.poll(async () => (await presence(page)).online, { timeout: 20_000 }).toBe(2)
  // (bob's window registered while it was still opening; the canvas of his link follows at once)
  await expect.poll(async () => member(await presence(page), 'bob')?.surface_id, { timeout: 15_000 }).toBe('second')
  const seen = await presence(page)
  expect(seen.live).toBe(true)
  expect(seen.members[0]).toMatchObject({ principal: 'alice', self: true, owner: true, surface_id: 'free' })
  expect(member(seen, 'bob')).toMatchObject({ self: false, owner: false, surface_id: 'second', devices: 1, groups: [{ group_id: 'editors', title: '编辑组' }] })
  expect(await labels(page)).toEqual([{ name: 'alice', group: '所有者' }, { name: 'bob', group: '编辑组' }])

  // bob: himself first with his own group; alice's group is not his to see
  await expect.poll(async () => (await presence(bobPage)).online, { timeout: 20_000 }).toBe(2)
  const bobs = await presence(bobPage)
  expect(bobs.members[0]).toMatchObject({ principal: 'bob', self: true, groups: [{ group_id: 'editors', title: '编辑组' }] })
  const alice = member(bobs, 'alice')!
  expect(alice.surface_id).toBe('free')
  expect('groups' in alice || 'owner' in alice).toBe(false)
  expect(await labels(bobPage)).toEqual([{ name: 'bob', group: '编辑组' }, { name: 'alice', group: null }])

  // bob switches to the other canvas (as the canvas selector does: the active Surface of his user state): reported at once
  await switchCanvas(bobPage, 'free')
  await expect.poll(async () => member(await presence(page), 'bob')?.surface_id, { timeout: 5_000 }).toBe('free')
  // a second window of his is still one person, shown on the canvas he moved to last
  const bobPage2 = await bobContext.newPage()
  await openWorkspacePage(bobPage2, BOB, ws.workspace_id)
  await retain(bobPage2)
  await expect.poll(async () => member(await presence(page), 'bob')?.devices, { timeout: 20_000 }).toBe(2)
  expect((await presence(page)).online).toBe(2)
  await switchCanvas(bobPage2, 'second')
  await expect.poll(async () => member(await presence(page), 'bob')?.surface_id, { timeout: 5_000 }).toBe('second')
  // one window goes away: it leaves at once, the other stays
  await bobPage2.close()
  await expect.poll(async () => member(await presence(page), 'bob')?.devices, { timeout: 40_000, intervals: [1_000] }).toBe(1)
  expect(member(await presence(page), 'bob')?.surface_id).toBe('free')

  // bob closes his window: he leaves (at once when the leave gets out, else when his heartbeats stop)
  await bobContext.close()
  await expect.poll(async () => (await presence(page)).members.map((m) => m.principal), { timeout: 45_000, intervals: [1_000] }).toEqual(['alice'])
})

test('§15-6/§15-8 工作日志：提交带说明、许愿格运行的开始和完成，倒序；筛选“我的”“Agent 与宏”；向更早翻页', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `worklog ${Date.now()}`)
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', allow: ['read', 'update', 'append', 'structure'] }] })).ok).toBe(true)
  const say = async (token: string, message: string) => {
    const r = await api.rpc(token, 'doc.commit', { protocol_version: '1.0', workspace_id: ws.workspace_id, epoch: ws.epoch, idempotency_key: `say/${Math.random().toString(36).slice(2)}`, session_id: 'test-api', message,
      operations: [{ op: 'entity.create', entity_id: `f-${Math.random().toString(36).slice(2, 10)}`, type_id: 'buckyos.container', parent_id: 'data', order_key: 'z', payload: { kind: 'folder', title: message } }] })
    expect(r.status, JSON.stringify(r)).toBe('accepted')
  }
  await say(ALICE, '整理季度数据')
  await say(BOB, 'bob 补充了说明')
  await openCanvas(page, ALICE, ws.workspace_id)

  // run the wish (the Mock executor): analysis, run, apply
  await page.getByTestId('aiws-canvas-block-blk-wish').dblclick()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-analysis')).toBeVisible({ timeout: 20_000 })
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-next', 'run')
  await page.getByTestId('aiws-wish-next').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-wish-candidate')).toHaveAttribute('data-ready', 'true')
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'current', { timeout: 30_000 })

  // 全部: newest first; the run's start and end; the macro's Commit marked as such; the messages as written
  await expect.poll(async () => (await workLog(page, 'all')).state.entries.some((e) => e.kind === 'run' && e.phase === 'end' && e.state === 'succeeded'), { timeout: 20_000 }).toBe(true)
  const { state: all, text } = await workLog(page, 'all')
  expect(all.error).toBeNull()
  const order = keys(all.entries)
  expect([...order].sort().reverse()).toEqual(order)
  const runs = all.entries.filter((e) => e.kind === 'run')
  expect(runs.map((e) => e.phase)).toEqual(['end', 'start'])
  expect(runs[0]).toMatchObject({ actor: 'alice', actor_kind: 'macro', stage: 'run', state: 'succeeded', mine: true })
  expect(runs[0].target?.entity_id).toBe('wish-analysis')
  expect(text[all.entries.indexOf(runs[0])]).toMatch(/完成$/)
  expect(text[all.entries.indexOf(runs[1])]).toMatch(/^开始/)
  const applied = all.entries.find((e) => e.kind === 'commit' && e.run_id === runs[0].run_id)
  expect(applied?.actor_kind).toBe('macro')
  expect(all.entries.indexOf(applied!)).toBeLessThan(all.entries.indexOf(runs[1]))
  expect(text[all.entries.indexOf(applied!)]).toBe('写入了许愿格的结果')
  const said = all.entries.filter((e) => e.kind === 'commit' && e.message && /整理季度数据|bob 补充了说明/.test(e.message))
  expect(said.map((e) => [e.actor, e.message])).toEqual([['bob', 'bob 补充了说明'], ['alice', '整理季度数据']])
  expect(text).toContain('bob 补充了说明')
  // the owner sees the permission change too
  expect(all.entries.some((e) => e.kind === 'acl')).toBe(true)

  // 我的: nothing of bob's
  const mine = (await workLog(page, 'mine')).state.entries
  expect(mine.length).toBeGreaterThan(0)
  expect(mine.every((e) => e.actor === 'alice' && e.mine)).toBe(true)
  expect(mine.some((e) => e.message === 'bob 补充了说明')).toBe(false)
  // Agent 与宏: the runs and the macro's Commit only
  const automation = (await workLog(page, 'automation')).state.entries
  expect(automation.map((e) => e.id)).toEqual(all.entries.filter((e) => e.kind === 'run' || e.actor_kind === 'macro' || e.actor_kind === 'agent').map((e) => e.id))

  // pages of three join up to the whole log
  const paged = await page.evaluate(async () => {
    const feed = (window as unknown as Win).__aiwsTestHooks!.activity!.workLog('all', 3)
    await feed.refresh()
    const first = feed.snapshot().entries.length
    await feed.loadMore()
    await feed.loadMore()
    return { first, more: feed.snapshot().more, ids: feed.snapshot().entries.map((e) => e.id) }
  })
  expect(paged.first).toBe(3)
  expect(paged.ids).toEqual(all.entries.slice(0, 9).map((e) => e.id))
  expect(paged.more).toBe(all.entries.length > 9)
})
