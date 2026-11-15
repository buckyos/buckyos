/* A wish's three stages on a real Zone with the real model, from the browser (notepads/aiws-ai-native-phase1-todo.md
 * T7-3; 许愿格交互升级 §3.3): analyze → code (the model writes the macro and trial-runs it; only the macro is stored) → run
 * (its results written as it ends) → the data changes → run again (the stored macro only, no model loop; the commentary
 * follows the data) → a renamed field changes nothing → a deleted declared field asks for coding again. Every stage is
 * a click on the wish's card; nothing waits to be applied. The CLI as an executor runs the macro restricted; with a
 * second real account, a run is written as its macro identity (权限需求 §4.5, Q31–Q32). */

import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Locator, Page } from '@playwright/test'
import type { FreshnessInfo, Operation, WishRunView, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { cli, expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>

test.use({ viewport: { width: 1600, height: 1000 } })

const MONTHS = ['2026-07-01', '2026-08-01', '2026-09-01']

async function commit(actor: Actor, ws: Workspace, operations: Operation[]) {
  const result = await actor.client.commit(ws, operations)
  expect(result.status, JSON.stringify(result)).toBe('accepted')
}

type WishRead = { content: { payload: Record<string, any>; key_revs: Record<string, number> } } // eslint-disable-line @typescript-eslint/no-explicit-any

async function wish(actor: Actor, ws: Workspace): Promise<WishRead['content']> {
  return (await actor.client.ok<WishRead>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'wish-sales' })).content
}

async function freshness(actor: Actor, ws: Workspace): Promise<FreshnessInfo> {
  return (await actor.client.ok<{ items: FreshnessInfo[] }>('doc.freshness', { workspace_id: ws.workspace_id, entity_ids: ['wish-sales'] })).items[0]
}

async function textOf(actor: Actor, ws: Workspace, id: string): Promise<string> {
  return (await actor.client.ok<{ text: string }>('doc.read', { workspace_id: ws.workspace_id, entity_id: id, as: 'body' })).text
}

async function setAmount(actor: Actor, ws: Workspace, record: string, value: string) {
  const cell = await actor.client.ok<{ content: { rev: number } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'orders', selector: { kind: 'table_cell', record_id: record, field_id: 'amount' } })
  await commit(actor, ws, [{ op: 'table.set_values', source_id: 'orders', values: [{ record_id: record, field_id: 'amount', value, expect: { rev: cell.content.rev } }] } as Operation])
}

type WishStage = 'analyze' | 'code' | 'run'

/** The wish's card, large enough to work with: the canvas fitted, the Block selected by its title, then fitted. */
async function showCard(page: Page): Promise<Locator> {
  const top = page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  const card = page.getByTestId('aiws-wish-card-wish-sales')
  await card.getByTestId('aiws-wish-title').click()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-selection').click()
  await expect(card.getByTestId('aiws-wish-tab-prompt')).toBeVisible()
  return card
}

/** A stage from the card — its tab's next step, or the run button — waited for until it wrote its record (no apply). */
async function step(actor: Actor, ws: Workspace, card: Locator, stage: WishStage, by: 'prompt' | 'analysis' | 'macro' | 'run-button') {
  const before = ((await wish(actor, ws)).payload.history?.[stage] ?? [])[0]?.id
  if (by === 'run-button') {
    await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'ready', { timeout: 60_000 })
    await card.getByTestId('aiws-wish-run').click()
  } else {
    await card.getByTestId(`aiws-wish-tab-${by}`).click()
    await expect(card.getByTestId('aiws-wish-next')).toHaveAttribute('data-action', stage)
    await card.getByTestId('aiws-wish-next').click()
  }
  let record: { id: string; ok: boolean; error?: unknown } | undefined
  await expect.poll(async () => { record = ((await wish(actor, ws)).payload.history?.[stage] ?? [])[0]; return Boolean(record && record.id !== before) }, { timeout: 600_000, intervals: [3_000] }).toBe(true)
  await expect(card).toHaveAttribute('data-running', '', { timeout: 60_000 })
  expect(record!.ok, JSON.stringify(record!.error)).toBe(true)
  return record!
}

