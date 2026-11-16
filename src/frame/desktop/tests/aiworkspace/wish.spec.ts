/* 许愿格交互升级 §5: the wish card — four tabs, the next step and the run button on the card; each stage writes when it
 * ends (no "apply"); records with their links, rollback, invalidation warnings, lines to the inputs and results, the
 * run's lines in the status bar; history that travels with a clone. Every run executes in this browser with the scripted
 * model (mock-llm.mjs; its marks inject warnings, failures and slowness). Also the HTML extension API (v1 calls and
 * `aiws` v2 bindings), and the local executor's interrupted run resumed by this browser. */

import { devices, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { expect, fitAll, mainMenu, openCanvas, openProperties, openStatus, openWorkspacePage, scriptedModel, test, wishCard, wishRun, wishStageEnded, wishStep, wishThreeSteps, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const WISH = 'wish-analysis'
const BLOCK = 'blk-wish'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function derived(api: Api, ws: { workspace_id: string }, wishId = WISH): Promise<Json[]> {
  return (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.derived?.wish_id === wishId)
}

/** The demo's wish with its request replaced (the scripted model's marks go into it). */
async function setPrompt(api: Api, ws: { workspace_id: string; epoch: string }, prompt: string, wishId = WISH) {
  const wish = await api.read(ALICE, ws.workspace_id, wishId)
  const r = await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'prompt', value: prompt, expect: { rev: wish.content.key_revs.prompt } }] }])
  expect(r.status, JSON.stringify(r)).toBe('accepted')
}

async function setRevenue(api: Api, ws: { workspace_id: string; epoch: string }, record: string, value: number) {
  const rev = (await api.cell(ALICE, ws.workspace_id, 'sales', record, 'revenue')).rev
  const r = await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: record, field_id: 'revenue', value, expect: { rev } }] }])
  expect(r.status, JSON.stringify(r)).toBe('accepted')
}

function records(page: Page, wishId = WISH) {
  return wishCard(page, wishId).getByTestId('aiws-wish-record')
}

test('§5 1/2/5/6/7/14: a new wish — analyze, code and run from its card; each stage writes when it ends and its record says what it used; running the stored macro calls no model', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  const next = card.getByTestId('aiws-wish-next')
  // 1. a wish never analysed: only the request can be shown, the next step analyses, no run button
  await expect(card).toHaveAttribute('data-tab', 'prompt')
  await expect(card.getByTestId('aiws-wish-tab-prompt')).toBeEnabled()
  for (const tab of ['analysis', 'macro', 'results']) await expect(card.getByTestId(`aiws-wish-tab-${tab}`)).toBeDisabled()
  await expect(next).toHaveAttribute('data-action', 'analyze')
  await expect(next).toHaveAttribute('data-primary', 'true')
  await expect(next).toHaveText('分析需求')
  await expect(card.getByTestId('aiws-wish-run')).toHaveCount(0)
  await expect(card.getByTestId('aiws-wish-prompt')).toHaveValue(/原始销售数据/)
  const head = await api.headSeq(ALICE, ws.workspace_id)

  // 2. analyze: written as it ends (one commit), the card goes to "需求", its record says when, how long, what it used
  const analysed = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze')
  await expect(card).toHaveAttribute('data-tab', 'analysis')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect(analysed).toMatchObject({ stage: 'analyze', action: 'analyze', ok: true, by: 'alice' })
  expect(analysed.usage.model).toMatch(/@mock-llm$/)
  expect(analysed.usage.input_tokens).toBeGreaterThan(0)
  expect(analysed.usage.output_tokens).toBeGreaterThan(0)
  expect(analysed.usage.calls).toBeGreaterThanOrEqual(2)
  const payload = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload
  expect(payload.analysis.status).toBe('ready')
  expect(payload.analysis.checks.find((c: Json) => c.id === 'total').level).toBe('error')
  expect(payload.inputs).toEqual([{ entity_id: 'sales', name: 'data', label: '原始销售数据', version: { mode: 'follow' } }])
  const first = records(page).first()
  await expect(first).toHaveAttribute('data-ok', 'true')
  await expect(first).toHaveAttribute('data-expanded', 'true')
  await expect(first).toHaveAttribute('data-current', 'true')
  await expect(first).toContainText('执行“分析需求”')
  await expect(first).toContainText(/输入 [\d,]+ \/ 输出 [\d,]+ token/)
  await expect(first).toContainText('@mock-llm')
  await expect(first).toContainText('要生成 2 项结果')
  await expect(next).toHaveAttribute('data-action', 'code')
  await expect(next).toHaveText('编写宏代码')
  await expect(next).toHaveAttribute('data-primary', 'true')
  // nothing to apply, anywhere
  await expect(page.getByTestId('aiws-workspace').getByRole('button', { name: '应用', exact: true })).toHaveCount(0)
  // its contract opens from the record: the checks with their level
  await first.locator('[data-testid="aiws-wish-record-link"][data-kind="analysis"]').click()
  const dialog = page.getByTestId('aiws-wish-dialog')
  await expect(dialog).toHaveAttribute('data-kind', 'analysis')
  await expect(dialog.getByTestId('aiws-wish-contract')).toContainText('summary')
  await expect(dialog.getByTestId('aiws-wish-checkdefs')).toContainText('错误级')
  await expect(dialog.getByTestId('aiws-wish-checkdefs')).toContainText('汇总合计等于输入合计')
  await dialog.getByTestId('aiws-wish-dialog-close').click()
  await expect(dialog).toHaveCount(0)

  // 5. code: only the macro is written; the card goes to "宏", the run button appears (blue: no AI), no result yet
  const coded = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'code')
  await expect(card).toHaveAttribute('data-tab', 'macro')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  expect(coded).toMatchObject({ stage: 'code', action: 'code', ok: true })
  expect(coded.summary.trial).toEqual({ results: 2, checks_passed: 1, checks_total: 1 })
  expect(coded.outputs.program.ai).toBe(false)
  expect(await derived(api, ws)).toHaveLength(0)
  const run = card.getByTestId('aiws-wish-run')
  await expect(run).toBeVisible()
  await expect(run).toHaveAttribute('data-ai', 'false')
  await expect(run).toHaveAttribute('data-state', 'ready')
  await expect(records(page).first()).toContainText('试运行产出 2 项，1/1 项检查通过')
  await expect(next).toHaveAttribute('data-action', 'run')
  await expect(next).toHaveText('执行宏')
  // the macro opens from the record
  await records(page).first().locator('[data-testid="aiws-wish-record-link"][data-kind="program"]').click()
  await expect(dialog).toHaveAttribute('data-kind', 'program')
  await expect(dialog.getByTestId('aiws-wish-code')).toContainText("aiws.result.table('summary'")
  await dialog.getByTestId('aiws-wish-dialog-close').click()

  // 6. run: the results and their Blocks are written as it ends; the card goes to "结果(1)"
  const ran = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'run')
  await expect(card).toHaveAttribute('data-tab', 'results')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(1)')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 3)
  expect(ran).toMatchObject({ stage: 'run', action: 'run', ok: true, summary: { created: 2, updated: 0 } })
  expect(ran.usage).toBeUndefined() // the macro used no AI
  const results = await derived(api, ws)
  expect(results).toHaveLength(2)
  const lastRun = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.last_run
  const cells: string[] = lastRun.result_bindings.results.summary.cells
  expect(cells).toHaveLength(2)
  await fitAll(page)
  await expect(page.getByTestId(`aiws-canvas-block-${cells[0]}`)).toBeVisible()
  await expect(records(page).first()).toContainText('新建 2 项，更新 0 项')
  // the run joins this window's undo stack
  await expect(page.getByTestId('aiws-undo')).toBeEnabled()

  // the stored macro runs again from the run button: no model is called
  let modelCalls = 0
  await page.route(`${scriptedModel().baseUrl}/chat/completions`, (route) => { modelCalls += 1; return route.abort() })
  await setRevenue(api, ws, 's-1', 500000)
  await expect(card.getByTestId('aiws-wish-results-stale')).toBeVisible()
  const again = await wishRun(page, api, ALICE, ws.workspace_id, WISH)
  expect(again).toMatchObject({ ok: true, summary: { created: 0, updated: 2 } })
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(2)')
  expect(modelCalls).toBe(0)
  await page.unroute(`${scriptedModel().baseUrl}/chat/completions`)

  // 14. the wish never used the side drawer: it has no wish tab, the view menu offers none
  await expect(page.locator('[data-testid="aiws-side-panel"][data-tab="wish"]')).toHaveCount(0)
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-panels')
  await expect(page.getByTestId('aiws-menu-side-relations')).toBeVisible()
  await expect(page.getByTestId('aiws-menu-side-wish')).toHaveCount(0)
  await page.keyboard.press('Escape')
})

