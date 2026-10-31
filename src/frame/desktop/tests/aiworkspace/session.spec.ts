/* Presentation sessions (doc/workspace/BuckyOS AI Workspace 第四期规划.md §10, P4-02/P4-05/P4-06/P4-14/P4-16/P4-17)
 * against the real backend: a room on the workspace's own host, the presenter's commands confirmed and followed by
 * an audience member, a guest who only watches, a public display; a choice question asked, answered and published; a
 * comment over the stage; the end, the save and the replay with a comment. The third phase's presentation fixture is
 * the courseware, with a choice question on its first page. */

import type { Browser, Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { asUser, test, expect, workspaceOrigin, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const FIXTURE = resolve('../aiworkspace/fixtures/presentation/commits.json')
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function courseware(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [index, commit] of fixture.commits.entries()) {
    expect((await api.commit(ALICE, ws, commit.operations, `session-fx/${index + 1}`)).status).toBe('accepted')
  }
  // the classroom extension's choice question on the first page, bound to the session feeds it reads (Q20)
  const bindings: Record<string, Json> = {}
  for (const feed of ['tasks', 'my_responses', 'responses', 'results', 'participants']) {
    const r = await api.rpc(ALICE, 'session.feed_sample', { workspace_id: ws.workspace_id, feed })
    expect(r.ok).toBe(true)
    bindings[feed] = { entity_id: r.entity_id }
  }
  const info = await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })
  const committed = await api.commit(ALICE, { workspace_id: ws.workspace_id, epoch: info.epoch }, [{
    op: 'entity.create', entity_id: 'choice1', type_id: 'buckyos.cell', parent_id: 's-launch', order_key: 'zz1', placement: { x: 850, y: 200, w: 650, h: 520 },
    payload: { view: { type: 'classroom.choice', version: 1 }, config: { question: '哪一项是对的？', options: ['甲', '乙', '丙'], multiple: false, slots: 1 }, bindings },
  }], 'session-fx/choice')
  expect(committed.status).toBe('accepted')
  return { workspace_id: ws.workspace_id, epoch: info.epoch }
}

function roomUrl(workspaceId: string, sessionId: string, link?: string): string {
  return `${workspaceOrigin(workspaceId)}/w?session=${encodeURIComponent(sessionId)}${link ? `#k=${link}` : ''}`
}

async function open(browser: Browser, who: string | { guest: true } | null, url: string): Promise<Page> {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } })
  const page = await context.newPage()
  if (who) await asUser(page, who)
  else await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
  await page.goto(url)
  return page
}

async function join(page: Page, name: string) {
  await expect(page.getByTestId('aiws-session-gate')).toBeVisible({ timeout: 30_000 })
  await page.getByTestId('aiws-session-name').fill(name)
  await page.getByTestId('aiws-session-join').click()
}