/** A workspace with an order table, a wish asking for monthly totals and a commentary, and its Blocks. */
async function setup(owner: Actor): Promise<Workspace> {
  const created = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  owner.client.owned.add(created.workspace_id)
  const ws: Workspace = { workspace_id: created.workspace_id, epoch: created.epoch }
  await commit(owner, ws, [
    { op: 'entity.create', entity_id: 'sf-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 's',
      payload: { kind: 'folder', system: 'surface_content', surface_id: 'sf', title: '季度经营' } },
    { op: 'entity.create', entity_id: 'sf', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 's', name: '季度经营',
      payload: { kind: 'surface', layout: { mode: 'free' }, title: '季度经营', content_folder_id: 'sf-content' } },
    { op: 'entity.create', entity_id: 'orders', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'a', name: '订单',
      payload: { fields: [
        { field_id: 'month', name: '月份', type: 'date' }, { field_id: 'customer', name: '客户', type: 'text' },
        { field_id: 'amount', name: '销售额', type: 'decimal', scale: 2 },
        { field_id: 'status', name: '状态', type: 'select', options: [{ option_id: 'done', label: '已结算' }, { option_id: 'open', label: '未结算' }] }] } },
  ] as Operation[])
  const records = Array.from({ length: 12 }, (_, i) => ({ record_id: `o${i}`, values: {
    month: MONTHS[i % 3], customer: `客户${i % 4}`, amount: `${100 + i * 10}.00`, status: i === 11 ? 'open' : 'done' } }))
  await commit(owner, ws, [{ op: 'table.insert_records', source_id: 'orders', records } as Operation])
  await commit(owner, ws, [
    { op: 'entity.create', entity_id: 'wish-sales', type_id: 'buckyos.wish', parent_id: 'sf-content', order_key: 'w', name: '月度销售',
      payload: { title: '月度销售', executor: 'xllm', output_mode: 'overwrite', output: { container_id: 'sf-content', surface_id: 'sf', name: '月度销售结果' },
        prompt: '用左边的订单表：只算状态为已结算的订单，按月份汇总销售额和订单数，做成一张月度汇总表；再写两三句中文解读，说出总销售额和最高的月份。' } },
    { op: 'entity.create', entity_id: 'blk-orders', type_id: 'buckyos.cell', parent_id: 'sf', order_key: 'b', placement: { x: 0, y: 0, w: 520, h: 320 },
      payload: { view: { type: 'table' }, source_ref: { entity_id: 'orders' }, title: '订单' } },
    { op: 'entity.create', entity_id: 'blk-wish', type_id: 'buckyos.cell', parent_id: 'sf', order_key: 'd', placement: { x: 560, y: 0, w: 420, h: 300 },
      payload: { view: { type: 'wish' }, source_ref: { entity_id: 'wish-sales' } } },
  ] as Operation[])

  return ws
}

