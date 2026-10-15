import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import type { Page, TestInfo } from '@playwright/test'
import type { EntityEnvelope, FreshnessInfo, Result, RichTextContent, WorkspaceInfo, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { createSample, createTemplate, expect, expectAllCommitted, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>

async function content<T>(actor: Actor, ws: Workspace, entity_id: string) {
  return (await actor.client.ok<{ content: T }>('doc.read', { workspace_id: ws.workspace_id, entity_id })).content
}

async function dataView(page: Page, entityId: string, parents: string[] = []) {
  const sources = page.getByTestId('aiws-top-sources')
  if (await sources.isVisible()) await sources.click()
  for (const parent of parents) {
    const item = page.locator(`[data-testid="aiws-tree-item"][data-entity-id="${parent}"]`)
    if (await item.getAttribute('aria-expanded') === 'false') await item.getByRole('button', { name: '展开', exact: true }).first().click()
  }
  const detail = page.getByTestId(`aiws-detail-${entityId}`)
  if (!await detail.isVisible()) await page.getByTestId(`aiws-tree-${entityId}`).click()
  await expect(detail).toBeVisible()
  return detail
}

async function wishPanel(page: Page) {
  await dataView(page, 'wish-analysis', ['canvas-content', 'sf-analysis-content'])
  const panel = page.getByTestId('aiws-wish-wish-analysis')
  await expect(panel.getByTestId('aiws-wish-executor')).toHaveValue('mock')
  return panel
}

async function generated(actor: Actor, ws: Workspace) {
  const { entities } = await actor.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id })
  const results = entities.filter((entity) => entity.derived?.wish_id === 'wish-analysis')
  expect(results).toHaveLength(4)
  expect(results.every((entity) => entity.derived?.simulated === true && entity.derived?.executor === 'mock')).toBe(true)
  const kpi = results.find((entity) => entity.type_id === 'buckyos.record')
  const summary = results.find((entity) => entity.type_id === 'buckyos.richtext')
  if (!kpi || !summary) throw new Error('applied analysis is missing its KPI or summary')
  return { kpi, summary, results }
}

async function imageHash(page: Page) {
  const image = page.getByTestId('aiws-asset-image')
  await expect(image).toBeVisible()
  await expect.poll(() => image.evaluate((element: HTMLImageElement) => element.naturalWidth)).toBe(16)
  const bytes = await image.evaluate(async (element: HTMLImageElement) => Array.from(new Uint8Array(await (await fetch(element.src)).arrayBuffer())))
  return createHash('sha256').update(Buffer.from(bytes)).digest('hex')
}

async function exportShare(actor: Actor, ws: Workspace, info: TestInfo, name: string) {
  await actor.page.goto('/workspace')
  const card = actor.page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${ws.workspace_id}"]`)
  await expect(card).toBeVisible()
  await card.getByLabel('导出方式', { exact: true }).selectOption('share')
  await card.getByLabel('自包含', { exact: true }).check()
  const [download] = await Promise.all([actor.page.waitForEvent('download'), card.getByTestId('aiws-export').click()])
  const path = info.outputPath(name)
  await download.saveAs(path)
  await expect(card.getByTestId('aiws-export-manifest')).toContainText('分享包')
  await expect(card.getByTestId('aiws-export-manifest')).not.toContainText('缺失')
  expect(readFileSync(path).length).toBeGreaterThan(0)
  return path
}

