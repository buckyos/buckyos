import { readFileSync } from 'node:fs'
import { expect, editCell, prepareOffline, saveSummary, test } from './offline-fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

test('V17 reconnect after others deleted a record and changed the same cell: both inputs need attention, nothing is applied twice, the record stays deleted', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `v17-conflict ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update', 'delete'] })
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })

  // alice, offline
  await expect(await editCell(page, 'task-44', '预算', 'budget', '20')).toHaveAttribute('data-state', 'saved_locally')
  await expect(await editCell(page, 'task-41', '负责人', 'owner', '给已删记录的修改')).toHaveAttribute('data-state', 'saved_locally')
  await expect(await editCell(page, 'task-42', '负责人', 'owner', '不受影响的修改')).toHaveAttribute('data-state', 'saved_locally')
  expect((await saveSummary(page)).pending).toBe(3)

  // bob, online meanwhile: (iii) the same cell, (ii) deletes a record alice edited
  const budget = await api.cell(BOB, ws.workspace_id, 'tasks', 'task-44', 'budget')
  expect((await api.commit(BOB, ws, [{ op: 'table.set_values', source_id: 'tasks', values: [{ record_id: 'task-44', field_id: 'budget', value: '99', expect: { rev: budget.rev } }] }])).status).toBe('accepted')
  const record = (await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks', selector: { kind: 'table_record', record_id: 'task-41' } })).content
  expect((await api.commit(BOB, ws, [{ op: 'table.delete_records', source_id: 'tasks', records: [{ record_id: 'task-41', expect: { rev: record.rev } }] }])).status).toBe('accepted')
  const head = await api.headSeq(ALICE, ws.workspace_id)

  await net.up()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live', { timeout: 40_000 })
  await expect.poll(async () => (await saveSummary(page)).attention, { timeout: 30_000 }).toBe(2)
  await expect.poll(async () => (await saveSummary(page)).local).toBe(0)
  // the independent edit went through exactly once
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-42', 'owner')).value).toBe('不受影响的修改')
  // the two refused inputs: in 需要处理, readable, with the reason
  const conflict = page.locator('[data-testid="aiws-edit-entry"][data-code="REVISION_CONFLICT"]')
  await expect(conflict).toHaveAttribute('data-state', 'needs_attention')
  await expect(conflict.getByTestId('aiws-edit-mine')).toHaveText('20')
  const deleted = page.locator('[data-testid="aiws-edit-entry"][data-code="TARGET_DELETED"]')
  await expect(deleted).toHaveAttribute('data-state', 'needs_attention')
  await expect(deleted.getByTestId('aiws-edit-mine')).toHaveText('给已删记录的修改')
  // the working view shows what the backend has, not my refused input
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-row')).toHaveCount(4)
  await expect(all.getByTestId('aiws-conflict-theirs')).toHaveText('99.00')
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-44', 'budget')).value).toBe('99.00')
  // the deleted record was not resurrected
  const gone = await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks', selector: { kind: 'table_record', record_id: 'task-41' } })
  expect(gone.ok).toBe(false)
  // both are still in the replica database (they survive a restart) until I decide
  expect((await saveSummary(page)).pending).toBe(2)

  // I decide: drop the edit of the deleted record, re-apply my budget on top of the current value
  await deleted.getByTestId('aiws-edit-dismiss').click()
  await expect(deleted).toHaveCount(0)
  await all.getByTestId('aiws-conflict-keep-mine').click()
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-44', 'budget')).value, { timeout: 30_000 }).toBe('20.00')
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  const after = await saveSummary(page)
  expect([after.unsaved, after.local, after.attention]).toEqual([0, 0, 0])
})

test('V17 permission revoked while offline: the old right is not honoured, the input is kept and readable; later it can be sent again', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `v17-revoke ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update'] })
  await prepareOffline(page, net, BOB, ws.workspace_id)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  const cell = await editCell(page, 'task-43', '负责人', 'owner', 'bob 离线写的')
  await expect(cell).toHaveAttribute('data-state', 'saved_locally')
  const end = page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]')
  await end.click()
  await page.keyboard.press('End')
  await page.keyboard.type(' bob 离线写的文字')
  const rich = page.locator('[data-testid="aiws-edit-entry"][data-edit-id="rt:notes"]')
  await expect(rich).toHaveAttribute('data-state', 'saved_locally')
  expect((await saveSummary(page)).pending).toBe(2)

  // (i) alice takes the write permission away
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await net.up()
  await expect.poll(async () => (await saveSummary(page)).attention, { timeout: 40_000 }).toBe(2)
  // the rich text: refused as well, the editor is back on the confirmed content, my text is kept as a draft
  await expect(rich).toHaveAttribute('data-state', 'needs_attention')
  await expect(rich).toHaveAttribute('data-code', 'PERMISSION_DENIED')
  await expect(page.getByTestId('aiws-richtext-notes')).not.toContainText('bob 离线写的文字')
  await expect(page.getByTestId('aiws-drafts')).toContainText('已保留为草稿（1）')
  await expect(rich.getByTestId('aiws-edit-mine')).toContainText('bob 离线写的文字')
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).not.toContain('bob 离线写的文字')
  const entry = page.locator('[data-testid="aiws-edit-entry"][data-code="PERMISSION_DENIED"][data-edit-id^="cell:"]')
  await expect(entry).toHaveAttribute('data-state', 'needs_attention')
  await expect(entry.getByTestId('aiws-edit-mine')).toHaveText('bob 离线写的')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-43', 'owner')).value).toBe('赵')
  // not shown as accepted: the cell carries my input marked 需要处理 and is no longer editable (the right is gone); the row is still stored
  const refused = page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-43-owner')
  await expect(refused).toHaveAttribute('data-state', 'needs_attention')
  await expect(refused.getByTestId('aiws-edit-state')).toHaveText('需要处理')
  await expect(refused).toHaveAttribute('data-editable', 'false')
  expect((await saveSummary(page)).pending).toBe(2)

  // the permission comes back: the kept inputs are sent again unchanged, exactly once each
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update'] })
  await entry.getByTestId('aiws-edit-requeue').click()
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-43', 'owner')).value, { timeout: 30_000 }).toBe('bob 离线写的')
  await rich.getByTestId('aiws-edit-requeue').click()
  await expect.poll(async () => JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes')).split('bob 离线写的文字').length - 1, { timeout: 30_000 }).toBe(1)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(0)
  // the accepted update comes back through the change stream into the editor
  await expect(page.getByTestId('aiws-richtext-notes')).toContainText('bob 离线写的文字')
})