test('§5 3/13: a warning of the analysis comes first, in bold, and coding still goes on; changing the request after it asks first — cancel keeps the text, going on makes the requirements and the macro out of date', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish warn ${Date.now()}`)
  await setPrompt(api, ws, '按区域汇总「原始销售数据」的销售额，并写一段解读 #warn')
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  const next = card.getByTestId('aiws-wish-next')
  // 3. the analysis warns: its record says so first, in bold; the next step is still coding
  const analysed = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze')
  expect(analysed.warnings).toEqual(['没有找到成本列，利润率无法计算。'])
  const warnings = records(page).first().getByTestId('aiws-wish-record-warnings')
  await expect(warnings).toHaveText(/没有找到成本列，利润率无法计算。/)
  expect(await warnings.evaluate((el) => el.tagName)).toBe('B')
  // the warning precedes the outcome in the record's sentence
  const sentence = await records(page).first().locator('.aiws-wr-sentence').innerText()
  expect(sentence.indexOf('没有找到成本列')).toBeLessThan(sentence.indexOf('要生成 2 项结果'))
  await expect(next).toHaveAttribute('data-action', 'code')
  await expect(next).toBeEnabled()
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'code')
  await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'ready')

  // 13. the request changes after its requirements: the card asks first
  await card.getByTestId('aiws-wish-tab-prompt').click()
  const prompt = card.getByTestId('aiws-wish-prompt')
  const before = await prompt.inputValue()
  await prompt.fill(`${before}，按销售额从高到低`)
  const ask = page.getByTestId('aiws-wish-invalidate')
  await expect(ask).toContainText('该修改会让需求和宏失效。')
  await page.getByTestId('aiws-wish-invalidate-cancel').click()
  await expect(ask).toHaveCount(0)
  await expect(prompt).toHaveValue(before)
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.prompt).toBe(before)
  await expect(card.getByTestId('aiws-wish-tab-analysis')).toHaveAttribute('data-stale', 'false')
  // going on: the edit is saved, the requirements carry the stale mark, coding and running wait for a new analysis
  await prompt.fill(`${before}，按销售额从高到低`)
  await page.getByTestId('aiws-wish-invalidate-ok').click()
  await expect(ask).toHaveCount(0)
  await card.getByTestId('aiws-wish-title').click()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.prompt).toBe(`${before}，按销售额从高到低`)
  await expect(card.getByTestId('aiws-wish-tab-analysis')).toHaveAttribute('data-stale', 'true')
  await expect(card.getByTestId('aiws-wish-tab-macro')).toHaveAttribute('data-stale', 'true')
  await expect(next).toHaveAttribute('data-action', 'analyze')
  await expect(next).toHaveText('分析需求')
  await card.getByTestId('aiws-wish-tab-analysis').click()
  await expect(next).toHaveAttribute('data-action', 'code')
  await expect(next).toBeDisabled()
  await expect(card.getByTestId('aiws-wish-next-reason')).toHaveText('愿望已修改，先重新分析')
  await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'disabled')
  await expect(card.getByTestId('aiws-wish-run')).toBeDisabled()
  // the edit is an ordinary commit: undone, the requirements are current again
  await page.getByTestId('aiws-undo').click()
  await expect(card.getByTestId('aiws-wish-tab-analysis')).toHaveAttribute('data-stale', 'false')
  await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'ready')
})

test('§5 4: an analysis that finds no input fails, naming it, and coding waits; the input bound in the properties, analyzing again succeeds', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish noinput ${Date.now()}`)
  const wish = await api.read(ALICE, ws.workspace_id, WISH)
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: WISH, keys: [
    { key: 'inputs', value: [], expect: { rev: wish.content.key_revs.inputs } },
    { key: 'prompt', value: '汇总「成本明细」里各项成本的合计 #noinput', expect: { rev: wish.content.key_revs.prompt } }] }])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const failed = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze')
  expect(failed, JSON.stringify(failed.error)).toMatchObject({ ok: false, error: { code: 'INVALID_OPERATION', sub_code: 'NEEDS_INPUT' } })
  expect(failed.error.missing_inputs).toEqual([{ label: '成本明细', message: '工作区里找不到「成本明细」表' }])
  // only the record is written (one commit), not the requirements
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.analysis).toBeUndefined()
  // the card shows the failure on "需求": what is missing, where to bind it; coding waits
  await expect(card).toHaveAttribute('data-tab', 'analysis')
  await expect(card.getByTestId('aiws-wish-status')).toHaveAttribute('data-kind', 'failed')
  await expect(card.getByTestId('aiws-wish-status')).toContainText('上次分析失败')
  await expect(card.getByTestId('aiws-wish-status')).toContainText('成本明细')
  const record = records(page).first()
  await expect(record).toHaveAttribute('data-ok', 'false')
  await expect(record).toContainText('成本明细')
  await expect(record).toContainText('在属性面板里绑定输入后重新分析。')
  await expect(card.getByTestId('aiws-wish-next')).toHaveAttribute('data-action', 'code')
  await expect(card.getByTestId('aiws-wish-next')).toBeDisabled()
  await expect(card.getByTestId('aiws-wish-next-reason')).toHaveText('先分析需求')
  // the input bound in the wish's properties
  await card.getByTestId('aiws-wish-title').click()
  await openProperties(page)
  const props = page.getByTestId('aiws-wish-props')
  await expect(props).toContainText('还没有输入')
  await props.getByTestId('aiws-wish-props-add-input').selectOption('sales')
  await expect(props.getByTestId('aiws-wish-props-input')).toHaveCount(1)
  await expect(props.getByTestId('aiws-wish-props-input')).toContainText('原始销售数据')
  await page.keyboard.press('Escape')
  // analyzing again finds it
  const ok = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze', { tab: 'prompt' })
  expect(ok.ok).toBe(true)
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.analysis.status).toBe('ready')
  await expect(card.getByTestId('aiws-wish-status')).not.toHaveAttribute('data-kind', 'failed')
  await expect(card.getByTestId('aiws-wish-next')).toHaveAttribute('data-action', 'code')
  await expect(card.getByTestId('aiws-wish-next')).toBeEnabled()
  // both records are kept, newest first
  await expect(records(page)).toHaveCount(2)
  await expect(records(page).nth(1)).toHaveAttribute('data-ok', 'false')
})

