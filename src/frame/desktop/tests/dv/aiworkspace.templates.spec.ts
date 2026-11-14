/* The built-in templates' wishes on a real Zone with the real models, from the browser, in Chinese (the commits the
 * template dialog replays, `api/demos.ts`): the quarterly analysis — analyze, code, apply, the data changes, run — and the short film —
 * character sheets (generated pictures) → storyboard (a shot list and a frame per shot) → a preview clip (a generated
 * video), each wish reading the previous one's result folder. Every stage goes through the Wish panel. */

import type { Locator, Page, TestInfo } from '@playwright/test'
import type { FreshnessInfo, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
type Binding = { type: string; entity_id: string }
type WishRead = { content: { payload: Record<string, any>; key_revs: Record<string, number> } } // eslint-disable-line @typescript-eslint/no-explicit-any

test.use({ viewport: { width: 1600, height: 1000 }, locale: 'zh-CN' })

const STAGE_MS = 1_800_000

/** A template's workspace as the dialog creates it, with its content in Chinese. */
async function template(actor: Actor, kind: 'quarterly' | 'film'): Promise<Workspace> {
  const { quarterlyDemoCommits, filmDemoCommits } = await import('../../src/app/aiworkspace/api/demos')
  const { setAiwsLocale } = await import('../../src/app/aiworkspace/i18n')
  setAiwsLocale('zh-CN')
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(created.workspace_id)
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

/** The Wish panel of the wish a canvas Block shows. */
async function openWish(page: Page, cellId: string, wishId: string): Promise<Locator> {
  const top = page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  await page.getByTestId(`aiws-canvas-block-${cellId}`).click()
  await page.getByTestId('aiws-near-wish-open').click()
  const panel = page.getByTestId(`aiws-wish-${wishId}`)
  await expect(panel.getByTestId('aiws-wish-executor')).toHaveValue('xllm')
  return panel
}

/** What went wrong in a stage, for the failure message: the panel's error, the analysis' blockers, the latest run. */
async function why(actor: Actor, ws: Workspace, wishId: string, panel: Locator) {
  const error = await panel.getByTestId('aiws-wish-message').textContent({ timeout: 1000 }).catch(() => null)
  const w = await wish(actor, ws, wishId)
  const runs = await actor.client.ok<{ runs: Record<string, unknown>[] }>('proc.list', { workspace_id: ws.workspace_id, wish_id: wishId, limit: 1 })
  const run = runs.runs[0]
  return JSON.stringify({ error, blockers: w.payload.analysis?.blockers, warnings: w.payload.analysis?.warnings, run: run && { stage: run.stage, state: run.state, error: run.error } })
}

/** Click a stage's button and wait until the panel is idle again (the run ended, or failed). */
async function stage(panel: Locator, button: string) {
  await panel.getByTestId(button).click()
  await expect(panel).not.toHaveAttribute('data-busy', '', { timeout: 30_000 })
  await expect(panel).toHaveAttribute('data-busy', '', { timeout: STAGE_MS })
}

/** Apply the waiting candidate: every program check passed (a failed one keeps Apply disabled until a person accepts it). */
async function apply(panel: Locator) {
  const failed = panel.getByTestId('aiws-wish-failed-checks')
  expect(await failed.count() ? await panel.getByTestId('aiws-wish-candidate').textContent() : '', 'a program check failed').toBe('')
  await panel.getByTestId('aiws-wish-apply').click()
  await expect(panel.getByTestId('aiws-wish-message')).toBeVisible({ timeout: 60_000 })
}

/** Analyze (written back as the run ends), code (the macro is written and trial-run in this browser), apply. */
async function analyzeCodeApply(actor: Actor, ws: Workspace, info: TestInfo, cellId: string, wishId: string) {
  const page = actor.page
  const panel = await openWish(page, cellId, wishId)
  const main = panel.getByTestId('aiws-wish-main')
  await stage(panel, 'aiws-wish-analyze')
  await expect.poll(async () => await main.getAttribute('data-main'), { timeout: 30_000 }).not.toBe('in_progress')
  if (await main.getAttribute('data-main') !== 'needs_coding') throw new Error(`the analysis did not end ready: ${await why(actor, ws, wishId, panel)}`)
  await screenshot(page, info, `${wishId}-analyzed`)
  await stage(panel, 'aiws-wish-code')
  const candidate = panel.getByTestId('aiws-wish-candidate')
  if (!await candidate.isVisible() || await candidate.getAttribute('data-stage') !== 'code') throw new Error(`the coding left no candidate: ${await why(actor, ws, wishId, panel)}`)
  await expect(candidate).toHaveAttribute('data-ready', 'true', { timeout: 60_000 })
  await screenshot(page, info, `${wishId}-candidate`)
  await apply(panel)
  await expect(main).toHaveAttribute('data-main', 'current', { timeout: 60_000 })
  const w = await wish(actor, ws, wishId)
  expect(w.payload.program?.api_version).toBe(3)
  return { panel, wish: w, results: w.payload.last_run.result_bindings.results as Record<string, Binding> }
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

test('[template-quarterly] the quarterly analysis wish: analyze without questions, code, apply, then run after a correction', async ({ owner }, info) => {
  test.setTimeout(2_400_000)
  const ws = await template(owner, 'quarterly')
  const { panel, results } = await analyzeCodeApply(owner, ws, info, 'blk-wish', 'wish-analysis')
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
  await stage(panel, 'aiws-wish-run')
  const candidate = panel.getByTestId('aiws-wish-candidate')
  if (!await candidate.isVisible() || await candidate.getAttribute('data-stage') !== 'run') throw new Error(`the run left no candidate: ${await why(owner, ws, 'wish-analysis', panel)}`)
  await expect(candidate).toHaveAttribute('data-ready', 'true', { timeout: 60_000 })
  await apply(panel)
  await expect.poll(async () => (await freshness(owner, ws, 'wish-analysis')).status, { timeout: 60_000 }).toBe('current')
  const after = (await owner.client.ok<{ text: string }>('doc.read', { workspace_id: ws.workspace_id, entity_id: text.entity_id, as: 'body' })).text
  expect(after, 'the summary was written again from the corrected data').not.toBe(before)
  expect((await wish(owner, ws, 'wish-analysis')).payload.last_run.mode).toBe('run')
  await screenshot(owner.page, info, 'quarterly-ran')
})

test('[template-film] the short-film wishes: character sheets → storyboard → preview clip, with the Zone image and video models', async ({ owner }, info) => {
  test.setTimeout(9_000_000)
  const ws = await template(owner, 'film')

  const sheets = await analyzeCodeApply(owner, ws, info, 'blk-wish-1', 'wish-characters')
  const pictures = Object.values(sheets.results).filter((r) => r.type === 'image')
  expect(pictures.length, JSON.stringify(sheets.results)).toBeGreaterThanOrEqual(3)
  for (const p of pictures) {
    const m = await media(owner, ws, p)
    expect(m.type).toMatch(/^image\//)
    expect(m.size).toBeGreaterThan(10_000)
    expect(m.image?.width ?? 0, 'the picture has its size').toBeGreaterThan(0)
  }
  await screenshot(owner.page, info, 'film-sheets')

  const board = await analyzeCodeApply(owner, ws, info, 'blk-wish-2', 'wish-storyboard')
  const inputs = (board.wish.payload.inputs ?? []) as { entity_id: string }[]
  // wish ①'s result folder, or its pictures and notes one by one
  expect(inputs.some((i) => i.entity_id.startsWith('wish-characters-out')), `the storyboard reads the character sheets: ${JSON.stringify(inputs)}`).toBe(true)
  const frames = Object.values(board.results).filter((r) => r.type === 'image')
  expect(frames.length, JSON.stringify(board.results)).toBeGreaterThanOrEqual(6)
  expect(Object.values(board.results).some((r) => r.type === 'table'), 'the shot list').toBe(true)
  for (const f of frames) expect((await media(owner, ws, f)).type).toMatch(/^image\//)
  await screenshot(owner.page, info, 'film-storyboard')

  const cut = await analyzeCodeApply(owner, ws, info, 'blk-wish-3', 'wish-video')
  const clips = Object.values(cut.results).filter((r) => r.type === 'asset')
  expect(clips.length, JSON.stringify(cut.results)).toBeGreaterThanOrEqual(1)
  const clip = await media(owner, ws, clips[0])
  expect(clip.type).toBe('video/mp4')
  expect(clip.size).toBeGreaterThan(100_000)
  expect(Object.values(cut.results).some((r) => r.type === 'richtext'), 'the note with the shot count and duration').toBe(true)
  await screenshot(owner.page, info, 'film-cut')
  await info.attach('film-results', { body: Buffer.from(JSON.stringify({ sheets: sheets.results, storyboard: board.results, cut: cut.results, macros: [sheets, board, cut].map((s) => s.wish.payload.program?.manifest) }, null, 2)), contentType: 'application/json' })
})
