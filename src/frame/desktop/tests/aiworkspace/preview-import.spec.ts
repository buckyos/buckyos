/* "Import to AI Workspace" from Preview's Open with… (preview component optimization TODO T9): a file opened in the
 * Preview App is handed, as its original (name, media type, bytes), to the AI Workspace window, where the canvas shown
 * takes it through the same steps as a dropped file. A PDF asks (a file Block, or pages) and lands in the workspace on
 * the real backend; an Office document shown in Preview as a converted page is still handed over as the DOCX it is;
 * a file no importer accepts is not offered; a hand-over can be dropped before any page takes it. Screenshots go to
 * test-results/aiworkspace-preview-import/. */

import type { Page } from '@playwright/test'
import { backToList, openCanvas, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-preview-import'

async function canvasWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title })
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-p-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: '画布', system: 'surface_content', surface_id: 'sf-p' } },
    { op: 'entity.create', entity_id: 'sf-p', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: '画布', content_folder_id: 'sf-p-content' } },
  ])
  return ws as { workspace_id: string; epoch: string }
}

/** Open a file of the mock Files' Documents in the Preview App and its Open with… sheet. */
async function openWithFromPreview(page: Page, name: RegExp) {
  // the AI Workspace window is maximised over the desktop icons: out of the way, the import brings it back
  const workspaceWindow = page.getByTestId('window-aiworkspace')
  if (await workspaceWindow.isVisible()) await workspaceWindow.getByRole('button', { name: /^(Minimize|最小化)$/ }).click()
  await page.getByTestId('desktop-app-files').click()
  const files = page.getByTestId('window-files')
  await expect(files).toBeVisible()
  await files.locator('aside').getByRole('button', { name: /Documents|文档/ }).first().click()
  await files.getByRole('cell', { name }).first().dblclick()
  const preview = page.getByTestId('window-preview').getByTestId('content-preview')
  await expect(preview).toHaveAttribute('data-status', /ready|error/, { timeout: 20_000 })
  await preview.hover()
  const more = page.getByTestId('window-preview').getByTestId('content-preview-open-with')
  if (await more.isVisible()) await more.click()
  else {
    await page.getByTestId('window-preview').getByTestId('content-preview-more').click()
    await page.getByTestId('content-preview-menu-open-with').click()
  }
  const sheet = page.getByTestId('preview-open-with')
  await expect(sheet).toBeVisible()
  return sheet
}

test('PV-01 a PDF opened in Preview is imported into the workspace shown, after the ask step', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, 'Preview 导入')
  await openCanvas(page, ALICE, ws.workspace_id)
  const before = (await api.outline(ALICE, ws.workspace_id)).filter((e: { kind?: string; deleted?: boolean }) => !e.deleted).length

  const sheet = await openWithFromPreview(page, /^Friend Contract Draft\.pdf/)
  await page.screenshot({ path: `${SHOTS}/pv01-open-with.png` })
  await sheet.getByTestId('preview-open-with-import-aiws').click()
  await expect(sheet).toBeHidden()
  // The AI Workspace window comes up; its canvas takes the file and asks, as for a dropped PDF.
  await expect(page.getByTestId('aiws-pdf-ask')).toBeVisible({ timeout: 20_000 })
  await expect(page.getByTestId('aiws-external-import')).toHaveCount(0)
  await page.screenshot({ path: `${SHOTS}/pv01-ask.png` })
  await page.getByTestId('aiws-pdf-ask-file').click()
  await expect.poll(async () => {
    const outline = await api.outline(ALICE, ws.workspace_id)
    return outline.filter((e: { deleted?: boolean }) => !e.deleted).length
  }, { timeout: 30_000 }).toBeGreaterThan(before)
  const outline = await api.outline(ALICE, ws.workspace_id)
  expect(JSON.stringify(outline)).toContain('Friend Contract Draft.pdf')
})

test('PV-02 a DOCX shown converted is handed over as the DOCX; a Markdown file is not offered; a hand-over can be dropped', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, 'Preview 导入 Office')
  await openCanvas(page, ALICE, ws.workspace_id)

  const markdown = await openWithFromPreview(page, /^Kyoto Trip Plan\.md/)
  await expect(markdown.getByTestId('preview-open-with-import-aiws')).toHaveCount(0)
  await markdown.getByRole('button', { name: /Close|关闭/ }).click()

  // The mock Files' DOCX is shown by Preview as a converted page; what goes over is the original: its name and its
  // bytes, a zip container (a placeholder without Office parts, so the canvas keeps it as a file, as for a drop; the
  // Workspace records the type it sniffs), never the HTML page Preview showed.
  const docx = await openWithFromPreview(page, /^2026 Q1 Review\.docx/)
  await docx.getByTestId('preview-open-with-import-aiws').click()
  await expect.poll(async () => JSON.stringify(await api.outline(ALICE, ws.workspace_id)), { timeout: 30_000 }).toContain('2026 Q1 Review.docx')
  const stored = JSON.stringify(await api.outline(ALICE, ws.workspace_id))
  expect(stored).toMatch(/"media_type":"(application\/zip|application\/vnd\.openxmlformats-officedocument\.wordprocessingml\.document)","meta_rev":\d+,"name":"2026 Q1 Review\.docx"/)
  expect(stored).not.toContain('text/html')
  await page.screenshot({ path: `${SHOTS}/pv02-docx-as-file.png` })

  // With no workspace open the file waits, says for what, and can be dropped.
  await page.getByTestId('window-drag-aiworkspace').click()
  await backToList(page)
  const pdf = await openWithFromPreview(page, /^Friend Contract Draft\.pdf/)
  await pdf.getByTestId('preview-open-with-import-aiws').click()
  const waiting = page.getByTestId('aiws-external-import')
  await expect(waiting).toBeVisible({ timeout: 20_000 })
  await expect(waiting).toContainText('Friend Contract Draft.pdf')
  await page.screenshot({ path: `${SHOTS}/pv02-waiting.png` })
  await waiting.getByTestId('aiws-external-import-cancel').click()
  await expect(waiting).toHaveCount(0)
})