/** Roll a tab's current record back from the card, going on when the card asks because later changes would be lost
 * (`lost`: what the question must name). Returns the rollback record. */
async function rollback(page: Page, api: Api, ws: { workspace_id: string }, stage: 'analyze' | 'code' | 'run', lost?: string) {
  const before = (await api.history(ALICE, ws.workspace_id, WISH))[stage][0]?.id
  const current = wishCard(page, WISH).locator('[data-testid="aiws-wish-record"][data-current="true"]')
  await current.getByTestId('aiws-wish-rollback').click()
  if (lost) {
    const ask = page.getByTestId('aiws-wish-rollback-confirm')
    await expect(ask).toContainText(lost)
    await page.getByTestId('aiws-wish-rollback-confirm-ok').click()
  }
  let record: Json
  await expect.poll(async () => { record = (await api.history(ALICE, ws.workspace_id, WISH))[stage][0]; return record?.id !== before && record?.action === 'rollback' }).toBe(true)
  return record
}

/** The text of a rich-text result (its paragraphs joined). */
async function richText(api: Api, ws: { workspace_id: string }, entityId: string): Promise<string> {
  const flat = (n: Json): string => (n.text ?? '') + (n.content ?? []).map(flat).join('')
  return flat(await api.ast(ALICE, ws.workspace_id, entityId))
}