test('V17 access removed entirely while offline: sending stops, everything is kept and exportable', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `v17-noaccess ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read', 'update'] })
  await prepareOffline(page, net, BOB, ws.workspace_id)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  await expect(await editCell(page, 'task-43', '负责人', 'owner', '无权后仍保留')).toHaveAttribute('data-state', 'saved_locally')
  await api.rpc(ALICE, 'ws.revoke', { workspace_id: ws.workspace_id, subject: 'bob' })
  const head = await api.headSeq(ALICE, ws.workspace_id)
  await net.up()
  await expect(page.getByTestId('aiws-stopped')).toContainText('已停止发送', { timeout: 40_000 })
  await expect(page.locator('[data-testid="aiws-edit-entry"][data-state="needs_attention"]').getByTestId('aiws-edit-mine')).toHaveText('无权后仍保留')
  const [download] = await Promise.all([page.waitForEvent('download'), page.getByTestId('aiws-export-pending').click()])
  const file = JSON.parse(readFileSync(await download.path(), 'utf8')) as { replica: { pending: { request: unknown }[] } }
  expect(file.replica.pending).toHaveLength(1)
  expect(JSON.stringify(file.replica.pending[0].request)).toContain('无权后仍保留')
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
})

test('V17 an unknown result is settled by key: response dropped after acceptance, and request lost before arrival — applied exactly once each', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `v17-unknown ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  const seen: { key: string; mode: string }[] = []
  let mode: 'pass' | 'drop-response' | 'lose-request' = 'drop-response'
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postDataJSON() as { method: string; params: { idempotency_key?: string } }
    if (body.method !== 'doc.commit' || mode === 'pass') { await route.continue(); return }
    seen.push({ key: body.params.idempotency_key ?? '', mode })
    if (mode === 'drop-response') {
      mode = 'pass'
      await route.fetch() // the backend accepts it …
      await route.abort('connectionreset') // … and the answer never arrives
    } else {
      mode = 'pass'
      await route.abort('connectionrefused') // never reached the backend
    }
  })

  // 1. accepted, response lost
  const first = await editCell(page, 'task-40', '负责人', 'owner', '应答丢失')
  await expect.poll(async () => (await saveSummary(page)).pending, { timeout: 40_000 }).toBe(0)
  expect(seen).toHaveLength(1)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('应答丢失')
  await expect(first).toContainText('应答丢失')

  // 2. request lost: doc.get_submission says not_found, the same key is sent again
  mode = 'lose-request'
  await editCell(page, 'task-41', '负责人', 'owner', '请求丢失')
  await expect.poll(async () => (await saveSummary(page)).pending, { timeout: 40_000 }).toBe(0)
  expect(seen).toHaveLength(2)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-41', 'owner')).value).toBe('请求丢失')
  const changes = await api.rpc(ALICE, 'doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq: head })
  expect((changes.changes as { idempotency_key: string }[]).map((change) => change.idempotency_key)).toEqual(seen.map((item) => item.key))
  const summary = await saveSummary(page)
  expect([summary.unsaved, summary.local, summary.attention]).toEqual([0, 0, 0])
})

