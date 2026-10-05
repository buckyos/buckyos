import type { Page } from '@playwright/test'
import { expect, expectAllCommitted, openWorkspace, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

function block(page: Page, blockId: string) {
  return page.getByTestId('aiws-richtext-notes').locator(`[data-block-id="${blockId}"]`)
}

async function caretToEnd(page: Page, blockId: string) {
  await block(page, blockId).click()
  await page.keyboard.press('End')
}

/** IME-style input: a composition that is updated a few times and then committed (CDP, like a real IME). */
async function compose(page: Page, steps: string[], committed: string) {
  const cdp = await page.context().newCDPSession(page)
  for (const text of steps) {
    await cdp.send('Input.imeSetComposition', { text, selectionStart: text.length, selectionEnd: text.length })
    await page.waitForTimeout(60)
  }
  await cdp.send('Input.insertText', { text: committed })
  await cdp.detach()
}

async function editorJson(page: Page, entityId: string) {
  return page.evaluate((id) => {
    const hooks = window.__aiwsTestHooks
    if (!hooks || !hooks.editors[id] || !hooks.canonicalize) throw new Error('test hooks missing')
    return hooks.canonicalize(hooks.editors[id]())
  }, entityId)
}

async function expectEditorMatchesBackend(page: Page, api: Api, token: string, workspaceId: string, entityId: string) {
  await expectAllCommitted(page)
  await expect.poll(async () => JSON.stringify(await editorJson(page, entityId)) === JSON.stringify(await api.ast(token, workspaceId, entityId)), { timeout: 15_000 }).toBe(true)
  expect(await editorJson(page, entityId)).toEqual(await api.ast(token, workspaceId, entityId))
}

declare global {
  interface Window {
    __aiwsTestHooks?: {
      editors: Record<string, () => unknown>
      canonicalize?: (ast: unknown) => unknown
      replica?: { failTransactions(kind: 'quota' | 'error', count: number): Promise<void>; killWorker(): void }
    }
  }
}

test('(c) Chinese IME input into the rich text survives a reload; backend AST equals the editor document', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `c ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await expect(block(page, 'n-end')).toContainText('以上为未完成任务的实时视图。')
  // opening a document creates no commit
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(6)

  await caretToEnd(page, 'n-end')
  await compose(page, ['z', 'zh', 'zho', 'zhon', 'zhong', 'zhong w', 'zhong we', 'zhong wen'], '中文')
  await compose(page, ['s', 'sh', 'shu', 'shu r', 'shu ru'], '输入')
  await page.keyboard.type('测试 OK 😀')
  await expect(block(page, 'n-end')).toHaveText('以上为未完成任务的实时视图。中文输入测试 OK 😀')
  await expectEditorMatchesBackend(page, api, ALICE, ws.workspace_id, 'notes')
  const ast = await api.ast(ALICE, ws.workspace_id, 'notes')
  expect(JSON.stringify(ast)).toContain('以上为未完成任务的实时视图。中文输入测试 OK 😀')
  // no pinyin left behind by the composition
  expect(JSON.stringify(ast)).not.toMatch(/zhong|shu ru/)

  // Enter splits the block: the first half keeps its id, the second half gets a new valid one
  await page.keyboard.press('Enter')
  await page.keyboard.type('新的一段')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('换行后')
  await expectEditorMatchesBackend(page, api, ALICE, ws.workspace_id, 'notes')
  const after = await api.ast(ALICE, ws.workspace_id, 'notes')
  const ids = (after.content as { attrs: { block_id: string } }[]).map((node) => node.attrs.block_id)
  expect(new Set(ids).size).toBe(ids.length)
  expect(ids.slice(0, 5)).toEqual(['n-title', 'n-intro', 'n-list', 'n-embed', 'n-end'])
  expect(ids[5]).toMatch(/^[a-z0-9][a-z0-9_-]{0,63}$/)
  expect(JSON.stringify(after.content[5])).toContain('hard_break')

  // marks: bold through the keyboard shortcut
  await page.keyboard.press('Enter')
  await page.keyboard.press('Control+b')
  await page.keyboard.type('加粗')
  await page.keyboard.press('Control+b')
  await page.keyboard.type('常规')
  await expectEditorMatchesBackend(page, api, ALICE, ws.workspace_id, 'notes')
  const last = (await api.ast(ALICE, ws.workspace_id, 'notes')).content.at(-1)
  expect(last.content).toEqual([{ type: 'text', marks: [{ type: 'strong' }], text: '加粗' }, { type: 'text', text: '常规' }])

  await page.reload()
  await page.getByTestId('desktop-app-aiworkspace').click()
  await page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${ws.workspace_id}"]`).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-richtext-notes')).toContainText('中文输入测试 OK 😀')
  await expect(page.getByTestId('aiws-richtext-notes')).toContainText('新的一段')
  await expectEditorMatchesBackend(page, api, ALICE, ws.workspace_id, 'notes')
})