test('§5 8/9/10/11: records of every run — the newest result opens on the canvas, an earlier one as it was; an earlier macro as its code was; a run overwrites a manual edit and says so, its rollback brings it back; the macro rolls back and runs', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish history ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  const dialog = page.getByTestId('aiws-wish-dialog')
  await wishThreeSteps(page, api, ALICE, ws.workspace_id, WISH)
  const firstMacro = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.program.digest

  // 9. coding again on the stored macro makes another version; the earlier record opens the code it wrote
  await card.getByTestId('aiws-wish-tab-analysis').click()
  await expect(card.getByTestId('aiws-wish-next')).toHaveText('重新编写')
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'code')
  await expect(card).toHaveAttribute('data-tab', 'macro')
  await expect(records(page)).toHaveCount(2)
  await records(page).first().locator('[data-testid="aiws-wish-record-link"][data-kind="program"]').click()
  await expect(dialog.getByTestId('aiws-wish-code')).toContainText('.sort((a, b) => b["合计"] - a["合计"])')
  await dialog.getByTestId('aiws-wish-dialog-close').click()
  const older = records(page).nth(1)
  await expect(older).toHaveAttribute('data-expanded', 'false')
  await older.getByTestId('aiws-wish-record-toggle').click()
  await expect(older).toHaveAttribute('data-expanded', 'true')
  await older.locator('[data-testid="aiws-wish-record-link"][data-kind="program"]').click()
  await expect(dialog).toContainText('宏代码（当时的版本）')
  await expect(dialog.getByTestId('aiws-wish-code')).toContainText("aiws.result.table('summary'")
  await expect(dialog.getByTestId('aiws-wish-code')).not.toContainText('.sort(')
  await dialog.getByTestId('aiws-wish-dialog-close').click()

  // 11. the new macro rolled back: the earlier one is current again and runs at once
  const rolled = await rollback(page, api, ws, 'code')
  expect(rolled.ok).toBe(true)
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.program.digest).toBe(firstMacro)
  await expect(records(page).first()).toHaveAttribute('data-action', 'rollback')
  await expect(records(page).nth(1)).toHaveAttribute('data-rolled-back', 'true')
  await expect(records(page).nth(1).getByTestId('aiws-wish-record-rolled-back')).toHaveText('已回滚')
  await expect(records(page).nth(2)).toHaveAttribute('data-current', 'true')
  await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'ready')
  await setRevenue(api, ws, 's-1', 300000)
  expect((await wishRun(page, api, ALICE, ws.workspace_id, WISH)).ok).toBe(true)
  const commentaryId = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.last_run.result_bindings.results.commentary.entity_id as string
  const secondText = await richText(api, ws, commentaryId)

  // 8. a third run: "结果(3)", three records; the newest names the result Block on this canvas, an earlier one its version
  await setRevenue(api, ws, 's-1', 400000)
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'run', { tab: 'results' })
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(3)')
  await expect(records(page)).toHaveCount(3)
  const lastRun = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.last_run
  const summaryCell = lastRun.result_bindings.results.summary.cells[0] as string
  const commentary = lastRun.result_bindings.results.commentary.entity_id as string
  await records(page).first().locator('[data-testid="aiws-wish-record-link"][data-kind="result"]', { hasText: '汇总' }).click()
  await expect(page.getByTestId(`aiws-canvas-block-${summaryCell}`)).toHaveClass(/is-selected/)
  // (the camera went to that Block: the whole canvas again, the wish's card with it)
  await fitAll(page)
  const now = await richText(api, ws, commentary)
  const earliest = records(page).nth(2)
  await earliest.getByTestId('aiws-wish-record-toggle').click()
  await earliest.locator('[data-testid="aiws-wish-record-link"][data-kind="result"]', { hasText: '解读' }).click()
  await expect(dialog).toHaveAttribute('data-kind', 'version')
  await expect(dialog).toContainText('解读（当时的版本）')
  await expect(dialog).toContainText('合计')
  await expect(dialog).not.toContainText(now.replace(/^## 解读/, '').trim())
  await dialog.getByTestId('aiws-wish-dialog-close').click()

  // 10. a manual edit of a result is overwritten by the next run, which says so first; its rollback brings the edit back
  const read = await api.read(ALICE, ws.workspace_id, commentary)
  const ast = read.content.content
  const block = ast.content[0].attrs.block_id
  expect((await api.commit(ALICE, ws, [{ op: 'richtext.replace_block', entity_id: commentary, block_id: block, expect: { hash: read.content.blocks[block].hash },
    node: { type: 'heading', attrs: { block_id: block, level: 1 }, content: [{ type: 'text', text: '人工改过的标题' }] } }])).status).toBe('accepted')
  const overwrote = await wishRun(page, api, ALICE, ws.workspace_id, WISH)
  expect(overwrote.ok).toBe(true)
  expect(overwrote.warnings[0]).toMatchObject({ code: 'overwrote_manual' })
  expect(await richText(api, ws, commentary)).not.toContain('人工改过的标题')
  await expect(records(page).first().getByTestId('aiws-wish-record-warnings')).toContainText('人工修改')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(4)')
  await rollback(page, api, ws, 'run')
  expect(await richText(api, ws, commentary)).toContain('人工改过的标题')
  await expect(records(page).nth(1)).toHaveAttribute('data-rolled-back', 'true')
  // the count of runs that made results does not go down
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(4)')
  // rolling the run before back may lose that edit, made after it: the card names it and asks; the result is back to
  // what it was before that run (a rich text is undone block by block: the edited heading was not that run's)
  await rollback(page, api, ws, 'run', '解读：被修改过')
  expect(await richText(api, ws, commentary)).toContain(secondText.replace(/^## 解读|^解读/, ''))

  // 13 (the macro): editing the current macro by hand asks first, results exist; going on, the results are out of date
  await card.getByTestId('aiws-wish-tab-macro').click()
  await card.locator('[data-testid="aiws-wish-record"][data-current="true"]').locator('[data-testid="aiws-wish-record-link"][data-kind="program"]').click()
  await expect(dialog.getByTestId('aiws-wish-program-view')).toHaveAttribute('data-editable', 'true')
  await dialog.getByTestId('aiws-wish-program-start-edit').click()
  await expect(page.getByTestId('aiws-wish-invalidate')).toContainText('该修改会让结果过期。')
  await page.getByTestId('aiws-wish-invalidate-cancel').click()
  await expect(dialog.getByTestId('aiws-wish-program-edit')).toHaveCount(0)
  await dialog.getByTestId('aiws-wish-program-start-edit').click()
  await page.getByTestId('aiws-wish-invalidate-ok').click()
  const editor = dialog.getByTestId('aiws-wish-program-edit')
  await editor.fill(`${await editor.inputValue()}\n// 手工改过\n`)
  await dialog.getByTestId('aiws-wish-program-save').click()
  await expect(editor).toHaveCount(0)
  await dialog.getByTestId('aiws-wish-dialog-close').click()
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveAttribute('data-stale', 'true')
  expect((await api.rpc(ALICE, 'doc.freshness', { workspace_id: ws.workspace_id, entity_ids: [WISH] })).items[0].config_changed).toBe(true)
})

test('§5 7/12: a macro that calls AI shows its run button in the AI colours and its records name the model; a warning-level check that fails is written, warning first; an error-level one fails the run and changes nothing', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish checks ${Date.now()}`)
  await setPrompt(api, ws, '按区域汇总「原始销售数据」的销售额，并写一段解读 #checkwarn #ai')
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze')
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'code')
  // (new numbers: the macro's AI call is not answered from the cache its trial run filled)
  await setRevenue(api, ws, 's-2', 99000)
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'run')
  const payload = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload
  expect(payload.analysis.checks.find((c: Json) => c.id === 'positive').level).toBe('warning')
  // 7. the macro declares AI: the run button is the AI one, its run says what the macro's AI used
  expect(payload.program.manifest.ai).toBeTruthy()
  const run = card.getByTestId('aiws-wish-run')
  await expect(run).toHaveAttribute('data-ai', 'true')
  await expect(run).toHaveClass(/is-ai/)
  const first = (await api.history(ALICE, ws.workspace_id, WISH)).run[0]
  expect(first.usage.calls).toBe(1)
  expect(first.usage.model).toMatch(/@mock-llm$/)
  await expect(records(page).first()).toContainText('@mock-llm')

  // 12. a warning-level check fails: the results are written, the failure heads the record's warnings
  await setRevenue(api, ws, 's-1', -5)
  const warned = await wishRun(page, api, ALICE, ws.workspace_id, WISH)
  expect(warned.ok).toBe(true)
  expect(warned.warnings[0]).toMatchObject({ code: 'check_failed', id: 'positive' })
  expect(warned.checks.find((c: Json) => c.id === 'positive')).toMatchObject({ status: 'failed', level: 'warning' })
  expect(warned.summary.updated).toBe(2)
  await expect(records(page).first().getByTestId('aiws-wish-record-warnings')).toContainText('检查 positive 没通过')

  // the same check at error level (analysed and coded again on data that passes it), then data that fails it: the run
  // fails, nothing of the results changes
  await setRevenue(api, ws, 's-1', 120000)
  await setPrompt(api, ws, '按区域汇总「原始销售数据」的销售额，并写一段解读 #checkerror #ai')
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze', { tab: 'prompt' })
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.analysis.checks.find((c: Json) => c.id === 'positive').level).toBe('error')
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'code', { tab: 'analysis' })
  const before = await derived(api, ws)
  const lastRun = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.last_run.run_id
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await setRevenue(api, ws, 's-1', -5)
  const failed = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'run', { tab: 'macro' })
  expect(failed).toMatchObject({ ok: false, error: { sub_code: 'CHECKS_FAILED' } })
  // only the failed record was written
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  expect((await derived(api, ws)).map((e: Json) => e.content_rev)).toEqual(before.map((e: Json) => e.content_rev))
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.last_run.run_id).toBe(lastRun)
  await expect(card).toHaveAttribute('data-tab', 'results')
  await expect(records(page).first()).toHaveAttribute('data-ok', 'false')
  await expect(card.getByTestId('aiws-wish-status')).toHaveAttribute('data-kind', 'failed')
  await expect(card.getByTestId('aiws-wish-status')).toContainText('上次运行失败')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(2)')
})

test('§5 15: a stage under way is a line of the status bar; the status panel shows its lines and cancels it — no record; a coding that never finishes fails and keeps no macro', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish status ${Date.now()}`)
  await setPrompt(api, ws, '按区域汇总「原始销售数据」的销售额 #slow=2500')
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  await card.getByTestId('aiws-wish-next').click()
  // the collapsed status bar: one line for the one run
  const line = page.getByTestId('aiws-status-run')
  await expect(line).toHaveAttribute('data-wish', WISH, { timeout: 20_000 })
  await expect(line).toHaveAttribute('data-stage', 'analyze')
  await expect(card).toHaveAttribute('data-running', 'analyze')
  await expect(card.getByTestId('aiws-wish-status')).toHaveAttribute('data-kind', 'running')
  await expect(card.getByTestId('aiws-wish-next')).toHaveAttribute('data-action', 'cancel')
  await expect(card.getByTestId('aiws-wish-working')).toBeVisible()
  // the status panel: "运行中" above the work log, a block per run with its lines, and its cancel
  await openStatus(page)
  const block = page.getByTestId('aiws-status-runs').getByTestId('aiws-status-run-block')
  await expect(block).toHaveAttribute('data-wish', WISH)
  await expect(block.getByTestId('aiws-status-run-meta')).toBeVisible()
  await expect(block.locator('[data-testid="aiws-status-run-line"][data-kind="tool"]').first()).toBeVisible({ timeout: 30_000 })
  await block.getByTestId('aiws-status-run-cancel').click()
  await expect(page.getByTestId('aiws-status-runs')).toHaveCount(0, { timeout: 20_000 })
  await expect(line).toHaveCount(0)
  await expect(card).toHaveAttribute('data-running', '')
  await expect.poll(async () => (await api.runs(ALICE, ws.workspace_id, WISH))[0].state).toBe('cancelled')
  // a cancelled run is not recorded
  expect((await api.history(ALICE, ws.workspace_id, WISH)).analyze).toHaveLength(0)
  await expect(card.getByTestId('aiws-wish-tab-analysis')).toBeDisabled()
  await page.getByTestId('aiws-status').click()

  // a coding that gives up: it fails, the macro tab shows why, no macro and no run button
  await setPrompt(api, ws, '按区域汇总「原始销售数据」的销售额 #codefail')
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze', { tab: 'prompt' })
  const failed = await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'code')
  expect(failed.ok).toBe(false)
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.program).toBeUndefined()
  await expect(card).toHaveAttribute('data-tab', 'macro')
  await expect(records(page).first()).toHaveAttribute('data-ok', 'false')
  await expect(card.getByTestId('aiws-wish-status')).toContainText('上次编写宏失败')
  await expect(card.getByTestId('aiws-wish-run')).toHaveCount(0)
  await expect(card.getByTestId('aiws-wish-tab-results')).toBeDisabled()
})