test('the history is replaced while offline (EPOCH_MISMATCH): sending stops, pending inputs are kept and exportable, the replica can be dropped', async ({ page, api, net }) => {
  const ws = await api.sample(ALICE, `epoch ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await net.down()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  await expect(await editCell(page, 'task-40', '负责人', 'owner', '旧历史上的修改')).toHaveAttribute('data-state', 'saved_locally')

  // the workspace is restored from a backup: a new epoch
  const exportResult = await api.rpc(ALICE, 'doc.export', { workspace_id: ws.workspace_id, mode: 'personal_backup', self_contained: true })
  const pkg = await (await fetch(`${api.base}/export/${ws.workspace_id}/${exportResult.export_id}`, { headers: { authorization: `Bearer ${ALICE}` } })).arrayBuffer()
  const begin = await api.rpc(ALICE, 'ws.begin_import', {})
  expect((await fetch(`${api.base}/upload/${begin.upload_id}`, { method: 'PUT', headers: { authorization: `Bearer ${ALICE}` }, body: pkg })).ok).toBe(true)
  const restored = await api.rpc(ALICE, 'ws.import', { upload_id: begin.upload_id, semantics: 'restore', replace: true })
  expect(restored.epoch).not.toBe(ws.epoch)
  const head = await api.headSeq(ALICE, ws.workspace_id)

  await net.up()
  await expect(page.getByTestId('aiws-stopped')).toContainText('历史已被恢复操作替换', { timeout: 40_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'stopped')
  // nothing was sent onto the new history, nothing was dropped
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  expect((await api.cell(ALICE, ws.workspace_id, 'tasks', 'task-40', 'owner')).value).toBe('林')
  expect((await saveSummary(page)).pending).toBe(1)
  await expect(page.locator('[data-testid="aiws-edit-entry"][data-code="EPOCH_MISMATCH"]').getByTestId('aiws-edit-mine')).toHaveText('旧历史上的修改')
  const [download] = await Promise.all([page.waitForEvent('download'), page.getByTestId('aiws-export-pending').click()])
  expect(readFileSync(await download.path(), 'utf8')).toContain('旧历史上的修改')

  // the user confirms: the replica is removed and the window is back in online direct mode on the new history
  await page.getByTestId('aiws-destroy').click()
  await page.getByTestId('aiws-destroy-confirm').click()
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'direct', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-reason', 'not_prepared')
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-40-owner')).toContainText('林')
})