test('[journey-sales] quarterly analysis previews, applies and refreshes after a source correction', async ({ owner }, info) => {
  const ws = await createTemplate(owner, 'quarterly')
  const page = owner.page
  const panel = await wishPanel(page)
  const initial = (await owner.client.info(ws)).head_seq
  await panel.getByTestId('aiws-wish-analyze').click()
  await expect(panel.getByTestId('aiws-wish-analysis')).toContainText('模拟执行器')
  await expect(panel.getByTestId('aiws-wish-input')).toHaveCount(1)
  await expect(panel.getByTestId('aiws-wish-input')).toContainText('原始销售数据')
  const analysed = (await owner.client.info(ws)).head_seq
  expect(analysed).toBe(initial + 1)

  await panel.getByTestId('aiws-wish-execute').click()
  await expect(panel.getByTestId('aiws-wish-candidate')).toHaveAttribute('data-ready', 'true')
  await expect(panel.getByTestId('aiws-wish-candidate')).toContainText('模拟')
  await expect(panel.getByTestId('aiws-wish-result')).toHaveCount(4)
  expect((await owner.client.info(ws)).head_seq).toBe(analysed)
  await screenshot(page, info, 'sales-candidate')
  await panel.getByTestId('aiws-wish-apply').click()
  await expect(panel.getByTestId('aiws-wish-message')).toContainText('已应用')
  await expectAllCommitted(page)
  expect((await owner.client.info(ws)).head_seq).toBe(analysed + 1)
  const first = await generated(owner, ws)
  expect((await content<{ props: { revenue: string } }>(owner, ws, first.kpi.entity_id)).props.revenue).toBe('770000.00')
  expect(JSON.stringify(await content<RichTextContent>(owner, ws, first.summary.entity_id))).toMatch(/770,?000/)
  await expect(panel.getByTestId('aiws-freshness-wish-analysis')).toHaveAttribute('data-status', 'current')

  const sales = await dataView(page, 'sales')
  await sales.getByTestId('aiws-cell-s-1-revenue').getByRole('button').first().click()
  const input = page.getByLabel('销售额 s-1', { exact: true })
  await input.fill('138000')
  await input.press('Enter')
  await expectAllCommitted(page)
  const corrected = await owner.client.ok<{ content: { value: number } }>('doc.read', {
    workspace_id: ws.workspace_id, entity_id: 'sales',
    selector: { kind: 'table_cell', record_id: 's-1', field_id: 'revenue' },
  })
  expect(corrected.content.value).toBe(138000)
  const freshness = await owner.client.ok<{ items: FreshnessInfo[] }>('doc.freshness', { workspace_id: ws.workspace_id, entity_ids: ['wish-analysis'] })
  expect(freshness.items).toHaveLength(1)
  expect(freshness.items[0].status).toBe('stale')
  await info.attach('sales-source-freshness', { body: Buffer.from(JSON.stringify(freshness.items[0])), contentType: 'application/json' })
  await wishPanel(page)
  await expect.soft(panel.getByTestId('aiws-freshness-wish-analysis'), 'the wish must show the source correction without reloading').toHaveAttribute('data-status', 'stale')
  await screenshot(page, info, 'sales-source-corrected')
  const beforeRefresh = (await owner.client.info(ws)).head_seq
  await panel.getByTestId('aiws-wish-execute').click()
  await expect(panel.getByTestId('aiws-wish-candidate')).toHaveAttribute('data-ready', 'true')
  expect((await owner.client.info(ws)).head_seq).toBe(beforeRefresh)
  expect((await content<{ props: { revenue: string } }>(owner, ws, first.kpi.entity_id)).props.revenue).toBe('770000.00')
  await panel.getByTestId('aiws-wish-apply').click()
  await expect(panel.getByTestId('aiws-wish-message')).toContainText('已应用')
  await expectAllCommitted(page)
  const refreshed = await generated(owner, ws)
  expect(refreshed.results.map((entity) => entity.entity_id).sort()).toEqual(first.results.map((entity) => entity.entity_id).sort())
  expect((await content<{ props: { revenue: string } }>(owner, ws, refreshed.kpi.entity_id)).props.revenue).toBe('780000.00')
  expect(JSON.stringify(await content<RichTextContent>(owner, ws, refreshed.summary.entity_id))).toMatch(/780,?000/)
  await page.reload()
  await wishPanel(page)
  await expect(panel.getByTestId('aiws-freshness-wish-analysis')).toHaveAttribute('data-status', 'current')
  await screenshot(page, info, 'sales-refreshed')
  await info.attach('sales-verdict', {
    body: Buffer.from(JSON.stringify({ executor: 'mock', simulated: true, original_total: 770000, corrected_total: 780000, source_record: 's-1', source_before: 128000, source_after: 138000 })),
    contentType: 'application/json',
  })
})