test('§5 16: lines on a free canvas — the input Block to the wish, a ghost for an input not on it (it opens the input), the wish to its result Blocks; they follow a moved Block, take no pointer, are not exported; the properties hide them; rolling the first run back removes its Blocks', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish links ${Date.now()}`)
  const wish = await api.read(ALICE, ws.workspace_id, WISH)
  // a second input that has no Block on this canvas
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'memo', type_id: 'buckyos.richtext', parent_id: 'data', order_key: 'zz', name: '口径说明', payload: { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: 'memo-1' }, content: [{ type: 'text', text: '销售额含税。' }] }] } } },
    { op: 'entity.set_keys', entity_id: WISH, keys: [{ key: 'inputs', value: [...wish.content.payload.inputs, { entity_id: 'memo', version: { mode: 'follow' } }], expect: { rev: wish.content.key_revs.inputs } }] },
  ])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.getByTestId('aiws-wishlinks')).toBeAttached()
  const links = page.getByTestId(`aiws-wishlinks-${BLOCK}`)
  await expect(links).toHaveAttribute('data-wish-id', WISH)
  const input = page.getByTestId(`aiws-wishlink-${BLOCK}-input-blk-sales`)
  await expect(input).toBeAttached()
  await expect(input).toHaveAttribute('data-kind', 'input')
  expect(await input.evaluate((el) => getComputedStyle(el).pointerEvents)).toBe('none')
  const ghost = page.getByTestId(`aiws-wishlink-ghost-${BLOCK}-memo`)
  await expect(ghost).toHaveAttribute('data-state', 'ok')
  // the ghost opens its input
  await ghost.click()
  await expect(page.getByTestId('aiws-detail-memo')).toBeVisible()
  await page.getByTestId('aiws-top-canvas').click()
  await expect(page.getByTestId('aiws-canvas-view')).toBeVisible()
  // three steps (the analysis binds the inputs it uses: the table only, its ghost goes): a line from the wish to each
  // result Block
  await wishThreeSteps(page, api, ALICE, ws.workspace_id, WISH)
  const cells: string[] = (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.last_run.result_bindings.results.summary.cells
  await fitAll(page)
  for (const cell of cells) await expect(page.getByTestId(`aiws-wishlink-${BLOCK}-result-${cell}`)).toHaveAttribute('data-kind', 'result')
  // the input Block moves: its line follows
  const at = await input.evaluate((el) => `${(el as SVGElement).style.left},${(el as SVGElement).style.top}`)
  // (a little: the table stays the input's Block nearest the wish, the KPI card shows the same data further away)
  expect((await api.commit(ALICE, ws, [{ op: 'tree.place', entity_id: 'blk-sales', placement: { x: 40, y: 260, w: 900, h: 420 } }])).status).toBe('accepted')
  await expect.poll(() => input.evaluate((el) => `${(el as SVGElement).style.left},${(el as SVGElement).style.top}`)).not.toBe(at)
  // decorations, not entities: the draw.io export has no edge
  const download = page.waitForEvent('download')
  await mainMenu(page, 'aiws-menu-surface', 'aiws-menu-export-drawio')
  const exported = readFileSync(await (await download).path(), 'utf8')
  expect(exported).not.toContain('edge="1"')
  expect((await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.view_type === 'connector')).toHaveLength(0)
  await expect(ghost).toHaveCount(0)
  // "显示连线" off in the wish Block's properties: its lines and ghosts are gone
  await fitAll(page)
  await wishCard(page, WISH).getByTestId('aiws-wish-title').click()
  await openProperties(page)
  const toggle = page.getByTestId('aiws-wish-field-show_links')
  await expect(toggle).toBeChecked()
  // (the box shows what was written: it changes once the Block has it)
  await toggle.click()
  await expect(toggle).not.toBeChecked()
  await expect(links).toHaveCount(0)
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, BLOCK)).content.payload.config?.show_links).toBe(false)
  await toggle.click()
  await expect(toggle).toBeChecked()
  await expect(links).toBeAttached()
  await page.keyboard.press('Escape')
  // the run that created the results rolled back: its results and their Blocks go, `last_run` with them
  const rolled = await rollback(page, api, ws, 'run')
  expect(rolled.ok).toBe(true)
  await expect.poll(async () => (await derived(api, ws)).length).toBe(0)
  for (const cell of cells) await expect(page.getByTestId(`aiws-canvas-block-${cell}`)).toHaveCount(0)
  for (const cell of cells) await expect(page.getByTestId(`aiws-wishlink-${BLOCK}-result-${cell}`)).toHaveCount(0)
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.last_run).toBeUndefined()
  await expect(wishCard(page, WISH).getByTestId('aiws-wish-tab-results')).toHaveText('结果(1)')
})

test('§5 17: a clone carries the records; a member who may only read sees them but not the logs, and has no next step, run or rollback; a template made from the canvas has no history', async ({ page, api, browser }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish clone ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await wishThreeSteps(page, api, ALICE, ws.workspace_id, WISH)
  const history = await api.history(ALICE, ws.workspace_id, WISH)
  const cloned = await api.rpc(ALICE, 'ws.clone', { workspace_id: ws.workspace_id })
  const copy = await api.history(ALICE, cloned.workspace_id, WISH)
  expect(copy.results_count).toBe(1)
  for (const stage of ['analyze', 'code', 'run'] as const) expect(copy[stage].map((r: Json) => r.id)).toEqual(history[stage].map((r: Json) => r.id))
  // bob may only read the clone
  await api.rpc(ALICE, 'acl.set', { workspace_id: cloned.workspace_id, changes: [{ subject: 'bob', allow: ['read'] }] })
  const other = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const reader = await other.newPage()
  await openCanvas(reader, BOB, cloned.workspace_id)
  const card = wishCard(reader, WISH)
  await expect(card).toHaveAttribute('data-tab', 'results')
  await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(1)')
  await expect(card.getByTestId('aiws-wish-next')).toHaveCount(0)
  await expect(card.getByTestId('aiws-wish-rollback')).toHaveCount(0)
  await expect(card.getByTestId('aiws-wish-run')).toHaveAttribute('data-state', 'disabled')
  const run = records(reader).first()
  await expect(run).toContainText('新建 2 项')
  const log = run.locator('[data-testid="aiws-wish-record-link"][data-kind="log"]')
  await expect(log).toHaveAttribute('data-gone', 'true')
  await expect(log).toHaveAttribute('title', '要有许愿格的编辑权限才能看日志')
  // the request a record names opens for a reader
  await card.getByTestId('aiws-wish-tab-macro').click()
  await records(reader).first().locator('[data-testid="aiws-wish-record-link"][data-kind="analysis"]').click()
  await expect(reader.getByTestId('aiws-wish-dialog').getByTestId('aiws-wish-contract')).toContainText('summary')
  await other.close()
  // a template of the canvas: the wish comes without its history
  const pkg = `wish-tpl-${Date.now()}`
  const installed = await api.rpc(ALICE, 'ext.install', { from: { workspace_id: ws.workspace_id, templates: [{ surfaces: ['sf-analysis'], data: ['sales', 'intro', 'def-kpi'], template: { id: 'wish-tpl', title: '许愿格模板' } }] }, package: { name: pkg, version: '1.0.0' }, mode: 'dev' })
  expect(installed.ok, JSON.stringify(installed)).toBe(true)
  const made = await api.rpc(ALICE, 'ext.create_from_template', { pkg_id: installed.pkg_id, template_id: 'wish-tpl', title: `from template ${Date.now()}` })
  expect(made.ok, JSON.stringify(made)).toBe(true)
  const wishes = (await api.outline(ALICE, made.workspace_id)).filter((e: Json) => e.type_id === 'buckyos.wish')
  expect(wishes).toHaveLength(1)
  const fromTemplate = (await api.read(ALICE, made.workspace_id, wishes[0].entity_id)).content.payload
  expect(fromTemplate.prompt).toBeTruthy()
  expect(fromTemplate.history).toBeUndefined()
})

test('export and import carry Surfaces, Blocks, definitions, wishes with their records, and dependency records; staleness survives import', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish4 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await wishThreeSteps(page, api, ALICE, ws.workspace_id, WISH)
  await setRevenue(api, ws, 's-1', 1)
  const exported = await api.rpc(ALICE, 'doc.export', { workspace_id: ws.workspace_id, mode: 'share', self_contained: true })
  const imported = await api.importPackage(ALICE, await api.download(exported.download), 'new')
  expect(imported.ok).toBe(true)
  const outline = await api.outline(ALICE, imported.workspace_id)
  const ids = new Set(outline.map((e: { entity_id: string }) => e.entity_id))
  for (const id of ['sf-analysis', 'sf-analysis-content', 'sf-detail', 'blk-sales', 'blk-kpi', 'def-kpi', WISH, 'wish-analysis-out']) expect(ids.has(id), id).toBe(true)
  const results = outline.filter((e: { derived?: { wish_id: string } }) => e.derived?.wish_id === WISH)
  expect(results).toHaveLength(2)
  expect(outline.find((e: { entity_id: string }) => e.entity_id === 'blk-kpi').placement).toEqual({ x: 980, y: 40, w: 420, h: 130 })
  const fresh = await api.rpc(ALICE, 'doc.freshness', { workspace_id: imported.workspace_id, entity_ids: results.map((r: { entity_id: string }) => r.entity_id) })
  expect(fresh.items.every((i: { status: string }) => i.status === 'stale')).toBe(true)
  // the records travel with the wish, and their objects can be read there
  const history = await api.history(ALICE, imported.workspace_id, WISH)
  expect(history.results_count).toBe(1)
  expect(history.code[0].objects.source).toBeTruthy()
  // the imported workspace opens on the canvas with the result group in place and the results stale
  await openCanvas(page, ALICE, imported.workspace_id)
  await expect(wishCard(page, WISH).getByTestId('aiws-wish-results-stale')).toBeVisible()
  await records(page).first().locator('[data-testid="aiws-wish-record-link"][data-kind="log"]').click()
  await expect(page.getByTestId('aiws-wish-dialog')).toHaveAttribute('data-kind', 'log')
  await expect(page.getByTestId('aiws-wish-dialog-error')).toHaveCount(0)
  await page.getByTestId('aiws-wish-dialog-close').click()
  await fitAll(page)
  await expect(page.getByTestId('aiws-canvas-block-wish-analysis-out')).toBeVisible()
})

test('the output mode in the properties: "新建一组" makes a second group beside the first, the first untouched', async ({ page, api }) => {
  test.slow()
  const ws = await api.demo(ALICE, 'quarterly', `wish new-group ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await wishThreeSteps(page, api, ALICE, ws.workspace_id, WISH)
  const first = await derived(api, ws)
  await wishCard(page, WISH).getByTestId('aiws-wish-title').click()
  await openProperties(page)
  await page.getByTestId('aiws-wish-props-output-mode').selectOption('new')
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, WISH)).content.payload.output_mode).toBe('new')
  await page.keyboard.press('Escape')
  const again = await wishRun(page, api, ALICE, ws.workspace_id, WISH)
  expect(again.summary).toMatchObject({ created: 2 })
  const groups = (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.kind === 'group' && e.parent_id === 'sf-analysis')
  expect(groups.length).toBe(2)
  const now = await derived(api, ws)
  expect(now).toHaveLength(4)
  for (const r of first) expect(now.find((e: Json) => e.entity_id === r.entity_id).content_rev).toBe(r.content_rev)
})