test('(d) two users: rich text converges, different fields both succeed, same field shows a conflict and keeps my input', async ({ browser, api }) => {
  const ws = await api.sample(ALICE, `d ${Date.now()}`)
  expect((await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update', 'append', 'comment'] })).ok).toBe(true)
  const aliceContext = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const bobContext = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const alice = await aliceContext.newPage()
  const bob = await bobContext.newPage()
  await openWorkspace(alice, ALICE, ws.workspace_id)
  await openWorkspace(bob, BOB, ws.workspace_id)
  await expect(bob.getByTestId('aiws-principal')).toHaveText('bob')

  // ---- rich text: both type at the same time, into the same paragraph and into different blocks
  await caretToEnd(alice, 'n-end')
  await caretToEnd(bob, 'n-title')
  await Promise.all([alice.keyboard.type('【甲的补充】', { delay: 25 }), bob.keyboard.type('（乙改标题）', { delay: 25 })])
  await caretToEnd(bob, 'n-end')
  await Promise.all([alice.keyboard.type('甲甲甲', { delay: 30 }), bob.keyboard.type('乙乙乙', { delay: 30 })])
  for (const page of [alice, bob]) {
    await expect(block(page, 'n-title')).toContainText('（乙改标题）')
    await expect(block(page, 'n-end')).toContainText('【甲的补充】')
    await expect(block(page, 'n-end')).toContainText('甲甲甲')
    await expect(block(page, 'n-end')).toContainText('乙乙乙')
  }
  await expectEditorMatchesBackend(alice, api, ALICE, ws.workspace_id, 'notes')
  await expectEditorMatchesBackend(bob, api, BOB, ws.workspace_id, 'notes')
  expect(await editorJson(alice, 'notes')).toEqual(await editorJson(bob, 'notes'))

  // ---- one record, different fields, edits open at the same time: both succeed
  const aliceAll = alice.getByTestId('aiws-table-cell-all-tasks')
  const bobAll = bob.getByTestId('aiws-table-cell-all-tasks')
  await aliceAll.getByTestId('aiws-cell-task-42-owner').getByRole('button').first().click()
  await bobAll.getByTestId('aiws-cell-task-42-budget').getByRole('button').first().click()
  await alice.getByLabel('负责人 task-42', { exact: true }).fill('甲')
  await bob.getByLabel('预算 task-42', { exact: true }).fill('1888')
  await alice.getByLabel('负责人 task-42', { exact: true }).press('Enter')
  await bob.getByLabel('预算 task-42', { exact: true }).press('Enter')
  await expectAllCommitted(alice)
  await expectAllCommitted(bob)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-42', 'owner')).value).toBe('甲')
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-42', 'budget')).value).toBe('1888.00')
  await expect(bobAll.getByTestId('aiws-cell-task-42-owner')).toContainText('甲')
  await expect(aliceAll.getByTestId('aiws-cell-task-42-budget')).toContainText('1888.00')

  // ---- same cell: both start from the same value; the second to commit gets a conflict
  await aliceAll.getByTestId('aiws-cell-task-41-title').getByRole('button').first().click()
  await bobAll.getByTestId('aiws-cell-task-41-title').getByRole('button').first().click()
  await alice.getByLabel('任务 task-41', { exact: true }).fill('甲的标题')
  await bob.getByLabel('任务 task-41', { exact: true }).fill('乙的标题')
  await alice.getByLabel('任务 task-41', { exact: true }).press('Enter')
  await expectAllCommitted(alice)
  // bob's other view already shows alice's value, and he is still typing in the first one
  await expect(bob.getByTestId('aiws-cell-frame-cell-open-tasks').getByTestId('aiws-cell-task-41-title')).toContainText('甲的标题')
  await expect(bob.getByLabel('任务 task-41', { exact: true })).toHaveValue('乙的标题')
  await bob.getByLabel('任务 task-41', { exact: true }).press('Enter')
  const conflicted = bobAll.getByTestId('aiws-cell-task-41-title')
  await expect(conflicted).toHaveAttribute('data-state', 'needs_attention')
  await expect(conflicted.getByTestId('aiws-edit-state')).toHaveText('需要处理')
  await expect(conflicted.getByTestId('aiws-conflict-theirs')).toHaveText('甲的标题')
  await expect(conflicted.getByTestId('aiws-conflict-mine')).toHaveText('乙的标题')
  await expect(bob.getByTestId('aiws-save-summary')).toHaveAttribute('data-attention', '1')
  // nothing was overwritten, and bob's input is still there after more changes arrive
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'title')).value).toBe('甲的标题')
  await aliceAll.getByTestId('aiws-cell-task-40-owner').getByRole('button').first().click()
  await alice.getByLabel('负责人 task-40', { exact: true }).fill('丙')
  await alice.getByLabel('负责人 task-40', { exact: true }).press('Enter')
  await expect(bobAll.getByTestId('aiws-cell-task-40-owner')).toContainText('丙')
  await expect(conflicted.getByTestId('aiws-conflict-mine')).toHaveText('乙的标题')
  // bob decides: his input overwrites, now based on the value he has seen
  await conflicted.getByTestId('aiws-conflict-keep-mine').click()
  await expectAllCommitted(bob)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'title')).value).toBe('乙的标题')
  await expect(aliceAll.getByTestId('aiws-cell-task-41-title')).toContainText('乙的标题')

  // bob has no `structure` capability: the UI does not offer field management, and the API refuses it
  await expect(bob.getByTestId('aiws-fields-cell-all-tasks')).toHaveCount(0)
  await expect(alice.getByTestId('aiws-fields-cell-all-tasks')).toBeVisible()
  await aliceContext.close()
  await bobContext.close()
})

