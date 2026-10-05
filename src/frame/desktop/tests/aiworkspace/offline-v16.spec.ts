import { readFileSync } from 'node:fs'
import type { Page } from '@playwright/test'
import { expect, editCell, openCard, prepareOffline, saveSummary, test } from './offline-fixtures'

const ALICE = 'tok-alice'

// `window.__aiwsTestHooks` is declared in collab.spec.ts (dev override only): `replica` injects storage faults into the Worker.

async function exported(page: Page, testId: string) {
  const [download] = await Promise.all([page.waitForEvent('download'), page.getByTestId(testId).first().click()])
  return JSON.parse(readFileSync(await download.path(), 'utf8')) as {
    unsaved: { edit_id: string; label: string; operations: { op: string; [key: string]: unknown }[]; mine: unknown; reason: string }[]
    replica: { pending: { idempotency_key: string; state: string; request: { operations: unknown[] } }[]; drafts: unknown[] } | null
  }
}

test('V16 a second tab is not a second writer: online direct, read-only without the backend, takes over only after the holder is gone', async ({ page, context, api, net }) => {
  const ws = await api.sample(ALICE, `v16-tabs ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)

  const second = await context.newPage()
  await second.goto(`${net.origin}/?scenario=normal`)
  await second.getByTestId('desktop-app-aiworkspace').click()
  await openCard(second, ws.workspace_id)
  // the lock is taken: this window does not open the replica at all
  await expect(second.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'direct')
  await expect(second.getByTestId('aiws-mode')).toHaveAttribute('data-reason', 'not_holder')
  await expect(second.getByTestId('aiws-mode')).toContainText('此窗口未启用离线')
  const locks = await second.evaluate(async (name) => (await navigator.locks.query()).held?.filter((lock) => lock.name === name).length, `aiworkspace-replica:${ws.workspace_id}`)
  expect(locks).toBe(1)
  // online it edits directly through the backend; the holder sees the change through the change stream
  await expect(second.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  const direct = await editCell(second, 'task-41', '负责人', 'owner', '第二窗口直连')
  await expect(direct).toContainText('第二窗口直连')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'owner')).value).toBe('第二窗口直连')
  await expect(page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-41-owner')).toContainText('第二窗口直连')
  // asking to take over while the holder is alive changes nothing
  const sessionBefore = await second.getByTestId('aiws-workspace').getAttribute('data-session-id') as string
  await second.getByTestId('aiws-takeover').click()
  await expect(second.getByTestId('aiws-workspace')).not.toHaveAttribute('data-session-id', sessionBefore, { timeout: 30_000 })
  await expect(second.getByTestId('aiws-mode')).toHaveAttribute('data-reason', 'not_holder')
  await expect(second.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(second.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-41-owner')).toContainText('第二窗口直连')

  // ---- the backend becomes unreachable
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  await expect(second.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  // the non-holder is read-only and says why
  await expect(second.getByTestId('aiws-direct-readonly')).toContainText('只读')
  const blocked = second.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-40-owner')
  await expect(blocked).toHaveAttribute('data-editable', 'false')
  await expect(blocked.getByRole('button').first()).toBeDisabled()
  // the holder keeps working on its replica
  const held = await editCell(page, 'task-40', '负责人', 'owner', '持有者离线修改')
  await expect(held).toHaveAttribute('data-state', 'saved_locally')
  expect((await saveSummary(page)).pending).toBe(1)

  // ---- the holder closes: the lock is released and the other window may take over on the user's action (still offline)
  await page.close()
  await second.getByTestId('aiws-takeover').click()
  await expect(second.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica', { timeout: 30_000 })
  const adopted = second.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-40-owner')
  await expect(adopted).toContainText('持有者离线修改')
  await expect(adopted).toHaveAttribute('data-state', 'saved_locally')
  expect((await saveSummary(second)).pending).toBe(1)
  const head = await api.headSeq(ALICE, ws.workspace_id)

  await net.up()
  await expect.poll(async () => (await saveSummary(second)).pending, { timeout: 40_000 }).toBe(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('持有者离线修改')
})

test('V16 storage failure (quota / transaction error): never "saved" without a durable row; the input stays exportable', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `v16-quota ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })

  // a first edit is stored normally
  const durable = await editCell(page, 'task-41', '负责人', 'owner', '已落盘的修改')
  await expect(durable).toHaveAttribute('data-state', 'saved_locally')

  // from now on every transaction of the replica database fails (as a full disk would make it)
  await page.evaluate(() => window.__aiwsTestHooks?.replica?.failTransactions('quota', 1000))
  const cell = await editCell(page, 'task-40', '负责人', 'owner', '写满时的输入')
  await expect(cell).toHaveAttribute('data-state', 'unsaved')
  await expect(cell.getByTestId('aiws-edit-state')).toHaveText('未保存')
  await expect(page.getByTestId('aiws-storage-problem')).toContainText('配额')
  const end = page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]')
  await end.click()
  await page.keyboard.press('End')
  await page.keyboard.type(' 写满时的文字')
  const rich = page.locator('[data-testid="aiws-edit-entry"][data-edit-id="rt:notes"]')
  await expect(rich).toHaveAttribute('data-state', 'unsaved')
  await expect(rich).toContainText('没有保存')
  const summary = await saveSummary(page)
  expect([summary.unsaved, summary.local, summary.pending]).toEqual([2, 1, 1])

  // what the database really holds: the one durable row; the failed inputs are exportable from memory
  const file = await exported(page, 'aiws-export-unsaved')
  expect(file.replica?.pending.map((row) => row.state)).toEqual(['queued'])
  expect(JSON.stringify(file.replica?.pending[0].request)).toContain('已落盘的修改')
  expect(JSON.stringify(file.replica)).not.toContain('写满时的输入')
  const table = file.unsaved.find((item) => item.edit_id.includes('task-40'))
  expect(JSON.stringify(table?.operations)).toContain('写满时的输入')
  expect(table?.reason).toContain('配额')
  expect(JSON.stringify(file.unsaved.find((item) => item.edit_id === 'rt:notes')?.mine)).toContain('写满时的文字')

  // storage works again: the editor's retry stores the text, and that is when it becomes "saved"
  await page.evaluate(() => window.__aiwsTestHooks?.replica?.failTransactions('quota', 0))
  await expect(rich).toHaveAttribute('data-state', 'saved_locally', { timeout: 20_000 })
  await expect(page.getByTestId('aiws-storage-problem')).toHaveCount(0)
  // the table input was never stored: it is still unsaved, and a reload proves nothing pretended otherwise
  await expect(cell).toHaveAttribute('data-state', 'unsaved')
  await page.reload()
  await page.getByTestId('desktop-app-aiworkspace').click()
  await openCard(page, ws.workspace_id)
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-cell-task-41-owner')).toContainText('已落盘的修改')
  await expect(all.getByTestId('aiws-cell-task-40-owner')).toContainText('林')
  await expect(all.getByTestId('aiws-cell-task-40-owner')).not.toContainText('写满时的输入')
  await expect(page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]')).toContainText('写满时的文字')

  const head = await api.headSeq(ALICE, ws.workspace_id)
  await net.up()
  await expect.poll(async () => (await saveSummary(page)).pending, { timeout: 40_000 }).toBe(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('林')
})