test.describe('phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('§5 20: the sample\'s "任务汇总" on a phone\'s flow page — the card in the column, no lines; analyze, code and run by taps; its records reach the results; dialogs fill the screen', async ({ page, api }) => {
    test.slow()
    const ws = await api.sampleWithWish(ALICE, `wish phone ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id, 'surface-main')
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
    const wishId = 'wish-task-summary'
    const card = wishCard(page, wishId)
    await card.scrollIntoViewIfNeeded()
    await expect(card).toHaveAttribute('data-tab', 'prompt')
    const viewport = page.viewportSize()!
    const box = (await card.boundingBox())!
    expect(box.x).toBeGreaterThanOrEqual(0)
    expect(box.x + box.width).toBeLessThanOrEqual(viewport.width)
    // the tabs one row, the next step below them
    const tabs = (await card.locator('.aiws-wc-tabs').boundingBox())!
    const next = (await card.getByTestId('aiws-wish-next').boundingBox())!
    expect(next.y).toBeGreaterThanOrEqual(tabs.y + tabs.height - 1)
    await expect(page.getByTestId('aiws-wishlinks')).toHaveCount(0)
    // the three steps by taps
    for (const [action, stage, tab] of [['analyze', 'analyze', 'analysis'], ['code', 'code', 'macro'], ['run', 'run', 'results']] as const) {
      const before = (await api.history(ALICE, ws.workspace_id, wishId))[stage][0]?.id
      await expect(card.getByTestId('aiws-wish-next')).toHaveAttribute('data-action', action)
      await card.getByTestId('aiws-wish-next').tap()
      const record = await wishStageEnded(page, api, ALICE, ws.workspace_id, wishId, stage, before)
      expect(record.ok, JSON.stringify(record.error)).toBe(true)
      await expect(card).toHaveAttribute('data-tab', tab)
    }
    await expect(card.getByTestId('aiws-wish-tab-results')).toHaveText('结果(1)')
    const lastRun = (await api.read(ALICE, ws.workspace_id, wishId)).content.payload.last_run
    const summaryCells: string[] = lastRun.result_bindings.results.summary.cells
    await expect(page.getByTestId(`aiws-cell-frame-${summaryCells[0]}`)).toBeAttached()
    // the record's result reaches its Block on the page
    await records(page, wishId).first().locator('[data-testid="aiws-wish-record-link"][data-kind="result"]').first().tap()
    await expect(page.getByTestId(`aiws-cell-frame-${summaryCells[0]}`)).toBeInViewport()
    // a record's window fills the screen
    await card.scrollIntoViewIfNeeded()
    await card.getByTestId('aiws-wish-tab-macro').tap()
    await records(page, wishId).first().locator('[data-testid="aiws-wish-record-link"][data-kind="program"]').tap()
    const dialog = page.getByTestId('aiws-wish-dialog')
    await expect(dialog.getByTestId('aiws-wish-code')).toBeVisible()
    const shown = (await dialog.boundingBox())!
    expect(shown.width).toBeGreaterThanOrEqual(viewport.width - 2)
    expect(shown.height).toBeGreaterThanOrEqual(viewport.height - 2)
    await dialog.getByTestId('aiws-wish-dialog-close').tap()
    await expect(dialog).toHaveCount(0)
  })
})

test('UI19 an HTML extension Block reads data, submits through the store (undoable) and uploads; a crash falls back locally', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `html ${Date.now()}`)
  const js = `
    aiws.on('run', async function (payload) {
      var rows = await aiws.query('SELECT _id FROM "' + aiws.context.source.entity_id + '" ORDER BY _id LIMIT 5');
      if (payload && payload.crash) { setTimeout(function () { throw new Error('boom from extension'); }, 0); return 'crashing'; }
      var up = await aiws.upload('<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>', 'tiny.svg', 'image/svg+xml');
      var r = await aiws.submit([{ op: 'entity.create', entity_id: 'ext-note-' + Date.now().toString(36), type_id: 'buckyos.annotation', parent_id: 'sf-analysis-content', order_key: 'zz', payload: { kind: 'note', body: '扩展写入 ' + rows.rows.length + ' 行' } }], '扩展便签');
      return { rows: rows.rows.length, upload: up.object_id, outcome: r.status };
    });
    window.__run = function (p) { return new Promise(function (resolve) { document.getElementById('out').textContent = 'running'; aiws.on('ping', function () { return 1; }); resolve(); }); };
    document.getElementById('out').textContent = 'ready';
    aiws.ready();
  `
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'def-ext', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'zx', payload: { def_id: 'test.ext', kind: 'html', title: '测试扩展', html: { html: '<div id="out">init</div>', js, api_version: 6 } } },
    { op: 'entity.create', entity_id: 'blk-ext', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zzx', placement: { x: 40, y: 720, w: 400, h: 200 }, payload: { view: { type: 'html' }, source_ref: { entity_id: 'sales' }, def_ref: { entity_id: 'def-ext' }, title: '扩展' } },
  ])
  expect(r.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  // static until activated
  await expect(page.getByTestId('aiws-html-static-blk-ext')).toBeVisible()
  await page.getByTestId('aiws-canvas-block-blk-ext').click()
  await page.getByTestId('aiws-near-run').click()
  await expect(page.getByTestId('aiws-html-active-blk-ext')).toBeVisible()
  const frame = page.frameLocator('.aiws-html-frame')
  await expect(frame.locator('#out')).toHaveText('ready')
  // drive the extension's handler from the test through the host bridge (the same path a wish executor uses)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const result = await page.evaluate(async () => {
    const iframe = document.querySelector('.aiws-html-frame') as HTMLIFrameElement
    const win = iframe.contentWindow as unknown as { aiws: { context: { source: { entity_id: string } }; query: (sql: string, params?: unknown[]) => Promise<{ rows: unknown[] }>; upload: (d: string, n: string, t: string) => Promise<{ object_id: string }>; submit: (ops: unknown[], label: string) => Promise<{ status: string }> } }
    const rows = await win.aiws.query('SELECT _id FROM "' + win.aiws.context.source.entity_id + '" ORDER BY _id LIMIT ?', [5])
    const up = await win.aiws.upload('<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>', 'tiny.svg', 'image/svg+xml')
    const outcome = await win.aiws.submit([{ op: 'entity.create', entity_id: 'ext-note-1', type_id: 'buckyos.annotation', parent_id: 'sf-analysis-content', order_key: 'zz', payload: { kind: 'note', body: `扩展写入 ${rows.rows.length} 行` } }], '扩展便签')
    return { rows: rows.rows.length, upload: up.object_id, outcome: outcome.status }
  })
  expect(result.rows).toBe(5)
  expect(result.upload).toMatch(/^fobj:|^jobj:|:/)
  expect(result.outcome).toBe('accepted')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  await expect(page.getByTestId('aiws-undo')).toHaveText(/撤销 1/)
  expect((await api.read(ALICE, ws.workspace_id, 'ext-note-1')).content.payload.body).toBe('扩展写入 5 行')
  // undo takes the extension's write back
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).some((e: { entity_id: string }) => e.entity_id === 'ext-note-1')).toBe(false)
  // a crash inside the extension: this Block falls back, the others keep working
  await page.evaluate(() => { const iframe = document.querySelector('.aiws-html-frame') as HTMLIFrameElement; (iframe.contentWindow as unknown as { eval: (s: string) => void }).eval('setTimeout(function(){ throw new Error("boom from extension") }, 0)') })
  await expect(page.getByTestId('aiws-html-failed-blk-ext')).toBeVisible()
  await expect(page.getByTestId('aiws-html-failed-blk-ext')).toContainText('boom')
  await expect(page.getByTestId('aiws-canvas-block-blk-sales')).toBeVisible()
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  await expect(page.getByTestId('aiws-near-toolbar')).toBeVisible()
})

test('aiws v2: an HTML Block reads by binding name, writes typed values in one batch (one undo step), and watches its data', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `aiws2 ${Date.now()}`)
  const js = `document.getElementById('out').textContent = 'ready ' + aiws.version + ' ' + Object.keys(aiws.bindings).join(','); window.__seen = 0; aiws.watch('orders', function () { window.__seen++; }); aiws.ready();`
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'def-v2', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'zx', payload: { def_id: 'test.v2', kind: 'html', title: 'v2', allow_no_source: true, html: { html: '<div id="out">init</div>', js, api_version: 2 } } },
    { op: 'entity.create', entity_id: 'blk-v2', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zzx', placement: { x: 40, y: 720, w: 400, h: 200 }, payload: { view: { type: 'html' }, def_ref: { entity_id: 'def-v2' }, bindings: { orders: { entity_id: 'sales' }, intro: { entity_id: 'intro' } }, title: 'v2' } },
  ])
  expect(r.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-v2').click()
  await page.getByTestId('aiws-near-run').click()
  const frame = page.frameLocator('.aiws-html-frame')
  await expect(frame.locator('#out')).toHaveText('ready 2 intro,orders')
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const result = await page.evaluate(async () => {
    const iframe = document.querySelector('.aiws-html-frame') as HTMLIFrameElement
    type Aiws = { input: (n: string) => { rows: (o?: unknown) => Promise<Record<string, unknown>[]>; fields: () => Promise<{ name: string }[]>; markdown: () => Promise<string> }; batch: (fn: () => Promise<void>, label?: string) => Promise<{ status: string }>; table: (n: string) => { upsert: (rows: unknown[], o: unknown) => Promise<unknown>; setColumn: (f: string, v: Record<string, unknown>) => Promise<unknown> } }
    const aiws = (iframe.contentWindow as unknown as { aiws: Aiws }).aiws
    const rows = await aiws.input('orders').rows({ fields: ['区域', '产品', '销售额'], filter: { 区域: '华东' } })
    const fields = await aiws.input('orders').fields()
    const md = await aiws.input('intro').markdown()
    const outcome = await aiws.batch(async () => {
      await aiws.table('orders').upsert([{ 区域: '华东', 产品: '智能手表', 销售额: 130000 }, { 区域: '东北', 产品: '智能手表', 销售额: 1000 }], { key: ['区域', '产品'] })
      await aiws.table('orders').setColumn('目标', { 's-2': 99000 })
    }, '批量写入')
    return { rows: rows.length, first: rows[0], fields: fields.map((f) => f.name), md, status: outcome.status }
  })
  expect(result.rows).toBe(3)
  expect(result.first).toHaveProperty('_id')
  expect(result.fields).toContain('销售额')
  expect(result.md).toContain('# 2026 Q2 经营分析')
  expect(result.status).toBe('accepted')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'revenue')).value).toBe(130000)
  expect((await api.cell(ALICE, ws.workspace_id, 'sales', 's-2', 'target')).value).toBe(99000)
  await expect(page.getByTestId('aiws-undo')).toHaveText(/撤销 1/)
  // the binding is watched: a change to the data reaches the Block
  const frameDoc = await (await page.locator('.aiws-html-frame').elementHandle())?.contentFrame()
  await expect.poll(() => frameDoc?.evaluate(() => (window as unknown as { __seen?: number }).__seen ?? 0)).toBeGreaterThan(0)
})

test('内核架构改进 T3-4: the local executor — this browser hosts the model loop and runs the macro in a Worker; the service writes each stage; a reload leaves the run interrupted and this browser resumes it', async ({ page, api }) => {
  test.slow()
  const llm = scriptedModel().baseUrl
  const ws = await api.demo(ALICE, 'quarterly', `local ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'analyze')
  // coding: the macro's trial run is in the Worker on the fetched snapshot
  await wishStep(page, api, ALICE, ws.workspace_id, WISH, 'code')
  const runs = await api.runs(ALICE, ws.workspace_id, WISH)
  expect((runs[0].execution as { mode: string }).mode).toBe('local')
  expect(((runs[0].context as { executor: { kind: string } }).executor).kind).toBe('browser')
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.program.api_version).toBe(3)
  // the executor dies with the page while the model is slow: the run's lease lapses (interrupted, AIWS_RUN_LEASE_MS);
  // this browser still holds the claim and resumes the run as a new attempt. The model answers the attempt's first turn
  // only: what it used is kept beside the claim and counted with the second attempt, which starts its loop again
  // (余下任务 §7.1 4: the service opens the new attempt's stage afresh)
  const before = (await api.history(ALICE, ws.workspace_id, WISH)).code[0].id
  let calls = 0
  let second = 0
  await page.route(`${llm}/chat/completions`, (route) => {
    calls += 1
    if (calls === 1) return route.continue()
    if (calls > 2) { second += 1; return route.continue() }
    /* the model never answers the second turn */
  })
  await card.getByTestId('aiws-wish-tab-analysis').click()
  await card.getByTestId('aiws-wish-next').click()
  await expect(card).toHaveAttribute('data-running', 'code')
  await expect.poll(async () => (await api.runs(ALICE, ws.workspace_id, WISH))[0].state).toBe('running')
  await expect.poll(() => calls).toBe(2)
  await openCanvas(page, ALICE, ws.workspace_id)
  // the lease lapses without the browser's heartbeats; the service notices on the next read
  await expect.poll(async () => { const latest = (await api.runs(ALICE, ws.workspace_id, WISH))[0]; await api.rpc(ALICE, 'proc.get', { workspace_id: ws.workspace_id, run_id: latest.run_id }); return latest.state }, { timeout: 30_000 }).toBe('interrupted')
  const resume = card.getByTestId('aiws-wish-next')
  await expect(resume).toHaveAttribute('data-action', 'resume', { timeout: 30_000 })
  await expect(resume).toHaveText('继续')
  await resume.click()
  const resumed = await wishStageEnded(page, api, ALICE, ws.workspace_id, WISH, 'code', before)
  expect(resumed.ok).toBe(true)
  expect(((await api.runs(ALICE, ws.workspace_id, WISH))[0].execution as { attempt: number }).attempt).toBe(2)
  expect(second).toBeGreaterThan(1)
  expect(resumed.usage.calls).toBe(second + 1)
  await page.unroute(`${llm}/chat/completions`)
})

