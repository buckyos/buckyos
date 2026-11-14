/* A wish's three stages on a real Zone with the real model, from the browser (notepads/aiws-ai-native-phase1-todo.md
 * T7-3): analyze → code (the model writes the macro and trial-runs it; applying stores it with its results) → the data
 * changes → run (the stored macro only, no model loop; the commentary follows the data) → a renamed field changes
 * nothing → a deleted declared field asks for coding again. The CLI as an executor runs the macro restricted; with a
 * second real account, a run is written as its macro identity (权限需求 §4.5, Q31–Q32). */

import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Page } from '@playwright/test'
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

async function applyCandidate(page: Page, stage: 'code' | 'run') {
  const candidate = page.getByTestId('aiws-wish-candidate')
  await expect(candidate).toHaveAttribute('data-stage', stage, { timeout: 600_000 })
  await expect(candidate).toHaveAttribute('data-ready', 'true', { timeout: 60_000 })
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toBeVisible({ timeout: 60_000 })
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

test('[macro] analyze, code, apply, the data changes, run: the commentary follows the data', async ({ owner }, info) => {
  test.setTimeout(1_500_000)
  const ws = await setup(owner)
  const page = owner.page
  await openWorkspace(owner, ws)
  const top = page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  await page.getByTestId('aiws-canvas-block-blk-wish').click()
  await page.getByTestId('aiws-near-wish-open').click()
  await expect(page.getByTestId('aiws-wish-executor')).toHaveValue('xllm')

  // analyze: the task contract, written back as the run ends
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'needs_coding', { timeout: 600_000 })
  const analyzed = await wish(owner, ws)
  expect(analyzed.payload.analysis.schema_version).toBe('wish.analysis.v3')
  await screenshot(page, info, 'macro-analyzed')

  // code: the model writes the macro and trial-runs it in this browser; applying stores it with its results
  await page.getByTestId('aiws-wish-code').click()
  await applyCandidate(page, 'code')
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'current', { timeout: 60_000 })
  const coded = await wish(owner, ws)
  expect(coded.payload.program.api_version).toBe(3)
  const declared = Object.values(coded.payload.program.manifest.inputs as Record<string, { entity_id: string; fields?: { field_id: string }[] }>)
  expect(declared.some((i) => i.entity_id === 'orders' && (i.fields ?? []).some((f) => f.field_id === 'amount')), JSON.stringify(coded.payload.program.manifest)).toBe(true)
  const bindings = coded.payload.last_run.result_bindings.results as Record<string, { type: string; entity_id: string }>
  const text = Object.values(bindings).find((b) => b.type === 'richtext')
  expect(text, JSON.stringify(bindings)).toBeTruthy()
  const before = await textOf(owner, ws, text!.entity_id)
  await screenshot(page, info, 'macro-coded')

  // the data changes: stale; running the macro (no model loop) updates the table and the commentary together
  await setAmount(owner, ws, 'o0', '5000.00')
  await expect.poll(async () => (await freshness(owner, ws)).status, { timeout: 60_000 }).toBe('stale')
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'stale', { timeout: 60_000 })
  await page.getByTestId('aiws-wish-run').click()
  await applyCandidate(page, 'run')
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
  await page.getByTestId('aiws-wish-run').click()
  await applyCandidate(page, 'run')
  await expect.poll(async () => (await freshness(owner, ws)).status, { timeout: 60_000 }).toBe('current')

  // a declared field deleted: the wish needs coding again, and a run is refused before it starts
  const status = await owner.client.ok<{ content: { def_rev: number } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'orders', selector: { kind: 'table_field', field_id: 'amount' } })
  await commit(owner, ws, [{ op: 'table.delete_field', source_id: 'orders', field_id: 'amount', expect: { rev: status.content.def_rev } } as Operation])
  await expect.poll(async () => (await freshness(owner, ws)).needs_coding?.reasons?.[0]?.code, { timeout: 60_000 }).toBe('field_deleted')
  await expect(page.getByTestId('aiws-wish-main')).toHaveAttribute('data-main', 'needs_coding', { timeout: 60_000 })
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
 * claim, the stage folder; `settle` previews and applies as `actor`. */
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
    await expect.poll(async () => { view = await actor.client.ok<WishRunView>('proc.get', { workspace_id: ws.workspace_id, run_id: runId, choices: {} }); return view.state }, { timeout: 120_000 }).not.toMatch(/^(queued|snapshotting|running|validating)$/)
    expect(view!.state, JSON.stringify(view)).toBe('waiting_confirmation')
    expect(view!.preview?.ready, JSON.stringify(view!.preview)).toBe(true)
    await actor.client.ok('proc.apply', { workspace_id: ws.workspace_id, run_id: runId, plan_digest: view!.preview!.plan_digest, session_id: 'dv-aiws' })
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
  const facts = (coded.candidate as unknown as { facts: Record<string, unknown> }).facts
  expect(['NotCapable', 'PermissionDenied']).toContain(facts.claim)
  expect(['NotCapable', 'PermissionDenied']).toContain(facts.env)
  const first = await wish(owner, ws)
  expect(first.payload.program.api_version).toBe(3)

  // the data changes; running the stored macro updates the table and the commentary
  await setAmount(owner, ws, 'o0', '9000.00')
  const done = await ex.rerun()
  expect((done.candidate as unknown as { facts: { total: number } }).facts.total).toBeGreaterThan((facts.total as number))
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
  await peer.page.getByTestId('aiws-canvas-block-blk-wish').click()
  await peer.page.getByTestId('aiws-near-wish-open').click()
  await expect(peer.page.getByTestId('aiws-wish-run-only')).toBeVisible()
  await expect(peer.page.getByTestId('aiws-wish-run')).toBeDisabled()
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