test('[macro] analyze, code, run, the data changes, run again: the commentary follows the data', async ({ owner }, info) => {
  test.setTimeout(1_500_000)
  const ws = await setup(owner)
  const page = owner.page
  await openWorkspace(owner, ws)
  const card = await showCard(page)

  // analyze: the task contract, written as the run ends
  await step(owner, ws, card, 'analyze', 'prompt')
  await expect(card).toHaveAttribute('data-tab', 'analysis')
  const analyzed = await wish(owner, ws)
  expect(analyzed.payload.analysis.schema_version).toBe('wish.analysis.v3')
  expect((analyzed.payload.analysis.checks as { kind: string; level?: string }[]).filter((c) => c.kind === 'program').every((c) => c.level === 'error' || c.level === 'warning')).toBe(true)
  await screenshot(page, info, 'macro-analyzed')

  // code: the model writes the macro and trial-runs it in this browser; only the macro is stored
  const codedRecord = await step(owner, ws, card, 'code', 'analysis')
  await expect(card).toHaveAttribute('data-tab', 'macro')
  await expect(card.getByTestId('aiws-wish-run')).toBeVisible()
  const coded = await wish(owner, ws)
  expect(coded.payload.program.api_version).toBe(3)
  expect(coded.payload.last_run ?? null, 'coding writes no results').toBeNull()
  expect((codedRecord as { usage?: { model?: string } }).usage?.model, 'the record names the model that answered').toBeTruthy()
  const declared = Object.values(coded.payload.program.manifest.inputs as Record<string, { entity_id: string; fields?: { field_id: string }[] }>)
  expect(declared.some((i) => i.entity_id === 'orders' && (i.fields ?? []).some((f) => f.field_id === 'amount')), JSON.stringify(coded.payload.program.manifest)).toBe(true)
  await screenshot(page, info, 'macro-coded')

  // run: the results and their Blocks, written as it ends
  await step(owner, ws, card, 'run', 'macro')
  await expect(card).toHaveAttribute('data-tab', 'results')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(1)')
  const first = await wish(owner, ws)
  const bindings = first.payload.last_run.result_bindings.results as Record<string, { type: string; entity_id: string }>
  const text = Object.values(bindings).find((b) => b.type === 'richtext')
  expect(text, JSON.stringify(bindings)).toBeTruthy()
  const before = await textOf(owner, ws, text!.entity_id)
  await screenshot(page, info, 'macro-ran-first')

  // the data changes: stale; running the macro (no model loop) updates the table and the commentary together
  await setAmount(owner, ws, 'o0', '5000.00')
  await expect.poll(async () => (await freshness(owner, ws)).status, { timeout: 60_000 }).toBe('stale')
  await expect(card.getByTestId('aiws-wish-results-stale')).toBeVisible({ timeout: 60_000 })
  await step(owner, ws, card, 'run', 'run-button')
  await expect.poll(async () => (await freshness(owner, ws)).status, { timeout: 60_000 }).toBe('current')
  const ran = await wish(owner, ws)
  expect(ran.payload.last_run.mode).toBe('run')
  expect(ran.payload.program.digest).toBe(coded.payload.program.digest)
  const after = await textOf(owner, ws, text!.entity_id)
  expect(after, 'the commentary was produced again from the new data').not.toBe(before)
  await screenshot(page, info, 'macro-ran')

  // a renamed field changes nothing: the macro reads it by id under the name it was coded with
  const field = await owner.client.ok<{ content: { def_rev: number } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'orders', selector: { kind: 'table_field', field_id: 'amount' } })
  await commit(owner, ws, [{ op: 'table.update_field', source_id: 'orders', field_id: 'amount', changes: { name: '金额' }, expect: { rev: field.content.def_rev } } as Operation])
  expect((await freshness(owner, ws)).needs_coding ?? null).toBeNull()
  await setAmount(owner, ws, 'o1', '7000.00')
  await step(owner, ws, card, 'run', 'run-button')
  await expect.poll(async () => (await freshness(owner, ws)).status, { timeout: 60_000 }).toBe('current')

  // a declared field deleted: the wish needs coding again, and its run button says so instead of running
  const status = await owner.client.ok<{ content: { def_rev: number } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'orders', selector: { kind: 'table_field', field_id: 'amount' } })
  await commit(owner, ws, [{ op: 'table.delete_field', source_id: 'orders', field_id: 'amount', expect: { rev: status.content.def_rev } } as Operation])
  await expect.poll(async () => (await freshness(owner, ws)).needs_coding?.reasons?.[0]?.code, { timeout: 60_000 }).toBe('field_deleted')
  await expect(card).toHaveAttribute('data-main', 'needs_coding', { timeout: 60_000 })
  await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'disabled')
  await screenshot(page, info, 'macro-needs-coding')
})

const HOSTED_MACRO = `
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额', '状态'] }, ai: { complete: { max_calls: 2, max_tokens: 2000 } } })
  let claim = 'readable'
  try { Deno.readTextFileSync('../claim.json') } catch (e) { claim = e.name }
  let env = 'readable'
  try { Deno.env.toObject() } catch (e) { env = e.name }
  const { rows } = await aiws.sql('SELECT "月份", round(sum("销售额"), 2) AS "销售额", count(*) AS "订单数" FROM sales WHERE "状态" = ? GROUP BY 1 ORDER BY 1', ['已结算'])
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal', 订单数: 'number' } })
  const total = Math.round(rows.reduce((s, r) => s + r['销售额'], 0) * 100) / 100
  aiws.facts({ total, months: rows.length, claim, env })
  const { text } = await aiws.ai.complete({ system: '用一句中文写解读，只用给出的数字，不要计算新的数字。', prompt: JSON.stringify({ total, months: rows.length }), max_tokens: 400 })
  aiws.result.text('commentary', text)
}
`

/** `actor` works runs of wish-sales through the CLI as their executor: `run-start` (a local run it executes itself),
 * claim, the stage folder; `settle` waits until the stage has written its product and its record. */