test('余下任务 B2-1: an analysis that needs data from outside fails NEEDS_EXTERNAL_DATA; the card says a browser cannot create a data source and opens the request', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `external ${Date.now()}`)
  await setPrompt(api, ws, '按收盘价算每只股票的周涨跌 #external')
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  await card.getByTestId('aiws-wish-next').click()
  const failed = await wishStageEnded(page, api, ALICE, ws.workspace_id, WISH, 'analyze', undefined)
  expect(failed, JSON.stringify(failed.error)).toMatchObject({ ok: false, error: { sub_code: 'NEEDS_EXTERNAL_DATA' } })
  expect(failed.error.missing_inputs[0].wanted.meaning).toBe('Daily closing prices of the tracked stocks')
  await expect(card.getByTestId('aiws-wish-status')).toContainText('工作区以外的数据：Daily closing prices of the tracked stocks')
  await expect(card.getByTestId('aiws-wish-status')).toContainText('浏览器里不能新建数据源')
  // alice owns the workspace: the request goes to her Agent
  await card.getByTestId('aiws-wish-request-source').click()
  await expect(page.getByTestId('aiws-source-request-meaning')).toHaveValue('Daily closing prices of the tracked stocks')
  await expect(page.getByTestId('aiws-source-request-to')).toHaveAttribute('data-to', 'agent')
})