test('S-01 a public session: presenter, audience, guest and display in one room; ask, answer, publish; replay (P4-02, P4-05, P4-14, P4-17)', async ({ api, browser }) => {
  test.setTimeout(240_000)
  const ws = await courseware(api, `session ${Date.now()}`)
  const created = await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro', title: '第一讲' })
  expect(created.ok).toBe(true)
  const sid = created.session.session_id as string
  const at = (await api.rpc(ALICE, 'session.invite', { session_id: sid, kind: 'at' })).invite.token as string
  const dt = (await api.rpc(ALICE, 'session.invite', { session_id: sid, kind: 'dt' })).invite.token as string

  // the presenter's room, before the start: the stage, the start button
  const presenter = await open(browser, ALICE, roomUrl(ws.workspace_id, sid))
  await expect(presenter.getByTestId('aiws-session-presenter')).toHaveAttribute('data-state', 'prepared', { timeout: 30_000 })
  // the audience joins through the link with its own identity; a guest only watches in this room
  const bob = await open(browser, BOB, roomUrl(ws.workspace_id, sid, at))
  await join(bob, '小明')
  await expect(bob.getByTestId('aiws-session-audience')).toBeVisible({ timeout: 30_000 })
  const guest = await open(browser, { guest: true }, roomUrl(ws.workspace_id, sid, at))
  await join(guest, '访客甲')
  await expect(guest.getByTestId('aiws-session-audience')).toBeVisible({ timeout: 30_000 })
  // a public display shows the room without an identity
  const wall = await open(browser, null, roomUrl(ws.workspace_id, sid, dt))
  await expect(wall.getByTestId('aiws-session-display')).toBeVisible({ timeout: 30_000 })

  // start, then step: the service confirms, everyone follows
  await presenter.getByTestId('aiws-session-start').click()
  await expect(presenter.getByTestId('aiws-session-presenter')).toHaveAttribute('data-state', 'live')
  await expect(bob.getByTestId('aiws-session-audience')).toHaveAttribute('data-state', 'live')
  await expect(guest.getByTestId('aiws-session-watch-only')).toBeVisible()
  // the private notes never reach the audience's page
  expect(await bob.content()).not.toContain('开场：介绍我们是谁')

  // ask everyone the question on the first page; bob answers; the guest cannot
  await presenter.getByTestId('aiws-choice-ask').click()
  await expect(bob.getByTestId('aiws-choice-option-1')).toBeEnabled({ timeout: 15_000 })
  await expect(guest.getByTestId('aiws-choice-option-1')).toBeDisabled()
  await bob.getByTestId('aiws-choice-option-1').click()
  await bob.getByTestId('aiws-choice-submit').click()
  await expect(bob.getByTestId('aiws-choice-submitted')).toContainText('B', { timeout: 15_000 })
  await expect(presenter.getByTestId('aiws-choice-count-1')).toHaveText('1', { timeout: 15_000 })
  // nobody else learns bob's answer
  expect(await guest.content()).not.toContain('"choice"')
  // close and publish: the result reaches the audience
  await presenter.getByTestId('aiws-choice-close').click()
  presenter.once('dialog', (d) => { void d.accept('B') })
  await presenter.getByTestId('aiws-choice-publish').click()
  await expect(bob.getByTestId('aiws-choice-result')).toContainText('B', { timeout: 15_000 })

  // a comment flies over everyone's stage
  await bob.getByTestId('aiws-session-danmaku-input').fill('讲得好')
  await bob.getByTestId('aiws-session-danmaku-send').click()
  await expect(presenter.getByTestId('aiws-session-danmaku').filter({ hasText: '讲得好' })).toBeVisible({ timeout: 15_000 })

  // the next step, followed by the audience and the display
  await presenter.getByTestId('aiws-session-next').click()
  await expect(presenter.getByTestId('aiws-session-stage')).toHaveAttribute('data-step', 's2', { timeout: 15_000 })
  await expect(bob.getByTestId('aiws-session-stage')).toHaveAttribute('data-step', 's2', { timeout: 15_000 })
  await expect(wall.getByTestId('aiws-session-stage')).toHaveAttribute('data-step', 's2', { timeout: 15_000 })

  // the end: the room becomes its replay; the owner saves it
  presenter.once('dialog', (d) => { void d.accept() })
  await presenter.getByTestId('aiws-session-end').click()
  await expect(presenter.getByTestId('aiws-session-replay')).toBeVisible({ timeout: 30_000 })
  await expect(bob.getByTestId('aiws-session-replay')).toBeVisible({ timeout: 30_000 })
  await presenter.getByTestId('aiws-session-save').click()
  await expect(presenter.getByTestId('aiws-session-replay-state')).toHaveText(/已保存/, { timeout: 15_000 })
  // the replay plays back from the start: the first step, then the second
  await expect(bob.getByTestId('aiws-session-timeline')).toBeVisible({ timeout: 30_000 })
  await bob.getByTestId('aiws-session-replay-scrub').fill('0')
  await expect(bob.getByTestId('aiws-session-stage')).toHaveAttribute('data-step', 's1', { timeout: 15_000 })
  const max = Number(await bob.getByTestId('aiws-session-replay-scrub').getAttribute('max'))
  await bob.getByTestId('aiws-session-replay-scrub').fill(String(max))
  await expect(bob.getByTestId('aiws-session-stage')).toHaveAttribute('data-step', 's2', { timeout: 15_000 })
  // after the class: a comment of the session
  await bob.getByTestId('aiws-session-post-input').fill('这节课很清楚')
  await bob.getByTestId('aiws-session-post-send').click()
  await expect(bob.getByTestId('aiws-session-post-comment').filter({ hasText: '这节课很清楚' })).toBeVisible({ timeout: 15_000 })
  // the sessions of the courseware (历次演示)
  const list = await api.rpc(ALICE, 'session.list', { workspace_id: ws.workspace_id })
  expect(list.sessions[0].state).toBe('archived')
  for (const p of [presenter, bob, guest, wall]) await p.context().close()
})