function executor(actor: Actor, token: string, baseURL: string, ws: Workspace) {
  const work = mkdtempSync(join(tmpdir(), 'aiws-cli-dv-'))
  const run = (...args: string[]) => cli(token, baseURL, work, 'aiws', ...args, '--workspace', ws.workspace_id)
  let n = 0
  const start = (stage: 'analyze' | 'code' | 'run') => {
    const started = run('run-start', '--wish', 'wish-sales', '--stage', stage) as { run_id: string; claim_token: string }
    const claimed = run('run-claim', '--run', started.run_id, '--claim-token', started.claim_token)
    writeFileSync(join(work, 'claim.json'), JSON.stringify(claimed))
    const dir = `stage-${++n}`
    run('run-files', '--run', started.run_id, '--attempt', String(claimed.attempt), '--dir', dir)
    return { run_id: started.run_id, attempt: String(claimed.attempt), dir, stage: claimed.stage as { kind: string; manifest?: unknown; budget?: Record<string, unknown> } }
  }
  const settle = async (runId: string) => {
    let view: WishRunView | null = null
    await expect.poll(async () => { view = await actor.client.ok<WishRunView>('proc.get', { workspace_id: ws.workspace_id, run_id: runId }); return view.state }, { timeout: 120_000 }).not.toMatch(/^(queued|snapshotting|running|validating|applying)$/)
    // the stage wrote at once: its product and its record, nothing to apply (许愿格交互升级 §3.3)
    expect(view!.state, JSON.stringify(view)).toBe('succeeded')
    expect(view!.applied?.status, JSON.stringify(view!.applied)).toBe('accepted')
    expect(view!.record?.ok, JSON.stringify(view!.record)).toBe(true)
    return view!
  }
  const analyze = async (results: unknown[]) => {
    const a = start('analyze')
    const analysis = { schema_version: 'wish.analysis.v3', status: 'ready', inputs: [{ name: 'sales', ref: 'orders' }],
      context_prompt: '读取 sales 的全部订单，只算 状态=已结算，按月份汇总销售额与订单数。', output_contract: { results }, checks: [], blockers: [], warnings: [] }
    writeFileSync(join(work, 'analysis.json'), JSON.stringify({ analysis }))
    expect(run('run-tool', '--run', a.run_id, '--attempt', a.attempt, '--name', 'submit_analysis', '--args-file', 'analysis.json').status).toBe('ok')
    run('run-report', '--run', a.run_id, '--attempt', a.attempt, '--finished')
    return settle(a.run_id)
  }
  const code = async (source: string) => {
    const c = start('code')
    writeFileSync(join(work, c.dir, 'program/main.js'), source)
    const trial = run('run-program', '--run', c.run_id, '--attempt', c.attempt, '--dir', c.dir)
    expect(trial.status, JSON.stringify(trial)).toBe('ok')
    expect(run('run-tool', '--run', c.run_id, '--attempt', c.attempt, '--name', 'finish', '--args', '{"summary":"DV"}').status).toBe('ok')
    run('run-report', '--run', c.run_id, '--attempt', c.attempt, '--finished')
    return { trial, view: await settle(c.run_id) }
  }
  const rerun = async () => {
    const r = start('run')
    expect(r.stage.kind).toBe('program')
    const ran = run('run-program', '--run', r.run_id, '--attempt', r.attempt, '--dir', r.dir)
    expect(ran.status, JSON.stringify(ran)).toBe('ok')
    run('run-report', '--run', r.run_id, '--attempt', r.attempt, '--finished')
    return settle(r.run_id)
  }
  return { run, start, settle, analyze, code, rerun }
}

const MONTHLY_AND_COMMENTARY = [{ name: 'monthly', type: 'table', title: '月度汇总', key: ['月份'] }, { name: 'commentary', type: 'richtext', title: '解读' }]