test('V16 the Replica Worker dies: later edits stay unsaved and exportable; reopening finds exactly what was durable', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `v16-worker ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  const durable = await editCell(page, 'task-41', '负责人', 'owner', 'Worker 退出前')
  await expect(durable).toHaveAttribute('data-state', 'saved_locally')

  await page.evaluate(() => window.__aiwsTestHooks?.replica?.killWorker())
  const cell = await editCell(page, 'task-40', '负责人', 'owner', 'Worker 退出后')
  // while the Worker does not answer the edit is unsaved — it never shows a saved state
  await expect(cell).toHaveAttribute('data-state', 'unsaved')
  await expect(page.getByTestId('aiws-storage-problem')).toContainText('Replica Worker', { timeout: 30_000 })
  await expect(cell).toHaveAttribute('data-state', 'unsaved')
  await expect(cell).toContainText('Worker 退出后')
  const file = await exported(page, 'aiws-export-unsaved')
  expect(JSON.stringify(file.unsaved)).toContain('Worker 退出后')
  expect(file.replica).toHaveProperty('error')

  // a fresh Worker on the same database: the durable row is there, the lost one is not
  await page.getByTestId('aiws-reopen').click()
  await expect(page.getByTestId('aiws-storage-problem')).toHaveCount(0, { timeout: 30_000 })
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica')
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-cell-task-41-owner')).toContainText('Worker 退出前')
  await expect(all.getByTestId('aiws-cell-task-40-owner')).toContainText('林')
  expect((await saveSummary(page)).pending).toBe(1)
})

