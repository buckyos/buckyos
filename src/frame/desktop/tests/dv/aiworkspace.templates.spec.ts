/* The built-in templates' wishes on a real Zone with the real models, from the browser, in Chinese (the commits the
 * template dialog replays, `api/demos.ts`): the quarterly analysis — analyze, code, run, the data changes, run again — and
 * the short film — character sheets (generated pictures) → storyboard (a shot list and a frame per shot) → a preview clip
 * (a generated video), each wish reading the previous one's result folder. Every stage is a click on the wish's card
 * (许愿格交互升级 §3.3): no apply, each stage writes when it ends. */

import type { Locator, Page, TestInfo } from '@playwright/test'
import type { FreshnessInfo, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
type Binding = { type: string; entity_id: string }
type WishRead = { content: { payload: Record<string, any>; key_revs: Record<string, number> } } // eslint-disable-line @typescript-eslint/no-explicit-any

test.use({ viewport: { width: 1600, height: 1000 }, locale: 'zh-CN' })

const STAGE_MS = 1_800_000

/** `AIWS_DV_KEEP=1`: the workspaces stay after the test (a readable title, left out of the cleanup) to be looked at. */
const KEEP = process.env.AIWS_DV_KEEP === '1'

/** A template's workspace as the dialog creates it, with its content in Chinese. */
async function template(actor: Actor, kind: 'quarterly' | 'film', info: TestInfo): Promise<Workspace> {
  const { quarterlyDemoCommits, filmDemoCommits } = await import('../../src/app/aiworkspace/api/demos')
  const { setAiwsLocale } = await import('../../src/app/aiworkspace/i18n')
  setAiwsLocale('zh-CN')
  const title = KEEP ? `${kind === 'film' ? 'AI 短片工作流' : '季度经营分析'}（DV ${new Date().toISOString().slice(0, 16).replace('T', ' ')} UTC）` : actor.client.prefix
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title })
  if (KEEP) {
    const url = `https://aiws-${created.workspace_id.replace(/^ws_/, '')}.${new URL(actor.page.url()).host}/w`
    console.log(`kept workspace ${created.workspace_id} "${title}": ${url}`)
    await info.attach('kept-workspace', { body: Buffer.from(JSON.stringify({ workspace_id: created.workspace_id, title, url })), contentType: 'application/json' })
  } else actor.client.owned.add(created.workspace_id)
  const ws: Workspace = { workspace_id: created.workspace_id, epoch: created.epoch }
  for (const c of kind === 'quarterly' ? quarterlyDemoCommits() : filmDemoCommits()) {
    const result = await actor.client.commit(ws, c.operations as Operation[])
    expect(result.status, JSON.stringify(result)).toBe('accepted')
  }
  await openWorkspace(actor, ws)
  return ws
}

async function wish(actor: Actor, ws: Workspace, id: string) {
  return (await actor.client.ok<WishRead>('doc.read', { workspace_id: ws.workspace_id, entity_id: id })).content
}

async function freshness(actor: Actor, ws: Workspace, id: string): Promise<FreshnessInfo> {
  return (await actor.client.ok<{ items: FreshnessInfo[] }>('doc.freshness', { workspace_id: ws.workspace_id, entity_ids: [id] })).items[0]
}

type WishStage = 'analyze' | 'code' | 'run'
type WishRecord = { id: string; stage: WishStage; ok: boolean; error?: unknown; warnings?: unknown[]; usage?: unknown }

async function history(actor: Actor, ws: Workspace, wishId: string): Promise<Record<WishStage, WishRecord[]>> {
  const h = (await wish(actor, ws, wishId)).payload.history ?? {}
  return { analyze: h.analyze ?? [], code: h.code ?? [], run: h.run ?? [] }
}

/** The card of the wish a canvas Block shows, large enough to work with: the canvas fitted, the Block selected by its
 * title (the card's own controls take a click themselves) and fitted in turn. */
async function showCard(page: Page, cellId: string, wishId: string): Promise<Locator> {
  const top = page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  const card = page.getByTestId(`aiws-wish-card-${wishId}`)
  await card.getByTestId('aiws-wish-title').click()
  await expect(page.getByTestId(`aiws-canvas-block-${cellId}`)).toHaveClass(/is-selected/)
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-selection').click()
  await expect(card.getByTestId('aiws-wish-tab-prompt')).toBeVisible()
  return card
}