// The hosted dispatch itself is not exercised here: since 权限需求 S6 the claim token reaches only the configured Agent.
test('[macro-cli] an executor outside the browser works a wish through the CLI: a restricted macro run, its AI with its own session', async ({ owner, baseURL }) => {
  test.setTimeout(900_000)
  const ws = await setup(owner)
  const token = (await owner.page.context().cookies()).find((c) => c.name === 'buckyos_session_token')!.value
  const ex = executor(owner, token, baseURL!, ws)

  // analyze: an analysis worked out by the executor, through the stage tool
  await ex.analyze(MONTHLY_AND_COMMENTARY)

  // code: the macro runs restricted (no claim.json, no environment), its SQL by the service, its AI with this session
  const { trial, view: coded } = await ex.code(HOSTED_MACRO)
  expect(String(trial.text)).toContain('reads sales: 月份, 销售额, 状态')
  // the trial's facts are in its answer (the trial results go into the record only, never into the document)
  const facts = JSON.parse((/\nfacts: (\{.*\})\nChecks:/.exec(String(trial.text)) ?? [])[1] ?? '{}') as Record<string, unknown>
  expect(['NotCapable', 'PermissionDenied']).toContain(facts.claim)
  expect(['NotCapable', 'PermissionDenied']).toContain(facts.env)
  expect(coded.record?.summary, JSON.stringify(coded.record)).toMatchObject({ trial: { results: 2 } })
  const first = await wish(owner, ws)
  expect(first.payload.program.api_version).toBe(3)
  expect(first.payload.last_run ?? null, 'coding writes no results').toBeNull()

  // running the stored macro writes the table and the commentary; the data changes, running again updates them
  await ex.rerun()
  const monthly = (await wish(owner, ws)).payload.last_run.result_bindings.results.monthly as { entity_id: string }
  const table = await textOf(owner, ws, monthly.entity_id)
  await setAmount(owner, ws, 'o0', '9000.00')
  const done = await ex.rerun()
  expect(done.record?.summary, JSON.stringify(done.record)).toMatchObject({ created: 0 })
  expect(await textOf(owner, ws, monthly.entity_id), 'the monthly totals follow the data').not.toBe(table)
  expect((await freshness(owner, ws)).status).toBe('current')
})

const PAGE_MACRO = `
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额', '状态'] } })
  const { rows } = await aiws.sql('SELECT "月份", round(sum("销售额"), 2) AS "销售额" FROM sales WHERE "状态" = ? GROUP BY 1 ORDER BY 1', ['已结算'])
  aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal' } })
  aiws.result.html('dash', { html: '<div id=n>月度看板</div>', bindings: { monthly: 'result:monthly' } })
}
`

const EDITOR = ['read', 'comment', 'append', 'update', 'delete', 'structure']