test('[journey-handoff] an author exports a short report and an isolated user imports, edits and re-exports it', async ({ owner, peer }, info) => {
  const ws = await createSample(owner)
  const page = owner.page
  const authorNote = '【作者核对】本报告的任务状态已逐项检查，图片作为原始资料随包交付。'
  await page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]').click()
  await page.keyboard.press('End')
  await page.keyboard.insertText(authorNote)
  const table = page.getByTestId('aiws-table-cell-all-tasks')
  await table.getByTestId('aiws-cell-task-41-title').getByRole('button').first().click()
  await page.getByLabel('任务 task-41', { exact: true }).fill('交接前已核对')
  await page.getByLabel('任务 task-41', { exact: true }).press('Enter')
  await expectAllCommitted(page)
  await expect.poll(async () => JSON.stringify(await owner.client.ast(ws))).toContain(authorNote)
  const authorAst = await owner.client.ast(ws)
  const authorImage = await imageHash(page)
  const authorRoot = await owner.client.ok<{ content_root: string }>('doc.checkpoint', { workspace_id: ws.workspace_id })
  await screenshot(page, info, 'handoff-author')
  const sharePath = await exportShare(owner, ws, info, 'author-share.zip')
  expect(await peer.client.call<Result<WorkspaceInfo>>('ws.get_info', { workspace_id: ws.workspace_id })).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })

  try {
    await peer.page.goto('/workspace')
    const form = peer.page.getByTestId('aiws-import')
    await form.getByTestId('aiws-import-file').setInputFiles(sharePath)
    await expect(form.getByTestId('aiws-import-submit')).toBeDisabled()
    await form.getByLabel('作为新的工作区', { exact: false }).check()
    const [response] = await Promise.all([peer.page.waitForResponse((reply) => {
      if (new URL(reply.url()).pathname.replace(/\/+$/, '') !== '/kapi/aiworkspace' || reply.request().method() !== 'POST') return false
      return reply.request().postDataJSON()?.method === 'ws.import'
    }), form.getByTestId('aiws-import-submit').click()])
    const body = await response.json() as { result: Result<Workspace> }
    if (!body.result.ok) throw new Error(`import rejected: ${body.result.error.code}`)
    const imported = body.result
    expect(imported.workspace_id).not.toBe(ws.workspace_id)
    await expect(peer.page.getByTestId('aiws-list-message')).toContainText('已导入为新的工作区')
    const importedRoot = await peer.client.ok<{ content_root: string }>('doc.checkpoint', { workspace_id: imported.workspace_id })
    expect(importedRoot.content_root).toBe(authorRoot.content_root)
    expect(await peer.client.ast(imported)).toEqual(authorAst)
    expect((await peer.client.cell(imported, 'task-41', 'title')).value).toBe('交接前已核对')
    await openWorkspace(peer, imported)
    await expect(peer.page.getByTestId('aiws-richtext-notes')).toContainText(authorNote)
    expect(await imageHash(peer.page)).toBe(authorImage)
    const receivedNote = '【接收者修订】已独立打开并继续修改。'
    await peer.page.getByTestId('aiws-richtext-notes').locator('[data-block-id="n-end"]').click()
    await peer.page.keyboard.press('End')
    await peer.page.keyboard.insertText(receivedNote)
    await expectAllCommitted(peer.page)
    await expect.poll(async () => JSON.stringify(await peer.client.ast(imported))).toContain(receivedNote)
    await peer.page.reload()
    await expect(peer.page.getByTestId('aiws-richtext-notes')).toContainText(receivedNote)
    expect(await owner.client.ast(ws)).toEqual(authorAst)
    expect((await owner.client.ok<{ content_root: string }>('doc.checkpoint', { workspace_id: ws.workspace_id })).content_root).toBe(authorRoot.content_root)
    await screenshot(peer.page, info, 'handoff-receiver')
    const revisedPath = await exportShare(peer, imported, info, 'receiver-share.zip')
    expect(createHash('sha256').update(readFileSync(revisedPath)).digest('hex')).not.toBe(createHash('sha256').update(readFileSync(sharePath)).digest('hex'))
    await info.attach('handoff-verdict', {
      body: Buffer.from(JSON.stringify({ source: ws.workspace_id, imported: imported.workspace_id, initial_content_root: authorRoot.content_root, asset_sha256: authorImage, source_unchanged: true, receiver_edit: receivedNote })),
      contentType: 'application/json',
    })
  } finally {
    const copies = (await peer.client.list()).workspaces.filter((copy) => copy.title === owner.client.prefix)
    for (const copy of copies) await peer.client.ok('ws.delete', { workspace_id: copy.workspace_id })
    const remaining = (await peer.client.list()).workspaces.filter((copy) => copy.title === owner.client.prefix)
    await info.attach('handoff-cleanup', { body: Buffer.from(JSON.stringify({ deleted: copies.map((copy) => copy.workspace_id), remaining })), contentType: 'application/json' })
    expect(remaining).toEqual([])
  }
})