test('S-02 attendance against a roster and a random pick that never repeats (P4-14)', async ({ api, browser }) => {
  test.setTimeout(240_000)
  const ws = await courseware(api, `roster ${Date.now()}`)
  // the courseware's roster (Q17: principals only) and the online audience control against it
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'roster1', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'r', name: '名册', payload: { fields: [{ field_id: 'principal', name: 'principal', type: 'text' }, { field_id: 'name', name: '姓名', type: 'text' }] } },
    { op: 'table.insert_records', source_id: 'roster1', records: [
      { record_id: 'r1', values: { principal: 'bob', name: '小明' } },
      { record_id: 'r2', values: { principal: 'did:bns:carol', name: '小红' } },
      { record_id: 'r3', values: { principal: 'zed', name: '小张' } },
    ] },
  ], 'roster/table')).status).toBe('accepted')
  const participants = (await api.rpc(ALICE, 'session.feed_sample', { workspace_id: ws.workspace_id, feed: 'participants' })).entity_id
  const choice = await api.read(ALICE, ws.workspace_id, 'choice1')
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.set_keys', entity_id: 'choice1', keys: [{ key: 'config', value: { ...choice.content.payload.config, roster: 'roster1' }, expect: { rev: choice.content.key_revs.config } }] },
    { op: 'entity.create', entity_id: 'attendance', type_id: 'buckyos.cell', parent_id: 's-launch', order_key: 'zz2', placement: { x: 100, y: 200, w: 650, h: 520 },
      payload: { view: { type: 'classroom.audience', version: 1 }, config: { view: 'roster', public: false, roster: 'roster1' }, bindings: { participants: { entity_id: participants } } } },
  ], 'roster/blocks')).status).toBe('accepted')
  const created = await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro' })
  const sid = created.session.session_id as string
  const at = (await api.rpc(ALICE, 'session.invite', { session_id: sid, kind: 'at' })).invite.token as string
  const presenter = await open(browser, ALICE, roomUrl(ws.workspace_id, sid))
  await expect(presenter.getByTestId('aiws-session-start')).toBeVisible({ timeout: 30_000 })
  await presenter.getByTestId('aiws-session-start').click()
  // bob (an account), carol (an External DID) and a Limited account who is not on the roster
  const pages: Record<string, Page> = {}
  for (const [who, name] of [['tok-bob', '小明'], ['tok-ext', '小红'], ['tok-lim', '客人']] as const) {
    pages[who] = await open(browser, who, roomUrl(ws.workspace_id, sid, at))
    await join(pages[who], name)
    await expect(pages[who].getByTestId('aiws-session-audience')).toBeVisible({ timeout: 30_000 })
  }
  // present: bob and carol; absent: zed; online but not listed: the Limited account
  await expect(presenter.getByTestId('aiws-audience-present')).toHaveAttribute('data-count', '2', { timeout: 15_000 })
  await expect(presenter.getByTestId('aiws-audience-absent')).toHaveAttribute('data-count', '1')
  await expect(presenter.getByTestId('aiws-audience-unlisted')).toHaveAttribute('data-count', '1')
  // the audience never reads who is here (the feed is the presenter's)
  await expect(pages['tok-bob'].getByTestId('aiws-audience-attendance')).toContainText('只有老师能看到谁在场')
  // a pick: one of the two listed and online is called; the other is not
  await presenter.getByTestId('aiws-choice-pick').click()
  const called = async () => {
    for (let i = 0; i < 60; i++) {
      for (const who of ['tok-bob', 'tok-ext']) if (await pages[who].getByTestId('aiws-choice-called').count()) return who
      await presenter.waitForTimeout(250)
    }
    throw new Error('nobody was called')
  }
  const first = await called()
  const other = first === 'tok-bob' ? 'tok-ext' : 'tok-bob'
  await expect(pages[other].getByTestId('aiws-choice-called')).toHaveCount(0)
  await expect(pages['tok-lim'].getByTestId('aiws-choice-called')).toHaveCount(0)
  // the first one goes away; the presenter takes the turn back, closes and picks again: the other one, never the same
  await pages[first].context().close()
  await presenter.getByTestId('aiws-choice-revoke').click()
  await presenter.getByTestId('aiws-choice-close').click()
  await presenter.getByTestId('aiws-choice-pick').click()
  await expect(pages[other].getByTestId('aiws-choice-called')).toBeVisible({ timeout: 15_000 })
  await pages[other].getByTestId('aiws-choice-option-2').click()
  await pages[other].getByTestId('aiws-choice-submit').click()
  await expect(presenter.getByTestId('aiws-choice-count-2')).toHaveText('1', { timeout: 15_000 })
  // with nobody left in the pool, a third pick says so instead of repeating
  await presenter.getByTestId('aiws-choice-close').click()
  await presenter.getByTestId('aiws-choice-pick').click()
  await expect(presenter.getByTestId('aiws-choice-problem')).toBeVisible()
  // the picks are on the time line with their seeds: the replay reads them, it never draws again
  const events = await api.rpc(ALICE, 'session.events', { session_id: sid, after: 0, limit: 2000 })
  expect(events.ok).toBe(true)
  const run = created.session.workspace_id as string
  const tasks = (await api.rpc(ALICE, 'doc.query', { workspace_id: run, source_id: choice.content.payload.bindings.tasks.entity_id, limit: 100 })).rows as Json[]
  expect(tasks.filter((r) => r.values.mode === 'assigned')).toHaveLength(2)
  for (const p of [presenter, pages['tok-ext'], pages['tok-bob'], pages['tok-lim']]) await p.context().close().catch(() => undefined)
})