test('[macro-permissions] a run is written as its macro identity: an editor\'s page waits for a manager, only run starts nothing, an intersection group bounds the reads', async ({ owner, peer, baseURL }, info) => {
  test.setTimeout(900_000)
  const ws = await setup(owner)
  const at = { workspace_id: ws.workspace_id }
  const cookie = async (actor: Actor) => (await actor.page.context().cookies()).find((c) => c.name === 'buckyos_session_token')!.value
  const audits = async () => (await owner.client.ok<{ items: { action: string }[] }>('acl.audit', at)).items.filter((i) => i.action === 'macro.changed').length
  await owner.client.ok('acl.set_group', { ...at, group_id: 'writers', title: 'DV 写结果' })
  await owner.client.ok('acl.bind_macro', { ...at, wish: '~wish-sales', group_id: 'writers', mode: 'union' })

  // an editor without manage analyzes and codes through the CLI (no model, no AI): its page is written, not trusted
  await owner.client.ok('acl.set', { ...at, changes: [{ subject: peer.userId, resource: '/', allow: EDITOR }] })
  const editor = executor(peer, await cookie(peer), baseURL!, ws)
  await editor.analyze([{ name: 'monthly', type: 'table', title: '月度汇总', key: ['月份'] }, { name: 'dash', type: 'html', title: '月度看板' }])
  expect(await audits(), 'an analysis is no change of the macro').toBe(0)
  await editor.code(PAGE_MACRO)
  expect(await audits(), 'a new macro of a bound wish is on the record').toBe(1)
  // the editor runs it: the results, its page among them, written as the editor's run
  await editor.rerun()
  expect(await audits(), 'a run is no change of the macro').toBe(1)
  const coded = await wish(owner, ws)
  const dash = coded.payload.last_run.result_bindings.results.dash as { entity_id: string; cells: string[] }
  const def = async () => (await owner.client.ok<{ content: { payload: Record<string, any>; key_revs: Record<string, number> } }>('doc.read', { ...at, entity_id: dash.entity_id })).content // eslint-disable-line @typescript-eslint/no-explicit-any
  expect((await def()).payload.trusted ?? null, 'an editor\'s page is not vouched for').toBeNull()

  // the owner's canvas: starting the untrusted page asks first
  const page = owner.page
  await openWorkspace(owner, ws)
  const top = page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  const toggle = page.getByTestId('aiws-run-toggle')
  if ((await toggle.getAttribute('data-interactive')) !== 'true') await toggle.click()
  const cell = dash.cells[0]
  await expect(page.getByTestId(`aiws-html-start-${cell}`)).toHaveAttribute('data-untrusted', 'true')
  await page.getByTestId(`aiws-html-start-${cell}`).click()
  await expect(page.getByTestId(`aiws-html-untrusted-${cell}`)).toBeVisible()
  await screenshot(page, info, 'permissions-untrusted-page')

  // a manager vouches: trusted by the owner, and it starts without asking
  const current = await def()
  await commit(owner, ws, [{ op: 'entity.set_keys', entity_id: dash.entity_id, keys: [{ key: 'trusted', value: true, expect: { rev: current.key_revs.trusted ?? 0 } }] } as Operation])
  expect((await def()).payload.trusted.by).toBe(owner.userId)
  await openWorkspace(owner, ws)
  if (await top.isVisible()) await top.click()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  if ((await toggle.getAttribute('data-interactive')) !== 'true') await toggle.click()
  await page.getByTestId(`aiws-html-start-${cell}`).click()
  await expect(page.getByTestId(`aiws-html-untrusted-${cell}`)).toHaveCount(0)
  await expect(page.getByTestId(`aiws-html-active-${cell}`)).toBeVisible({ timeout: 30_000 })
  await screenshot(page, info, 'permissions-trusted-page')

  // the data changes; the owner's run moves only data, and is no change of the macro
  await setAmount(owner, ws, 'o0', '9000.00')
  await executor(owner, await cookie(owner), baseURL!, ws).rerun()
  expect(await audits()).toBe(1)

  // holding only run: nothing it could run would ever be applied, so nothing starts
  await owner.client.ok('acl.set', { ...at, changes: [{ subject: peer.userId, resource: '/', allow: ['read', 'run'] }] })
  const refused = await peer.client.call<{ ok: boolean; error?: { code: string; sub_code?: string } }>('proc.start', { ...at, program: 'wish.xllm@1', idempotency_key: `dv-run-only-${Date.now()}`,
    params: { wish_id: 'wish-sales', stage: 'run', execution: { mode: 'local' } } })
  expect(refused, JSON.stringify(refused)).toMatchObject({ ok: false, error: { code: 'PERMISSION_DENIED', sub_code: 'MACRO_CANNOT_WRITE' } })
  await openWorkspace(peer, ws)
  const ptop = peer.page.getByTestId('aiws-top-canvas')
  if (await ptop.isVisible()) await ptop.click()
  await peer.page.getByTestId('aiws-zoom-menu').click()
  await peer.page.getByTestId('aiws-fit-all').click()
  // the card: its run button disabled with why, no next step (no update)
  const peerCard = peer.page.getByTestId('aiws-wish-card-wish-sales')
  await expect(peerCard.getByTestId('aiws-wish-run')).toBeDisabled()
  await expect(peerCard.getByTestId('aiws-wish-run')).toHaveAttribute('title', /只有运行权限/)
  await expect(peerCard.getByTestId('aiws-wish-next')).toHaveCount(0)
  await screenshot(peer.page, info, 'permissions-run-only')

  // bound with intersection to a group that cannot read the input: even the owner's run reads no more than the group
  await owner.client.ok('acl.set', { ...at, changes: [
    { subject: '@writers', resource: '/', allow: EDITOR },
    { subject: '@writers', resource: '~orders', deny: ['read'] }] })
  await owner.client.ok('acl.bind_macro', { ...at, wish: '~wish-sales', group_id: 'writers', mode: 'intersection' })
  await setAmount(owner, ws, 'o1', '9100.00')
  const started = await owner.client.ok<WishRunView & { claim_token: string }>('proc.start', { ...at, program: 'wish.xllm@1', idempotency_key: `dv-bounded-${Date.now()}`,
    params: { wish_id: 'wish-sales', stage: 'run', execution: { mode: 'local' } } })
  await owner.client.call('proc.claim', { ...at, run_id: started.run_id, claim_token: started.claim_token })
  let bounded: WishRunView | null = null
  await expect.poll(async () => { bounded = await owner.client.ok<WishRunView>('proc.get', { ...at, run_id: started.run_id }); return bounded.state }, { timeout: 60_000 }).toBe('failed')
  expect(JSON.stringify(bounded!.error)).toContain('not readable')
  await owner.client.ok('acl.bind_macro', { ...at, wish: '~wish-sales', group_id: null })
})