/** What went wrong in a stage, for the failure message: the card's line of state, the stage's record, the latest run. */
async function why(actor: Actor, ws: Workspace, wishId: string, card: Locator, stage: WishStage) {
  const status = await card.getByTestId('aiws-wish-status').textContent({ timeout: 1000 }).catch(() => null)
  const record = (await history(actor, ws, wishId))[stage][0]
  const runs = await actor.client.ok<{ runs: Record<string, unknown>[] }>('proc.list', { workspace_id: ws.workspace_id, wish_id: wishId, limit: 1 })
  const run = runs.runs[0]
  return JSON.stringify({ status, record: record && { ok: record.ok, error: record.error, warnings: record.warnings }, run: run && { stage: run.stage, state: run.state, error: run.error } })
}

/** A tab's next step (or the run button): click it and wait until the stage wrote its record (许愿格交互升级 §3.3: no
 * apply — each stage writes when it ends); a failed record fails the test with why. */
async function step(actor: Actor, ws: Workspace, card: Locator, wishId: string, stage: WishStage, by: { tab: 'prompt' | 'analysis' | 'macro' | 'results' } | 'run-button') {
  const before = (await history(actor, ws, wishId))[stage][0]?.id
  if (by === 'run-button') {
    await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'ready', { timeout: 60_000 })
    await card.getByTestId('aiws-wish-run').click()
  } else {
    await card.getByTestId(`aiws-wish-tab-${by.tab}`).click()
    const next = card.getByTestId('aiws-wish-next')
    await expect(next).toHaveAttribute('data-action', stage === 'code' ? /^(code|repair)$/ : stage)
    await expect(next).toBeEnabled({ timeout: 60_000 })
    await next.click()
  }
  let record: WishRecord | undefined
  await expect.poll(async () => { record = (await history(actor, ws, wishId))[stage][0]; return Boolean(record && record.id !== before) }, { timeout: STAGE_MS, intervals: [5_000] }).toBe(true)
  await expect(card).toHaveAttribute('data-running', '', { timeout: 60_000 })
  if (!record!.ok) throw new Error(`the ${stage} stage failed: ${await why(actor, ws, wishId, card, stage)}`)
  return record!
}

/** Analyze, code (the macro is written and trial-run in this browser), run: three clicks on the card's next step, each
 * stage written as it ends, its tab shown with its record. */
async function threeSteps(actor: Actor, ws: Workspace, info: TestInfo, cellId: string, wishId: string) {
  const page = actor.page
  const card = await showCard(page, cellId, wishId)
  await step(actor, ws, card, wishId, 'analyze', { tab: 'prompt' })
  await expect(card).toHaveAttribute('data-tab', 'analysis')
  expect((await wish(actor, ws, wishId)).payload.analysis?.status).toBe('ready')
  await screenshot(page, info, `${wishId}-analyzed`)
  await step(actor, ws, card, wishId, 'code', { tab: 'analysis' })
  await expect(card).toHaveAttribute('data-tab', 'macro')
  await expect(card.getByTestId('aiws-wish-run')).toBeVisible()
  await screenshot(page, info, `${wishId}-coded`)
  await step(actor, ws, card, wishId, 'run', { tab: 'macro' })
  await expect(card).toHaveAttribute('data-tab', 'results')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(1)')
  const w = await wish(actor, ws, wishId)
  expect(w.payload.program?.api_version).toBe(3)
  return { card, wish: w, results: w.payload.last_run.result_bindings.results as Record<string, Binding> }
}

/** An asset result: its bytes are served by the Workspace with the media type it was generated as. */
async function media(actor: Actor, ws: Workspace, b: Binding) {
  const read = await actor.client.ok<{ content: { payload: { object_id: string; image?: { width: number; height: number } } } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: b.entity_id })
  const objectId = read.content.payload.object_id
  const served = await actor.page.evaluate(async ({ ws, id, token }) => {
    const r = await fetch(`/kapi/aiworkspace/asset/${ws}/${id}`, { headers: { Authorization: `Bearer ${token}` } })
    return { status: r.status, type: r.headers.get('content-type'), size: (await r.arrayBuffer()).byteLength }
  }, { ws: ws.workspace_id, id: objectId, token: await actor.client.session() })
  expect(served.status, `${b.entity_id} ${objectId}`).toBe(200)
  return { objectId, type: served.type ?? '', size: served.size, image: read.content.payload.image }
}

