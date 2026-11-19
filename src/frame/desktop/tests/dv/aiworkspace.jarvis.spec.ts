/* Jarvis analyses, the browser codes, the macro runs (notepads/aiws-ai-native-remaining-todo.md C2-1, C2-2) on a real
 * Zone with the real model and the owner's real Agent:
 *
 *     pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.jarvis.spec.ts
 *
 * The wish's card is selected and the Agent's panel opened from the collaboration zone: its conversation is the one
 * marked as the AI Workspace's, and the first message carries the workspace, the canvas and the selected wish. The
 * Agent works the task out and hands it in with `buckyos aiws run-start --stage analyze` (manual aiworkspace_run,
 * "Asked from the AI Workspace panel"): the analysis record is written by the Agent, not by this browser. Coding and
 * running are then clicks on the card, as in aiworkspace.macro.spec.ts.
 *
 * Needs a ready Agent of the admin user (the Jarvis template, the aiworkspace_run manual installed), owner access to
 * the workspace for it (an Agent nobody configured is judged as its owner) and AI resources for both. Skipped when the
 * owner has no Agent. Left behind: the panel's conversation with the Agent (titled "AI Workspace", marked
 * `ui.source = aiworkspace`) and the messages of this run. */

import type { Locator } from '@playwright/test'
import type { Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
type HistoryRecord = { id: string; ok: boolean; by?: string; error?: unknown }

test.use({ viewport: { width: 1600, height: 1000 } })

const MONTHS = ['2026-07-01', '2026-08-01', '2026-09-01']

async function commit(actor: Actor, ws: Workspace, operations: Operation[]) {
  const result = await actor.client.commit(ws, operations)
  expect(result.status, JSON.stringify(result)).toBe('accepted')
}

async function history(actor: Actor, ws: Workspace, stage: 'analyze' | 'code' | 'run'): Promise<HistoryRecord[]> {
  const read = await actor.client.ok<{ content: { payload: { history?: { [stage: string]: HistoryRecord[] } } } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'wish-sales' })
  return read.content.payload.history?.[stage] ?? []
}

async function newRecord(actor: Actor, ws: Workspace, stage: 'analyze' | 'code' | 'run', before: string | undefined, timeout: number): Promise<HistoryRecord> {
  let record: HistoryRecord | undefined
  await expect.poll(async () => { record = (await history(actor, ws, stage))[0]; return Boolean(record && record.id !== before) }, { timeout, intervals: [5_000] }).toBe(true)
  return record!
}

/** An order table and a wish asking for monthly totals and a short commentary, on a free canvas `sf`. */
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

/** A stage from the card's tab ("next step"), waited for until it wrote its record. */
async function cardStep(actor: Actor, ws: Workspace, card: Locator, stage: 'code' | 'run', tab: 'analysis' | 'macro') {
  const before = (await history(actor, ws, stage))[0]?.id
  await card.getByTestId(`aiws-wish-tab-${tab}`).click()
  await expect(card.getByTestId('aiws-wish-next')).toHaveAttribute('data-action', stage)
  await card.getByTestId('aiws-wish-next').click()
  const record = await newRecord(actor, ws, stage, before, 600_000)
  expect(record.ok, JSON.stringify(record.error)).toBe(true)
  return record
}

test('[jarvis] the Agent analyses the selected wish from the panel and hands it in with run-start; the browser codes, the macro runs', async ({ owner }, info) => {
  test.setTimeout(2_400_000)
  const ws = await setup(owner)
  const page = owner.page
  await openWorkspace(owner, ws)
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  const card = page.getByTestId('aiws-wish-card-wish-sales')
  await card.getByTestId('aiws-wish-title').click()

  // the panel: the owner's Agent, its AI Workspace conversation, the selected wish going along
  await page.getByTestId('aiws-status-row').getByTestId('aiws-collab-jarvis').click()
  const panel = page.getByTestId('aiws-jarvis')
  await expect(panel.getByTestId('aiws-jarvis-conversation').or(panel.getByTestId('aiws-jarvis-none')).or(panel.getByTestId('aiws-jarvis-failed'))).toBeVisible({ timeout: 60_000 })
  test.skip(await panel.getByTestId('aiws-jarvis-none').isVisible(), 'the admin user has no Agent')
  await expect(panel.getByTestId('aiws-jarvis-conversation')).toBeVisible()
  await expect(panel.getByTestId('aiws-jarvis-context')).toContainText('季度经营')
  await screenshot(page, info, 'jarvis-panel')

  const before = (await history(owner, ws, 'analyze'))[0]?.id
  const composer = panel.getByTestId('message-composer').getByRole('textbox')
  await composer.fill('请分析选中的这个许愿格要做的事（输入是画布上的订单表）。分析好以后，按 aiworkspace_run 手册用 run-start 把分析交付给这个许愿格。只做分析，不要编码，也不要运行。')
  await composer.press('Enter')
  await expect(panel.getByTestId('aiws-jarvis-context')).toHaveCount(0)

  // the Agent's analysis: a record of the analyze stage, started by the Agent
  const analyzed = await newRecord(owner, ws, 'analyze', before, 1_200_000)
  expect(analyzed.ok, JSON.stringify(analyzed.error)).toBe(true)
  expect(analyzed.by, 'the analysis was started by the Agent, not by the browser').not.toBe(owner.userId)
  const wish = await owner.client.ok<{ content: { payload: { analysis?: { schema_version?: string } } } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'wish-sales' })
  expect(wish.content.payload.analysis?.schema_version).toBe('wish.analysis.v3')
  await screenshot(page, info, 'jarvis-analyzed')

  // the browser codes from the Agent's analysis, then runs the macro
  await panel.getByTestId('aiws-jarvis-close').click()
  await cardStep(owner, ws, card, 'code', 'analysis')
  await cardStep(owner, ws, card, 'run', 'macro')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText(/[(（]1[)）]$/)
  await screenshot(page, info, 'jarvis-ran')
})