test('S-03 the presenter\'s voice through the host, recorded on the time line (P4-15 voice part)', async ({ api, playwright }) => {
  test.setTimeout(240_000)
  const media = await playwright.chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', '--autoplay-policy=no-user-gesture-required'] })
  const ws = await courseware(api, `voice ${Date.now()}`)
  const created = await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro' })
  const sid = created.session.session_id as string
  const at = (await api.rpc(ALICE, 'session.invite', { session_id: sid, kind: 'at' })).invite.token as string
  const presenter = await open(media, ALICE, roomUrl(ws.workspace_id, sid))
  await presenter.context().grantPermissions(['microphone'])
  await expect(presenter.getByTestId('aiws-session-start')).toBeVisible({ timeout: 30_000 })
  await expect(presenter.getByTestId('aiws-session-voice-unavailable')).toHaveCount(0)
  await presenter.getByTestId('aiws-session-start').click()
  await presenter.getByTestId('aiws-session-voice').click()
  await expect(presenter.getByTestId('aiws-session-voice')).toHaveAttribute('aria-pressed', 'true', { timeout: 15_000 })
  const bob = await open(media, BOB, roomUrl(ws.workspace_id, sid, at))
  await join(bob, '小明')
  await expect(bob.getByTestId('aiws-session-listen')).toBeVisible({ timeout: 20_000 })
  // a few seconds of voice, then the presenter stops: the segments name their place on the time line
  await presenter.waitForTimeout(3500)
  await presenter.getByTestId('aiws-session-voice').click()
  let segments: Json[] = []
  for (let i = 0; i < 40 && segments.length < 2; i++) {
    await presenter.waitForTimeout(250)
    const events = await api.rpc(ALICE, 'session.events', { session_id: sid, after: 0, limit: 2000 })
    segments = (events.events as Json[]).filter((e) => e.type === 'voice.segment')
  }
  expect(segments.length).toBeGreaterThanOrEqual(2)
  expect(segments.every((s) => s.data.media_type === 'audio/ogg' && s.data.t_rel_end > s.data.t_rel_start)).toBe(true)
  await media.close()
})