test('[template-quarterly] the quarterly analysis wish: analyze without questions, code, run — then run again after a correction', async ({ owner }, info) => {
  test.setTimeout(2_400_000)
  const ws = await template(owner, 'quarterly', info)
  const { card, results } = await threeSteps(owner, ws, info, 'blk-wish', 'wish-analysis')
  const all = Object.values(results)
  expect(all.some((r) => r.type === 'richtext'), JSON.stringify(results)).toBe(true)
  expect(all.some((r) => r.type === 'table' || r.type === 'record'), JSON.stringify(results)).toBe(true)
  const text = all.find((r) => r.type === 'richtext')!
  const before = (await owner.client.ok<{ text: string }>('doc.read', { workspace_id: ws.workspace_id, entity_id: text.entity_id, as: 'body' })).text
  await screenshot(owner.page, info, 'quarterly-coded')

  // a source correction: stale; running the stored macro (no model loop) brings every result up to date
  const cell = await owner.client.ok<{ content: { rev: number } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'sales', selector: { kind: 'table_cell', record_id: 's-1', field_id: 'revenue' } })
  const changed = await owner.client.commit(ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'revenue', value: 238000, expect: { rev: cell.content.rev } }] } as Operation])
  expect(changed.status, JSON.stringify(changed)).toBe('accepted')
  await expect.poll(async () => (await freshness(owner, ws, 'wish-analysis')).status, { timeout: 60_000 }).toBe('stale')
  await expect(card.getByTestId('aiws-wish-results-stale')).toBeVisible({ timeout: 60_000 })
  await step(owner, ws, card, 'wish-analysis', 'run', 'run-button')
  await expect.poll(async () => (await freshness(owner, ws, 'wish-analysis')).status, { timeout: 60_000 }).toBe('current')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(2)')
  const after = (await owner.client.ok<{ text: string }>('doc.read', { workspace_id: ws.workspace_id, entity_id: text.entity_id, as: 'body' })).text
  expect(after, 'the summary was written again from the corrected data').not.toBe(before)
  expect((await wish(owner, ws, 'wish-analysis')).payload.last_run.mode).toBe('run')
  await screenshot(owner.page, info, 'quarterly-ran')
})

test('[template-film] the short-film wishes: character sheets → storyboard → preview clip, with the Zone image and video models', async ({ owner }, info) => {
  test.setTimeout(9_000_000)
  const ws = await template(owner, 'film', info)

  const sheets = await threeSteps(owner, ws, info, 'blk-wish-1', 'wish-characters')
  const pictures = Object.values(sheets.results).filter((r) => r.type === 'image')
  expect(pictures.length, JSON.stringify(sheets.results)).toBeGreaterThanOrEqual(3)
  for (const p of pictures) {
    const m = await media(owner, ws, p)
    expect(m.type).toMatch(/^image\//)
    expect(m.size).toBeGreaterThan(10_000)
    expect(m.image?.width ?? 0, 'the picture has its size').toBeGreaterThan(0)
  }
  await screenshot(owner.page, info, 'film-sheets')

  const board = await threeSteps(owner, ws, info, 'blk-wish-2', 'wish-storyboard')
  const inputs = (board.wish.payload.inputs ?? []) as { entity_id: string }[]
  // wish ①'s result folder, or its pictures and notes one by one
  expect(inputs.some((i) => i.entity_id.startsWith('wish-characters-out')), `the storyboard reads the character sheets: ${JSON.stringify(inputs)}`).toBe(true)
  const frames = Object.values(board.results).filter((r) => r.type === 'image')
  expect(frames.length, JSON.stringify(board.results)).toBeGreaterThanOrEqual(6)
  expect(Object.values(board.results).some((r) => r.type === 'table'), 'the shot list').toBe(true)
  for (const f of frames) expect((await media(owner, ws, f)).type).toMatch(/^image\//)
  await screenshot(owner.page, info, 'film-storyboard')

  const cut = await threeSteps(owner, ws, info, 'blk-wish-3', 'wish-video')
  const clips = Object.values(cut.results).filter((r) => r.type === 'asset')
  expect(clips.length, JSON.stringify(cut.results)).toBeGreaterThanOrEqual(1)
  const clip = await media(owner, ws, clips[0])
  expect(clip.type).toBe('video/mp4')
  expect(clip.size).toBeGreaterThan(100_000)
  expect(Object.values(cut.results).some((r) => r.type === 'richtext'), 'the note with the shot count and duration').toBe(true)
  await screenshot(owner.page, info, 'film-cut')
  await info.attach('film-results', { body: Buffer.from(JSON.stringify({ sheets: sheets.results, storyboard: board.results, cut: cut.results, macros: [sheets, board, cut].map((s) => s.wish.payload.program?.manifest) }, null, 2)), contentType: 'application/json' })
})