test('余下任务 A5: the stage loop runs on xllm — the iterations used up fail the stage as EXECUTOR_FAILED naming the limit; no analysis is written', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `iterations ${Date.now()}`)
  // the deployment's limit comes with the claim: one tool round for this test's runs
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postDataJSON() as { method?: string } | null
    if (body?.method !== 'proc.claim') return route.fallback()
    const response = await route.fetch()
    const json = await response.json()
    const stage = json?.result?.stage
    if (stage) { stage.iterations = 1; if (stage.budget) stage.budget.iterations = 1 }
    return route.fulfill({ response, json })
  })
  await openCanvas(page, ALICE, ws.workspace_id)
  const card = wishCard(page, WISH)
  await card.getByTestId('aiws-wish-next').click()
  const failed = await wishStageEnded(page, api, ALICE, ws.workspace_id, WISH, 'analyze', undefined)
  expect(failed.ok).toBe(false)
  expect(JSON.stringify(failed.error)).toContain('EXECUTOR_FAILED')
  expect(JSON.stringify(failed.error)).toContain('tool iteration limit (1) reached')
  expect((await api.read(ALICE, ws.workspace_id, WISH)).content.payload.analysis ?? null).toBeNull()
  await expect(card.getByTestId('aiws-wish-status')).toHaveAttribute('data-kind', 'failed')
})
