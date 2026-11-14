import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import type { Page, TestInfo } from '@playwright/test'
import type { Result, WorkspaceInfo, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { chooseFromDevice, createSample, expect, expectAllCommitted, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>

async function imageHash(page: Page) {
  const image = page.getByTestId('aiws-asset-image')
  await expect(image).toBeVisible()
  await expect.poll(() => image.evaluate((element: HTMLImageElement) => element.naturalWidth)).toBe(16)
  const bytes = await image.evaluate(async (element: HTMLImageElement) => Array.from(new Uint8Array(await (await fetch(element.src)).arrayBuffer())))
  return createHash('sha256').update(Buffer.from(bytes)).digest('hex')
}

async function exportShare(actor: Actor, ws: Workspace, info: TestInfo, name: string) {
  await actor.page.goto('/workspace')
  await actor.page.getByTestId('aiws-home-tab-all').click()
  const card = actor.page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${ws.workspace_id}"]`)
  await expect(card).toBeVisible()
  // the card's ⋯ → 导出… (UI framework PRD §9.2); downloaded to this device
  await card.getByTestId('aiws-home-card-more').click()
  await card.getByTestId('aiws-home-export').click()
  const dialog = actor.page.getByTestId('aiws-export-dialog')
  await dialog.getByLabel('导出方式', { exact: true }).selectOption('share')
  await dialog.getByLabel('自包含', { exact: true }).check()
  const [download] = await Promise.all([actor.page.waitForEvent('download'), dialog.getByTestId('aiws-export').click()])
  const path = info.outputPath(name)
  await download.saveAs(path)
  await expect(dialog.getByTestId('aiws-export-manifest')).toContainText('分享包')
  await expect(dialog.getByTestId('aiws-export-manifest')).not.toContainText('缺失')
  expect(readFileSync(path).length).toBeGreaterThan(0)
  return path
}

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
    // the home's Import: the system file picker first, then what the package becomes (§9.3)
    await chooseFromDevice(peer.page, () => peer.page.getByTestId('aiws-home-import').click(), sharePath)
    const form = peer.page.getByTestId('aiws-import-dialog').getByTestId('aiws-import')
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