test('(e) Ctrl+Z undoes exactly one step: an editor step, then a cell commit, in stack order', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `e ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const undoButton = page.getByTestId('aiws-undo')
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(undoButton).toHaveText('撤销 0')

  // two editor steps (typing separated by a pause)
  await caretToEnd(page, 'n-end')
  await page.keyboard.type('第一步')
  await expect(undoButton).toHaveText('撤销 1')
  await page.waitForTimeout(900)
  await page.keyboard.type('第二步')
  await expect(undoButton).toHaveText('撤销 2')
  await expectAllCommitted(page)
  // two cell commits
  await all.getByTestId('aiws-cell-task-40-owner').getByRole('button').first().click()
  await page.getByLabel('负责人 task-40', { exact: true }).fill('撤销甲')
  await page.getByLabel('负责人 task-40', { exact: true }).press('Enter')
  await expect(undoButton).toHaveText('撤销 3')
  await all.getByTestId('aiws-cell-task-41-owner').getByRole('button').first().click()
  await page.getByLabel('负责人 task-41', { exact: true }).fill('撤销乙')
  await page.getByLabel('负责人 task-41', { exact: true }).press('Enter')
  await expect(undoButton).toHaveText('撤销 4')
  await expectAllCommitted(page)
  const owner = async (record: string) => (await api.cell(ALICE, ws.workspace_id, 'tasks', record, 'owner')).value

  // 1st Ctrl+Z (focus is on the table): only the last cell commit is compensated
  await page.keyboard.press('Control+z')
  await expect(undoButton).toHaveText('撤销 3')
  await expect.poll(() => owner('task-41')).toBe('王')
  expect(await owner('task-40')).toBe('撤销甲')
  await expect(block(page, 'n-end')).toContainText('第一步第二步')
  await expect(all.getByTestId('aiws-cell-task-41-owner')).toContainText('王')

  // 2nd: the other cell commit
  await page.keyboard.press('Control+z')
  await expect(undoButton).toHaveText('撤销 2')
  await expect.poll(() => owner('task-40')).toBe('林')
  await expect(block(page, 'n-end')).toContainText('第一步第二步')

  // 3rd, pressed inside the editor: exactly one editor step, nothing else
  await caretToEnd(page, 'n-end')
  await page.keyboard.press('Control+z')
  await expect(undoButton).toHaveText('撤销 1')
  await expect(block(page, 'n-end')).toHaveText('以上为未完成任务的实时视图。第一步')
  await expectAllCommitted(page)
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).not.toContain('第二步')
  expect(await owner('task-40')).toBe('林')

  // redo brings back exactly that step; then two more undos empty the stack
  await page.keyboard.press('Control+Shift+z')
  await expect(block(page, 'n-end')).toHaveText('以上为未完成任务的实时视图。第一步第二步')
  await expect(undoButton).toHaveText('撤销 2')
  await page.keyboard.press('Control+z')
  await page.keyboard.press('Control+z')
  await expect(undoButton).toHaveText('撤销 0')
  await expect(block(page, 'n-end')).toHaveText('以上为未完成任务的实时视图。')
  await expectAllCommitted(page)
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).not.toContain('第一步')

  // redo of a commit = undo of its compensating commit
  await page.getByTestId('aiws-redo').click() // editor step 1
  await page.getByTestId('aiws-redo').click() // editor step 2
  await page.getByTestId('aiws-redo').click() // cell task-40
  await expect.poll(() => owner('task-40')).toBe('撤销甲')
  expect(await owner('task-41')).toBe('王')

  // a commit somebody else changed afterwards cannot be undone silently: the conflict is shown, nothing changes
  const cell = await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')
  expect((await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'tasks', values: [{ record_id: 'task-40', field_id: 'owner', value: '后来者', expect: { rev: cell.rev } }] }])).status).toBe('accepted')
  await expect(all.getByTestId('aiws-cell-task-40-owner')).toContainText('后来者')
  await page.getByTestId('aiws-undo').click()
  await expect(page.getByTestId('aiws-undo-problem')).toContainText('REVISION_CONFLICT')
  expect(await owner('task-40')).toBe('后来者')
})

test('(f) Mock run: candidate, apply, simulated labels, and undo of the whole application', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `f ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-cell-task-41-risk')).toBeVisible()
  await page.getByTestId('aiws-side-mock').click()
  await expect(page.getByTestId('aiws-mock-simulated')).toHaveText('模拟结果')
  await page.getByTestId('aiws-mock-today').fill('2026-10-13')
  await page.getByTestId('aiws-mock-start').click()
  const run = page.getByTestId('aiws-mock-run')
  await expect(run).toHaveAttribute('data-state', 'waiting_confirmation')
  await expect(page.getByTestId('aiws-mock-prepare')).toContainText('将影响 5 处')
  await expect(page.getByTestId('aiws-mock-prepare')).toContainText('summary')
  // a candidate is not an applied state
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(6)
  await expect(page.getByTestId('aiws-outline-item')).toHaveCount(13)

  await page.getByTestId('aiws-mock-apply').click()
  await expect(run).toHaveAttribute('data-state', 'succeeded')
  await expect(page.getByTestId('aiws-mock-result')).toContainText('模拟结果')
  // risk column filled, each written cell marked as simulated
  await expect(all.getByTestId('aiws-cell-task-41-risk')).toContainText('高')
  await expect(all.getByTestId('aiws-cell-task-42-risk')).toContainText('中')
  await expect(all.getByTestId('aiws-cell-task-44-risk')).toContainText('低')
  await expect(all.getByTestId('aiws-cell-task-41-risk').getByTestId('aiws-derived')).toHaveText('模拟')
  // the summary rich text and its cell appear, labelled as simulated
  const summary = page.getByTestId('aiws-cell-frame-cell-summary')
  await expect(summary).toContainText('任务摘要（模拟生成）')
  await expect(summary).toContainText('共 5 项任务，未完成 4 项，逾期 1 项。')
  await expect(page.getByTestId('aiws-outline-item')).toHaveCount(15)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'risk')).value).toBe('option-high')

  // one application is one commit: one undo removes the summary, its cell and every risk value
  await expect(page.getByTestId('aiws-undo')).toHaveText('撤销 1')
  await page.getByTestId('aiws-undo').click()
  await expect(page.getByTestId('aiws-cell-frame-cell-summary')).toHaveCount(0)
  await expect(page.getByTestId('aiws-outline-item')).toHaveCount(13)
  await expect(all.getByTestId('aiws-cell-task-41-risk').getByTestId('aiws-derived')).toHaveCount(0)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'risk')).is_set).toBe(false)
  // redo = undo of the compensating commit: everything is back
  await page.getByTestId('aiws-redo').click()
  await expect(page.getByTestId('aiws-cell-frame-cell-summary')).toContainText('任务摘要（模拟生成）')
  await expect(all.getByTestId('aiws-cell-task-41-risk')).toContainText('高')

  // a human edit on a derived cell is marked as a manual override
  await all.getByTestId('aiws-cell-task-42-risk').getByRole('button').first().click()
  await page.getByLabel('风险 task-42', { exact: true }).selectOption('option-low')
  await expect(all.getByTestId('aiws-cell-task-42-risk').getByTestId('aiws-manual-override')).toBeVisible()
  await expectAllCommitted(page)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-42', 'risk')).meta.manual_override).toBe(true)

  // the same input again: nothing to change, and the UI says so instead of offering an empty application
  await page.getByTestId('aiws-mock-start').click()
  await expect(run).toHaveAttribute('data-state', 'failed')
  await expect(page.getByTestId('aiws-mock-failed')).toContainText('nothing to change')
  await expect(page.getByTestId('aiws-mock-apply')).toHaveCount(0)

  // another "today" gives a new candidate; the manually overridden cell is skipped with a warning; a cancelled run cannot be applied
  await page.getByTestId('aiws-mock-today').fill('2026-12-15')
  await page.getByTestId('aiws-mock-start').click()
  await expect(run).toHaveAttribute('data-state', 'waiting_confirmation')
  await expect(page.getByTestId('aiws-mock-warnings')).toContainText('task-42')
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await page.getByTestId('aiws-mock-cancel').click()
  await expect(run).toHaveAttribute('data-state', 'cancelled')
  await expect(page.getByTestId('aiws-mock-apply')).toHaveCount(0)
  await expect(page.getByTestId('aiws-mock-message')).toContainText('已取消')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
})
